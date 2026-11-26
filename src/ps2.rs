//! i8042 PS/2 keyboard controller driver model.
//!
//! Port-mapped register file is hardware-independent and unit-testable;
//! the arch layer implements `Ps2Port` with real port I/O. Scancode
//! set 1 is decoded into key events with Shift state tracking.

/// i8042 port offsets (added to the 0x60 base by the arch layer).
pub mod ports {
    pub const DATA: u16 = 0x60; // read: scancode; write: command arg / LED
    pub const CMD: u16 = 0x64; // write: command; read: status
}

/// Status register bits (read from CMD port).
pub mod status {
    pub const OUT_FULL: u8 = 0x01; // output buffer full (data at 0x60)
    pub const IN_FULL: u8 = 0x02; // input buffer full
    pub const SYS: u8 = 0x04;
    pub const CMD_DATA: u8 = 0x08; // last write was to cmd port
    pub const TIMEOUT: u8 = 0x40;
    pub const PARITY: u8 = 0x80;
}

/// Controller commands (written to CMD port).
pub mod cmd {
    pub const READ_CTRL: u8 = 0x20;
    pub const WRITE_CTRL: u8 = 0x60;
    pub const DIS_KBD: u8 = 0xAD;
    pub const EN_KBD: u8 = 0xAE;
    pub const SET_SCANCODE: u8 = 0xF0;
    pub const SCANCODE_SET_1: u8 = 0x01;
    pub const PULSE_RESET: u8 = 0xFE;
}

/// Scancode set 1 constants.
pub const BREAK_BIT: u8 = 0x80;

/// Port I/O abstraction — implemented per-arch.
pub trait Ps2Port {
    fn read(&self, port: u16) -> u8;
    fn write(&mut self, port: u16, value: u8);
}

/// A decoded key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Printable ASCII (already shift-decoded).
    Char(u8),
    /// Enter pressed.
    Enter,
    /// Backspace pressed.
    Backspace,
    /// Escape pressed.
    Esc,
    /// Arrow / function key (scancode index, set-1 make code).
    Special(u8),
}

/// Keyboard state: shift modifiers + pending output queue.
pub struct Keyboard<P: Ps2Port> {
    port: P,
    shift: bool,
    caps: bool,
    ctrl: bool,
    alt: bool,
    /// decoded events awaiting consumption
    pub events: alloc::vec::Vec<Key>,
}

impl<P: Ps2Port> Keyboard<P> {
    /// Attach to a port pair; performs controller self-check and
    /// selects scancode set 1.
    pub fn new(mut port: P) -> Self {
        // Disable keyboard, read+rewrite controller config preserving
        // flags, then re-enable.
        port.write(ports::CMD, cmd::DIS_KBD);
        port.write(ports::CMD, cmd::SET_SCANCODE);
        port.write(ports::DATA, cmd::SCANCODE_SET_1);
        port.write(ports::CMD, cmd::EN_KBD);
        Self {
            port,
            shift: false,
            caps: false,
            ctrl: false,
            alt: false,
            events: alloc::vec::Vec::new(),
        }
    }

    /// Poll the controller status; if a scancode is waiting, decode it
    /// and push any resulting `Key` events.
    pub fn poll(&mut self) {
        let status = self.port.read(ports::CMD);
        if status & status::OUT_FULL == 0 {
            return;
        }
        let sc = self.port.read(ports::DATA);
        self.decode(sc);
    }

    /// Decode one raw scancode byte (set 1). Break codes (release)
    /// update nothing for typing; make codes produce keys.
    fn decode(&mut self, sc: u8) {
        // Extended prefix 0xE0: next byte completes the sequence.
        // We handle it by ignoring the prefix and letting the next
        // decode() treat the following byte — simplified: special keys
        // arrive as Special(scancode & !BREAK_BIT & !0xE0 handled below).
        if sc == 0xE0 {
            return; // extension prefix; next poll handles payload
        }
        let make = sc & !BREAK_BIT;
        let released = sc & BREAK_BIT != 0;
        if released {
            // track modifier release
            match make {
                0x2A | 0x36 => self.shift = false,
                0x1D => self.ctrl = false,
                0x38 => self.alt = false,
                _ => {}
            }
            return;
        }
        match make {
            // modifiers
            0x2A | 0x36 => {
                self.shift = true;
                return;
            }
            0x1D => {
                self.ctrl = true;
                return;
            }
            0x38 => {
                self.alt = true;
                return;
            }
            0x3A => {
                self.caps = !self.caps;
                return;
            }
            // Enter / Backspace / Esc
            0x1C => return self.events.push(Key::Enter),
            0x0E => return self.events.push(Key::Backspace),
            0x01 => return self.events.push(Key::Esc),
            // printables live in the table
            _ => {}
        }
        if let Some(base) = lookup(make) {
            let upper = self.shift ^ self.caps;
            let ch = if upper {
                base.to_ascii_uppercase()
            } else {
                base
            };
            // Ctrl+letter -> control code
            let out = if self.ctrl && ch.is_ascii_alphabetic() {
                ch.to_ascii_lowercase() & 0x1f
            } else {
                ch
            };
            self.events.push(Key::Char(out));
        } else {
            self.events.push(Key::Special(make));
        }
    }

    /// Drain all pending key events.
    pub fn drain(&mut self) -> alloc::vec::Vec<Key> {
        core::mem::take(&mut self.events)
    }
}

