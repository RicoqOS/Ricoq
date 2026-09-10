//! Composition of CSpace, VSpace, memory, and TCB task resources.

#[derive(Debug, PartialEq, Eq)]
enum LayoutError {
    InvalidPageSize,
    UnalignedAddress,
    AddressOverflow,
    OverlappingPages,
    InvalidEntry,
}

struct TaskLayout {
    stack_pointer: usize,
}

impl TaskLayout {
    /// Validates the fixed page roles before any kernel object is allocated.
    fn validate(
        page_size: usize,
        code_address: usize,
        entry: usize,
        private_memory_address: usize,
        stack_address: usize,
        ipc_buffer_address: usize,
        reserved_pages: &[usize],
    ) -> Result<Self, LayoutError> {
        if page_size == 0 || !page_size.is_power_of_two() {
            return Err(LayoutError::InvalidPageSize);
        }
        let addresses = [
            code_address,
            private_memory_address,
            stack_address,
            ipc_buffer_address,
        ];
        for (index, address) in addresses.iter().enumerate() {
            if !address.is_multiple_of(page_size) {
                return Err(LayoutError::UnalignedAddress);
            }
            address
                .checked_add(page_size)
                .ok_or(LayoutError::AddressOverflow)?;
            if addresses[..index].contains(address) {
                return Err(LayoutError::OverlappingPages);
            }
        }
        for (index, address) in reserved_pages.iter().enumerate() {
            if !address.is_multiple_of(page_size) {
                return Err(LayoutError::UnalignedAddress);
            }
            address
                .checked_add(page_size)
                .ok_or(LayoutError::AddressOverflow)?;
            if addresses.contains(address) ||
                reserved_pages[..index].contains(address)
            {
                return Err(LayoutError::OverlappingPages);
            }
        }
        let code_end = code_address
            .checked_add(page_size)
            .ok_or(LayoutError::AddressOverflow)?;
        if entry < code_address ||
            entry >= code_end ||
            !entry.is_multiple_of(4)
        {
            return Err(LayoutError::InvalidEntry);
        }
        let stack_pointer = stack_address
            .checked_add(page_size)
            .ok_or(LayoutError::AddressOverflow)?;
        Ok(Self { stack_pointer })
    }
}

#[cfg(not(test))]
mod platform {
    use core::ops::Range;

    use super::TaskLayout;
    use crate::boot::Bootstrap;
    use crate::cspace::CSpace;
    use crate::errors::BootstrapError;
    use crate::ipc::FaultRoute;
    use crate::thread::{Registers, Thread, ThreadConfig};
    use crate::vspace::{Frame, TaskMapping, VSpace};

