MEMORY
{
  /* Need to leave space for the SoftDevice 
    These values are confirmed working for S140 7.3.0
  */
  FLASH (rx)     : ORIGIN = 0x27000, LENGTH = 0x100000 - 0x27000

  /* SRAM required by Softdevice depend on
   * - Attribute Table Size (Number of Services and Characteristics)
   * - Vendor UUID count
   * - Max ATT MTU
   * - Concurrent connection peripheral + central + secure links
   * - Event Len, HVN queue, Write CMD queue
   *
   *  TODO: How to set SRAM optimally for this specific board?
   */ 
  RAM (rwx) :  ORIGIN = 0x20000000 , LENGTH = 0x20040000 - 0x20000000
}