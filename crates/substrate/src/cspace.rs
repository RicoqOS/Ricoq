//! Creation and explicit population of child capability spaces.

use core::ops::Range;

use crate::boot::{Bootstrap, Notification};
use crate::errors::BootstrapError;
use crate::free_slots::FreeSlots;
use crate::ipc::FaultRoute;

/// A capability to install explicitly in a task's CSpace.
#[derive(Clone)]
pub struct DelegatedCapability {
    source: sel4::cap::Unspecified,
    destination: usize,
    rights: sel4::CapRights,
    badge: sel4::Word,
}

impl DelegatedCapability {
    /// Installs a derived cap with the requested rights and badge.
    pub fn new<T: sel4::CapType>(
        source: sel4::Cap<T>,
        destination: usize,
        rights: sel4::CapRights,
        badge: sel4::Word,
    ) -> Self {
        Self {
            source: source.upcast(),
            destination,
            rights,
            badge,
        }
    }

    /// Delegates only signalling authority for a notification.
    pub fn notification(source: &Notification, destination: usize) -> Self {
        Self::new(source.cap(), destination, sel4::CapRights::write_only(), 0)
    }
}

pub(crate) struct CSpace {
    root: sel4::cap::CNode,
    size_bits: usize,
}

impl CSpace {
    /// Creates an empty CNode and installs only explicit delegations.
    pub(crate) fn create(
        bootstrap: &mut Bootstrap<'_>,
        size_bits: usize,
        capabilities: &[DelegatedCapability],
        fault_route: Option<&FaultRoute<'_>>,
    ) -> Result<Self, BootstrapError> {
        validate(size_bits, capabilities, fault_route)?;
        let root = bootstrap
            .allocate_variable_object::<sel4::cap_type::CNode>(size_bits)?;
        let cspace = Self { root, size_bits };
        for capability in capabilities {
            let destination_bits = capability
                .destination
                .try_into()
                .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
            let destination = root.absolute_cptr_from_bits_with_depth(
                destination_bits,
                size_bits,
            );
            let source = sel4::init_thread::slot::CNODE
                .cap()
                .absolute_cptr(capability.source);
            destination
                .mint(&source, capability.rights.clone(), capability.badge)
                .map_err(|_| BootstrapError::CSpaceCreationFailed)?;
        }
        if let Some(route) = fault_route {
            let destination_bits = route
                .child_slot
                .try_into()
                .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
            let destination = root.absolute_cptr_from_bits_with_depth(
                destination_bits,
                size_bits,
            );
            let source = sel4::init_thread::slot::CNODE
                .cap()
                .absolute_cptr(route.endpoint.cap());
            let rights = sel4::CapRightsBuilder::none()
                .write(true)
                .grant(true)
                .build();
            destination
                .mint(&source, rights, route.badge.raw())
                .map_err(|_| BootstrapError::CSpaceCreationFailed)?;
        }
        Ok(cspace)
    }

    /// Returns the parent-held CSpace root capability.
    pub(crate) fn root(&self) -> sel4::cap::CNode {
        self.root
    }

    /// Encodes the guard required for this single-level CSpace.
    pub(crate) fn root_data(&self) -> sel4::CNodeCapData {
        sel4::CNodeCapData::new(0, sel4::WORD_SIZE - self.size_bits)
    }
}

/// Rejects invalid, null, out-of-range, or duplicate destinations.
fn validate(
    size_bits: usize,
    capabilities: &[DelegatedCapability],
    fault_route: Option<&FaultRoute<'_>>,
) -> Result<(), BootstrapError> {
    let shift = u32::try_from(size_bits)
        .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
    let capacity = 1usize
        .checked_shl(shift)
        .ok_or(BootstrapError::InvalidTaskConfiguration)?;
    if size_bits == 0 || size_bits >= sel4::WORD_SIZE {
        return Err(BootstrapError::InvalidTaskConfiguration);
    }
    for (index, capability) in capabilities.iter().enumerate() {
        if capability.destination == 0 ||
            capability.destination >= capacity ||
            capability.source.bits() == 0 ||
            capabilities[..index]
                .iter()
                .any(|prior| prior.destination == capability.destination)
        {
            return Err(BootstrapError::InvalidTaskConfiguration);
        }
    }
    if let Some(route) = fault_route &&
        (route.child_slot == 0 ||
            route.child_slot >= capacity ||
            route.endpoint.cap().bits() == 0 ||
            capabilities.iter().any(|capability| {
                capability.destination == route.child_slot
            }))
    {
        return Err(BootstrapError::InvalidTaskConfiguration);
    }
    Ok(())
}

/// Copies a capability into the next tracked root slot.
pub(crate) fn copy_to_root<T: sel4::CapType>(
    slots: &mut FreeSlots,
    source: sel4::Cap<T>,
    rights: sel4::CapRights,
) -> Result<sel4::Cap<T>, BootstrapError> {
    slots
        .try_allocate(|index| {
            let slot = sel4::init_thread::Slot::from_index(index);
            let destination = sel4::init_thread::slot::CNODE
                .cap()
                .absolute_cptr(slot.cptr());
            let source =
                sel4::init_thread::slot::CNODE.cap().absolute_cptr(source);
            destination
                .copy(&source, rights)
                .map_err(|_| BootstrapError::CSpaceCreationFailed)?;
            Ok(slot.downcast::<T>().cap())
        })?
        .ok_or(BootstrapError::NoFreeSlots)
}

/// Deletes a construction suffix in reverse ownership order.
pub(crate) fn delete_root_slots(
    slots: Range<usize>,
) -> Result<(), BootstrapError> {
    for index in slots.rev() {
        let slot: sel4::init_thread::Slot =
            sel4::init_thread::Slot::from_index(index);
        sel4::init_thread::slot::CNODE
            .cap()
            .absolute_cptr(slot.cptr())
            .delete()
            .map_err(|_| BootstrapError::RollbackFailed)?;
    }
    Ok(())
}
