//! VGA text-mode console (80x25, 0xB8000 cell = char | attr<<8).
//!
//! `TextConsole` is generic over `ScreenBuf` so the same line discipline
//! runs on the real memory-mapped buffer (arch layer) and on a Vec in
//! tests. Handles cursor advance, newline, scroll-up, and printable
//! filtering.

/// Standard VGA text geometry.
pub const COLS: usize = 80;
pub const ROWS: usize = 25;
pub const CELLS: usize = COLS * ROWS;

/// Attribute byte: fg in low nibble, bg in high nibble.
pub mod attr {
    pub const BLACK: u8 = 0x0;
    pub const BLUE: u8 = 0x1;
    pub const GREEN: u8 = 0x2;
    pub const CYAN: u8 = 0x3;
    pub const RED: u8 = 0x4;
    pub const MAGENTA: u8 = 0x5;
    pub const BROWN: u8 = 0x6;
    pub const LIGHT_GRAY: u8 = 0x7;
    pub const DARK_GRAY: u8 = 0x8;
    pub const LIGHT_BLUE: u8 = 0x9;
    pub const LIGHT_GREEN: u8 = 0xa;
    pub const LIGHT_CYAN: u8 = 0xb;
    pub const LIGHT_RED: u8 = 0xc;
    pub const LIGHT_MAGENTA: u8 = 0xd;
    pub const YELLOW: u8 = 0xe;
    pub const WHITE: u8 = 0xf;

    /// fg on bg helper.
    pub const fn fg(fg: u8) -> u8 {
        fg & 0x0f
    }
    /// bg in high nibble.
    pub const fn bg(bg: u8) -> u8 {
        (bg & 0x0f) << 4
    }
    /// full attribute.
    pub const fn make(fg: u8, bg: u8) -> u8 {
        (bg << 4) | (fg & 0x0f)
    }
}

/// A cell store — one u16 per character cell.
pub trait ScreenBuf {
    /// Write one packed cell (char | attr<<8) at index.
    fn put(&mut self, index: usize, cell: u16);
    /// Read one packed cell.
    fn get(&self, index: usize) -> u16;
    /// Total cells (normally CELLS).
    fn len(&self) -> usize;
}

/// In-memory screen for tests.
#[derive(Debug)]
pub struct VecScreen {
    pub cells: alloc::vec::Vec<u16>,
}

impl VecScreen {
    pub fn new() -> Self {
        Self {
            cells: alloc::vec![0u16; CELLS],
        }
    }
    /// Decode a cell into (char, attr).
    pub fn cell(&self, index: usize) -> (u8, u8) {
        let c = self.cells[index];
        (c as u8, (c >> 8) as u8)
    }
}

impl ScreenBuf for VecScreen {
    fn put(&mut self, index: usize, cell: u16) {
        self.cells[index] = cell;
    }
    fn get(&self, index: usize) -> u16 {
        self.cells[index]
    }
    fn len(&self) -> usize {
        self.cells.len()
    }
}

/// Text console state machine over any ScreenBuf.
pub struct TextConsole<B: ScreenBuf> {
    buf: B,
    col: usize,
    row: usize,
    attr: u8,
}

impl<B: ScreenBuf> TextConsole<B> {
    /// Create a console; clears the screen to spaces with `attr`.
    pub fn new(mut buf: B, attr: u8) -> Self {
        for i in 0..buf.len().min(CELLS) {
            buf.put(i, cell(b' ', attr));
        }
        Self { buf, col: 0, row: 0, attr }
    }

    /// Set the active attribute for subsequent writes.
    pub fn set_attr(&mut self, attr: u8) {
        self.attr = attr;
    }

    /// Write one byte: handles \n, \r, \t, and printable chars.
    pub fn put_byte(&mut self, b: u8) {
        match b {
            b'\n' => self.newline(),
            b'\r' => self.col = 0,
            b'\t' => {
                let next = (self.col + 8) & !7;
                while self.col < next.min(COLS) {
                    self.put_char(b' ');
                }
            }
            0x20..=0x7e | 0x80..=0xff => self.put_char(b),
            _ => {} // ignore other control chars
        }
    }

    /// Write a string.
    pub fn write_str(&mut self, s: &str) {
        for &b in s.as_bytes() {
            self.put_byte(b);
        }
    }