    /// One read-only executable page shared with a task.
    pub struct SharedCode<'a> {
        /// Source frame capability retained by the creator.
        pub frame: &'a Frame,
        /// Page-aligned virtual address in the task.
        pub address: usize,
        /// Initial program counter within this page.
        pub entry: usize,
    }

    /// Complete fixed-footprint resource description for one isolated task.
    pub struct TaskConfig<'a> {
        /// Size of the child CNode in capability-slot bits.
        pub cspace_size_bits: usize,
        /// Executable code mapped read-only into the task.
        pub code: SharedCode<'a>,
        /// Page-aligned virtual address for private writable memory.
        pub private_memory_address: usize,
        /// Page-aligned virtual address for the task's stack page.
        pub stack_address: usize,
        /// Page-aligned virtual address for the task's IPC buffer page.
        pub ipc_buffer_address: usize,
        /// Page-aligned addresses with translation tables but no mapped
        /// frame.
        pub reserved_pages: &'a [usize],
        /// Initial values for the first four architecture argument registers.
        pub initial_arguments: [usize; 4],
        /// Capabilities explicitly installed in the otherwise empty CSpace.
        pub delegated_capabilities: &'a [DelegatedCapability],
        /// Optional badged route for kernel-delivered task faults.
        pub fault_route: Option<FaultRoute<'a>>,
    }

    /// Parent-owned handles for one constructed seL4 protection domain.
    pub struct Task {
        thread: Thread,
        cspace: CSpace,
        vspace: VSpace,
        private_memory: Frame,
        stack: Frame,
        ipc_buffer: Frame,
        root_slots: Range<usize>,
    }

    impl Task {
        /// Starts this task without affecting any other task.
        pub fn start(&self) -> Result<(), BootstrapError> {
            self.thread.start()
        }

        /// Suspends this task without affecting any other task.
        pub fn suspend(&self) -> Result<(), BootstrapError> {
            self.thread.suspend()
        }

        /// Resumes this task without affecting any other task.
        pub fn resume(&self) -> Result<(), BootstrapError> {
            self.thread.start()
        }

        /// Reads the task register state.
        pub fn read_registers(&self) -> Result<Registers, BootstrapError> {
            self.thread.read_registers()
        }

        /// Writes the task's complete register state without resuming it.
        pub fn write_registers(
            &self,
            registers: &mut Registers,
        ) -> Result<(), BootstrapError> {
            self.thread.write_registers(registers)
        }

        /// Maps a recovery frame into a previously prepared task address.
        ///
        /// # Safety
        ///
        /// The task must be blocked or suspended, `address` must be unmapped,
        /// and no live task reference may cover the page until it is mapped.
        pub unsafe fn map_recovery_frame(
            &self,
            bootstrap: &mut Bootstrap<'_>,
            frame: &Frame,
            address: usize,
        ) -> Result<TaskMapping, BootstrapError> {
            let checkpoint = bootstrap.checkpoint();
            let mapping = match self.vspace.map_frame(
                bootstrap,
                frame,
                address,
                sel4::CapRights::read_write(),
            ) {
                Ok(mapping) => mapping,
                Err(error) => {
                    bootstrap.rollback(checkpoint)?;
                    return Err(error);
                },
            };
            let slots = bootstrap.committed_slots(checkpoint)?;
            let root_slot = slots
                .clone()
                .next()
                .filter(|_| slots.len() == 1)
                .ok_or(BootstrapError::RollbackFailed)?;
            Ok(TaskMapping::new(mapping, root_slot))
        }

        /// Returns the parent-held capability to this task's CSpace root.
        pub fn cspace(&self) -> sel4::cap::CNode {
            self.cspace.root()
        }

        /// Returns the parent-held capability to this task's VSpace root.
        pub fn vspace(&self) -> sel4::cap::VSpace {
            self.vspace.root()
        }

        /// Returns the unmapped owner capability for private writable memory.
        pub fn private_memory(&self) -> &Frame {
            &self.private_memory
        }

        /// Returns the unmapped owner capability for the task's stack page.
        pub fn stack(&self) -> &Frame {
            &self.stack
        }

        /// Returns the unmapped owner capability for the IPC buffer page.
        pub fn ipc_buffer(&self) -> &Frame {
            &self.ipc_buffer
        }

        /// Returns the root CSpace slots retaining this task's resources.
        pub fn root_slots(&self) -> Range<usize> {
            self.root_slots.clone()
        }
    }

    impl Bootstrap<'_> {
        /// Constructs a suspended task, rolling back root slots on failure.
        pub fn create_task(
            &mut self,
            config: TaskConfig<'_>,
        ) -> Result<Task, BootstrapError> {
            let layout = TaskLayout::validate(
                Frame::BYTES,
                config.code.address,
                config.code.entry,
                config.private_memory_address,
                config.stack_address,
                config.ipc_buffer_address,
                config.reserved_pages,
            )
            .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
            let checkpoint = self.checkpoint();
            match self.create_task_inner(config, layout, checkpoint) {
                Ok(task) => Ok(task),
                Err(error) => match self.rollback(checkpoint) {
                    Ok(()) => Err(error),
                    Err(_) => Err(BootstrapError::RollbackFailed),
                },
            }
        }

        /// Performs construction after layout validation establishes safety.
        fn create_task_inner(
            &mut self,
            config: TaskConfig<'_>,
            layout: TaskLayout,
            checkpoint: usize,
        ) -> Result<Task, BootstrapError> {
            let cspace = CSpace::create(
                self,
                config.cspace_size_bits,
                config.delegated_capabilities,
                config.fault_route.as_ref(),
            )?;
            let page_addresses = [
                config.code.address,
                config.private_memory_address,
                config.stack_address,
                config.ipc_buffer_address,
            ];
            let vspace =
                VSpace::create(self, &page_addresses, config.reserved_pages)?;
            vspace.map_frame(
                self,
                config.code.frame,
                config.code.address,
                sel4::CapRights::read_only(),
            )?;

            let private_memory = self.allocate_frame()?;
            vspace.map_frame(
                self,
                &private_memory,
                config.private_memory_address,
                sel4::CapRights::read_write(),
            )?;
            let stack = self.allocate_frame()?;
            vspace.map_frame(
                self,
                &stack,
                config.stack_address,
                sel4::CapRights::read_write(),
            )?;
            let ipc_buffer = self.allocate_frame()?;
            let ipc_buffer_mapping = vspace.map_frame(
                self,
                &ipc_buffer,
                config.ipc_buffer_address,
                sel4::CapRights::read_write(),
            )?;
            let fault_endpoint = match config.fault_route {
                Some(route) => {
                    let bits = route.child_slot.try_into().map_err(|_| {
                        BootstrapError::InvalidTaskConfiguration
                    })?;
                    sel4::CPtr::from_bits(bits)
                },
                None => sel4::init_thread::slot::NULL.cptr(),
            };

            let thread = Thread::create(
                self,
                &cspace,
                &vspace,
                ThreadConfig {
                    ipc_buffer_address: config.ipc_buffer_address,
                    ipc_buffer_mapping,
                    entry: config.code.entry,
                    stack_pointer: layout.stack_pointer,
                    arguments: config.initial_arguments,
                    fault_endpoint,
                },
            )?;
            let root_slots = self.committed_slots(checkpoint)?;
            Ok(Task {
                thread,
                cspace,
                vspace,
                private_memory,
                stack,
                ipc_buffer,
                root_slots,
            })
        }
    }

    const _: () = {
        assert!(Frame::BYTES.is_power_of_two());
        assert!(Frame::BYTES.is_multiple_of(16));
        assert!(core::mem::size_of::<sel4::IpcBuffer>() <= Frame::BYTES);
        assert!(core::mem::align_of::<sel4::IpcBuffer>() <= Frame::BYTES);
    };

    pub use crate::cspace::DelegatedCapability;
}

