//! Decoding of the fault IPC ABI used by the pinned AArch64 kernel.

#[cfg(not(test))]
mod adapter {
    use super::{Fault, FaultDecodeError};
    use crate::boot::Bootstrap;
    use crate::cspace::DelegatedCapability;
    use crate::errors::BootstrapError;
    use crate::ipc::{
        Badge, BadgeError, Endpoint, IpcError, Message, ReceivedMessage,
    };

    /// A nonzero badge assigned to one fault capability path.
    pub type FaultBadge = Badge;
    /// Failure to construct a valid fault badge.
    pub type FaultBadgeError = BadgeError;

    /// A shared endpoint adapted for kernel fault IPC validation and decoding.
    pub struct FaultEndpoint {
        endpoint: Endpoint,
    }

    /// A task's badged capability route to a shared fault endpoint.
    #[derive(Clone)]
    pub struct FaultRoute {
        delegation: DelegatedCapability,
    }

    impl FaultRoute {
        /// Derives one fault route through the generic endpoint mechanism.
        pub fn new(
            endpoint: &FaultEndpoint,
            badge: FaultBadge,
            child_slot: usize,
        ) -> Self {
            Self {
                delegation: DelegatedCapability::badged_endpoint(
                    &endpoint.endpoint,
                    child_slot,
                    badge,
                ),
            }
        }

        pub(crate) fn child_slot(&self) -> usize {
            self.delegation.destination()
        }

        pub(crate) fn delegation(&self) -> &DelegatedCapability {
            &self.delegation
        }
    }

    /// One decoded fault retaining its exact classic-kernel reply authority.
    #[must_use = "reply to the received fault before receiving another fault"]
    pub struct ReceivedFault<'a> {
        source: FaultBadge,
        fault: Fault,
        received: ReceivedMessage<'a>,
    }

    impl ReceivedFault<'_> {
        /// Returns the badge assigned to the fault capability path.
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
        /// Generic endpoint state prevented a valid fault receive.
        Ipc(IpcError),
        /// The fault payload is malformed or unsupported.
        Decode(FaultDecodeError),
    }

    impl FaultEndpoint {
        /// Blocks until seL4 delivers and validates one fault.
        pub fn receive(&self) -> Result<ReceivedFault<'_>, FaultReceiveError> {
            let received =
                self.endpoint.receive(None).map_err(|error| match error {
                    IpcError::MessageTooLong { length, capacity } => {
                        FaultReceiveError::MessageTooLong { length, capacity }
                    },
                    IpcError::ReceiveCapacityExceeded { count, .. } => {
                        FaultReceiveError::UnexpectedCapabilities {
                            caps_unwrapped: 0,
                            extra_caps: count,
                        }
                    },
                    other => FaultReceiveError::Ipc(other),
                })?;
            let source = match received.badge() {
                Some(source) => source,
                None => {
                    received.finish();
                    return Err(FaultReceiveError::MissingSourceBadge);
                },
            };
            let message = received.message();
            if message.caps_unwrapped() != 0 || message.extra_caps() != 0 {
                let error = FaultReceiveError::UnexpectedCapabilities {
                    caps_unwrapped: message.caps_unwrapped(),
                    extra_caps: message.extra_caps(),
                };
                received.finish();
                return Err(error);
            }
            let fault = match Fault::decode(message.label(), message.words()) {
                Ok(fault) => fault,
                Err(error) => {
                    received.finish();
                    return Err(FaultReceiveError::Decode(error));
                },
            };
            Ok(ReceivedFault {
                source,
                fault,
                received,
            })
        }

        /// Replies through the authority retained by this exact received
        /// fault.
        pub fn reply(&self, fault: ReceivedFault<'_>) {
            let empty = Message::empty_reply();
            fault.received.reply(&empty);
        }
    }

    impl Bootstrap<'_> {
        /// Allocates a generic endpoint and exposes its fault-specific
        /// adapter.
        pub fn allocate_fault_endpoint(
            &mut self,
        ) -> Result<FaultEndpoint, BootstrapError> {
            self.allocate_endpoint()
                .map(|endpoint| FaultEndpoint { endpoint })
        }
    }
}

#[cfg(not(test))]
pub use adapter::{
    FaultBadge, FaultBadgeError, FaultEndpoint, FaultReceiveError, FaultRoute,
    ReceivedFault,
};

/// A decoded fault supported by the pinned kernel configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// A failed capability lookup.
    CapabilityFault(CapabilityFault),
    /// A syscall number outside the seL4 syscall range.
    UnknownSyscall(UnknownSyscallFault),
    /// An instruction or data access fault.
    VmFault(VmFault),
}

