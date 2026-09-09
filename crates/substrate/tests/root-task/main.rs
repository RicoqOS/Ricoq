#![no_std]
#![no_main]

//! Kernel-backed isolated-task integration test.

use core::arch::global_asm;
use core::ptr;

use sel4_root_task::root_task;
use substrate_sel4::{
    Bootstrap, BootstrapError, DelegatedCapability, Frame, SharedCode,
    TaskConfig,
};

const TASK_CODE_ADDRESS: usize = 0x0100_0000;
const PRIVATE_ADDRESS: usize = 0x0200_0000;
const STACK_ADDRESS: usize = 0x0200_1000;
const IPC_BUFFER_ADDRESS: usize = 0x0200_2000;
const TASK_ONE_VALUE: usize = 0x1111_1111_1111_1111;
const TASK_TWO_VALUE: usize = 0x2222_2222_2222_2222;
const COMPLETION_SLOT: usize = 1;
const UNDELEGATED_SLOT: sel4::Word = 2;
const PROBE_SLOT: sel4::Word = 3;

// A self-contained page avoids implicitly mapping root-task text into a child.
global_asm!(
    r#"
    .pushsection .task_code,"ax",@progbits
    .global isolated_task_entry
isolated_task_entry:
    str x1, [x0]
    str x1, [x2]
    mov x0, #2
    mov x1, #0
    mov x7, x3
    svc #0
    mov x0, #1
    mov x1, #0
    svc #0
1:
    wfe
    b 1b
    .popsection
"#,
);

#[repr(C, align(4096))]
struct ScratchPage([u8; Frame::BYTES]);

const _: () = {
    assert!(core::mem::align_of::<ScratchPage>() == Frame::BYTES);
    assert!(core::mem::size_of::<ScratchPage>() == Frame::BYTES);
};

#[used]
#[unsafe(link_section = ".task_scratch")]
static mut TASK_SCRATCH: ScratchPage = ScratchPage([0xee; Frame::BYTES]);

/// Builds identical layouts with task-specific capabilities and arguments.
fn task_config<'a>(
    code: &'a Frame,
    capabilities: &'a [DelegatedCapability],
    value: usize,
) -> TaskConfig<'a> {
    TaskConfig {
        cspace_size_bits: 4,
        code: SharedCode {
            frame: code,
            address: TASK_CODE_ADDRESS,
            entry: TASK_CODE_ADDRESS,
        },
        private_memory_address: PRIVATE_ADDRESS,
        stack_address: STACK_ADDRESS,
        ipc_buffer_address: IPC_BUFFER_ADDRESS,
        initial_arguments: [
            PRIVATE_ADDRESS,
            value,
            IPC_BUFFER_ADDRESS,
            sel4::sys::syscall_id::Send as usize,
        ],
        delegated_capabilities: capabilities,
    }
}

/// Reads an owner frame through the test's exclusive scratch mapping.
fn read_frame_word(
    frame: &Frame,
    scratch: *mut usize,
) -> Result<usize, BootstrapError> {
    // SAFETY: the reserved scratch page is unmapped and exclusively controlled
    // by this single-threaded root-task test.
    unsafe { frame.map(scratch as usize)? };
    // SAFETY: the frame mapping covers one aligned word at `scratch` and stays
    // live until the following unmap.
    let value = unsafe { scratch.read_volatile() };
    // SAFETY: `value` ended the only access to the temporary mapping.
    unsafe { frame.unmap()? };
    Ok(value)
}