#[cfg(not(test))]
pub use platform::{DelegatedCapability, SharedCode, Task, TaskConfig};

#[cfg(test)]
mod tests {
    use super::{LayoutError, TaskLayout};

    const PAGE: usize = 4096;

    /// Provides one valid baseline for focused invariant tests.
    fn valid() -> Result<TaskLayout, LayoutError> {
        TaskLayout::validate(PAGE, 0x1000, 0x1000, 0x2000, 0x3000, 0x4000, &[])
    }

    #[test]
    /// Guards the downward-growing stack's exclusive upper bound.
    fn accepts_distinct_aligned_pages_and_computes_stack_top() {
        assert_eq!(valid().map(|layout| layout.stack_pointer), Ok(0x4000));
    }

    #[test]
    /// Rejects layouts the kernel cannot map as base pages.
    fn rejects_invalid_page_size_and_alignment() {
        assert_eq!(
            TaskLayout::validate(
                0,
                0x1000,
                0x1000,
                0x2000,
                0x3000,
                0x4000,
                &[]
            )
            .map(|_| ()),
            Err(LayoutError::InvalidPageSize),
        );
        assert_eq!(
            TaskLayout::validate(
                PAGE,
                0x1001,
                0x1000,
                0x2000,
                0x3000,
                0x4000,
                &[]
            )
            .map(|_| ()),
            Err(LayoutError::UnalignedAddress),
        );
    }

    #[test]
    /// Prevents aliasing and wraparound before kernel invocations.
    fn rejects_overflow_and_overlapping_pages() {
        assert_eq!(
            TaskLayout::validate(
                PAGE,
                usize::MAX - (PAGE - 1),
                usize::MAX - (PAGE - 1),
                0x2000,
                0x3000,
                0x4000,
                &[],
            )
            .map(|_| ()),
            Err(LayoutError::AddressOverflow),
        );
        assert_eq!(
            TaskLayout::validate(
                PAGE,
                0x1000,
                0x1000,
                0x2000,
                0x2000,
                0x4000,
                &[]
            )
            .map(|_| ()),
            Err(LayoutError::OverlappingPages),
        );
    }

    #[test]
    /// Keeps initial execution inside aligned shared code.
    fn rejects_entry_outside_code_or_instruction_alignment() {
        for entry in [0x0ffc, 0x2000, 0x1002] {
            assert_eq!(
                TaskLayout::validate(
                    PAGE,
                    0x1000,
                    entry,
                    0x2000,
                    0x3000,
                    0x4000,
                    &[],
                )
                .map(|_| ()),
                Err(LayoutError::InvalidEntry),
            );
        }
    }

    #[test]
    fn validates_reserved_unmapped_pages() {
        assert!(
            TaskLayout::validate(
                PAGE,
                0x1000,
                0x1000,
                0x2000,
                0x3000,
                0x4000,
                &[0x5000, 0x6000],
            )
            .is_ok()
        );
        for reserved in [&[0x2000][..], &[0x5001][..], &[0x5000, 0x5000][..]] {
            assert!(
                TaskLayout::validate(
                    PAGE, 0x1000, 0x1000, 0x2000, 0x3000, 0x4000, reserved,
                )
                .is_err()
            );
        }
    }
}
