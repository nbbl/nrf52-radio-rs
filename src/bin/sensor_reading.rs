//! Provide a (mocked) sensor reading via BLE:
//! Advertise a GATT service of a battery level.
//! Copied and adapted from the `trouble` crate
//! (examples/apps/src/ble_bas_peripheral.rs).

#![no_std]
#![no_main]

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_futures::{join::join, select::select};
use embassy_nrf::{
    bind_interrupts, peripherals,
    uarte::{self, Baudrate, Config, Parity, Uarte, UarteRxWithIdle},
};
use nmea::ParseResult::{self, GGA};
use nrf_mpsl::MultiprotocolServiceLayer;
use nrf_sdc::SoftdeviceController;
use nrf52_radio_rs::Board;
use trouble_host::prelude::*;

/// Max number of connections
const CONNECTIONS_MAX: usize = 1;

/// Max number of L2CAP channels.
const L2CAP_CHANNELS_MAX: usize = 2; // Signal + att

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

#[gatt_service(uuid = service::LOCATION_AND_NAVIGATION)]
struct GnssService {
    #[characteristic(uuid = characteristic::CURRENT_TIME, read, notify)]
    utc_time: [u8; 6],
}

/// Run the BLE stack.
pub async fn run_ble(
    mut peri: Peripheral<'_, SoftdeviceController<'_>, DefaultPacketPool>,
    gnss_uarte: &mut UarteRxWithIdle<'_>,
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
                    let gnss = gnss_task(&server, &conn, gnss_uarte);
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

fn send_nmea_msg(parse_result: ParseResult) {
    match parse_result {
        GGA(gps_fix) => {
            let gps_fix_str = defmt::Debug2Format(&gps_fix);
            info!("[send_nmea_msg] received GPS fix: {}", gps_fix_str)
        }
        _ => {
            warn!("[send_nmea_msg] unexpected NMEA sentence received .")
        }
    }
}

async fn gnss_task<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
    gnss_uarte: &mut UarteRxWithIdle<'_>,
) {
    // TODO: Put GNSS module into standby until it is actually needed?
    // (when BLE connection is established.)

    // NMEA 0183 messages have a max length of 82 chars:
    let mut nmea_buf = [0u8; 82];
    let mut buf_idx: usize = 0;
    loop {
        match gnss_uarte
            .read_until_idle(&mut nmea_buf[buf_idx..buf_idx + 1])
            .await
        {
            Ok(rx_len) => {
                let nmea_sentence_terminated =
                    buf_idx > 0 && nmea_buf[buf_idx - 1..buf_idx + 1] == *"\r\n".as_bytes();
                if rx_len == 0 || nmea_sentence_terminated {
                    // let parsed = nmea::parse_bytes(&nmea_buf[..buf_idx + 1]);
                    info!(
                        "[gnss_task] received NMEA sentence: {}",
                        str::from_utf8(&nmea_buf[..buf_idx + 1]).unwrap_or("UTF8 error"),
                    );
                    buf_idx = 0;
                    // match parsed {
                    //     Ok(valid) => {
                    //         send_nmea_msg(valid);
                    //     }
                    //     Err(_) => {
                    //         info!("[gnss_task] invalid NMEA sentence received.");
                    //     }
                    // }
                } else {
                    buf_idx += 1;
                    continue;
                }
            }
            Err(e) => {
                warn!("[gnss_task] error receiving bytes: {:?}", e);
                break;
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
    let (_uarte_tx, mut uarte_rx) =
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
        run_ble(peripheral, &mut uarte_rx),
    )
    .await;
    panic!("[main] ble_background_task and run_ble terminated");
}
