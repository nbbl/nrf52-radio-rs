//! Firmware for the Wio Tracker L1.
//! Provides current time read from the L76K GNSS module
//! as a BLE GATT service.
//! Based on an example from the `trouble` crate
//! (examples/apps/src/ble_bas_peripheral.rs).

#![no_std]
#![no_main]

use bytemuck::{Pod, Zeroable, checked::try_cast};
use chrono::{Datelike, NaiveDateTime, Timelike};
use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_futures::{join::join, select::select};
use embassy_nrf::{
    bind_interrupts, peripherals,
    uarte::{self, Baudrate, Config, Parity, Uarte, UarteRxWithIdle, UarteTx},
};
use nmea::ParseResult::{self, ZDA};
use nrf_mpsl::MultiprotocolServiceLayer;
use nrf_sdc::SoftdeviceController;
use nrf52_radio_rs::Board;
use trouble_host::prelude::*;

/// Max number of connections
const CONNECTIONS_MAX: usize = 1;

/// Max number of L2CAP channels.
const L2CAP_CHANNELS_MAX: usize = 2; // Signal + att

/// PCAS message (proprietary NMEA message) to configure
/// the receiver to start searching for satellites (GPS and BeiDou).
const ENABLE_GNSS_MODULE: &[u8; 14] = b"$PCAS04,3*1A\r\n";

// GATT Server definition
#[gatt_server]
struct Server {
    battery_service: BatteryService,
    gnss_service: GnssService,
}

/// Battery service
#[gatt_service(uuid = service::BATTERY)]
struct BatteryService {
    /// Battery Level
    #[descriptor(uuid = descriptors::VALID_RANGE, read, value = [0, 100])]
    #[descriptor(uuid = descriptors::MEASUREMENT_DESCRIPTION, name = "hello", read, value = "Battery Level")]
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify, value = 10)]
    level: u8,
    #[characteristic(uuid = "408813df-5dd4-1f87-ec11-cdb001100000", write, read, notify)]
    status: bool,
}

/// Current Time characteristic of BLE
/// see GATT specification supplement (2023-12-23)
/// section 3.71
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct CurrentTime {
    year: u16,
    month: u8,
    day: u8,
    hours: u8,
    minutes: u8,
    seconds: u8,
    day_of_week: u8,
    fractions_256: u8,
    adj_reason: u8,
}

impl CurrentTime {
    pub fn from(date_time: &NaiveDateTime) -> Self {
        let date = date_time.date();
        let time = date_time.time();
        CurrentTime {
            year: date.year() as u16,
            month: date.month() as u8,
            day: date.day() as u8,
            hours: time.hour() as u8,
            minutes: time.minute() as u8,
            seconds: time.second() as u8,
            day_of_week: 0u8,
            fractions_256: 0u8,
            adj_reason: 0u8,
        }
    }

    pub fn to_bytes(self) -> [u8; 10] {
        try_cast(self).expect("CurrentTime should always have a representation of size 10 bytes")
    }
}

/// GNSS service
#[gatt_service(uuid = service::LOCATION_AND_NAVIGATION)]
struct GnssService {
    #[characteristic(uuid = characteristic::CURRENT_TIME, read, notify)]
    time: [u8; 10],
}

/// Run the BLE stack.
pub async fn run_ble(
    mut peri: Peripheral<'_, SoftdeviceController<'_>, DefaultPacketPool>,
    gnss_uarte_rx: &mut UarteRxWithIdle<'_>,
    gnss_uarte_tx: &mut UarteTx<'_>,
) {
    info!("[adv] start advertising and GATT service");
    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: "TrouBLE",
        appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
    }))
    .unwrap();

    let _ = async {
        loop {
            match advertise("Trouble Example", &mut peri, &server).await {
                Ok(conn) => {
                    // set up tasks when the connection is established to a central, so they don't run when no one is connected.
                    let gatt = gatt_events_task(&server, &conn);
                    let gnss = gnss_notify_task(&server, &conn, gnss_uarte_rx, gnss_uarte_tx);
                    let _ = select(gatt, gnss).await;
                }
                Err(e) => {
                    let e = defmt::Debug2Format(&e);
                    panic!("[adv] error: {:?}", e);
                }
            }
        }
    }
    .await;
}

/// This is a background task that is required to run forever alongside any other BLE tasks.
async fn ble_background_task(mut runner: Runner<'_, SoftdeviceController<'_>, DefaultPacketPool>) {
    loop {
        if let Err(e) = runner.run().await {
            let e = defmt::Debug2Format(&e);
            panic!("[ble_background_task] error: {:?}", e);
        }
    }
}
async fn send_nmea_msg<P: PacketPool>(
    gnss_service: &GnssService,
    conn: &GattConnection<'_, '_, P>,
    parse_result: ParseResult,
) {
    match parse_result {
        // TODO: Send messages for other NMEA sentences, e.g.:
        // GGA(gga_data) => {
        //     let gps_fix_str = defmt::Debug2Format(&gga_data);
        //     info!("[send_nmea_msg] received GPS fix: {}", gps_fix_str)
        //     ...
        // }
        ZDA(zda_data) => {
            let maybe_utc_dt = zda_data.utc_date_time();
            info!(
                "[send_nmea_msg] current UTC time: {}",
                defmt::Debug2Format(&maybe_utc_dt)
            );
            if let Some(dt) = maybe_utc_dt {
                let _ = gnss_service
                    .time
                    .notify(conn, &CurrentTime::from(&dt).to_bytes())
                    .await;
            }
        }
        _ => {
            warn!("[send_nmea_msg] unexpected NMEA sentence received")
        }
    }
}

