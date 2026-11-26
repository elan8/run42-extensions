//! STM32F407 firmware that mirrors input PA0 to output PD12.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};

const RCC_AHB1ENR: *mut u32 = 0x4002_3830 as *mut u32;
const GPIOA_IDR: *const u32 = 0x4002_0010 as *const u32;
const GPIOD_MODER: *mut u32 = 0x4002_0c00 as *mut u32;
const GPIOD_BSRR: *mut u32 = 0x4002_0c18 as *mut u32;

#[no_mangle]
extern "C" fn Reset() -> ! {
    unsafe {
        write_volatile(RCC_AHB1ENR, read_volatile(RCC_AHB1ENR) | 0b1001);
        write_volatile(GPIOD_MODER, 1 << 24);
        loop {
            let pressed = read_volatile(GPIOA_IDR) & 1 != 0;
            write_volatile(GPIOD_BSRR, 1 << if pressed { 12 } else { 28 });
        }
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
