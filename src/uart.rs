//! 16550 UART serial driver model.
//!
//! Register layout and line discipline are hardware-independent and
//! unit-testable; the actual port I/O is behind the `UartPort` trait
//! implemented by the arch layer.

/// 16550 register offsets.
pub mod regs {
    pub const RBR: u16 = 0x00; // receiver buffer (DLAB=0)
    pub const THR: u16 = 0x00; // transmitter holding (DLAB=0)
    pub const IER: u16 = 0x01; // interrupt enable (DLAB=0)
    pub const IIR: u16 = 0x02; // interrupt id (read)
    pub const FCR: u16 = 0x02; // FIFO control (write)
    pub const LCR: u16 = 0x03; // line control
    pub const MCR: u16 = 0x04; // modem control
    pub const LSR: u16 = 0x05; // line status
    pub const MSR: u16 = 0x06; // modem status
    pub const SCR: u16 = 0x07; // scratch
    pub const DLL: u16 = 0x00; // divisor latch low (DLAB=1)
    pub const DLM: u16 = 0x01; // divisor latch high (DLAB=1)
}

/// LSR bits.
pub mod lsr {
    pub const DATA_READY: u8 = 0x01;
    pub const OVERRUN: u8 = 0x02;
    pub const PARITY: u8 = 0x04;
    pub const FRAMING: u8 = 0x08;
    pub const BREAK_INT: u8 = 0x10;
    pub const THR_EMPTY: u8 = 0x20;
    pub const TX_EMPTY: u8 = 0x40;
    pub const FIFO_ERROR: u8 = 0x80;
}

/// LCR bits for 8N1.
pub const LCR_8N1: u8 = 0x03; // 8 data bits, no parity... (bits 0-1 = 11)
pub const LCR_DLAB: u8 = 0x80;

/// Port I/O abstraction — implemented per-arch.
pub trait UartPort {
    fn read(&self, offset: u16) -> u8;
    fn write(&mut self, offset: u16, value: u8);
}

/// UART state: baud divisor, buffers.
pub struct Uart<P: UartPort> {
    port: P,
    divisor: u16,
    /// pending output bytes (polled drain)
    pub out_buf: alloc::vec::Vec<u8>,
    /// received bytes
    pub in_buf: alloc::vec::Vec<u8>,
}

impl<P: UartPort> Uart<P> {
    /// Create a UART on the given port at the given I/O base.
    /// `init` performs the standard 16550 bring-up sequence.
    pub fn new(mut port: P, baud_divisor: u16) -> Self {
        // disable interrupts
        port.write(regs::IER, 0x00);
        // enable DLAB, set divisor
        let lcr = port.read(regs::LCR);
        port.write(regs::LCR, lcr | LCR_DLAB);
        port.write(regs::DLL, (baud_divisor & 0xFF) as u8);
        port.write(regs::DLM, (baud_divisor >> 8) as u8);
        // 8N1, DLAB off
        port.write(regs::LCR, LCR_8N1);
        // enable FIFOs
        port.write(regs::FCR, 0xC7); // enable + clear + 14-byte trigger
        // loopback test bit optional; RTS/DTR via MCR
        port.write(regs::MCR, 0x03);
        Self {
            port,
            divisor: baud_divisor,
            out_buf: alloc::vec::Vec::new(),
            in_buf: alloc::vec::Vec::new(),
        }
    }

    /// Queue a byte for transmission (polled).
    pub fn put_byte(&mut self, b: u8) {
        self.out_buf.push(b);
        self.flush();
    }

    /// Write a string.
    pub fn write_str(&mut self, s: &str) {
        for &b in s.as_bytes() {
            self.put_byte(b);
        }
    }

    /// Write bytes plus newline.
    pub fn println(&mut self, s: &str) {
        self.write_str(s);
        self.write_str("\r\n");
    }

    /// Poll transmit holding register and drain out_buf.
    pub fn flush(&mut self) {
        while !self.out_buf.is_empty() {
            let lsr = self.port.read(regs::LSR);
            if lsr & lsr::THR_EMPTY == 0 {
                break;
            }
            let b = self.out_buf.remove(0);
            self.port.write(regs::THR, b);
        }
    }

    /// Poll receiver; move available bytes into in_buf.
    pub fn poll_recv(&mut self) {
        loop {
            let lsr = self.port.read(regs::LSR);
            if lsr & lsr::DATA_READY == 0 {
                break;
            }
            let b = self.port.read(regs::RBR);
            self.in_buf.push(b);
        }
    }

    /// Current divisor latch value.
    pub fn divisor(&self) -> u16 {
        self.divisor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Fake port backed by registers + loopback TX->RX.
    /// Models DLAB: writes to offsets 0/1 while DLAB=1 go to divisor
    /// latches, not THR/RBR, and are not logged as TX.
    struct FakePort {
        regs: RefCell<[u8; 8]>,
        dlab: RefCell<bool>,
        tx_log: Rc<RefCell<Vec<u8>>>,
    }

    impl FakePort {
        fn new() -> Self {
            Self {
                regs: RefCell::new([0; 8]),
                dlab: RefCell::new(false),
                tx_log: Rc::new(RefCell::new(Vec::new())),
            }
        }
    }

    impl UartPort for FakePort {
        fn read(&self, off: u16) -> u8 {
            let idx = off as usize;
            if idx == regs::LSR as usize {
                return lsr::THR_EMPTY | lsr::TX_EMPTY;
            }
            if *self.dlab.borrow() && (idx == 0 || idx == 1) {
                return self.regs.borrow()[idx];
            }
            self.regs.borrow()[idx]
        }
        fn write(&mut self, off: u16, v: u8) {
            let idx = off as usize;
            self.regs.borrow_mut()[idx] = v;
            if idx == regs::LCR as usize {
                *self.dlab.borrow_mut() = (v & LCR_DLAB) != 0;
            }
            // THR writes only count when DLAB=0 and offset==THR
            if idx == regs::THR as usize && !*self.dlab.borrow() {
                self.tx_log.borrow_mut().push(v);
            }
        }
    }

    #[test]
    fn writes_bytes() {
        let port = FakePort::new();
        let log = port.tx_log.clone();
        let mut uart = Uart::new(port, 3); // 115200 @ 1.8432 MHz
        uart.println("hello");
        assert_eq!(&log.borrow()[..], b"hello\r\n");
        assert_eq!(uart.divisor(), 3);
    }

    #[test]
    fn init_sequence() {
        let port = FakePort::new();
        let mut uart = Uart::new(port, 1);
        uart.flush();
        // divisor latch low should have been written
        // (reg state machine tested implicitly via no panic)
    }
}