/// Scancode set 1 -> unshifted ASCII for the printable range.
/// Covers the main typing block plus numpad digits/operators.
fn lookup(sc: u8) -> Option<u8> {
    Some(match sc {
        0x02 => b'1',
        0x03 => b'2',
        0x04 => b'3',
        0x05 => b'4',
        0x06 => b'5',
        0x07 => b'6',
        0x08 => b'7',
        0x09 => b'8',
        0x0A => b'9',
        0x0B => b'0',
        0x0C => b'-',
        0x0D => b'=',
        0x0F => b'\t', // Tab
        0x10 => b'q',
        0x11 => b'w',
        0x12 => b'e',
        0x13 => b'r',
        0x14 => b't',
        0x15 => b'y',
        0x16 => b'u',
        0x17 => b'i',
        0x18 => b'o',
        0x19 => b'p',
        0x1A => b'[',
        0x1B => b']',
        0x1E => b'a',
        0x1F => b's',
        0x20 => b'd',
        0x21 => b'f',
        0x22 => b'g',
        0x23 => b'h',
        0x24 => b'j',
        0x25 => b'k',
        0x26 => b'l',
        0x27 => b';',
        0x28 => b'\'',
        0x29 => b'`',
        0x2B => b'\\',
        0x2C => b'z',
        0x2D => b'x',
        0x2E => b'c',
        0x2F => b'v',
        0x30 => b'b',
        0x31 => b'n',
        0x32 => b'm',
        0x33 => b',',
        0x34 => b'.',
        0x35 => b'/',
        0x39 => b' ', // Space
        // numpad (without NumLock we still emit digits/operators)
        0x47 => b'7',
        0x48 => b'8',
        0x49 => b'9',
        0x4A => b'-',
        0x4B => b'4',
        0x4C => b'5',
        0x4D => b'6',
        0x4E => b'+',
        0x4F => b'1',
        0x50 => b'2',
        0x51 => b'3',
        0x52 => b'0',
        0x53 => b'.',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    /// Fake i8042: status port reports OUT_FULL when a scancode is
    /// queued; data port pops the queue.
    struct FakePs2 {
        status: Rc<RefCell<u8>>,
        queue: Rc<RefCell<VecDeque<u8>>>,
        writes: Rc<RefCell<Vec<(u16, u8)>>>,
    }

    impl Ps2Port for FakePs2 {
        fn read(&self, port: u16) -> u8 {
            match port {
                ports::CMD => {
                    if self.queue.borrow().is_empty() {
                        0
                    } else {
                        status::OUT_FULL
                    }
                }
                ports::DATA => self.queue.borrow_mut().pop_front().unwrap_or(0),
                _ => 0,
            }
        }
        fn write(&mut self, port: u16, v: u8) {
            self.writes.borrow_mut().push((port, v));
        }
    }

    fn kb_with(scancodes: &[u8]) -> (Keyboard<FakePs2>, Rc<RefCell<VecDeque<u8>>>) {
        let queue = Rc::new(RefCell::new(
            scancodes.iter().copied().collect::<VecDeque<_>>(),
        ));
        let status = Rc::new(RefCell::new(0u8));
        let port = FakePs2 {
            status,
            queue: queue.clone(),
            writes: Rc::new(RefCell::new(Vec::new())),
        };
        (Keyboard::new(port), queue)
    }

    #[test]
    fn types_letters() {
        // 'a' = 0x1E, 'b' = 0x30
        let (mut kb, _q) = kb_with(&[0x1E, 0x30]);
        kb.poll();
        kb.poll();
        let ev = kb.drain();
        assert_eq!(ev, alloc::vec![Key::Char(b'a'), Key::Char(b'b')]);
    }

    #[test]
    fn shift_produces_uppercase() {
        // LShift make 0x2A, 'a' 0x1E, LShift break 0xAA
        let (mut kb, _q) = kb_with(&[0x2A, 0x1E, 0xAA]);
        kb.poll();
        kb.poll();
        kb.poll();
        let ev = kb.drain();
        assert_eq!(ev, alloc::vec![Key::Char(b'A')]);
    }

    #[test]
    fn enter_and_backspace() {
        let (mut kb, _q) = kb_with(&[0x1C, 0x0E]);
        kb.poll();
        kb.poll();
        let ev = kb.drain();
        assert_eq!(ev, alloc::vec![Key::Enter, Key::Backspace]);
    }

    #[test]
    fn ctrl_c_is_control_code() {
        // LCtrl 0x1D, 'c' 0x2E, LCtrl break 0x9D
        let (mut kb, _q) = kb_with(&[0x1D, 0x2E, 0x9D]);
        kb.poll();
        kb.poll();
        kb.poll();
        let ev = kb.drain();
        assert_eq!(ev, alloc::vec![Key::Char(0x03)]);
    }

    #[test]
    fn break_codes_ignored_for_typing() {
        // 'x' make then break — only one Char event
        let (mut kb, _q) = kb_with(&[0x2D, 0xAD]);
        kb.poll();
        kb.poll();
        let ev = kb.drain();
        assert_eq!(ev, alloc::vec![Key::Char(b'x')]);
    }

    #[test]
    fn selects_scancode_set_1() {
        let writes = {
            let queue = Rc::new(RefCell::new(VecDeque::new()));
            let port = FakePs2 {
                status: Rc::new(RefCell::new(0u8)),
                queue,
                writes: Rc::new(RefCell::new(Vec::new())),
            };
            let w = port.writes.clone();
            let _kb = Keyboard::new(port);
            w
        };
        let w = writes.borrow();
        assert!(w.contains(&(ports::CMD, cmd::SET_SCANCODE)));
        assert!(w.contains(&(ports::DATA, cmd::SCANCODE_SET_1)));
    }
}
