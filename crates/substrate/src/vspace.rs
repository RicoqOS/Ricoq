//! VSpace creation and base-page mappings.

use crate::boot::Bootstrap;
use crate::errors::BootstrapError;

/// A frame capability whose mappings are managed explicitly by its owner.
pub struct Frame(pub(crate) sel4::cap::Granule);

/// A root-held capability for one frame mapping in a task VSpace.
pub struct TaskMapping {
    mapping: sel4::cap::Granule,
    root_slot: usize,
}

impl TaskMapping {
    /// Removes this mapping from the task VSpace.
    ///
    /// # Safety
    ///
    /// The task must be stopped and no live access may depend on this mapping.
    pub unsafe fn unmap(&self) -> Result<(), BootstrapError> {
        self.mapping
            .frame_unmap()
            .map_err(|_| BootstrapError::FrameUnmappingFailed)
    }

    /// Returns the explicitly retained root CSpace slot.
    pub fn root_slot(&self) -> usize {
        self.root_slot
    }
}

impl Frame {
    /// Size and required virtual-address alignment of a base page.
    pub const BYTES: usize = sel4::FrameObjectType::GRANULE.bytes();

    /// Returns the owner capability used to derive mapping caps.
    pub(crate) fn cap(&self) -> sel4::cap::Granule {
        self.0
    }

    /// Maps this frame as read/write normal RAM in the initial address space.
    ///
    /// # Safety
    ///
    /// `address` must designate an exclusively owned, unmapped page backed by
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

    /// Removes the mapping held by this capability.
    ///
    /// # Safety
    ///
    /// The caller must exclusively control the mapping. No references or live
    /// code, stacks, or runtime data may depend on it after unmapping.
    pub unsafe fn unmap(&self) -> Result<(), BootstrapError> {
        self.0
            .frame_unmap()
            .map_err(|_| BootstrapError::FrameUnmappingFailed)
    }
}

pub(crate) struct VSpace {
    root: sel4::cap::VSpace,
}

impl VSpace {
    /// Creates an ASID-backed VSpace covering the requested pages.
    pub(crate) fn create(
        bootstrap: &mut Bootstrap<'_>,
        page_addresses: &[usize],
        reserved_pages: &[usize],
    ) -> Result<Self, BootstrapError> {
        let root = bootstrap.allocate_object::<sel4::cap_type::VSpace>()?;
        sel4::init_thread::slot::ASID_POOL
            .cap()
            .asid_pool_assign(root)
            .map_err(|_| BootstrapError::VSpaceCreationFailed)?;
        let vspace = Self { root };
        vspace.map_translation_tables(
            bootstrap,
            page_addresses,
            reserved_pages,
        )?;
        Ok(vspace)
    }

    /// Maps each distinct intermediate translation table once.
    fn map_translation_tables(
        &self,
        bootstrap: &mut Bootstrap<'_>,
        page_addresses: &[usize],
        reserved_pages: &[usize],
    ) -> Result<(), BootstrapError> {
        for level in 1..sel4::vspace_levels::NUM_LEVELS {
            let span_bits = sel4::vspace_levels::span_bits(level);
            let shift = u32::try_from(span_bits)
                .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
            let span = 1usize
                .checked_shl(shift)
                .ok_or(BootstrapError::InvalidTaskConfiguration)?;
            for (index, address) in page_addresses
                .iter()
                .chain(reserved_pages.iter())
                .enumerate()
            {
                let table_base = address / span;
                if page_addresses
                    .iter()
                    .chain(reserved_pages.iter())
                    .take(index)
                    .any(|prior| prior / span == table_base)
                {
                    continue;
                }
                let table_type =
                    sel4::TranslationTableObjectType::from_level(level)
                        .ok_or(BootstrapError::InvalidTaskConfiguration)?;
                let table = bootstrap
                    .allocate_blueprint(table_type.blueprint())?
                    .cast::<
                        sel4::cap_type::UnspecifiedIntermediateTranslationTable,
                    >();
                table
                    .generic_intermediate_translation_table_map(
                        table_type,
                        self.root,
                        *address,
                        sel4::VmAttributes::default(),
                    )
                    .map_err(|_| BootstrapError::VSpaceCreationFailed)?;
            }
        }
        Ok(())
    }

    /// Maps a rights-restricted cap while retaining the owner cap unmapped.
    pub(crate) fn map_frame(
        &self,
        bootstrap: &mut Bootstrap<'_>,
        frame: &Frame,
        address: usize,
        rights: sel4::CapRights,
    ) -> Result<sel4::cap::Granule, BootstrapError> {
        if !address.is_multiple_of(Frame::BYTES) {
            return Err(BootstrapError::InvalidTaskConfiguration);
        }
        let mapping = bootstrap.copy_cap(frame.cap(), rights.clone())?;
        mapping
            .frame_map(
                self.root,
                address,
                rights,
                sel4::VmAttributes::default(),
            )
            .map_err(|_| BootstrapError::FrameMappingFailed)?;
        Ok(mapping)
    }

    /// Returns the parent-held VSpace root capability.
    pub(crate) fn root(&self) -> sel4::cap::VSpace {
        self.root
    }
}

impl TaskMapping {
    pub(crate) fn new(mapping: sel4::cap::Granule, root_slot: usize) -> Self {
        Self { mapping, root_slot }
    }
}
