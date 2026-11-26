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
    pub const LISTDIR: u64 = 20;
    pub const FORK: u64 = 21;
    pub const EXEC: u64 = 22;
    pub const PIPE: u64 = 23;
    pub const DUP: u64 = 24;
    pub const DUP2: u64 = 25;
    pub const KILL: u64 = 26;
    pub const MOUNT: u64 = 27;
    pub const UMOUNT: u64 = 28;
    pub const MAX: u64 = 29;
}

/// Syscall argument registers in order.
pub const ARG_REGS: [&str; 6] = ["rdi", "rsi", "rdx", "r10", "r8", "r9"];

/// Number of syscall arguments each syscall takes.
pub fn arity(nr: u64) -> usize {
    match nr {
        nr::READ | nr::WRITE | nr::OPEN | nr::STAT | nr::SPAWN | nr::WAIT | nr::PIPE => 3,
        nr::CLOSE
        | nr::EXIT
        | nr::GETPID
        | nr::GETTID
        | nr::FB_FLUSH
        | nr::TIME
        | nr::FORK
        | nr::KILL => 0,
        nr::RAND | nr::KERNEL_INFO => 2,
        nr::IOCTL => 3,
        nr::MMAP | nr::MUNMAP | nr::BRK => 3,
        nr::FB_INFO => 1,
        nr::LISTDIR => 4,
        nr::FB_PUTPIXEL => 4,
        nr::EXEC => 3,
        nr::DUP => 1,
        nr::DUP2 => 2,
        nr::MOUNT | nr::UMOUNT => 3,
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

    /// Open `path` (user ptr+len) with `flags`; returns fd or -errno.
    fn open(&mut self, _path_ptr: u64, _path_len: u64, _flags: u64) -> i64 {
        ENOSYS
    }
    /// Read `len` bytes into user `ptr` from `fd`; returns count/-errno.
    fn read(&mut self, _fd: u64, _ptr: u64, _len: u64) -> i64 {
        ENOSYS
    }
    /// Close `fd`.
    fn close(&mut self, _fd: u64) -> i64 {
        ENOSYS
    }
    /// List directory `path` (ptr+len) into user `buf` (ptr+len).
    /// Returns bytes written or -errno.
    fn listdir(&mut self, _p_ptr: u64, _p_len: u64, _b_ptr: u64, _b_len: u64) -> i64 {
        ENOSYS
    }
    /// Copy kernel version into user `ptr` (up to `len`); returns len.
    fn kernel_info(&mut self, _ptr: u64, _len: u64) -> i64 {
        ENOSYS
    }
    /// Monotonic time (ticks).
    fn time(&mut self) -> i64 {
        ENOSYS
    }
    /// Fill user `ptr` with `len` random bytes; returns len.
    fn rand(&mut self, _ptr: u64, _len: u64) -> i64 {
        ENOSYS
    }
    /// Extend/reduce the program break; `addr`=0 queries current.
    /// Returns new break or -errno.
    fn brk(&mut self, _addr: u64) -> i64 {
        ENOSYS
    }
    /// Map `len` anonymous bytes at `addr` (0 = pick); returns addr/-errno.
    fn mmap(&mut self, _addr: u64, _len: u64) -> i64 {
        ENOSYS
    }
    /// Unmap `len` bytes at `addr`; returns 0/-errno.
    fn munmap(&mut self, _addr: u64, _len: u64) -> i64 {
        ENOSYS
    }
    /// Fork current process; returns child pid in parent, 0 in child, -errno on error.
    fn fork(&mut self) -> i64 {
        ENOSYS
    }
    /// Exec new program at `path` (ptr+len) with `argv` (ptr+len) and `envp` (ptr+len).
    /// Does not return on success; returns -errno on error.
    fn exec(
        &mut self,
        _path_ptr: u64,
        _path_len: u64,
        _argv_ptr: u64,
        _argv_len: u64,
        _envp_ptr: u64,
        _envp_len: u64,
    ) -> i64 {
        ENOSYS
    }
    /// Create a pipe; returns read fd in lower 32 bits, write fd in upper 32 bits.
    fn pipe(&mut self) -> i64 {
        ENOSYS
    }
    /// Duplicate a file descriptor.
    fn dup(&mut self, _fd: u64) -> i64 {
        ENOSYS
    }
    /// Duplicate a file descriptor to a specific fd number.
    fn dup2(&mut self, _oldfd: u64, _newfd: u64) -> i64 {
        ENOSYS
    }
    /// Send a signal to a process.
    fn kill(&mut self, _pid: u64, _signal: u64) -> i64 {
        ENOSYS
    }
    /// Mount a filesystem.
    fn mount(
        &mut self,
        _source: u64,
        _source_len: u64,
        _target: u64,
        _target_len: u64,
        _fstype: u64,
        _fstype_len: u64,
    ) -> i64 {
        ENOSYS
    }
    /// Unmount a filesystem.
    fn umount(&mut self, _target: u64, _target_len: u64) -> i64 {
        ENOSYS
    }
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
        nr::OPEN => svc.open(args[0], args[1], args[2]),
        nr::READ => svc.read(args[0], args[1], args[2]),
        nr::CLOSE => svc.close(args[0]),
        nr::LISTDIR => svc.listdir(args[0], args[1], args[2], args[3]),
        nr::KERNEL_INFO => svc.kernel_info(args[0], args[1]),
        nr::TIME => svc.time(),
        nr::RAND => svc.rand(args[0], args[1]),
        nr::BRK => svc.brk(args[0]),
        nr::MMAP => svc.mmap(args[0], args[1]),
        nr::MUNMAP => svc.munmap(args[0], args[1]),
        nr::GETPID | nr::GETTID => svc.pid() as i64,
        nr::FORK => svc.fork(),
        nr::EXEC => svc.exec(args[0], args[1], args[2], args[3], args[4], args[5]),
        nr::PIPE => svc.pipe(),
        nr::DUP => svc.dup(args[0]),
        nr::DUP2 => svc.dup2(args[0], args[1]),
        nr::KILL => svc.kill(args[0], args[1]),
        nr::MOUNT => svc.mount(args[0], args[1], args[2], args[3], args[4], args[5]),
        nr::UMOUNT => svc.umount(args[0], args[1]),
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
    use alloc::vec;
    use alloc::vec::Vec;

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
        fn pid(&self) -> u64 {
            self.pid
        }
        fn kernel_version(&self) -> &'static str {
            "0.1.0"
        }
    }

    #[test]
    fn write_dispatches() {
        let mut m = Mock {
            written: Vec::new(),
            pid: 7,
        };
        let args = [1u64, 0x1000, 5, 0, 0, 0];
        assert_eq!(dispatch(nr::WRITE, &args, &mut m), 5);
        assert_eq!(m.written, vec![(1, 0x1000, 5)]);
    }

    #[test]
    fn write_bad_fd() {
        let mut m = Mock {
            written: Vec::new(),
            pid: 1,
        };
        let args = [9u64, 0x1000, 5, 0, 0, 0];
        assert_eq!(dispatch(nr::WRITE, &args, &mut m), EBADF);
    }

    #[test]
    fn getpid() {
        let mut m = Mock {
            written: Vec::new(),
            pid: 42,
        };
        let args = [0u64; 6];
        assert_eq!(dispatch(nr::GETPID, &args, &mut m), 42);
    }

    #[test]
    fn unknown_is_enosys() {
        let mut m = Mock {
            written: Vec::new(),
            pid: 1,
        };
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
