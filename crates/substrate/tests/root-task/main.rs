//! Kernel-backed isolated-task integration test.
#![no_std]
#![no_main]

use core::arch::global_asm;
use core::ptr;

use sel4_root_task::root_task;
use substrate_sel4::{
    Badge, BadgeAllocator, Bootstrap, BootstrapError, DelegatedCapability,
    EXTRA_CAPACITY, Fault, FaultBadge, FaultBadgeError, FaultRoute, Frame,
    IpcError, MESSAGE_CAPACITY, Message, SharedCode, TaskConfig,
};

const TASK_CODE_ADDRESS: usize = 0x0100_0000;
const PRIVATE_ADDRESS: usize = 0x0200_0000;
const STACK_ADDRESS: usize = 0x0200_1000;
const IPC_BUFFER_ADDRESS: usize = 0x0200_2000;
const MISSING_PAGE_ADDRESS: usize = 0x0200_3000;
const TASK_ONE_VALUE: usize = 0x1111_1111_1111_1111;
const TASK_TWO_VALUE: usize = 0x2222_2222_2222_2222;
const VM_INITIAL_VALUE: usize = 0x3333_3333_3333_3333;
const VM_RECOVERED_VALUE: usize = 0x4444_4444_4444_4444;
const COMPLETION_SLOT: usize = 1;
const IPC_ENDPOINT_SLOT: usize = 2;
const TRANSFER_SOURCE_SLOT: usize = 3;
const TRANSFER_DESTINATION_SLOT: usize = 2;
const FAULT_SLOT: usize = 3;
const VM_FAULT_BADGE: sel4::Word = 0x41;
const UNKNOWN_SYSCALL_BADGE: sel4::Word = 0x42;
const UNKNOWN_SYSCALL_NUMBER: sel4::Word = 0x123;
const MISSING_PAGES: [usize; 1] = [MISSING_PAGE_ADDRESS];
const UNDELEGATED_SLOT: sel4::Word = 2;
const PROBE_SLOT: sel4::Word = 3;
const BASIC_BADGE: sel4::Word = 0x51;
const DEFERRED_A_BADGE: sel4::Word = 0x52;
const DEFERRED_B_BADGE: sel4::Word = 0x53;
const TRANSFER_BADGE: sel4::Word = 0x54;

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

    .global vm_fault_task_entry
vm_fault_task_entry:
    str x1, [x0]
    mov x0, #1
    mov x1, #0
    mov x7, x3
    svc #0
2:
    wfe
    b 2b

    .global unknown_syscall_task_entry
unknown_syscall_task_entry:
    mov x0, #0x11
    mov x1, #0x12
    mov x2, #0x13
    mov x3, #0x14
    mov x4, #0x15
    mov x5, #0x16
    mov x6, #0x17
    mov x7, #0x123
    .global unknown_syscall_instruction
unknown_syscall_instruction:
    svc #0
3:
    wfe
    b 3b

    .global ipc_client_entry
ipc_client_entry:
    mov x4, x0
    mov x2, x1
    mov x0, #2
    mov x1, #1
    mov x7, #{call_syscall}
    svc #0
    str x2, [x4]
    mov x0, #1
    mov x1, #0
    mov x7, #{send_syscall}
    svc #0
4:
    wfe
    b 4b

    .global capability_sender_entry
capability_sender_entry:
    mov x0, #2
    mov x1, #128
    mov x7, #{send_syscall}
    svc #0
5:
    wfe
    b 5b

    .global capability_receiver_entry
capability_receiver_entry:
    mov x0, #2
    mov x1, #0
    mov x7, #{send_syscall}
    svc #0
6:
    wfe
    b 6b
    .popsection
"#,
    call_syscall = const sel4::sys::syscall_id::Call,
    send_syscall = const sel4::sys::syscall_id::Send,
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
        reserved_pages: &[],
        initial_arguments: [
            PRIVATE_ADDRESS,
            value,
            IPC_BUFFER_ADDRESS,
            sel4::sys::syscall_id::Send as usize,
        ],
        delegated_capabilities: capabilities,
        fault_route: None,
    }
}

