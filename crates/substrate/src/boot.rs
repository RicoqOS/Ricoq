//! Boot-time resources for single-threaded substrate initialization.

use crate::errors::BootstrapError;
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

    /// Allocates an internal fixed-size kernel object.
    pub(crate) fn allocate_object<T>(
        &mut self,
    ) -> Result<sel4::Cap<T>, BootstrapError>
    where
        T: sel4::CapTypeForObjectOfFixedSize,
    {
        crate::object::allocate(self.bootinfo, &mut self.slots)
    }

    /// Allocates an internal variable-size kernel object.
    pub(crate) fn allocate_variable_object<T>(
        &mut self,
        size_bits: usize,
    ) -> Result<sel4::Cap<T>, BootstrapError>
    where
        T: sel4::CapTypeForObjectOfVariableSize,
    {
        crate::object::allocate_variable(
            self.bootinfo,
            &mut self.slots,
            size_bits,
        )
    }

    /// Allocates an architecture-selected object through the shared path.
    pub(crate) fn allocate_blueprint(
        &mut self,
        blueprint: sel4::ObjectBlueprint,
    ) -> Result<sel4::cap::Unspecified, BootstrapError> {
        crate::object::allocate_blueprint(
            self.bootinfo,
            &mut self.slots,
            blueprint,
        )
    }

    /// Derives a root-held cap while preserving sequential slot ownership.
    pub(crate) fn copy_cap<T: sel4::CapType>(
        &mut self,
        source: sel4::Cap<T>,
        rights: sel4::CapRights,
    ) -> Result<sel4::Cap<T>, BootstrapError> {
        crate::cspace::copy_to_root(&mut self.slots, source, rights)
    }

    /// Marks the start of an atomic construction sequence.
    pub(crate) fn checkpoint(&self) -> usize {
        self.slots.checkpoint()
    }

    /// Returns the resource slots retained since `checkpoint`.
    pub(crate) fn committed_slots(
        &self,
        checkpoint: usize,
    ) -> Result<core::ops::Range<usize>, BootstrapError> {
        self.slots
            .allocated_since(checkpoint)
            .ok_or(BootstrapError::RollbackFailed)
    }

    /// Deletes a failed allocation suffix before making its slots reusable.
    pub(crate) fn rollback(
        &mut self,
        checkpoint: usize,
    ) -> Result<(), BootstrapError> {
        let slots = self.committed_slots(checkpoint)?;
        crate::cspace::delete_root_slots(slots)?;
        if !self.slots.rewind(checkpoint) {
            return Err(BootstrapError::RollbackFailed);
        }
        Ok(())
    }

    /// Obtains the frame backing a page in the initial image.
    ///
    /// # Safety
    ///
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
    /// Returns the root-held capability for explicit delegation.
    pub(crate) fn cap(&self) -> sel4::cap::Notification {
        self.0
    }

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
