//! x86_64 arch shims for the kernel entry — port I/O, serial log,
//! halt, PS/2 keyboard, and VGA text buffer. Isolated here so the rest
//! of the kernel stays `forbid(unsafe)`.

const COM1: u16 = 0x3f8;
const VGA_TEXT: usize = 0xB8000;

fn outb(port: u16, val: u8) {
    // SAFETY: port I/O to the standard COM1 UART.
    unsafe {
        core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nostack));
    }
}

fn inb(port: u16) -> u8 {
    let v: u8;
    // SAFETY: port I/O read from COM1 line-status register.
    unsafe {
        core::arch::asm!("in al, dx", out("al") v, in("dx") port, options(nomem, nostack));
    }
    v
}

/// Initialize COM1: 115200 8N1.
pub fn init() {
    outb(COM1 + 1, 0x00); // disable interrupts
    outb(COM1 + 3, 0x80); // DLAB
    outb(COM1 + 0, 0x01); // divisor low 1
    outb(COM1 + 1, 0x00); // divisor high 0
    outb(COM1 + 3, 0x03); // 8N1
    outb(COM1 + 2, 0xC7); // FIFO enable + clear
    outb(COM1 + 4, 0x0B); // DTR/RTS/OUT2
}

/// Write a byte to COM1 (polling THR-empty).
fn putc(c: u8) {
    // wait for THR empty (bit 5 of LSR)
    while inb(COM1 + 5) & 0x20 == 0 {}
    outb(COM1, c);
}

/// Log a string to the serial console.
pub fn log(s: &str) {
    for &b in s.as_bytes() {
        putc(b);
    }
}

/// Halt the CPU until the next interrupt.
pub fn halt() {
    // SAFETY: cli+hlt is the standard idle sequence.
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        core::arch::asm!("hlt", options(nomem, nostack));
    }
}

/// Reload CR3 to flush the TLB and paging-structure caches.
/// Required after installing new page-table entries: a prior walk
/// may have cached a non-present translation (Linux does the same
/// via flush_tlb_* after pte_install).
pub fn flush_tlb() {
    // SAFETY: mov cr3,rAX;mov cr3,rAX is the canonical full flush.
    unsafe {
        core::arch::asm!(
            "mov rax, cr3",
            "mov cr3, rax",
            out("rax") _,
            options(nostack, preserves_flags),
        );
    }
}

/// Read %eax and %rbx/%ebx at entry.
///
#[allow(dead_code)]
pub fn boot_regs() -> (u32, u64) {
    let eax: u32;
    let rbx: u64;
    // SAFETY: reading argument registers at kernel entry.
    unsafe {
        core::arch::asm!(
            "mov {0:e}, eax",
            "mov {1}, rbx",
            out(reg) eax,
            out(reg) rbx,
            options(nomem, nostack, preserves_flags),
        );
    }
    (eax, rbx)
}

/* ---- PS/2 keyboard port ---- */

/// i8042 port pair at 0x60/0x64.
pub struct Ps2Hw;

impl kernel::ps2::Ps2Port for Ps2Hw {
    fn read(&self, port: u16) -> u8 {
        inb(port)
    }
    fn write(&mut self, port: u16, value: u8) {
        outb(port, value);
    }
}

/* ---- VGA text buffer ---- */

/// Memory-mapped VGA text cells at 0xB8000.
pub struct VgaBuf;

impl kernel::console::ScreenBuf for VgaBuf {
    fn put(&mut self, index: usize, cell: u16) {
        // SAFETY: 0xB8000 is the VGA text framebuffer, always mapped;
        // index < CELLS keeps writes inside the 80x25 region.
        unsafe {
            ((VGA_TEXT + index * 2) as *mut u16).write_volatile(cell);
        }
    }
    fn get(&self, index: usize) -> u16 {
        // SAFETY: same mapping argument as `put`.
        unsafe { ((VGA_TEXT + index * 2) as *const u16).read_volatile() }
    }
    fn len(&self) -> usize {
        kernel::console::CELLS
    }
}

/* ---- Physical memory access for usermem page tables ---- */

/// Identity-mapped physical memory — the kernel maps phys==virt in
/// the low 1 GiB, so a phys addr is directly dereferenceable.
pub struct IdentityPhysMem;

impl kernel::usermem::PhysMem for IdentityPhysMem {
    fn write_u64(&mut self, phys: u64, val: u64) {
        if phys >= 0x4000_0000 {
            log(&alloc::format!(
                "physmem: bad write phys={:#x} val={:#x}\n",
                phys,
                val
            ));
            return;
        }
        // SAFETY: low-1GiB identity map covers all frame addresses we
        // hand out; phys is 4K-aligned from the frame allocator.
        unsafe { (phys as *mut u64).write_volatile(val) }
    }
    fn read_u64(&self, phys: u64) -> u64 {
        if phys >= 0x4000_0000 {
            log(&alloc::format!("physmem: bad read phys={:#x}\n", phys));
            return 0;
        }
        // SAFETY: same identity-map argument as write_u64.
        unsafe { (phys as *const u64).read_volatile() }
    }
}