/// Reads an owner frame through the test's exclusive scratch mapping.
fn read_frame_word(
    frame: &Frame,
    scratch: *mut usize,
) -> Result<usize, BootstrapError> {
    unsafe { frame.map(scratch as usize)? };
    let value = unsafe { scratch.read_volatile() };
    unsafe { frame.unmap()? };
    Ok(value)
}

/// Installs one sender-side extra-cap slot in an isolated task's IPC buffer.
fn set_transfer_capability(
    frame: &Frame,
    scratch: *mut usize,
    capability: sel4::Word,
) -> Result<(), BootstrapError> {
    unsafe { frame.map(scratch as usize)? };
    // SAFETY: the mapped frame exclusively backs a page-aligned seL4 IPC
    // buffer and remains mapped for this mutation only.
    let buffer = unsafe { &mut *scratch.cast::<sel4::IpcBuffer>() };
    let slot = buffer
        .caps_or_badges_mut()
        .get_mut(0)
        .ok_or(BootstrapError::InvalidTaskConfiguration)?;
    *slot = capability;
    unsafe { frame.unmap()? };
    Ok(())
}

/// Constructs, runs, and inspects two isolated protection domains.
fn exercise(bootinfo: &sel4::BootInfo) -> Result<(), BootstrapError> {
    sel4::debug_println!("substrate: booting");
    let mut bootstrap = Bootstrap::new(bootinfo)?;

    unsafe extern "C" {
        static __root_image_start: u8;
        static __task_code_start: u8;
        static __task_code_end: u8;
        static mut __task_scratch_start: [u8; Frame::BYTES];
        static __task_scratch_end: u8;
        static vm_fault_task_entry: u8;
        static unknown_syscall_task_entry: u8;
        static unknown_syscall_instruction: u8;
        static ipc_client_entry: u8;
        static capability_sender_entry: u8;
        static capability_receiver_entry: u8;
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

    let code = unsafe { bootstrap.image_frame(image_start, code_start)? };
    let scratch_image =
        unsafe { bootstrap.image_frame(image_start, scratch as usize)? };
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

    let fault_endpoint = bootstrap.allocate_fault_endpoint()?;
    let vm_completion = bootstrap.allocate_notification()?;
    assert_eq!(FaultBadge::new(0), Err(FaultBadgeError::ReservedZero));
    let vm_badge = FaultBadge::new(VM_FAULT_BADGE)
        .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
    let unknown_badge = FaultBadge::new(UNKNOWN_SYSCALL_BADGE)
        .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
    assert_ne!(vm_badge, unknown_badge);
    let vm_capabilities = [DelegatedCapability::notification(
        &vm_completion,
        COMPLETION_SLOT,
    )];

    let vm_entry = TASK_CODE_ADDRESS
        .checked_add(
            (ptr::addr_of!(vm_fault_task_entry) as usize)
                .checked_sub(code_start)
                .ok_or(BootstrapError::InvalidTaskConfiguration)?,
        )
        .ok_or(BootstrapError::InvalidTaskConfiguration)?;
    let unknown_entry = TASK_CODE_ADDRESS
        .checked_add(
            (ptr::addr_of!(unknown_syscall_task_entry) as usize)
                .checked_sub(code_start)
                .ok_or(BootstrapError::InvalidTaskConfiguration)?,
        )
        .ok_or(BootstrapError::InvalidTaskConfiguration)?;
    let unknown_instruction = TASK_CODE_ADDRESS
        .checked_add(
            (ptr::addr_of!(unknown_syscall_instruction) as usize)
                .checked_sub(code_start)
                .ok_or(BootstrapError::InvalidTaskConfiguration)?,
        )
        .ok_or(BootstrapError::InvalidTaskConfiguration)?;
    let ipc_entry = TASK_CODE_ADDRESS
        .checked_add(
            (ptr::addr_of!(ipc_client_entry) as usize)
                .checked_sub(code_start)
                .ok_or(BootstrapError::InvalidTaskConfiguration)?,
        )
        .ok_or(BootstrapError::InvalidTaskConfiguration)?;
    let capability_sender = TASK_CODE_ADDRESS
        .checked_add(
            (ptr::addr_of!(capability_sender_entry) as usize)
                .checked_sub(code_start)
                .ok_or(BootstrapError::InvalidTaskConfiguration)?,
        )
        .ok_or(BootstrapError::InvalidTaskConfiguration)?;
    let capability_receiver = TASK_CODE_ADDRESS
        .checked_add(
            (ptr::addr_of!(capability_receiver_entry) as usize)
                .checked_sub(code_start)
                .ok_or(BootstrapError::InvalidTaskConfiguration)?,
        )
        .ok_or(BootstrapError::InvalidTaskConfiguration)?;

    assert_eq!(Badge::new(0), Err(substrate_sel4::BadgeError::ReservedZero));
    assert!(matches!(
        Message::new(0, &[0; MESSAGE_CAPACITY + 1]),
        Err(IpcError::MessageTooLong { .. })
    ));
    assert!(matches!(
        Message::with_capabilities(
            0,
            &[],
            &[sel4::init_thread::slot::CNODE.cap(); EXTRA_CAPACITY + 1],
        ),
        Err(IpcError::TooManyCapabilities { .. })
    ));
    let one_word = Message::new(0, &[1])
        .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
    assert!(matches!(
        one_word.word(1),
        Err(IpcError::InvalidMessageIndex { .. })
    ));

    let endpoint = bootstrap.allocate_endpoint()?;
    let basic_completion = bootstrap.allocate_notification()?;
    let mut ipc_badges = BadgeAllocator::new(BASIC_BADGE)
        .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
    let basic_badge = ipc_badges
        .allocate()
        .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
    assert_eq!(basic_badge.raw(), BASIC_BADGE);
    let basic_capabilities = [
        DelegatedCapability::notification(&basic_completion, COMPLETION_SLOT),
        DelegatedCapability::badged_endpoint(
            &endpoint,
            IPC_ENDPOINT_SLOT,
            basic_badge,
        ),
    ];
    let mut basic_config = task_config(&code, &basic_capabilities, 0);
    basic_config.code.entry = ipc_entry;
    basic_config.initial_arguments = [PRIVATE_ADDRESS, 7, 0, 0];
    let basic_task = bootstrap.create_task(basic_config)?;
    basic_task.start()?;
    let basic_request = endpoint
        .receive(None)
        .map_err(|_| BootstrapError::ThreadControlFailed)?;
    assert_eq!(basic_request.badge(), Some(basic_badge));
    assert_eq!(basic_request.message().words(), &[7]);
    let basic_reply = Message::new(0, &[49])
        .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
    basic_request.reply(&basic_reply);
    basic_completion.wait();
    assert_eq!(read_frame_word(basic_task.private_memory(), scratch)?, 49);
    sel4::debug_println!("ipc: basic request reply verified");

    let completion_a = bootstrap.allocate_notification()?;
    let completion_b = bootstrap.allocate_notification()?;
    let badge_a = ipc_badges
        .allocate()
        .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
    let badge_b = ipc_badges
        .allocate()
        .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
    assert_eq!(badge_a.raw(), DEFERRED_A_BADGE);
    assert_eq!(badge_b.raw(), DEFERRED_B_BADGE);
    let capabilities_a = [
        DelegatedCapability::notification(&completion_a, COMPLETION_SLOT),
        DelegatedCapability::badged_endpoint(
            &endpoint,
            IPC_ENDPOINT_SLOT,
            badge_a,
        ),
    ];
    let capabilities_b = [
        DelegatedCapability::notification(&completion_b, COMPLETION_SLOT),
        DelegatedCapability::badged_endpoint(
            &endpoint,
            IPC_ENDPOINT_SLOT,
            badge_b,
        ),
    ];
    let mut config_a = task_config(&code, &capabilities_a, 0);
    config_a.code.entry = ipc_entry;
    config_a.initial_arguments = [PRIVATE_ADDRESS, 10, 0, 0];
    let mut config_b = task_config(&code, &capabilities_b, 0);
    config_b.code.entry = ipc_entry;
    config_b.initial_arguments = [PRIVATE_ADDRESS, 20, 0, 0];
    let task_a = bootstrap.create_task(config_a)?;
    let task_b = bootstrap.create_task(config_b)?;
    let replies = bootstrap.allocate_reply_pool(2)?;

    task_a.start()?;
    let request_a = endpoint
        .receive(None)
        .map_err(|_| BootstrapError::ThreadControlFailed)?;
    assert_eq!(request_a.badge(), Some(badge_a));
    assert_eq!(request_a.message().words(), &[10]);
    let deferred_a = request_a
        .defer(&replies)
        .map_err(|_| BootstrapError::ThreadControlFailed)?;

    task_b.start()?;
    let request_b = endpoint
        .receive(None)
        .map_err(|_| BootstrapError::ThreadControlFailed)?;
    assert_eq!(request_b.badge(), Some(badge_b));
    assert_eq!(request_b.message().words(), &[20]);
    let reply_b = Message::new(0, &[22])
        .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
    request_b.reply(&reply_b);
    completion_b.wait();
    assert_eq!(read_frame_word(task_b.private_memory(), scratch)?, 22);

    let reply_a = Message::new(0, &[11])
        .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
    deferred_a.reply(&reply_a);
    completion_a.wait();
    assert_eq!(read_frame_word(task_a.private_memory(), scratch)?, 11);
    sel4::debug_println!("ipc: deferred badged replies verified");

    let transferred_notification = bootstrap.allocate_notification()?;
    let transfer_badge = ipc_badges
        .allocate()
        .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
    assert_eq!(transfer_badge.raw(), TRANSFER_BADGE);
    let sender_capabilities = [
        DelegatedCapability::badged_endpoint(
            &endpoint,
            IPC_ENDPOINT_SLOT,
            transfer_badge,
        ),
        DelegatedCapability::notification(
            &transferred_notification,
            TRANSFER_SOURCE_SLOT,
        ),
    ];
    let mut sender_config = task_config(&code, &sender_capabilities, 0);
    sender_config.code.entry = capability_sender;
    sender_config.initial_arguments = [0; 4];
    let sender_task = bootstrap.create_task(sender_config)?;
    set_transfer_capability(
        sender_task.ipc_buffer(),
        scratch,
        TRANSFER_SOURCE_SLOT as sel4::Word,
    )?;

    let mut receiver_config = task_config(&code, &[], 0);
    receiver_config.code.entry = capability_receiver;
    receiver_config.initial_arguments = [0; 4];
    let mut receiver_task = bootstrap.create_task(receiver_config)?;
    assert!(matches!(
        receiver_task.receive_slot(0),
        Err(IpcError::InvalidReceiveSlot)
    ));
    assert!(matches!(
        receiver_task.receive_slot(16),
        Err(IpcError::InvalidReceiveSlot)
    ));
    let receive_slot =
        receiver_task
            .receive_slot(TRANSFER_DESTINATION_SLOT)
            .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
    sender_task.start()?;
    let transfer = endpoint
        .receive(Some(receive_slot))
        .map_err(|_| BootstrapError::ThreadControlFailed)?;
    assert_eq!(transfer.badge(), Some(transfer_badge));
    assert_eq!(transfer.message().extra_caps(), 1);
    transfer.finish();
    assert!(matches!(
        receiver_task.receive_slot(TRANSFER_DESTINATION_SLOT),
        Err(IpcError::InvalidReceiveSlot)
    ));
    receiver_task.start()?;
    transferred_notification.wait();
    sel4::debug_println!("ipc: capability transfer used and tracked");

    let mut vm_config = task_config(&code, &vm_capabilities, VM_INITIAL_VALUE);
    vm_config.code.entry = vm_entry;
    vm_config.reserved_pages = &MISSING_PAGES;
    vm_config.initial_arguments = [
        MISSING_PAGE_ADDRESS,
        VM_INITIAL_VALUE,
        0,
        sel4::sys::syscall_id::Send as usize,
    ];
    vm_config.fault_route =
        Some(FaultRoute::new(&fault_endpoint, vm_badge, FAULT_SLOT));
    let vm_task = bootstrap.create_task(vm_config)?;

    let mut unknown_config = task_config(&code, &[], 0);
    unknown_config.code.entry = unknown_entry;
    unknown_config.fault_route =
        Some(FaultRoute::new(&fault_endpoint, unknown_badge, FAULT_SLOT));
    let unknown_task = bootstrap.create_task(unknown_config)?;

    sel4::debug_println!("fault: routes distinguished");

    vm_task.start()?;
    let vm_received = fault_endpoint
        .receive()
        .map_err(|_| BootstrapError::ThreadControlFailed)?;
    assert_eq!(vm_received.source(), vm_badge);
    let vm_fault = match vm_received.fault() {
        Fault::VmFault(fault) => fault,
        _ => return Err(BootstrapError::ThreadControlFailed),
    };
    assert_eq!(vm_fault.address, MISSING_PAGE_ADDRESS as u64);
    assert_eq!(vm_fault.instruction_pointer, vm_entry as u64);
    assert!(!vm_fault.instruction_fault);
    sel4::debug_println!("fault: VM context decoded");

    let mut registers = vm_task.read_registers()?;
    assert_eq!(
        registers.instruction_pointer(),
        vm_fault.instruction_pointer
    );
    assert_eq!(registers.general_register(0)?, MISSING_PAGE_ADDRESS as u64);
    assert_eq!(registers.general_register(1)?, VM_INITIAL_VALUE as u64);
    assert_eq!(
        registers.general_register(31),
        Err(BootstrapError::InvalidRegister)
    );
    registers.set_general_register(1, VM_RECOVERED_VALUE as u64)?;
    vm_task.write_registers(&mut registers)?;
    sel4::debug_println!("fault: register control verified");

    let recovery_frame = bootstrap.allocate_frame()?;
    let recovery_mapping = unsafe {
        vm_task.map_recovery_frame(
            &mut bootstrap,
            &recovery_frame,
            MISSING_PAGE_ADDRESS,
        )?
    };
    assert!(recovery_mapping.root_slot() >= vm_task.root_slots().end);
    fault_endpoint.reply(vm_received);
    vm_completion.wait();
    assert_eq!(
        read_frame_word(&recovery_frame, scratch)?,
        VM_RECOVERED_VALUE
    );
    sel4::debug_println!("fault: VM recovery continued");
    vm_task.suspend()?;

    unknown_task.resume()?;
    let unknown_received = fault_endpoint
        .receive()
        .map_err(|_| BootstrapError::ThreadControlFailed)?;
    assert_eq!(unknown_received.source(), unknown_badge);
    let unknown_fault = match unknown_received.fault() {
        Fault::UnknownSyscall(fault) => fault,
        _ => return Err(BootstrapError::ThreadControlFailed),
    };
    assert_eq!(
        unknown_fault.argument_registers,
        [0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x123]
    );
    assert_eq!(
        unknown_fault.instruction_pointer,
        unknown_instruction as u64
    );
    assert_eq!(
        unknown_fault.stack_pointer,
        (STACK_ADDRESS + Frame::BYTES) as u64
    );
    assert_eq!(unknown_fault.syscall_number, UNKNOWN_SYSCALL_NUMBER);
    sel4::debug_println!("fault: unknown syscall decoded");

    unsafe { scratch_image.map(scratch as usize)? };
    sel4::debug_println!("TEST_RESULT: PASS");
    Ok(())
}

#[root_task]
fn main(bootinfo: &sel4::BootInfoPtr) -> ! {
    if let Err(error) = exercise(bootinfo) {
        sel4::debug_println!("test: substrate operation failed: {error:?}");
        sel4::debug_println!("TEST_RESULT: FAIL");
    }
    sel4::init_thread::suspend_self()
}
