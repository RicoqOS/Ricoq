//! System entry point.
#![no_std]
#![forbid(missing_docs)]

mod boot;
mod free_slots;
pub use boot::run;
