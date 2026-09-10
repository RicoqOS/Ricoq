//! System entry point.
#![no_std]
#![forbid(missing_docs)]

mod boot;
mod cspace;
mod errors;
mod fault;
mod free_slots;
mod ipc;
mod object;
mod task;
mod thread;
mod vspace;

pub use boot::{Bootstrap, Notification, run};
pub use errors::BootstrapError;
pub use fault::{
    CapabilityFault, CapabilityLookupFailure, Fault, FaultDecodeError,
    UnknownSyscallFault, VmFault,
};
pub use ipc::{
    FaultBadge, FaultBadgeError, FaultEndpoint, FaultReceiveError, FaultRoute,
    ReceivedFault,
};
pub use task::{DelegatedCapability, SharedCode, Task, TaskConfig};
pub use thread::Registers;
pub use vspace::{Frame, TaskMapping};
