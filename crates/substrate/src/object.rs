//! Kernel-object creation from boot-time non-device Untypeds.

use sel4::{
    CapTypeForObjectOfFixedSize, CapTypeForObjectOfVariableSize,
    ObjectBlueprint,
};

use crate::errors::BootstrapError;
use crate::free_slots::FreeSlots;

/// Allocates one fixed-size object through the shared retype path.
pub(crate) fn allocate<T: CapTypeForObjectOfFixedSize>(
    bootinfo: &sel4::BootInfo,
    slots: &mut FreeSlots,
) -> Result<sel4::Cap<T>, BootstrapError> {
    allocate_blueprint(bootinfo, slots, T::object_blueprint())
        .map(sel4::cap::Unspecified::downcast)
}

/// Allocates one variable-size object through the shared retype path.
pub(crate) fn allocate_variable<T: CapTypeForObjectOfVariableSize>(
    bootinfo: &sel4::BootInfo,
    slots: &mut FreeSlots,
    size_bits: usize,
) -> Result<sel4::Cap<T>, BootstrapError> {
    allocate_blueprint(bootinfo, slots, T::object_blueprint(size_bits))
        .map(sel4::cap::Unspecified::downcast)
}

/// Retypes the first suitable Untyped without committing a failed slot.
pub(crate) fn allocate_blueprint(
    bootinfo: &sel4::BootInfo,
    slots: &mut FreeSlots,
    blueprint: ObjectBlueprint,
) -> Result<sel4::cap::Unspecified, BootstrapError> {
    slots
        .try_allocate(|index| {
            let slot = sel4::init_thread::Slot::from_index(index);
            let untyped_slots = bootinfo.untyped();
            for (index, descriptor) in bootinfo
                .untyped_list()
                .iter()
                .enumerate()
                .take(untyped_slots.len())
            {
                if descriptor.is_device() ||
                    descriptor.size_bits() < blueprint.physical_size_bits()
                {
                    continue;
                }
                match untyped_slots.index(index).cap().untyped_retype(
                    &blueprint,
                    &sel4::init_thread::slot::CNODE
                        .cap()
                        .absolute_cptr_for_self(),
                    slot.index(),
                    1,
                ) {
                    // Only a successful retype establishes T and consumes the
                    // slot.
                    Ok(()) => return Ok(slot.cap()),
                    // BootInfo sizes omit previous allocations. Failed retypes
                    // leave the destination empty, so the same slot is safe to
                    // retry.
                    Err(sel4::Error::NotEnoughMemory) => continue,
                    Err(error) => {
                        sel4::debug_println!(
                            "substrate: retype failed: {error:?}"
                        );
                        return Err(BootstrapError::KernelAllocationFailed);
                    },
                }
            }
            Err(BootstrapError::NoKernelMemory)
        })?
        .ok_or(BootstrapError::NoFreeSlots)
}
