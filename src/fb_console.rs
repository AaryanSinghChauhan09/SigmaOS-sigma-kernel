//! Framebuffer-based console (graphical text output).
//!
//! Uses the linear framebuffer for text rendering with the 8x8 font.

use crate::boot::FramebufferInfo;
use crate::fb::{draw_text, Framebuffer};
use crate::KernelError;
use alloc::string::ToString;

/// Framebuffer console state.
pub struct FbConsole {
    fb: Framebuffer,
    cursor_x: u32,
    cursor_y: u32,
    line_height: u32,
    fg_color: u32,
    bg_color: u32,
}

impl FbConsole {
    /// Create a framebuffer console from boot info.
    pub fn new(info: &FramebufferInfo) -> Result<Self, KernelError> {
        let fb = Framebuffer::from_boot(info)?;
        let line_height = 16; // 8x8 font scaled by 2
        Ok(Self {
            fb,
            cursor_x: 10,
            cursor_y: 10,
            line_height,
            fg_color: 0xFFFF_FFFF, // white
            bg_color: 0xFF00_0000, // dark blue
        })
    }

    /// Write a string to the console.
    pub fn write_str<M: FnMut(usize, u32)>(&mut self, s: &str, put: &mut M) {
        for ch in s.chars() {
            match ch {
                '\n' => self.newline(),
                '\r' => self.cursor_x = 10,
                '\t' => {
                    let next = ((self.cursor_x / 64) + 1) * 64;
                    self.cursor_x = next.min(self.fb.info.width - 10);
                }
                _ => {
                    // Check if we need to wrap
                    if self.cursor_x + 48 > self.fb.info.width {
                        self.newline();
                    }
                    draw_text(
                        &self.fb,
                        self.cursor_x,
                        self.cursor_y,
                        &ch.to_string(),
                        self.fg_color,
                        2,
                        put,
                    );
                    self.cursor_x += 24; // 8 chars * 2 scale + spacing
                }
            }
        }
    }

    /// Write a string with newline.
    pub fn println<M: FnMut(usize, u32)>(&mut self, s: &str, put: &mut M) {
        self.write_str(s, put);
        self.newline();
    }

    /// Clear the screen.
    pub fn clear<M: FnMut(usize, u32)>(&mut self, put: &mut M) {
        self.fb.clear(self.bg_color, put);
        self.cursor_x = 10;
        self.cursor_y = 10;
    }

    /// Set foreground color.
    pub fn set_fg(&mut self, color: u32) {
        self.fg_color = color;
    }

    /// Set background color.
    pub fn set_bg(&mut self, color: u32) {
        self.bg_color = color;
    }

    fn newline(&mut self) {
        self.cursor_x = 10;
        self.cursor_y += self.line_height;
        // Scroll if needed
        if self.cursor_y + self.line_height > self.fb.info.height {
            self.scroll_up();
        }
    }

    fn scroll_up(&mut self) {
        // In a real implementation, this would copy lines up
        // For now, just reset to top
        self.cursor_y = 10;
    }
}
