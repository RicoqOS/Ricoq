#![no_std]
#![no_main]

use core::ptr;

use sel4_root_task::root_task;
use substrate_api::BootstrapError;
use substrate_sel4::{Bootstrap, Frame};

const TEST_VALUE: u64 = 0x0123_4567_89ab_cdef;

#[repr(C, align(4096))]
struct TestPage([u8; Frame::BYTES]);

const _: () = {
    assert!(core::mem::align_of::<TestPage>() == Frame::BYTES);
    assert!(core::mem::size_of::<TestPage>() == Frame::BYTES);
};
#[used]
#[unsafe(link_section = ".vspace_test")]
static mut TEST_PAGE: TestPage = TestPage([0xee; Frame::BYTES]);

fn exercise(bootinfo: &sel4::BootInfo) -> Result<(), BootstrapError> {
    sel4::debug_println!("substrate: booting");
    let mut bootstrap = Bootstrap::new(bootinfo)?;
    sel4::debug_println!("substrate: cspace ready");
    let notification = bootstrap.allocate_notification()?;
    sel4::debug_println!("substrate: untyped ready");
    notification.signal();
    notification.wait();
    sel4::debug_println!("substrate: notification allocated");
    let frame = bootstrap.allocate_frame()?;
    sel4::debug_println!("vspace: frame allocated");

    unsafe extern "C" {
        static __root_image_start: u8;
        static mut __vspace_test_start: [u8; Frame::BYTES];
        static __vspace_test_end: u8;
    }
    // Rust can assume different static declarations do not alias; address the
    // reservation only through its linker symbol at runtime.
    let address = ptr::addr_of_mut!(__vspace_test_start).cast::<u64>();
    let image_start = ptr::addr_of!(__root_image_start) as usize;
    assert_eq!(
        (ptr::addr_of!(__vspace_test_end) as usize)
            .checked_sub(address as usize),
        Some(Frame::BYTES),
    );
    let original =
        unsafe { bootstrap.image_frame(image_start, address as usize)? };
    let original_value = unsafe { address.read_volatile() };
    unsafe { original.unmap()? };
    let mapping = unsafe { frame.map(address as usize) };
    let observed = if mapping.is_ok() {
        sel4::debug_println!("vspace: frame mapped");
        let value = unsafe {
            address.write_volatile(TEST_VALUE);
            address.read_volatile()
        };
        if value == TEST_VALUE {
            sel4::debug_println!("vspace: memory verified");
        }
        unsafe { frame.unmap()? };
        sel4::debug_println!("vspace: frame unmapped");
        Some(value)
    } else {
        None
    };
    unsafe { original.map(address as usize)? };
    assert_eq!(unsafe { address.read_volatile() }, original_value);
    sel4::debug_println!("vspace: image restored");
    mapping?;
    assert_eq!(observed, Some(TEST_VALUE));
    Ok(())
}

#[root_task]
fn main(bootinfo: &sel4::BootInfoPtr) -> ! {
    match exercise(bootinfo) {
        Ok(()) => sel4::debug_println!("TEST_RESULT: PASS"),
        Err(error) => {
            sel4::debug_println!(
                "test: substrate operation failed: {error:?}"
            );
            sel4::debug_println!("TEST_RESULT: FAIL");
        },
    }
    sel4::init_thread::suspend_self()
}
