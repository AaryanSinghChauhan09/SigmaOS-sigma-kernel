//! Syscall ABI — numbers and calling convention for SigmaOS.
//!
//! x86_64 System V-ish: number in rax, args rdi/rsi/rdx/r10/r8/r9,
//! return in rax (negative = errno-style code from KernelError::code).

/// Syscall numbers — stable ABI, append-only.
pub mod nr {
    pub const READ: u64 = 0;
    pub const WRITE: u64 = 1;
    pub const OPEN: u64 = 2;
    pub const CLOSE: u64 = 3;
    pub const STAT: u64 = 4;
    pub const IOCTL: u64 = 5;
    pub const MMAP: u64 = 6;
    pub const MUNMAP: u64 = 7;
    pub const BRK: u64 = 8;
    pub const EXIT: u64 = 9;
    pub const SPAWN: u64 = 10;
    pub const WAIT: u64 = 11;
    pub const GETPID: u64 = 12;
    pub const GETTID: u64 = 13;
    pub const KERNEL_INFO: u64 = 14;
    pub const FB_INFO: u64 = 15;
    pub const FB_PUTPIXEL: u64 = 16;
    pub const FB_FLUSH: u64 = 17;
    pub const TIME: u64 = 18;
    pub const RAND: u64 = 19;
    pub const MAX: u64 = 20;
}

/// Syscall argument registers in order.
pub const ARG_REGS: [&str; 6] = ["rdi", "rsi", "rdx", "r10", "r8", "r9"];

/// Number of syscall arguments each syscall takes.
pub fn arity(nr: u64) -> usize {
    match nr {
        nr::READ | nr::WRITE | nr::OPEN | nr::STAT | nr::SPAWN | nr::WAIT => 3,
        nr::CLOSE | nr::EXIT | nr::GETPID | nr::GETTID | nr::FB_FLUSH | nr::TIME
        | nr::RAND | nr::KERNEL_INFO => 0,
        nr::IOCTL => 3,
        nr::MMAP | nr::MUNMAP | nr::BRK => 3,
        nr::FB_INFO => 1,
        nr::FB_PUTPIXEL => 4,
        _ => 0,
    }
}

/// Validate a syscall number.
pub fn valid(nr: u64) -> bool {
    nr < nr::MAX
}

/// Dispatch table entry — filled in by kernel init.
/// Kept as a function pointer type for the syscall handler.
pub type SyscallFn = fn(&[u64]) -> i64;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_unique_and_bounded() {
        assert!(valid(nr::READ));
        assert!(valid(nr::MAX - 1));
        assert!(!valid(nr::MAX));
        assert!(!valid(u64::MAX));
    }

    #[test]
    fn arities() {
        assert_eq!(arity(nr::READ), 3);
        assert_eq!(arity(nr::EXIT), 0);
        assert_eq!(arity(nr::FB_PUTPIXEL), 4);
    }
}
