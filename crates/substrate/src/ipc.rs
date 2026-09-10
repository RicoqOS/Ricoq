//! Minimal endpoint operations for kernel fault delivery.

use crate::boot::Bootstrap;
use crate::errors::BootstrapError;
use crate::fault::{Fault, FaultDecodeError};

/// A nonzero badge assigned to one fault source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaultBadge(sel4::Word);

/// Failure to construct a valid fault badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultBadgeError {
    /// Badge zero is reserved for unbadged endpoint traffic.
    ReservedZero,
}

impl FaultBadge {
    /// Validates a raw badge for task fault routing.
    pub fn new(raw: sel4::Word) -> Result<Self, FaultBadgeError> {
        if raw == 0 {
            return Err(FaultBadgeError::ReservedZero);
        }
        Ok(Self(raw))
    }

    /// Returns the raw badge value received from seL4.
    pub fn raw(self) -> sel4::Word {
        self.0
    }
}

/// A shared endpoint that receives badged kernel fault IPC.
pub struct FaultEndpoint(sel4::cap::Endpoint);

/// A task's route to a shared fault endpoint.
#[derive(Clone, Copy)]
pub struct FaultRoute<'a> {
    pub(crate) endpoint: &'a FaultEndpoint,
    pub(crate) badge: FaultBadge,
    pub(crate) child_slot: usize,
}

impl<'a> FaultRoute<'a> {
    /// Assigns a source badge and child CSpace slot to an endpoint.
    pub fn new(
        endpoint: &'a FaultEndpoint,
        badge: FaultBadge,
        child_slot: usize,
    ) -> Self {
        Self {
            endpoint,
            badge,
            child_slot,
        }
    }
}

/// One fault and the badge identifying its originating task.
pub struct ReceivedFault {
    source: FaultBadge,
    fault: Fault,
}

impl ReceivedFault {
    /// Returns the badge assigned to the faulting task.
    pub fn source(&self) -> FaultBadge {
        self.source
    }

    /// Returns the safely decoded fault context.
    pub fn fault(&self) -> &Fault {
        &self.fault
    }
}

/// Failure to validate or decode received fault IPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaultReceiveError {
    /// The kernel delivered an unbadged message on a fault endpoint.
    MissingSourceBadge,
    /// Fault IPC must not transfer capabilities.
    UnexpectedCapabilities {
        /// Capabilities unwrapped by seL4.
        caps_unwrapped: usize,
        /// Extra capability slots in the message.
        extra_caps: usize,
    },
    /// The message length exceeds the pinned IPC-buffer capacity.
    MessageTooLong {
        /// Length reported by seL4.
        length: usize,
        /// Available message registers.
        capacity: usize,
    },
    /// The fault payload is malformed or unsupported.
    Decode(FaultDecodeError),
}

impl FaultEndpoint {
    /// Blocks until seL4 delivers one fault on this endpoint.
    pub fn receive(&self) -> Result<ReceivedFault, FaultReceiveError> {
        let (info, raw_badge) = self.0.recv(());
        let source = FaultBadge::new(raw_badge)
            .map_err(|_| FaultReceiveError::MissingSourceBadge)?;
        if info.caps_unwrapped() != 0 || info.extra_caps() != 0 {
            return Err(FaultReceiveError::UnexpectedCapabilities {
                caps_unwrapped: info.caps_unwrapped(),
                extra_caps: info.extra_caps(),
            });
        }
        let fault = sel4::with_ipc_buffer(|buffer| {
            let registers = buffer.msg_regs();
            if info.length() > registers.len() {
                return Err(FaultReceiveError::MessageTooLong {
                    length: info.length(),
                    capacity: registers.len(),
                });
            }
            Fault::decode(info.label(), &registers[..info.length()])
                .map_err(FaultReceiveError::Decode)
        })?;
        Ok(ReceivedFault { source, fault })
    }

    /// Replies to the current classic-kernel fault IPC and restarts its task.
    pub fn reply(&self, _fault: ReceivedFault) {
        sel4::with_ipc_buffer_mut(|buffer| {
            sel4::reply(buffer, sel4::MessageInfoBuilder::default().build());
        });
    }

    /// Returns the root-held endpoint capability for fault-route derivation.
    pub(crate) fn cap(&self) -> sel4::cap::Endpoint {
        self.0
    }
}

impl Bootstrap<'_> {
    /// Allocates a shared endpoint through the normal object allocator.
    pub fn allocate_fault_endpoint(
        &mut self,
    ) -> Result<FaultEndpoint, BootstrapError> {
        self.allocate_object::<sel4::cap_type::Endpoint>()
            .map(FaultEndpoint)
    }
}
