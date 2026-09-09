//! TCB configuration and initial register state.

use crate::boot::Bootstrap;
use crate::cspace::CSpace;
use crate::errors::BootstrapError;
use crate::vspace::VSpace;

pub(crate) struct Thread {
    tcb: sel4::cap::Tcb,
}

pub(crate) struct ThreadConfig {
    pub(crate) ipc_buffer_address: usize,
    pub(crate) ipc_buffer_mapping: sel4::cap::Granule,
    pub(crate) entry: usize,
    pub(crate) stack_pointer: usize,
    pub(crate) arguments: [usize; 4],
}

impl Thread {
    /// Configures a suspended TCB with validated space and register state.
    pub(crate) fn create(
        bootstrap: &mut Bootstrap<'_>,
        cspace: &CSpace,
        vspace: &VSpace,
        config: ThreadConfig,
    ) -> Result<Self, BootstrapError> {
        let ipc_buffer_address = config
            .ipc_buffer_address
            .try_into()
            .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
        let entry = config
            .entry
            .try_into()
            .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
        let stack_pointer = config
            .stack_pointer
            .try_into()
            .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
        let tcb = bootstrap.allocate_object::<sel4::cap_type::Tcb>()?;
        tcb.tcb_configure(
            sel4::init_thread::slot::NULL.cptr(),
            cspace.root(),
            cspace.root_data(),
            vspace.root(),
            ipc_buffer_address,
            config.ipc_buffer_mapping,
        )
        .map_err(|_| BootstrapError::ThreadConfigurationFailed)?;

        let mut registers = sel4::UserContext::default();
        *registers.pc_mut() = entry;
        *registers.sp_mut() = stack_pointer;
        for (index, argument) in config.arguments.into_iter().enumerate() {
            *registers.gpr_mut(index) = argument
                .try_into()
                .map_err(|_| BootstrapError::InvalidTaskConfiguration)?;
        }
        tcb.tcb_write_all_registers(false, &mut registers)
            .map_err(|_| BootstrapError::ThreadConfigurationFailed)?;
        Ok(Self { tcb })
    }

    /// Makes only this configured TCB runnable.
    pub(crate) fn start(&self) -> Result<(), BootstrapError> {
        self.tcb
            .tcb_resume()
            .map_err(|_| BootstrapError::ThreadControlFailed)
    }

    /// Stops only this TCB before parent-side memory inspection.
    pub(crate) fn suspend(&self) -> Result<(), BootstrapError> {
        self.tcb
            .tcb_suspend()
            .map_err(|_| BootstrapError::ThreadControlFailed)
    }
}