/* ---- KernelServices for syscall dispatch ---- */

/// Bridges syscall::dispatch to real kernel services: console/serial
/// output, pid, kernel version. `write` copies from the user buffer
/// (identity-mapped in the user CR3) to serial+VGA.
pub struct KernelSvc {
    pub con: &'static mut kernel::console::TextConsole<VgaBuf>,
}

impl kernel::syscall::KernelServices for KernelSvc {
    fn write(&mut self, fd: u64, ptr: u64, len: u64) -> i64 {
        if fd != 1 && fd != 2 {
            return -9; // EBADF
        }
        // SAFETY: ptr/len come from a user process whose pages are
        // mapped in the active CR3; we trust the syscall ABI contract.
        let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
        for &b in bytes {
            if b == b'\n' {
                self.con.newline_pub();
            } else {
                self.con.put_byte(b);
            }
            // serial echo
            let mut tmp = [0u8; 1];
            tmp[0] = b;
            log(core::str::from_utf8(&tmp).unwrap_or("?"));
        }
        len as i64
    }
    fn exit(&mut self, code: u64) -> ! {
        log("sigma-kernel: user process exited\n");
        self.con.println(&alloc::format!("process exit {}", code));
        loop {
            core::hint::spin_loop()
        }
    }
    fn pid(&self) -> u64 {
        1 // single-process kernel for now
    }
    fn kernel_version(&self) -> &'static str {
        kernel::KERNEL_VERSION
    }
    fn fork(&mut self) -> i64 {
        // Simplified fork: return ENOSYS for now
        // Full implementation requires:
        // 1. Copy current process's page tables (COW if possible)
        // 2. Allocate new process slot
        // 3. Copy register state
        // 4. Return child pid in parent, 0 in child
        -38 // ENOSYS
    }
    fn exec(
        &mut self,
        _path_ptr: u64,
        _path_len: u64,
        _argv_ptr: u64,
        _argv_len: u64,
        _envp_ptr: u64,
        _envp_len: u64,
    ) -> i64 {
        // Simplified exec: return ENOSYS for now
        // Full implementation requires:
        // 1. Load ELF from path
        // 2. Replace current address space
        // 3. Set up new stack and argv/envp
        // 4. Jump to new entry point
        -38 // ENOSYS
    }
}

/// Minimal KernelServices for the syscall trap path — serial-only
/// output so it needs no console global. Used by syscall_trap.
pub struct TrapSvc;

