//! Fixed-size kernel object creation from boot-time non-device Untypeds.

use sel4::CapTypeForObjectOfFixedSize;

use crate::errors::BootstrapError;
use crate::free_slots::FreeSlots;

pub(crate) fn allocate<T: CapTypeForObjectOfFixedSize>(
    bootinfo: &sel4::BootInfo,
    slots: &mut FreeSlots,
) -> Result<sel4::Cap<T>, BootstrapError> {
    slots
        .try_allocate(|index| {
            let slot = sel4::init_thread::Slot::from_index(index);
            let blueprint = T::object_blueprint();
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
                    Ok(()) => return Ok(slot.downcast::<T>().cap()),
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
