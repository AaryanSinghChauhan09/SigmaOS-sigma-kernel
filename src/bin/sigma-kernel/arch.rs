//! x86_64 arch shims for the kernel entry — port I/O, serial log,
//! halt, PS/2 keyboard, and VGA text buffer. Isolated here so the rest
//! of the kernel stays `forbid(unsafe)`.

const COM1: u16 = 0x3f8;
const VGA_TEXT: usize = 0xB8000;

fn outb(port: u16, val: u8) {
    // SAFETY: port I/O to the standard COM1 UART.
    unsafe { core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nostack)); }
}

fn inb(port: u16) -> u8 {
    let v: u8;
    // SAFETY: port I/O read from COM1 line-status register.
    unsafe { core::arch::asm!("in al, dx", out("al") v, in("dx") port, options(nomem, nostack)); }
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
        // SAFETY: low-1GiB identity map covers all frame addresses we
        // hand out; phys is 4K-aligned from the frame allocator.
        unsafe { (phys as *mut u64).write_volatile(val) }
    }
    fn read_u64(&self, phys: u64) -> u64 {
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
        loop { core::hint::spin_loop() }
    }
    fn pid(&self) -> u64 {
        1 // single-process kernel for now
    }
    fn kernel_version(&self) -> &'static str {
        kernel::KERNEL_VERSION
    }
}

/// Minimal KernelServices for the syscall trap path — serial-only
/// output so it needs no console global. Used by syscall_trap.
pub struct TrapSvc;

impl kernel::syscall::KernelServices for TrapSvc {
    fn write(&mut self, fd: u64, ptr: u64, len: u64) -> i64 {
        if fd != 1 && fd != 2 {
            return -9;
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
        loop { core::hint::spin_loop() }
    }
    fn pid(&self) -> u64 { 1 }
    fn kernel_version(&self) -> &'static str { kernel::KERNEL_VERSION }
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
    fn kernel_info(&mut self, ptr: u64, len: u64) -> i64 {
        let v = kernel::KERNEL_VERSION.as_bytes();
        let n = (v.len()).min(len as usize);
        // SAFETY: user buf mapped under active CR3.
        unsafe {
            core::ptr::copy_nonoverlapping(v.as_ptr(), ptr as *mut u8, n);
        }
        n as i64
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

    /* BRK/MMAP: grow the user break by mapping fresh 4K frames into
     * the live user CR3 via usermem::map_user_page (splits 2MiB
     * supervisor slots on demand). Modeled on Linux sys_brk/sys_mmap
     * anonymous path. */
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
                let phys = match crate::USER_ALLOC
                    .as_mut()
                    .and_then(|a| a.alloc_frame())
                {
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
                let phys = match crate::USER_ALLOC
                    .as_mut()
                    .and_then(|a| a.alloc_frame())
                {
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

/* ---- fd table + file syscalls for TrapSvc ---- */

const MAX_FDS: usize = 8;

#[derive(Clone, Copy)]
struct FdEnt {
    used: bool,
    path: [u8; 64],
    len: u32,
    off: u64,
}

static mut FDS: [FdEnt; MAX_FDS] = [FdEnt {
    used: false, path: [0; 64], len: 0, off: 0,
}; MAX_FDS];

fn fd_alloc() -> Option<usize> {
    // SAFETY: single-threaded syscall context.
    unsafe {
        FDS.iter().position(|f| !f.used)
    }
}

fn fd_get(fd: u64) -> Option<&'static mut FdEnt> {
    // SAFETY: index bounds-checked; syscall context is single-threaded.
    unsafe {
        let i = fd.checked_sub(3)? as usize;
        if i < MAX_FDS && FDS[i].used { Some(&mut *core::ptr::addr_of_mut!(FDS[i])) } else { None }
    }
}

/// Global PS/2 keyboard for the syscall input path.
static mut KBD: Option<kernel::ps2::Keyboard<Ps2Hw>> = None;

fn kbd() -> &'static mut kernel::ps2::Keyboard<Ps2Hw> {
    // SAFETY: single-threaded syscall context; lazy init once.
    unsafe {
        if KBD.is_none() {
            KBD = Some(kernel::ps2::Keyboard::new(Ps2Hw));
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

impl TrapSvc {
    /// open(path_ptr,path_len,flags): copy path, check VFS, alloc fd.
    fn sys_open(&mut self, path_ptr: u64, path_len: u64, _flags: u64) -> i64 {
        if path_len == 0 || path_len > 64 {
            return -22; // EINVAL
        }
        // SAFETY: user path buffer is mapped under active CR3.
        let bytes = unsafe {
            core::slice::from_raw_parts(path_ptr as *const u8, path_len as usize)
        };
        let path = match core::str::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => return -14, // EFAULT
        };
        let exists = unsafe { crate::sys_vfs() }.exists(path);
        if !exists {
            return -2; // ENOENT
        }
        let Some(slot) = fd_alloc() else { return -24 }; // EMFILE
        // SAFETY: slot just allocated, single-threaded.
        unsafe {
            let f = &mut *core::ptr::addr_of_mut!(FDS[slot]);
            f.used = true;
            f.len = path_len as u32;
            f.off = 0;
            f.path[..path_len as usize].copy_from_slice(bytes);
        }
        (slot as i64) + 3
    }

    /// read(fd,ptr,len): fd0=serial, fd>=3=VFS file at offset.
    fn sys_read(&mut self, fd: u64, ptr: u64, len: u64) -> i64 {
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
        let Some(f) = fd_get(fd) else { return -9 }; // EBADF
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
            core::ptr::copy_nonoverlapping(
                data[off..off + n].as_ptr(),
                ptr as *mut u8,
                n,
            );
        }
        f.off += n as u64;
        n as i64
    }

    fn sys_close(&mut self, fd: u64) -> i64 {
        match fd_get(fd) {
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
