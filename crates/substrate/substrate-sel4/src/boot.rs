//! Substrate initialization routines and seL4 kernel object bootstrapping.

use substrate_api::BootstrapError;

use crate::free_slots::FreeSlots;

/// A single allocation helper for retyping an untyped memory region into a
/// notification capability.
struct NotificationAllocator {
    /// The target CSpace slot that will hold the newly created notification
    /// capability.
    slot: sel4::init_thread::Slot,
    /// The untyped memory region used to back the notification object.
    untyped: sel4::cap::Untyped,
}

impl NotificationAllocator {
    /// Retypes untyped memory into a notification object inside a slot.
    ///
    /// # Errors
    ///
    /// Returns [`BootstrapError::KernelAllocationFailed`] if seL4 retype
    /// operation fails.
    fn allocate(self) -> Result<sel4::cap::Notification, BootstrapError> {
        self.untyped
            .untyped_retype(
                &sel4::ObjectBlueprint::Notification,
                &sel4::init_thread::slot::CNODE
                    .cap()
                    .absolute_cptr_for_self(),
                self.slot.index(),
                1,
            )
            .map_err(|error| {
                sel4::debug_println!("substrate: retype failed: {error:?}");
                BootstrapError::KernelAllocationFailed
            })?;
        // The successful retype establishes the slot's capability type.
        Ok(self.slot.downcast::<sel4::cap_type::Notification>().cap())
    }
}

/// Executes the core bootstrap sequence.
///
/// # Errors
///
/// Returns a [`BootstrapError`] if CSpace slots are exhausted.
fn bootstrap(bootinfo: &sel4::BootInfo) -> Result<(), BootstrapError> {
    let mut slots = FreeSlots::new(bootinfo.empty().range());
    let slot = slots
        .allocate()
        .map(sel4::init_thread::Slot::from_index)
        .ok_or(BootstrapError::NoFreeSlots)?;
    sel4::debug_println!("substrate: cspace ready");

    let untyped_slots = bootinfo.untyped();
    let (index, _) = bootinfo
        .untyped_list()
        .iter()
        .enumerate()
        .take(untyped_slots.len())
        .find(|(_, descriptor)| {
            !descriptor.is_device() &&
                descriptor.size_bits() >=
                    sel4::ObjectBlueprint::Notification
                        .physical_size_bits()
        })
        .ok_or(BootstrapError::NoKernelMemory)?;
    let untyped = untyped_slots.index(index).cap();
    sel4::debug_println!("substrate: untyped ready");

    let notification = NotificationAllocator { slot, untyped }.allocate()?;
    notification.signal();
    let (_info, _badge) = notification.wait();
    sel4::debug_println!("substrate: notification allocated");
    Ok(())
}

/// Consumes the initial thread's bootstrap lifecycle and suspends permanently.
pub fn run(bootinfo: &sel4::BootInfo) -> ! {
    sel4::debug_println!("substrate: booting");
    match bootstrap(bootinfo) {
        Ok(()) => sel4::debug_println!("TEST_RESULT: PASS"),
        Err(error) => {
            sel4::debug_println!("substrate: bootstrap failed: {:?}", error);
            sel4::debug_println!("TEST_RESULT: FAIL");
        },
    }
    sel4::init_thread::suspend_self()
}
