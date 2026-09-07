#![no_std]
#![no_main]

use sel4_root_task::root_task;

#[root_task]
fn main(_bootinfo: &sel4::BootInfoPtr) -> ! {
    sel4::debug_println!("BOOT_TEST: START");
    sel4::debug_println!("TEST_RESULT: PASS");
    sel4::init_thread::suspend_self()
}