/// Constructs, runs, and inspects two isolated protection domains.
fn exercise(bootinfo: &sel4::BootInfo) -> Result<(), BootstrapError> {
    sel4::debug_println!("substrate: booting");
    let mut bootstrap = Bootstrap::new(bootinfo)?;

    // SAFETY: the linker script defines these symbols and the ELF verifier
    // checks their page alignment, size, permissions, and loaded-image range.
    unsafe extern "C" {
        static __root_image_start: u8;
        static __task_code_start: u8;
        static __task_code_end: u8;
        static mut __task_scratch_start: [u8; Frame::BYTES];
        static __task_scratch_end: u8;
    }
    let image_start = ptr::addr_of!(__root_image_start) as usize;
    let code_start = ptr::addr_of!(__task_code_start) as usize;
    let code_end = ptr::addr_of!(__task_code_end) as usize;
    let scratch = ptr::addr_of_mut!(__task_scratch_start).cast::<usize>();
    let scratch_end = ptr::addr_of!(__task_scratch_end) as usize;
    assert_eq!(code_end.checked_sub(code_start), Some(Frame::BYTES));
    assert_eq!(
        scratch_end.checked_sub(scratch as usize),
        Some(Frame::BYTES)
    );

    // SAFETY: the linker and ELF checks prove the loaded image base and the
    // dedicated task-code page boundaries.
    let code = unsafe { bootstrap.image_frame(image_start, code_start)? };
    // SAFETY: the linker and ELF checks prove that scratch is one exclusively
    // controlled image page.
    let scratch_image =
        unsafe { bootstrap.image_frame(image_start, scratch as usize)? };
    // SAFETY: no test data is accessed through scratch while it is unmapped.
    unsafe { scratch_image.unmap()? };

    let completion_one = bootstrap.allocate_notification()?;
    let completion_two = bootstrap.allocate_notification()?;
    let first_task_slot = bootinfo
        .empty()
        .range()
        .start
        .checked_add(2)
        .ok_or(BootstrapError::InvalidTaskConfiguration)?;
    let invalid_source = first_task_slot
        .checked_add(1)
        .and_then(|slot| slot.try_into().ok())
        .ok_or(BootstrapError::InvalidTaskConfiguration)?;
    let invalid_capabilities = [DelegatedCapability::new(
        sel4::cap::Notification::from_bits(invalid_source),
        COMPLETION_SLOT,
        sel4::CapRights::write_only(),
        0,
    )];
    assert_eq!(
        bootstrap
            .create_task(task_config(&code, &invalid_capabilities, 0))
            .err(),
        Some(BootstrapError::CSpaceCreationFailed),
    );

    let capabilities_one = [DelegatedCapability::notification(
        &completion_one,
        COMPLETION_SLOT,
    )];
    let capabilities_two = [DelegatedCapability::notification(
        &completion_two,
        COMPLETION_SLOT,
    )];
    let task_one = bootstrap.create_task(task_config(
        &code,
        &capabilities_one,
        TASK_ONE_VALUE,
    ))?;
    let task_two = bootstrap.create_task(task_config(
        &code,
        &capabilities_two,
        TASK_TWO_VALUE,
    ))?;
    assert_eq!(task_one.root_slots().start, first_task_slot);
    sel4::debug_println!("task: resources constructed");

    assert_ne!(task_one.cspace().bits(), task_two.cspace().bits());
    assert_ne!(task_one.vspace().bits(), task_two.vspace().bits());
    sel4::debug_println!("task: cspaces and vspaces isolated");

    let child_cspace = task_one.cspace();
    let absent = child_cspace
        .absolute_cptr_from_bits_with_depth(PROBE_SLOT, 4)
        .copy(
            &child_cspace
                .absolute_cptr_from_bits_with_depth(UNDELEGATED_SLOT, 4),
            sel4::CapRights::all(),
        );
    assert_eq!(absent, Err(sel4::Error::FailedLookup));
    sel4::debug_println!("task: capability isolation verified");

    task_one.start()?;
    task_two.start()?;
    completion_one.wait();
    completion_two.wait();
    task_one.suspend()?;
    task_two.suspend()?;
    sel4::debug_println!("task: independent execution verified");

    let private_one = read_frame_word(task_one.private_memory(), scratch)?;
    let private_two = read_frame_word(task_two.private_memory(), scratch)?;
    assert_eq!(private_one, TASK_ONE_VALUE);
    assert_eq!(private_two, TASK_TWO_VALUE);
    assert_ne!(private_one, private_two);
    sel4::debug_println!("task: private memory isolation verified");

    let ipc_one = read_frame_word(task_one.ipc_buffer(), scratch)?;
    let ipc_two = read_frame_word(task_two.ipc_buffer(), scratch)?;
    assert_eq!(ipc_one, TASK_ONE_VALUE);
    assert_eq!(ipc_two, TASK_TWO_VALUE);
    sel4::debug_println!("task: per-task IPC buffers verified");

    // SAFETY: all temporary mappings are gone and the original scratch frame
    // remains exclusively owned by this test.
    unsafe { scratch_image.map(scratch as usize)? };
    sel4::debug_println!("TEST_RESULT: PASS");
    Ok(())
}

#[root_task]
/// Reports the deterministic result before suspending the root task.
fn main(bootinfo: &sel4::BootInfoPtr) -> ! {
    if let Err(error) = exercise(bootinfo) {
        sel4::debug_println!("test: substrate operation failed: {error:?}");
        sel4::debug_println!("TEST_RESULT: FAIL");
    }
    sel4::init_thread::suspend_self()
}
