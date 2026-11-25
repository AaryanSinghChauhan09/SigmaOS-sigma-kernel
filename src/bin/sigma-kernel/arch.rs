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
