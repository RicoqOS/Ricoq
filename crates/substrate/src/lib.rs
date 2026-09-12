//! System entry point.
#![no_std]
#![forbid(missing_docs)]

mod boot;
mod cspace;
mod errors;
mod fault;
mod free_slots;
mod ipc;
mod ipc_state;
mod object;
mod task;
mod thread;
mod vspace;

pub use boot::{Bootstrap, run};
pub use errors::BootstrapError;
pub use fault::{
    CapabilityFault, CapabilityLookupFailure, Fault, FaultBadge,
    FaultBadgeError, FaultDecodeError, FaultEndpoint, FaultReceiveError,
    FaultRoute, ReceivedFault, UnknownSyscallFault, VmFault,
};
pub use ipc::{
    Badge, BadgeAllocationError, BadgeAllocator, BadgeError, DeferredReply,
    EXTRA_CAPACITY, Endpoint, IpcError, MESSAGE_CAPACITY, Message,
    Notification, ReceiveSlot, ReceivedMessage, ReplyPool,
};
pub use task::{DelegatedCapability, SharedCode, Task, TaskConfig};
pub use thread::Registers;
pub use vspace::{Frame, TaskMapping};
