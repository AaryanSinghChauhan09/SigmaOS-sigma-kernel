//! Kernel error type — no_std compatible.

/// Errors produced by kernel subsystems.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelError {
    /// Boot information missing or malformed.
    BootInfo(&'static str),
    /// Memory allocation failure.
    OutOfMemory,
    /// Invalid argument passed to a subsystem.
    InvalidArg(&'static str),
    /// Hardware not present or not initialized.
    HardwareMissing(&'static str),
    /// Filesystem/archive error.
    Fs(&'static str),
    /// Driver error.
    Driver(&'static str),
}

impl KernelError {
    /// Stable numeric code for syscall returns.
    pub fn code(&self) -> i64 {
        match self {
            KernelError::BootInfo(_) => -1,
            KernelError::OutOfMemory => -2,
            KernelError::InvalidArg(_) => -3,
            KernelError::HardwareMissing(_) => -4,
            KernelError::Fs(_) => -5,
            KernelError::Driver(_) => -6,
        }
    }
}