impl kernel::syscall::KernelServices for TrapSvc {
    fn write(&mut self, fd: u64, ptr: u64, len: u64) -> i64 {
        // stdout/stderr redirect: a process spawned with an output fd
        // sends fd 1/2 writes to that fd (file or pipe) instead of
        // the console.
        let mut fd = fd;
        if fd == 1 || fd == 2 {
            let out = unsafe { crate::PROCS[crate::CUR_PROC].stdout_fd };
            if out >= 3 {
                fd = out;
            }
        }
        if fd != 1 && fd != 2 {
            // Check if it's a pipe write fd or a writable file fd.
            if let Some(f) = unsafe { crate::fd_get(fd) } {
                if f.kind == crate::FD_PIPE_WRITE {
                    return self.sys_pipe_write(f.pipe_id as usize, ptr, len);
                }
                if f.kind == crate::FD_FILE && f.write {
                    return self.sys_file_write(fd, ptr, len);
                }
            }
            return -9; // EBADF
        }
        // SAFETY: user pages are mapped under the active (user) CR3.
        let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
        for &b in bytes {
            if b == b'\n' {
                log("\n");
                // SAFETY: console global initialized at boot.
                unsafe { crate::sys_con().newline_pub() };
            } else if (0x20..0x7f).contains(&b) {
                let mut c = [0u8; 1];
                c[0] = b;
                log(core::str::from_utf8(&c).unwrap());
                unsafe { crate::sys_con().put_byte(b) };
            }
        }
        len as i64
    }
    fn exit(&mut self, code: u64) -> ! {
        log("sigma-kernel: user exit\n");
        loop {
            core::hint::spin_loop()
        }
    }
    fn pid(&self) -> u64 {
        // SAFETY: single-threaded syscall context.
        unsafe { crate::PROCS[crate::CUR_PROC].pid }
    }
    fn kernel_version(&self) -> &'static str {
        kernel::KERNEL_VERSION
    }
    fn open(&mut self, path_ptr: u64, path_len: u64, flags: u64) -> i64 {
        self.sys_open(path_ptr, path_len, flags)
    }
    fn read(&mut self, fd: u64, ptr: u64, len: u64) -> i64 {
        self.sys_read(fd, ptr, len)
    }
    fn close(&mut self, fd: u64) -> i64 {
        self.sys_close(fd)
    }
    fn listdir(&mut self, p_ptr: u64, p_len: u64, b_ptr: u64, b_len: u64) -> i64 {
        self.sys_listdir(p_ptr, p_len, b_ptr, b_len)
    }
    fn stat(&mut self, p_ptr: u64, p_len: u64, b_ptr: u64, b_len: u64) -> i64 {
        self.sys_stat(p_ptr, p_len, b_ptr, b_len)
    }
    fn kernel_info(&mut self, ptr: u64, len: u64) -> i64 {
        let v = kernel::KERNEL_VERSION.as_bytes();
        let n = (v.len()).min(len as usize);
        // SAFETY: user buf mapped under active CR3.
        unsafe {
            core::ptr::copy_nonoverlapping(v.as_ptr(), ptr as *mut u8, n);
        }
        n as i64
    }
    fn fork(&mut self) -> i64 {
        // Simplified fork: return ENOSYS for now
        // Full implementation requires:
        // 1. Copy current process's page tables (COW if possible)
        // 2. Allocate new process slot
        // 3. Copy register state
        // 4. Return child pid in parent, 0 in child
        -38 // ENOSYS
    }
    fn exec(
        &mut self,
        _path_ptr: u64,
        _path_len: u64,
        _argv_ptr: u64,
        _argv_len: u64,
        _envp_ptr: u64,
        _envp_len: u64,
    ) -> i64 {
        // Simplified exec: return ENOSYS for now
        // Full implementation requires:
        // 1. Load ELF from path
        // 2. Replace current address space
        // 3. Set up new stack and argv/envp
        // 4. Jump to new entry point
        -38 // ENOSYS
    }
    fn pipe(&mut self) -> i64 {
        self.sys_pipe()
    }
    fn dup(&mut self, fd: u64) -> i64 {
        self.sys_dup(fd)
    }
    fn dup2(&mut self, oldfd: u64, newfd: u64) -> i64 {
        self.sys_dup2(oldfd, newfd)
    }
    fn kill(&mut self, pid: u64, signal: u64) -> i64 {
        self.sys_kill(pid, signal)
    }
    fn mount(
        &mut self,
        _source: u64,
        _source_len: u64,
        _target: u64,
        _target_len: u64,
        _fstype: u64,
        _fstype_len: u64,
    ) -> i64 {
        -38 // ENOSYS - requires VFS mount support
    }
    fn umount(&mut self, _target: u64, _target_len: u64) -> i64 {
        -38 // ENOSYS - requires VFS mount support
    }
    fn mkdir(&mut self, p_ptr: u64, p_len: u64) -> i64 {
        self.sys_mkdir(p_ptr, p_len)
    }
    fn unlink(&mut self, p_ptr: u64, p_len: u64) -> i64 {
        self.sys_unlink(p_ptr, p_len)
    }
    fn rmdir(&mut self, p_ptr: u64, p_len: u64) -> i64 {
        self.sys_rmdir(p_ptr, p_len)
    }
    fn chdir(&mut self, p_ptr: u64, p_len: u64) -> i64 {
        self.sys_chdir(p_ptr, p_len)
    }
    fn getcwd(&mut self, b_ptr: u64, b_len: u64) -> i64 {
        self.sys_getcwd(b_ptr, b_len)
    }
    fn brk(&mut self, addr: u64) -> i64 {
        const PAGE: u64 = 4096;
        const ENOMEM: i64 = -12;
        const EINVAL: i64 = -22;
        unsafe {
            let cur = crate::USER_BRK;
            if addr == 0 {
                return cur as i64;
            }
            if addr < cur {
                // Shrink: just move the break; pages stay mapped.
                crate::USER_BRK = addr;
                return addr as i64;
            }
            if addr - cur > 64 << 20 {
                return EINVAL; // refuse >64MiB single grow
            }
            let pml4 = crate::USER_PML4;
            let end = (addr + PAGE - 1) & !(PAGE - 1);
            let mut v = cur & !(PAGE - 1);
            while v < end {
                let phys = match crate::USER_ALLOC.as_mut().and_then(|a| a.alloc_frame()) {
                    Some(p) => p,
                    None => return ENOMEM,
                };
                // SAFETY: phys is a freshly allocated frame we own;
                // identity-mapped, so direct zeroing is valid.
                unsafe { core::ptr::write_bytes(phys as *mut u8, 0, 4096) };
                let r = kernel::usermem::map_user_page(
                    pml4,
                    v,
                    phys,
                    crate::USER_ALLOC.as_mut().unwrap(),
                    &mut IdentityPhysMem,
                );
                if r.is_err() {
                    return ENOMEM;
                }
                v += PAGE;
            }
            crate::USER_BRK = end;
            flush_tlb();
            end as i64
        }
    }
    fn time(&mut self) -> i64 {
        rdtsc() as i64
    }
    fn rand(&mut self, ptr: u64, len: u64) -> i64 {
        // xorshift64* PRNG seeded from rdtsc
        let mut x = unsafe { RSEED };
        if x == 0 {
            x = rdtsc() | 1;
        }
        let n = (len as usize).min(1 << 20);
        // SAFETY: user buf mapped; n bounded.
        unsafe {
            let out = core::slice::from_raw_parts_mut(ptr as *mut u8, n);
            for b in out.iter_mut() {
                x ^= x >> 12;
                x ^= x << 25;
                x ^= x >> 27;
                unsafe { RSEED = x };
                *b = (x.wrapping_mul(0x2545F4914F6CDD1D) >> 56) as u8;
            }
        }
        n as i64
    }

    fn mmap(&mut self, addr: u64, len: u64) -> i64 {
        const PAGE: u64 = 4096;
        const ENOMEM: i64 = -12;
        const EINVAL: i64 = -22;
        if len == 0 || len > (256 << 20) {
            return EINVAL;
        }
        unsafe {
            // addr==0: allocate from a private mmap arena just above brk.
            let base = if addr == 0 {
                let b = crate::USER_BRK;
                crate::USER_BRK = (b + len + PAGE - 1) & !(PAGE - 1);
                b & !(PAGE - 1)
            } else {
                if addr & (PAGE - 1) != 0 {
                    return EINVAL;
                }
                addr
            };
            let pml4 = crate::USER_PML4;
            let end = (base + len + PAGE - 1) & !(PAGE - 1);
            let mut v = base;
            while v < end {
                let phys = match crate::USER_ALLOC.as_mut().and_then(|a| a.alloc_frame()) {
                    Some(p) => p,
                    None => return ENOMEM,
                };
                // SAFETY: phys is a freshly allocated frame we own;
                // identity-mapped, so direct zeroing is valid.
                unsafe { core::ptr::write_bytes(phys as *mut u8, 0, 4096) };
                let r = kernel::usermem::map_user_page(
                    pml4,
                    v,
                    phys,
                    crate::USER_ALLOC.as_mut().unwrap(),
                    &mut IdentityPhysMem,
                );
                if r.is_err() {
                    return ENOMEM;
                }
                v += PAGE;
            }
            flush_tlb();
            base as i64
        }
    }

    /* MUNMAP: clear present U/S leaf PTEs and return their frames
     * to the allocator. Supervisor/kernel pages are never touched —
     * user_pte only yields 4K leaves and we additionally require
     * P_USER before freeing (Linux unmap_region semantics). */
    fn munmap(&mut self, addr: u64, len: u64) -> i64 {
        const PAGE: u64 = 4096;
        const EINVAL: i64 = -22;
        if len == 0 || addr & (PAGE - 1) != 0 {
            return EINVAL;
        }
        let end = match addr.checked_add(len) {
            Some(e) => e,
            None => return EINVAL,
        };
        if end > (512 << 20) {
            return EINVAL; // stay inside the low user half we manage
        }
        unsafe {
            let pml4 = crate::USER_PML4;
            let mut v = addr;
            while v < end {
                if let Some((pte, e)) = kernel::usermem::user_pte(pml4, v, &IdentityPhysMem) {
                    if e & kernel::usermem::P_USER != 0 {
                        kernel::usermem::PhysMem::write_u64(&mut IdentityPhysMem, pte, 0);
                        if let Some(a) = crate::USER_ALLOC.as_mut() {
                            a.free_frame(e & !0xfff);
                        }
                    }
                }
                v += PAGE;
            }
            flush_tlb();
        }
        0
    }
}