/// Failure to decode a fault message without accessing invalid registers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaultDecodeError {
    /// The pinned kernel does not expose this fault class to the substrate.
    UnsupportedLabel(u64),
    /// The message does not contain exactly the registers required by its
    /// fault class.
    UnexpectedLength {
        /// Raw seL4 message label.
        label: u64,
        /// Required message-register count.
        expected: usize,
        /// Received message-register count.
        actual: usize,
    },
    /// A field documented by seL4 as boolean was not zero or one.
    InvalidBoolean {
        /// Raw seL4 message label.
        label: u64,
        /// Message-register index containing the invalid value.
        index: usize,
        /// Invalid raw value.
        value: u64,
    },
    /// The capability lookup failure discriminator is not defined by seL4.
    InvalidLookupFailureType(u64),
}

/// Context for an AArch64 VM fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmFault {
    /// Instruction address at which the fault occurred.
    pub instruction_pointer: u64,
    /// Virtual address that could not be accessed.
    pub address: u64,
    /// Whether instruction fetch, rather than data access, faulted.
    pub instruction_fault: bool,
    /// Raw architecture fault-syndrome value retained for policy layers.
    pub syndrome: u64,
}

/// Context for an AArch64 unknown-syscall fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownSyscallFault {
    /// X0 through X7 at the fault boundary.
    pub argument_registers: [u64; 8],
    /// Address of the faulting syscall instruction.
    pub instruction_pointer: u64,
    /// Stack pointer at the fault boundary.
    pub stack_pointer: u64,
    /// Link register at the fault boundary.
    pub link_register: u64,
    /// Saved process status register.
    pub status_register: u64,
    /// Raw syscall number supplied to seL4.
    pub syscall_number: u64,
}

/// Context for a capability fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityFault {
    /// Instruction address at which the capability lookup failed.
    pub instruction_pointer: u64,
    /// Capability pointer supplied to the failed operation.
    pub capability_address: u64,
    /// Whether the lookup failed while receiving.
    pub in_receive_phase: bool,
    /// Structured capability lookup failure.
    pub lookup_failure: CapabilityLookupFailure,
}

/// The variable-length lookup details carried by a capability fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityLookupFailure {
    /// The CSpace root was not a valid CNode.
    InvalidRoot,
    /// No capability was present at the current lookup depth.
    MissingCapability {
        /// Unresolved capability-pointer bits.
        bits_left: u64,
    },
    /// A CNode ended before all requested bits were resolved.
    DepthMismatch {
        /// Unresolved capability-pointer bits.
        bits_left: u64,
        /// Bits resolved by the final CNode.
        bits_found: u64,
    },
    /// A CNode guard did not match the capability pointer.
    GuardMismatch {
        /// Unresolved capability-pointer bits.
        bits_left: u64,
        /// Guard value found in the CNode capability.
        guard_found: u64,
        /// Number of guard bits compared.
        bits_found: u64,
    },
}

impl Fault {
    /// Decodes a complete fault message after validating its exact shape.
    pub fn decode(
        label: u64,
        registers: &[u64],
    ) -> Result<Self, FaultDecodeError> {
        match label {
            1 => decode_capability_fault(registers).map(Self::CapabilityFault),
            2 => decode_unknown_syscall(registers).map(Self::UnknownSyscall),
            5 => decode_vm_fault(registers).map(Self::VmFault),
            _ => Err(FaultDecodeError::UnsupportedLabel(label)),
        }
    }
}

fn expect_length(
    label: u64,
    registers: &[u64],
    expected: usize,
) -> Result<(), FaultDecodeError> {
    if registers.len() != expected {
        return Err(FaultDecodeError::UnexpectedLength {
            label,
            expected,
            actual: registers.len(),
        });
    }
    Ok(())
}

fn decode_boolean(
    label: u64,
    index: usize,
    value: u64,
) -> Result<bool, FaultDecodeError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(FaultDecodeError::InvalidBoolean {
            label,
            index,
            value,
        }),
    }
}

fn decode_vm_fault(registers: &[u64]) -> Result<VmFault, FaultDecodeError> {
    const LABEL: u64 = 5;
    expect_length(LABEL, registers, 4)?;
    Ok(VmFault {
        instruction_pointer: registers[0],
        address: registers[1],
        instruction_fault: decode_boolean(LABEL, 2, registers[2])?,
        syndrome: registers[3],
    })
}

fn decode_unknown_syscall(
    registers: &[u64],
) -> Result<UnknownSyscallFault, FaultDecodeError> {
    const LABEL: u64 = 2;
    expect_length(LABEL, registers, 13)?;
    let mut argument_registers = [0; 8];
    argument_registers.copy_from_slice(&registers[..8]);
    Ok(UnknownSyscallFault {
        argument_registers,
        instruction_pointer: registers[8],
        stack_pointer: registers[9],
        link_register: registers[10],
        status_register: registers[11],
        syscall_number: registers[12],
    })
}

