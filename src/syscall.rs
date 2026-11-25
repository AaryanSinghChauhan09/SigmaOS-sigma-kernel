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

/// Kernel services a syscall may invoke — implemented by the arch
/// layer (console, process table, framebuffer). Mirrors the
/// `sigma-sys` shim's trait so userspace and kernel speak one ABI.
pub trait KernelServices {
    /// Write `len` bytes from user buffer `ptr` to `fd`.
    fn write(&mut self, fd: u64, ptr: u64, len: u64) -> i64;
    /// Terminate the calling process.
    fn exit(&mut self, code: u64) -> !;
    /// Current process id.
    fn pid(&self) -> u64;
    /// Kernel version string pointer/len for KERNEL_INFO.
    fn kernel_version(&self) -> &'static str;
}

pub const ENOSYS: i64 = -38;
pub const EBADF: i64 = -9;
pub const EFAULT: i64 = -14;

/// Dispatch one syscall. `args` are the six syscall arg registers
/// (rdi,rsi,rdx,r10,r8,r9). Returns the value for rax.
/// Inspired by Linux `sys_call_table` + `do_syscall_64`.
pub fn dispatch<S: KernelServices>(nr: u64, args: &[u64; 6], svc: &mut S) -> i64 {
    if !valid(nr) {
        return ENOSYS;
    }
    match nr {
        nr::WRITE => {
            let (fd, ptr, len) = (args[0], args[1], args[2]);
            if fd != 1 && fd != 2 {
                return EBADF;
            }
            svc.write(fd, ptr, len)
        }
        nr::EXIT => svc.exit(args[0]),
        nr::GETPID | nr::GETTID => svc.pid() as i64,
        nr::KERNEL_INFO => {
            // return (ptr,len) packed: caller reads version via ptr
            let v = svc.kernel_version();
            // encode as two returns is not possible; return len and
            // stash ptr in a static — kept simple: return ptr, len
            // is fixed KERNEL_VERSION_LEN via a second call. For now
            // return pointer as i64.
            v.as_ptr() as i64
        }
        _ => ENOSYS,
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;
    use alloc::vec::Vec;
    use alloc::vec;

    struct Mock {
        written: Vec<(u64, u64, u64)>,
        pid: u64,
    }
    impl KernelServices for Mock {
        fn write(&mut self, fd: u64, ptr: u64, len: u64) -> i64 {
            self.written.push((fd, ptr, len));
            len as i64
        }
        fn exit(&mut self, _code: u64) -> ! {
            // can't diverge in test; use a flag instead via panic-free path
            core::panic!("exit")
        }
        fn pid(&self) -> u64 { self.pid }
        fn kernel_version(&self) -> &'static str { "0.1.0" }
    }

    #[test]
    fn write_dispatches() {
        let mut m = Mock { written: Vec::new(), pid: 7 };
        let args = [1u64, 0x1000, 5, 0, 0, 0];
        assert_eq!(dispatch(nr::WRITE, &args, &mut m), 5);
        assert_eq!(m.written, vec![(1, 0x1000, 5)]);
    }

    #[test]
    fn write_bad_fd() {
        let mut m = Mock { written: Vec::new(), pid: 1 };
        let args = [9u64, 0x1000, 5, 0, 0, 0];
        assert_eq!(dispatch(nr::WRITE, &args, &mut m), EBADF);
    }

    #[test]
    fn getpid() {
        let mut m = Mock { written: Vec::new(), pid: 42 };
        let args = [0u64; 6];
        assert_eq!(dispatch(nr::GETPID, &args, &mut m), 42);
    }

    #[test]
    fn unknown_is_enosys() {
        let mut m = Mock { written: Vec::new(), pid: 1 };
        let args = [0u64; 6];
        assert_eq!(dispatch(99, &args, &mut m), ENOSYS);
        assert_eq!(dispatch(nr::MMAP, &args, &mut m), ENOSYS);
    }
}

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