static mut RSEED: u64 = 0;

/// Read the TSC (timestamp counter).
pub fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    // SAFETY: rdtsc is always available on x86_64.
    unsafe {
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack));
    }
    ((hi as u64) << 32) | lo as u64
}

/* ---- file syscalls for TrapSvc (fd table now per-process in main.rs) ---- */

/// Global PS/2 keyboard for the syscall input path.
static mut KBD: Option<kernel::ps2::Keyboard<Ps2Hw>> = None;

fn kbd() -> &'static mut kernel::ps2::Keyboard<Ps2Hw> {
    // SAFETY: single-threaded syscall context; lazy init once.
    unsafe {
        if KBD.is_none() {
            KBD = Some(kernel::ps2::Keyboard::attached(Ps2Hw));
        }
        KBD.as_mut().unwrap()
    }
}

/// Read one key as a byte (Char->ascii, Enter->'\n', Backspace->0x7f).
/// None if no key pending.
fn kbd_read_byte() -> Option<u8> {
    let k = kbd();
    k.poll();
    for ev in k.drain() {
        match ev {
            kernel::ps2::Key::Char(c) => return Some(c),
            kernel::ps2::Key::Enter => return Some(b'\n'),
            kernel::ps2::Key::Backspace => return Some(0x7f),
            _ => {}
        }
    }
    None
}

