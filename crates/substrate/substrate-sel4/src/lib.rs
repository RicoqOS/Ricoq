//! System entry point.
#![no_std]
#![forbid(missing_docs)]

mod boot;
mod free_slots;
mod object;
mod vspace;

pub use boot::{Bootstrap, Notification, run};
pub use vspace::Frame;
