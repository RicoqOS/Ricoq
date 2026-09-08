#![no_std]

/// Bootstrap failures that do not expose backend capability types.
#[derive(Debug, PartialEq, Eq)]
pub enum BootstrapError {
    NoFreeSlots,
    NoKernelMemory,
    KernelAllocationFailed,
}
