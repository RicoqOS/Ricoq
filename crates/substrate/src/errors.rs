//! Internal substrate errors for Ricoq.

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
    /// A page address or image region is invalid.
    InvalidImageRegion,
    /// Mapping a frame into the bootstrap address space failed.
    FrameMappingFailed,
    /// Removing a frame mapping failed.
    FrameUnmappingFailed,
    /// A task configuration violates an address, CSpace, or capacity
    /// invariant.
    InvalidTaskConfiguration,
    /// Creating or assigning a task's VSpace failed.
    VSpaceCreationFailed,
    /// Installing a capability or creating a CSpace failed.
    CSpaceCreationFailed,
    /// Configuring a TCB or its initial register state failed.
    ThreadConfigurationFailed,
    /// Starting or suspending a task failed.
    ThreadControlFailed,
    /// Cleanup after failed construction could not restore slot accounting.
    RollbackFailed,
}
