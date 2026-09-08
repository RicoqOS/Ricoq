//! Internal substrate API for Ricoq.

#![no_std]
#![forbid(missing_docs)]

/// Bootstrap failures that do not expose backend capability types.
#[derive(Debug, PartialEq, Eq)]
pub enum BootstrapError {
    /// No CSpace slots are remaining for capability allocation.
    NoFreeSlots,
    /// No suitable untyped memory region was found matching allocation
    /// requirements.
    NoKernelMemory,
    /// Retyping or instantiating the kernel object failed.
    KernelAllocationFailed,
}