    /// Write a string + newline.
    pub fn println(&mut self, s: &str) {
        self.write_str(s);
        self.newline();
    }

    /// Clear screen + home cursor.
    pub fn clear(&mut self) {
        for i in 0..self.buf.len().min(CELLS) {
            self.buf.put(i, cell(b' ', self.attr));
        }
        self.col = 0;
        self.row = 0;
    }

    /// Current cursor position.
    pub fn cursor(&self) -> (usize, usize) {
        (self.col, self.row)
    }

    /// Borrow the underlying buffer (for the arch layer to flush).
    pub fn buf(&self) -> &B {
        &self.buf
    }
    /// Mutably borrow (arch layer may need it for direct cell edits).
    pub fn buf_mut(&mut self) -> &mut B {
        &mut self.buf
    }

    /// Move to the next line (public wrapper for interactive input).
    pub fn newline_pub(&mut self) {
        self.newline();
    }

    /// Erase the character before the cursor (public wrapper).
    pub fn backspace_pub(&mut self) {
        if self.col == 0 {
            if self.row == 0 {
                return;
            }
            self.row -= 1;
            self.col = COLS;
        }
        self.col -= 1;
        let idx = self.row * COLS + self.col;
        self.buf.put(idx, cell(b' ', self.attr));
    }

    fn put_char(&mut self, b: u8) {
        if self.col >= COLS {
            self.newline();
        }
        let idx = self.row * COLS + self.col;
        self.buf.put(idx, cell(b, self.attr));
        self.col += 1;
    }

    fn newline(&mut self) {
        self.col = 0;
        if self.row + 1 >= ROWS {
            self.scroll_up();
        } else {
            self.row += 1;
        }
    }

    fn scroll_up(&mut self) {
        // move rows 1..ROWS-1 up one line, blank the bottom row
        for r in 1..ROWS {
            for c in 0..COLS {
                let src = r * COLS + c;
                let dst = (r - 1) * COLS + c;
                let v = self.buf.get(src);
                self.buf.put(dst, v);
            }
        }
        let blank = cell(b' ', self.attr);
        for c in 0..COLS {
            self.buf.put((ROWS - 1) * COLS + c, blank);
        }
    }
}

/// Pack char + attr into a VGA cell.
pub const fn cell(ch: u8, attr: u8) -> u16 {
    (ch as u16) | ((attr as u16) << 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn con() -> TextConsole<VecScreen> {
        TextConsole::new(VecScreen::new(), attr::make(attr::LIGHT_GRAY, attr::BLACK))
    }

    #[test]
    fn writes_and_advances() {
        let mut c = con();
        c.write_str("hi");
        assert_eq!(c.cursor(), (2, 0));
        let (ch, a) = c.buf().cell(0);
        assert_eq!(ch, b'h');
        assert_eq!(a, attr::make(attr::LIGHT_GRAY, attr::BLACK));
        let (ch1, _) = c.buf().cell(1);
        assert_eq!(ch1, b'i');
    }

    #[test]
    fn newline_moves_down() {
        let mut c = con();
        c.write_str("a\nb");
        assert_eq!(c.cursor(), (1, 1));
        let (ch, _) = c.buf().cell(COLS + 0);
        assert_eq!(ch, b'b');
    }

    #[test]
    fn wraps_at_right_margin() {
        let mut c = con();
        for _ in 0..COLS {
            c.put_byte(b'x');
        }
        assert_eq!(c.cursor(), (COLS, 0)); // cursor sits past last col
        c.put_byte(b'y'); // this one wraps
        assert_eq!(c.cursor(), (1, 1));
    }

    #[test]
    fn scrolls_when_full() {
        let mut c = con();
        for _ in 0..ROWS {
            c.println("line");
        }
        // fill screen, then write without newline so it stays put
        c.write_str("overflow");
        let row_start = (ROWS - 1) * COLS;
        let (ch, _) = c.buf().cell(row_start);
        assert_eq!(ch, b'o');
    }

    #[test]
    fn ignores_control_chars() {
        let mut c = con();
        c.write_str("a\x07\x00b"); // BEL, NUL ignored
        assert_eq!(c.cursor(), (2, 0));
    }

    #[test]
    fn tab_advances_to_stop() {
        let mut c = con();
        c.put_byte(b'\t');
        assert_eq!(c.cursor(), (8, 0));
    }
}