fn decode_capability_fault(
    registers: &[u64],
) -> Result<CapabilityFault, FaultDecodeError> {
    const LABEL: u64 = 1;
    if registers.len() < 4 {
        return Err(FaultDecodeError::UnexpectedLength {
            label: LABEL,
            expected: 4,
            actual: registers.len(),
        });
    }
    let lookup_failure = match registers[3] {
        1 => {
            expect_length(LABEL, registers, 4)?;
            CapabilityLookupFailure::InvalidRoot
        },
        2 => {
            expect_length(LABEL, registers, 5)?;
            CapabilityLookupFailure::MissingCapability {
                bits_left: registers[4],
            }
        },
        3 => {
            expect_length(LABEL, registers, 6)?;
            CapabilityLookupFailure::DepthMismatch {
                bits_left: registers[4],
                bits_found: registers[5],
            }
        },
        4 => {
            expect_length(LABEL, registers, 7)?;
            CapabilityLookupFailure::GuardMismatch {
                bits_left: registers[4],
                guard_found: registers[5],
                bits_found: registers[6],
            }
        },
        other => {
            return Err(FaultDecodeError::InvalidLookupFailureType(other));
        },
    };
    Ok(CapabilityFault {
        instruction_pointer: registers[0],
        capability_address: registers[1],
        in_receive_phase: decode_boolean(LABEL, 2, registers[2])?,
        lookup_failure,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CapabilityLookupFailure, Fault, FaultDecodeError, UnknownSyscallFault,
    };

    #[test]
    fn decodes_vm_fault_context() {
        let fault = Fault::decode(5, &[0x1000, 0x4000, 0, 0x9200_0047]);
        assert_eq!(
            fault,
            Ok(Fault::VmFault(super::VmFault {
                instruction_pointer: 0x1000,
                address: 0x4000,
                instruction_fault: false,
                syndrome: 0x9200_0047,
            })),
        );
    }

    #[test]
    fn rejects_malformed_vm_faults_before_field_access() {
        assert_eq!(
            Fault::decode(5, &[0x1000, 0x4000, 0]),
            Err(FaultDecodeError::UnexpectedLength {
                label: 5,
                expected: 4,
                actual: 3,
            }),
        );
        assert_eq!(
            Fault::decode(5, &[0x1000, 0x4000, 2, 0]),
            Err(FaultDecodeError::InvalidBoolean {
                label: 5,
                index: 2,
                value: 2,
            }),
        );
    }

    #[test]
    fn decodes_unknown_syscall_register_context() {
        let registers = [
            10, 11, 12, 13, 14, 15, 16, 0x123, 0x8000, 0x9000, 0xa000, 0x40,
            0x123,
        ];
        assert_eq!(
            Fault::decode(2, &registers),
            Ok(Fault::UnknownSyscall(UnknownSyscallFault {
                argument_registers: [10, 11, 12, 13, 14, 15, 16, 0x123],
                instruction_pointer: 0x8000,
                stack_pointer: 0x9000,
                link_register: 0xa000,
                status_register: 0x40,
                syscall_number: 0x123,
            })),
        );
    }

    #[test]
    fn validates_capability_fault_shape_by_lookup_failure() {
        assert_eq!(
            Fault::decode(1, &[0x1000, 7, 0, 2, 61]),
            Ok(Fault::CapabilityFault(super::CapabilityFault {
                instruction_pointer: 0x1000,
                capability_address: 7,
                in_receive_phase: false,
                lookup_failure: CapabilityLookupFailure::MissingCapability {
                    bits_left: 61,
                },
            })),
        );
        assert_eq!(
            Fault::decode(1, &[0x1000, 7, 1, 4, 60, 3, 4]),
            Ok(Fault::CapabilityFault(super::CapabilityFault {
                instruction_pointer: 0x1000,
                capability_address: 7,
                in_receive_phase: true,
                lookup_failure: CapabilityLookupFailure::GuardMismatch {
                    bits_left: 60,
                    guard_found: 3,
                    bits_found: 4,
                },
            })),
        );
        assert_eq!(
            Fault::decode(1, &[0, 0, 0, 4, 0, 0]),
            Err(FaultDecodeError::UnexpectedLength {
                label: 1,
                expected: 7,
                actual: 6,
            }),
        );
    }

    #[test]
    fn rejects_unsupported_labels_and_invalid_lookup_types() {
        assert_eq!(
            Fault::decode(3, &[]),
            Err(FaultDecodeError::UnsupportedLabel(3)),
        );
        assert_eq!(
            Fault::decode(1, &[0, 0, 0, 9]),
            Err(FaultDecodeError::InvalidLookupFailureType(9)),
        );
    }
}
