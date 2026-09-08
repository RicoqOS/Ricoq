#![no_std]
#![no_main]

use sel4_root_task::root_task;

#[root_task]
fn main(bootinfo: &sel4::BootInfoPtr) -> ! {
    substrate_sel4::run(bootinfo)
}
