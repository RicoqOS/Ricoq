//! System entry point.
#![no_std]
#![forbid(missing_docs)]

mod boot;
mod cspace;
mod errors;
mod free_slots;
mod object;
mod task;
mod thread;
mod vspace;

pub use boot::{Bootstrap, Notification, run};
pub use errors::BootstrapError;
pub use task::{DelegatedCapability, SharedCode, Task, TaskConfig};
pub use vspace::Frame;
