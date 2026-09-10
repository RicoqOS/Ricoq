//! TCB configuration and initial register state.

use crate::boot::Bootstrap;
use crate::cspace::CSpace;
use crate::errors::BootstrapError;
use crate::vspace::VSpace;

pub(crate) struct Thread {
    tcb: sel4::cap::Tcb,
}

/// A task user-register context.
pub struct Registers(sel4::UserContext);

impl Registers {
    /// Returns the current program counter.
    pub fn instruction_pointer(&self) -> u64 {
        *self.0.pc()
    }

    /// Updates the program counter for the next register-state write.
    pub fn set_instruction_pointer(&mut self, value: u64) {
        *self.0.pc_mut() = value;
    }

    /// Returns the current stack pointer.
    pub fn stack_pointer(&self) -> u64 {
        *self.0.sp()
    }

    /// Updates the stack pointer for the next register-state write.
    pub fn set_stack_pointer(&mut self, value: u64) {
        *self.0.sp_mut() = value;
    }

    /// Returns general-purpose register X0 through X30.
    pub fn general_register(
        &self,
        index: usize,
    ) -> Result<u64, BootstrapError> {
        if index >= 31 {
            return Err(BootstrapError::InvalidRegister);
        }
        Ok(*self.0.gpr(index))
    }

    /// Updates general-purpose register X0 through X30.
    pub fn set_general_register(
        &mut self,
        index: usize,
        value: u64,
    ) -> Result<(), BootstrapError> {
        if index >= 31 {
            return Err(BootstrapError::InvalidRegister);
        }
        *self.0.gpr_mut(index) = value;
        Ok(())
    }
}

pub(crate) struct ThreadConfig {
    pub(crate) ipc_buffer_address: usize,
    pub(crate) ipc_buffer_mapping: sel4::cap::Granule,
    pub(crate) entry: usize,
    pub(crate) stack_pointer: usize,
    pub(crate) arguments: [usize; 4],
    pub(crate) fault_endpoint: sel4::CPtr,
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
            config.fault_endpoint,
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

    /// Reads all registers.
    pub(crate) fn read_registers(&self) -> Result<Registers, BootstrapError> {
        self.tcb
            .tcb_read_all_registers(false)
            .map(Registers)
            .map_err(|_| BootstrapError::RegisterReadFailed)
    }

    /// Writes all registers.
    pub(crate) fn write_registers(
        &self,
        registers: &mut Registers,
    ) -> Result<(), BootstrapError> {
        self.tcb
            .tcb_write_all_registers(false, &mut registers.0)
            .map_err(|_| BootstrapError::RegisterWriteFailed)
    }
}
