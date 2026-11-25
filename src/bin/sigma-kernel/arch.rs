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
