//! Boot-time resources for single-threaded substrate initialization.

use substrate_api::BootstrapError;

use crate::free_slots::FreeSlots;
use crate::vspace::Frame;

/// Owns allocation from the initial thread's free capability slots.
/// Construct once from the entrypoint's BootInfo before allocating objects.
pub struct Bootstrap<'a> {
    bootinfo: &'a sel4::BootInfo,
    slots: FreeSlots,
}

impl<'a> Bootstrap<'a> {
    /// Initializes free-slot accounting from the root-task entrypoint ABI.
    pub fn new(bootinfo: &'a sel4::BootInfo) -> Result<Self, BootstrapError> {
        if bootinfo.empty().len() == 0 {
            return Err(BootstrapError::NoFreeSlots);
        }
        Ok(Self {
            bootinfo,
            slots: FreeSlots::new(bootinfo.empty().range()),
        })
    }

    /// Creates a notification from normal RAM.
    pub fn allocate_notification(
        &mut self,
    ) -> Result<Notification, BootstrapError> {
        crate::object::allocate(self.bootinfo, &mut self.slots)
            .map(Notification)
    }

    /// Creates one base-page frame from normal RAM.
    pub fn allocate_frame(&mut self) -> Result<Frame, BootstrapError> {
        crate::object::allocate(self.bootinfo, &mut self.slots).map(Frame)
    }

    /// Obtains the frame backing a page in the initial image.
    ///
    /// # Safety
    /// `image_start` must be the page-aligned base loaded by the kernel
    /// loader. The caller must exclusively control this page's mapping and
    /// capability.
    pub unsafe fn image_frame(
        &self,
        image_start: usize,
        address: usize,
    ) -> Result<Frame, BootstrapError> {
        let offset = address
            .checked_sub(image_start)
            .ok_or(BootstrapError::InvalidImageRegion)?;
        let frames = self.bootinfo.user_image_frames();
        if !image_start.is_multiple_of(Frame::BYTES) ||
            !address.is_multiple_of(Frame::BYTES) ||
            offset / Frame::BYTES >= frames.len()
        {
            return Err(BootstrapError::InvalidImageRegion);
        }
        Ok(Frame(frames.index(offset / Frame::BYTES).cap()))
    }
}

/// An owned kernel notification capability.
pub struct Notification(sel4::cap::Notification);

impl Notification {
    /// Signals this notification.
    pub fn signal(&self) {
        self.0.signal();
    }

    /// Blocks until this notification is signalled.
    pub fn wait(&self) {
        self.0.wait();
    }
}

/// Initializes bootstrap resources and suspends the initial thread.
pub fn run(bootinfo: &sel4::BootInfo) -> ! {
    match Bootstrap::new(bootinfo) {
        Ok(_bootstrap) => sel4::debug_println!("substrate: ready"),
        Err(error) => {
            sel4::debug_println!("substrate: bootstrap failed: {error:?}")
        },
    }
    sel4::init_thread::suspend_self()
}
