//! Base-page operations on the initial thread's address space.

use substrate_api::BootstrapError;

/// A frame capability whose mapping is managed explicitly by its owner.
pub struct Frame(pub(crate) sel4::cap::Granule);

impl Frame {
    /// Size and required virtual-address alignment of a base page.
    pub const BYTES: usize = sel4::FrameObjectType::GRANULE.bytes();

    /// Maps this frame as read/write normal RAM in the initial address space.
    ///
    /// # Safety
    ///
    /// `address` must designate an exclusively owned, unmapped page with
    /// existing translation tables. No live references may cover that page.
    pub unsafe fn map(&self, address: usize) -> Result<(), BootstrapError> {
        if !address.is_multiple_of(Self::BYTES) {
            return Err(BootstrapError::InvalidImageRegion);
        }
        self.0
            .frame_map(
                sel4::init_thread::slot::VSPACE.cap(),
                address,
                sel4::CapRights::read_write(),
                sel4::VmAttributes::default(),
            )
            .map_err(|_| BootstrapError::FrameMappingFailed)
    }

    /// Removes this frame's mapping.
    ///
    /// # Safety
    ///
    /// The caller must exclusively control the mapping. No references or live
    /// code, stacks, or runtime data may depend on it during or after
    /// unmapping.
    pub unsafe fn unmap(&self) -> Result<(), BootstrapError> {
        self.0
            .frame_unmap()
            .map_err(|_| BootstrapError::FrameUnmappingFailed)
    }
}