/// Read one byte from the 16550 RX (nonblocking). None if empty.
pub fn serial_read_byte() -> Option<u8> {
    // LSR bit 0 = data ready
    if inb(0x3fd) & 1 == 0 {
        return None;
    }
    Some(inb(0x3f8))
}

/// Decode a (ptr,len) user path argument. The result borrows user
/// memory and is only valid for the duration of the syscall.
/// SAFETY: caller guarantees `ptr..ptr+len` is mapped under the
/// active user CR3.
unsafe fn user_path(ptr: u64, len: u64) -> Result<&'static str, i64> {
    if len == 0 || len > 64 {
        return Err(-22); // EINVAL
    }
    let b = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
    core::str::from_utf8(b).map_err(|_| -14) // EFAULT
}

/// Map a VFS error to an errno-style code.
fn fs_err(e: kernel::KernelError) -> i64 {
    match e {
        kernel::KernelError::Fs(m) if m.contains("exists") => -17, // EEXIST
        kernel::KernelError::Fs(m) if m.contains("no such") => -2, // ENOENT
        kernel::KernelError::Fs(m) if m.contains("not empty") => -39, // ENOTEMPTY
        kernel::KernelError::Fs(m) if m.contains("not a directory") => -20, // ENOTDIR
        kernel::KernelError::Fs(m) if m.contains("is a directory") => -21, // EISDIR
        _ => -5,                                                   // EIO
    }
}