async fn gnss_notify_task<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
    gnss_uarte_rx: &mut UarteRxWithIdle<'_>,
    gnss_uarte_tx: &mut UarteTx<'_>,
) {
    // TODO: Put GNSS module into standby until it is actually needed?
    // (when BLE connection is established.)

    // TODO: Necessary to send ENABLE_GNSS_MODULE?
    // TODO: Implement retrying of GNSS enabling?
    if let Err(err) = gnss_uarte_tx.write(ENABLE_GNSS_MODULE).await {
        panic!("[main] couldn't enable GNSS module: {:?} error", err);
    };

    // NMEA 0183 messages have a max length of 82 chars:
    let mut nmea_buf = [0u8; 82];
    let mut buf_idx: usize = 0;
    loop {
        match gnss_uarte_rx
            .read_until_idle(&mut nmea_buf[buf_idx..buf_idx + 1])
            .await
        {
            Ok(rx_len) => {
                // TODO: Handle case when buf_idx out of bounds.
                let nmea_sentence_terminated =
                    buf_idx > 0 && nmea_buf[buf_idx - 1..buf_idx + 1] == *"\r\n".as_bytes();
                if rx_len == 0 || nmea_sentence_terminated {
                    let parsed = nmea::parse_bytes(&nmea_buf[..buf_idx + 1]);
                    info!(
                        "[gnss_notify_task] received NMEA sentence: {}",
                        str::from_utf8(&nmea_buf[..buf_idx + 1]).unwrap_or("UTF8 error"),
                    );
                    buf_idx = 0;
                    if let Ok(valid_nmea) = parsed {
                        send_nmea_msg(&server.gnss_service, conn, valid_nmea).await;
                    }
                } else {
                    buf_idx += 1;
                    continue;
                }
            }
            Err(e) => {
                warn!("[gnss_notify_task] error receiving bytes: {:?} error", e);
                buf_idx = 0;
                continue;
            }
        };
    }
}

/// Stream Events until the connection closes.
///
/// This function will handle the GATT events and process them.
/// This is how we interact with read and write requests.
async fn gatt_events_task<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
) -> Result<(), Error> {
    let level = server.battery_service.level;
    let reason = loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => break reason,
            GattConnectionEvent::Gatt { event } => {
                match &event {
                    GattEvent::Read(event) => {
                        if event.handle() == level.handle {
                            let value = server.get(&level);
                            info!("[gatt] Read Event to Level Characteristic: {:?}", value);
                        }
                    }
                    GattEvent::Write(event) => {
                        if event.handle() == level.handle {
                            info!(
                                "[gatt] Write Event to Level Characteristic: {:?}",
                                event.data()
                            );
                        }
                    }
                    _ => {}
                };
                // This step is also performed at drop(), but writing it explicitly is necessary
                // in order to ensure reply is sent.
                match event.accept() {
                    Ok(reply) => reply.send().await,
                    Err(e) => warn!("[gatt] error sending response: {:?}", e),
                };
            }
            _ => {} // ignore other Gatt Connection Events
        }
    };
    info!("[gatt] disconnected: {:?}", reason);
    Ok(())
}

/// Create an advertiser to use to connect to a BLE Central, and wait for it to connect.
async fn advertise<'values, 'server, C: Controller>(
    name: &'values str,
    peripheral: &mut Peripheral<'values, C, DefaultPacketPool>,
    server: &'server Server<'values>,
) -> Result<GattConnection<'values, 'server, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut advertiser_data = [0; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::ServiceUuids16(&[[0x0f, 0x18]]),
            AdStructure::CompleteLocalName(name.as_bytes()),
        ],
        &mut advertiser_data[..],
    )?;
    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: &advertiser_data[..len],
                scan_data: &[],
            },
        )
        .await?;
    info!("[adv] advertising");
    let conn = advertiser.accept().await?.with_attribute_server(server)?;
    info!("[adv] connection established");
    Ok(conn)
}

/// Run the multiprotocol service layer task.
///
/// Required even when only a single protocol (BLE in this case)
/// is used.
#[embassy_executor::task]
async fn mpsl_task(mpsl: &'static MultiprotocolServiceLayer<'static>) {
    mpsl.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) -> ! {
    bind_interrupts!(struct Irqs {
        UARTE0 => uarte::InterruptHandler<peripherals::UARTE0>;
    });

    let board = Board::default();
    let (sdc, mpsl) = board.ble.init(board.timer0, board.rng).unwrap();

    let conf = {
        let mut c = Config::default();
        c.baudrate = Baudrate::BAUD9600;
        c.parity = Parity::EXCLUDED;
        c
    };
    let uarte = Uarte::new(board.uarte0, board.p0_26, board.p0_27, Irqs, conf);
    let (mut uarte_tx, mut uarte_rx) =
        uarte.split_with_idle(board.timer1, board.ppi_ch0, board.ppi_ch1);

    spawner.must_spawn(mpsl_task(mpsl));

    // Using a fixed "random" address can be useful for testing. In real scenarios, one would
    // use e.g. the MAC 6 byte array as the address (how to get that varies by the platform).
    let address: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);
    info!("Our address = {:?}", address);

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(sdc, &mut resources).set_random_address(address);
    let Host {
        peripheral, runner, ..
    } = stack.build();
    let _ = join(
        ble_background_task(runner),
        run_ble(peripheral, &mut uarte_rx, &mut uarte_tx),
    )
    .await;
    panic!("[main] ble_background_task and run_ble terminated");
}