impl TrapSvc {
    /// open(path_ptr,path_len,flags): copy path, check VFS, alloc fd.
    fn sys_open(&mut self, path_ptr: u64, path_len: u64, flags: u64) -> i64 {
        if path_len == 0 || path_len > 64 {
            return -22; // EINVAL
        }
        // Linux open(2) flag bits we honor.
        const O_ACCMODE: u64 = 3;
        const O_CREAT: u64 = 0x40;
        const O_TRUNC: u64 = 0x200;
        const O_APPEND: u64 = 0x400;
        let write = flags & O_ACCMODE != 0;
        // SAFETY: user path buffer is mapped under active CR3.
        let bytes =
            unsafe { core::slice::from_raw_parts(path_ptr as *const u8, path_len as usize) };
        let path = match core::str::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => return -14, // EFAULT
        };
        let exists = unsafe { crate::sys_vfs() }.exists(path);
        if !exists {
            if flags & O_CREAT == 0 {
                return -2; // ENOENT
            }
            // SAFETY: syscall context, interrupts disabled.
            if unsafe { crate::sys_vfs_mut() }.create_file(path).is_err() {
                return -5; // EIO (missing parents, etc.)
            }
        }
        if unsafe { crate::sys_vfs() }.is_dir(path) {
            return -21; // EISDIR
        }
        if write && flags & O_TRUNC != 0 {
            // SAFETY: syscall context, interrupts disabled.
            if unsafe { crate::sys_vfs_mut() }.truncate(path).is_err() {
                return -5;
            }
        }
        let append_off = if write && flags & O_APPEND != 0 {
            unsafe { crate::sys_vfs() }.file_len(path).unwrap_or(0)
        } else {
            0
        };
        // Use per-process fd allocation.
        let slot = match unsafe { crate::fd_alloc() } {
            Some(s) => s,
            None => return -24, // EMFILE
        };
        // SAFETY: slot just allocated, single-threaded.
        unsafe {
            let f = crate::fd_slot(slot).unwrap();
            f.used = true;
            f.kind = crate::FD_FILE;
            f.pipe_id = 0;
            f.write = write;
            f.len = path_len as u32;
            f.off = append_off;
            f.path[..path_len as usize].copy_from_slice(bytes);
        }
        (slot as i64) + 3
    }

    /// write to an FD_FILE opened for writing: copy user bytes into the
    /// VFS file at the fd's current offset.
    fn sys_file_write(&mut self, fd: u64, ptr: u64, len: u64) -> i64 {
        let (pbuf, plen, off) = {
            let Some(f) = (unsafe { crate::fd_get(fd) }) else {
                return -9; // EBADF
            };
            if f.kind != crate::FD_FILE || !f.write {
                return -9;
            }
            let mut b = [0u8; 64];
            b[..f.len as usize].copy_from_slice(&f.path[..f.len as usize]);
            (b, f.len as usize, f.off)
        };
        let path = match core::str::from_utf8(&pbuf[..plen]) {
            Ok(s) => s,
            Err(_) => return -14,
        };
        // SAFETY: user buf mapped under active CR3.
        let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
        let n = match unsafe { crate::sys_vfs_mut() }.write_at(path, bytes, off) {
            Ok(n) => n,
            Err(_) => return -5, // EIO
        };
        if let Some(f) = unsafe { crate::fd_get(fd) } {
            f.off += n as u64;
        }
        n as i64
    }

    /// stat(path_ptr,path_len,buf_ptr,buf_len): write {u64 mode, u64
    /// size} — mode uses Linux-ish bits (0o040755 dir / 0o100644 file).
    fn sys_stat(&mut self, p_ptr: u64, p_len: u64, b_ptr: u64, b_len: u64) -> i64 {
        if b_len < 16 {
            return -22; // EINVAL
        }
        let path = match unsafe { user_path(p_ptr, p_len) } {
            Ok(s) => s,
            Err(e) => return e,
        };
        let vfs = unsafe { crate::sys_vfs() };
        let (is_dir, size) = if vfs.is_dir(path) {
            (true, 0u64)
        } else if let Some(n) = vfs.file_len(path) {
            (false, n)
        } else {
            return -2; // ENOENT
        };
        let mode: u64 = if is_dir { 0o040755 } else { 0o100644 };
        // SAFETY: user buf mapped under active CR3; b_len >= 16 checked.
        unsafe {
            *(b_ptr as *mut u64) = mode;
            *((b_ptr + 8) as *mut u64) = size;
        }
        16
    }

    /// mkdir(path_ptr,path_len): create a directory in the VFS.
    fn sys_mkdir(&mut self, p_ptr: u64, p_len: u64) -> i64 {
        let path = match unsafe { user_path(p_ptr, p_len) } {
            Ok(s) => s,
            Err(e) => return e,
        };
        match unsafe { crate::sys_vfs_mut() }.create_dir(path) {
            Ok(()) => 0,
            Err(e) => fs_err(e),
        }
    }

    /// unlink(path_ptr,path_len): remove a file from the VFS.
    fn sys_unlink(&mut self, p_ptr: u64, p_len: u64) -> i64 {
        let path = match unsafe { user_path(p_ptr, p_len) } {
            Ok(s) => s,
            Err(e) => return e,
        };
        match unsafe { crate::sys_vfs_mut() }.unlink(path) {
            Ok(()) => 0,
            Err(e) => fs_err(e),
        }
    }

    /// rmdir(path_ptr,path_len): remove an empty directory from the VFS.
    fn sys_rmdir(&mut self, p_ptr: u64, p_len: u64) -> i64 {
        let path = match unsafe { user_path(p_ptr, p_len) } {
            Ok(s) => s,
            Err(e) => return e,
        };
        match unsafe { crate::sys_vfs_mut() }.remove_dir(path) {
            Ok(()) => 0,
            Err(e) => fs_err(e),
        }
    }

    /// chdir(path_ptr,path_len): set the current process's cwd.
    /// Path must be absolute, exist, and be a directory.
    fn sys_chdir(&mut self, p_ptr: u64, p_len: u64) -> i64 {
        let path = match unsafe { user_path(p_ptr, p_len) } {
            Ok(s) => s,
            Err(e) => return e,
        };
        if !path.starts_with('/') || path.len() > 63 {
            return -22; // EINVAL
        }
        // Normalize a trailing "/" the way the VFS lookup tolerates it.
        let norm = path.trim_end_matches('/');
        let norm = if norm.is_empty() { "/" } else { norm };
        let vfs = unsafe { crate::sys_vfs() };
        if !vfs.is_dir(norm) {
            return -2; // ENOENT
        }
        unsafe {
            let slot = &mut crate::PROCS[crate::CUR_PROC];
            slot.cwd.fill(0);
            slot.cwd[..norm.len()].copy_from_slice(norm.as_bytes());
            slot.cwd_len = norm.len() as u8;
        }
        0
    }

    /// getcwd(buf_ptr,buf_len): write the current process's cwd.
    fn sys_getcwd(&mut self, b_ptr: u64, b_len: u64) -> i64 {
        if b_len == 0 {
            return -22;
        }
        let (cwd, len) = unsafe {
            let s = &crate::PROCS[crate::CUR_PROC];
            (s.cwd, s.cwd_len as usize)
        };
        let n = len.min(b_len as usize);
        // SAFETY: user buf mapped under active CR3.
        unsafe {
            core::ptr::copy_nonoverlapping(cwd.as_ptr(), b_ptr as *mut u8, n);
        }
        n as i64
    }

    /// read(fd,ptr,len): fd0=serial or stdin_fd, fd>=3=VFS file or pipe.
    fn sys_read(&mut self, fd: u64, ptr: u64, len: u64) -> i64 {
        // stdin redirect: a process spawned with an input fd reads
        // fd 0 from that fd (file or pipe) instead of the console.
        let mut fd = fd;
        if fd == 0 {
            let inp = unsafe { crate::PROCS[crate::CUR_PROC].stdin_fd };
            if inp >= 3 {
                fd = inp;
            }
        }
        if fd == 0 {
            // console input: PS/2 keyboard, then serial RX
            let byte = kbd_read_byte().or_else(serial_read_byte);
            return match byte {
                Some(b) => {
                    // SAFETY: user buf mapped.
                    unsafe { *(ptr as *mut u8) = b };
                    1
                }
                None => -11, // EAGAIN
            };
        }
        let f = match unsafe { crate::fd_get(fd) } {
            Some(f) => f,
            None => return -9, // EBADF
        };
        if f.kind == crate::FD_PIPE_READ {
            return self.sys_pipe_read(f.pipe_id as usize, ptr, len);
        }
        let path = match core::str::from_utf8(&f.path[..f.len as usize]) {
            Ok(s) => s,
            Err(_) => return -14,
        };
        let data = unsafe { crate::sys_vfs() }.read(path);
        let Some(data) = data else { return -5 }; // EIO
        let off = f.off as usize;
        if off >= data.len() {
            return 0; // EOF
        }
        let n = (len as usize).min(data.len() - off);
        // SAFETY: user buf mapped; n <= remaining file bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(data[off..off + n].as_ptr(), ptr as *mut u8, n);
        }
        f.off += n as u64;
        n as i64
    }

    /// pipe_read(pipe_id, ptr, len): read bytes from a pipe buffer.
    /// Returns bytes read, 0 on EOF (write end closed), -11 if empty.
    fn sys_pipe_read(&mut self, pipe_id: usize, ptr: u64, len: u64) -> i64 {
        if pipe_id >= crate::MAX_PIPES {
            return -9; // EBADF
        }
        let p = unsafe { &mut crate::PIPES[pipe_id] };
        if !p.used || p.count == 0 {
            // Check if any write fd still references this pipe.
            let mut any_write = false;
            for i in 0..crate::NPROCS {
                for fd in 0..crate::MAX_FDS {
                    let f = unsafe { &crate::PROCS[i].fds[fd] };
                    if f.used && f.kind == crate::FD_PIPE_WRITE && f.pipe_id as usize == pipe_id {
                        any_write = true;
                        break;
                    }
                }
                if any_write {
                    break;
                }
            }
            return if any_write { -11 } else { 0 }; // EAGAIN or EOF
        }
        let n = len.min(p.count as u64) as usize;
        // Copy n bytes from pipe buffer (circular).
        let mut copied = 0;
        while copied < n {
            let b = p.buf[p.head];
            // SAFETY: user buf mapped.
            unsafe { *((ptr as *mut u8).add(copied)) = b };
            p.head = (p.head + 1) % crate::PIPE_BUF_SIZE;
            p.count -= 1;
            copied += 1;
        }
        copied as i64
    }

    /// pipe_write(pipe_id, ptr, len): write bytes to a pipe buffer.
    /// Returns bytes written or -11 if full.
    fn sys_pipe_write(&mut self, pipe_id: usize, ptr: u64, len: u64) -> i64 {
        if pipe_id >= crate::MAX_PIPES {
            return -9; // EBADF
        }
        let p = unsafe { &mut crate::PIPES[pipe_id] };
        if !p.used {
            return -9; // EBADF
        }
        let space = crate::PIPE_BUF_SIZE - p.count;
        let n = (len as usize).min(space);
        if n == 0 {
            return -11; // EAGAIN (buffer full)
        }
        let mut copied = 0;
        while copied < n {
            let b = unsafe { *((ptr as *const u8).add(copied)) };
            p.buf[p.tail] = b;
            p.tail = (p.tail + 1) % crate::PIPE_BUF_SIZE;
            p.count += 1;
            copied += 1;
        }
        copied as i64
    }

    /// pipe(): create a pipe, return (read_fd, write_fd) packed.
    fn sys_pipe(&mut self) -> i64 {
        // Find a free pipe slot.
        let pipe_id = unsafe {
            match crate::PIPES.iter().position(|p| !p.used) {
                Some(i) => i,
                None => return -28, // ENOSPC
            }
        };
        // Allocate two fds.
        let read_slot = match unsafe { crate::fd_alloc() } {
            Some(s) => s,
            None => return -24, // EMFILE
        };
        let write_slot = match unsafe { crate::fd_alloc() } {
            Some(s) => s,
            None => return -24,
        };
        unsafe {
            crate::PIPES[pipe_id].used = true;
            crate::PIPES[pipe_id].head = 0;
            crate::PIPES[pipe_id].tail = 0;
            crate::PIPES[pipe_id].count = 0;

            let rf = crate::fd_slot(read_slot).unwrap();
            rf.used = true;
            rf.kind = crate::FD_PIPE_READ;
            rf.pipe_id = pipe_id as u8;
            rf.write = false;
            rf.len = 0;
            rf.off = 0;

            let wf = crate::fd_slot(write_slot).unwrap();
            wf.used = true;
            wf.kind = crate::FD_PIPE_WRITE;
            wf.pipe_id = pipe_id as u8;
            wf.write = false;
            wf.len = 0;
            wf.off = 0;
        }
        // Return (write_fd << 32) | read_fd
        let read_fd = (read_slot as u64) + 3;
        let write_fd = (write_slot as u64) + 3;
        ((write_fd << 32) | read_fd) as i64
    }

    /// dup(fd): duplicate a file descriptor to a new fd.
    fn sys_dup(&mut self, fd: u64) -> i64 {
        let src = match unsafe { crate::fd_get(fd) } {
            Some(f) => *f,
            None => return -9, // EBADF
        };
        let slot = match unsafe { crate::fd_alloc() } {
            Some(s) => s,
            None => return -24, // EMFILE
        };
        unsafe {
            let dst = crate::fd_get((slot as u64) + 3).unwrap();
            *dst = src;
        }
        (slot as i64) + 3
    }

    /// dup2(oldfd, newfd): duplicate oldfd to newfd, closing newfd if open.
    fn sys_dup2(&mut self, oldfd: u64, newfd: u64) -> i64 {
        let src = match unsafe { crate::fd_get(oldfd) } {
            Some(f) => *f,
            None => return -9, // EBADF
        };
        if newfd == 0 || newfd == 1 || newfd == 2 {
            // Allow dup2 to stdout/stderr for redirection.
        }
        let new_slot = match newfd.checked_sub(3) {
            Some(s) if (s as usize) < crate::MAX_FDS => s as usize,
            _ => return -9, // EBADF for invalid fd range
        };
        unsafe {
            // Close existing fd at newfd if open.
            let dst = &mut crate::current_fds()[new_slot];
            if dst.used && newfd != oldfd {
                // For pipes, decrement refcount would go here.
                // For now, just mark unused.
                dst.used = false;
            }
            *dst = src;
        }
        newfd as i64
    }

    /// kill(pid, signal): send signal to process.
    fn sys_kill(&mut self, pid: u64, signal: u64) -> i64 {
        const SIGKILL: u64 = 9;
        const SIGTERM: u64 = 15;

        if pid == 0 {
            return -22; // EINVAL - can't kill self/group
        }

        unsafe {
            for i in 0..crate::NPROCS {
                if crate::PROCS[i].used && crate::PROCS[i].pid == pid {
                    match signal {
                        SIGKILL => {
                            crate::PROCS[i].exited = true;
                            crate::PROCS[i].exit_code = 137; // 128 + 9
                            return 0;
                        }
                        SIGTERM => {
                            crate::PROCS[i].exited = true;
                            crate::PROCS[i].exit_code = 143; // 128 + 15
                            return 0;
                        }
                        _ => return -22, // EINVAL - unsupported signal
                    }
                }
            }
        }
        -3 // ESRCH - process not found
    }

    fn sys_close(&mut self, fd: u64) -> i64 {
        match unsafe { crate::fd_get(fd) } {
            Some(f) => {
                f.used = false;
                0
            }
            None => -9,
        }
    }
}

impl TrapSvc {
    /// listdir(path_ptr,path_len,buf_ptr,buf_len): newline-joined names.
    fn sys_listdir(&mut self, p_ptr: u64, p_len: u64, b_ptr: u64, b_len: u64) -> i64 {
        if p_len == 0 || p_len > 64 || b_len == 0 {
            return -22;
        }
        // SAFETY: user buffers mapped under active CR3.
        let pb = unsafe { core::slice::from_raw_parts(p_ptr as *const u8, p_len as usize) };
        let path = match core::str::from_utf8(pb) {
            Ok(s) => s,
            Err(_) => return -14,
        };
        let names = unsafe { crate::sys_vfs() }.list(path);
        let Some(names) = names else { return -2 }; // ENOENT
                                                    // join with '\n'
        let mut out = alloc::string::String::new();
        for (i, n) in names.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(n);
        }
        let bytes = out.as_bytes();
        let n = bytes.len().min(b_len as usize);
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), b_ptr as *mut u8, n);
        }
        n as i64
    }
}
