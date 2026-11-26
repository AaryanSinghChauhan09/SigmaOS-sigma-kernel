//! Linear framebuffer writer — pixel formats and primitive drawing.
//!
//! Supports 32bpp XRGB/RGBX and 16bpp RGB565. All offsets are computed
//! from pitch; the backing store is a raw pointer wrapped by the arch
//! layer (`FbMem`).

use crate::boot::FramebufferInfo;
use crate::KernelError;

/// Supported output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbFormat {
    Xrgb8888,
    Rgb565,
    Bgrx8888,
}

/// A framebuffer view over physical memory.
pub struct Framebuffer {
    pub info: FramebufferInfo,
    pub format: FbFormat,
}

impl Framebuffer {
    /// Create from boot info; picks the best supported format.
    pub fn from_boot(info: &FramebufferInfo) -> Result<Self, KernelError> {
        if info.width == 0 || info.height == 0 {
            return Err(KernelError::InvalidArg("zero-size framebuffer"));
        }
        let format = match (info.bpp, info.format) {
            (32, _) => FbFormat::Xrgb8888,
            (24, _) => FbFormat::Xrgb8888, // packed into 32bpp pitch
            (16, _) => FbFormat::Rgb565,
            _ => return Err(KernelError::HardwareMissing("unsupported bpp")),
        };
        Ok(Self {
            info: info.clone(),
            format,
        })
    }

    /// Bytes per pixel for the chosen format.
    pub fn bpp_bytes(&self) -> usize {
        match self.format {
            FbFormat::Xrgb8888 | FbFormat::Bgrx8888 => 4,
            FbFormat::Rgb565 => 2,
        }
    }

    /// Byte offset of pixel (x, y).
    pub fn offset(&self, x: u32, y: u32) -> usize {
        y as usize * self.info.pitch as usize + x as usize * self.bpp_bytes()
    }

    /// In-bounds check.
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x < self.info.width && y < self.info.height
    }

    /// Encode a color for the format.
    pub fn encode(&self, r: u8, g: u8, b: u8) -> u32 {
        match self.format {
            FbFormat::Xrgb8888 => (0xFF << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32,
            FbFormat::Bgrx8888 => (0xFF << 24) | ((b as u32) << 16) | ((g as u32) << 8) | r as u32,
            FbFormat::Rgb565 => {
                let r5 = ((r as u32) >> 3) & 0x1F;
                let g6 = ((g as u32) >> 2) & 0x3F;
                let b5 = ((b as u32) >> 3) & 0x1F;
                (r5 << 11) | (g6 << 5) | b5
            }
        }
    }

    /// Fill a rectangle (clipped to screen).
    pub fn fill_rect<M: FnMut(usize, u32)>(
        &self,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        color: u32,
        put: &mut M,
    ) {
        let x1 = x.saturating_add(w).min(self.info.width);
        let y1 = y.saturating_add(h).min(self.info.height);
        for py in y..y1 {
            for px in x..x1 {
                if self.contains(px, py) {
                    put(self.offset(px, py), color);
                }
            }
        }
    }

    /// Clear the screen to a color.
    pub fn clear<M: FnMut(usize, u32)>(&self, color: u32, put: &mut M) {
        self.fill_rect(0, 0, self.info.width, self.info.height, color, put);
    }
}

/// 8x8 monochrome font for basic text rendering (public domain style).
/// Indexed by ASCII 0x20..0x7F -> 8 rows of 8 bits.
pub const FONT_8X8: [[u8; 8]; 96] = {
    let mut f = [[0u8; 8]; 96];
    // minimal glyphs: space is blank; others defined sparsely.
    // 'A'
    f[(b'A' - 0x20) as usize] = [0x18, 0x3C, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x00];
    // 'B'
    f[(b'B' - 0x20) as usize] = [0x7C, 0x66, 0x66, 0x7C, 0x66, 0x66, 0x7C, 0x00];
    // 'C'
    f[(b'C' - 0x20) as usize] = [0x3C, 0x66, 0x60, 0x60, 0x60, 0x66, 0x3C, 0x00];
    // 'S'
    f[(b'S' - 0x20) as usize] = [0x3C, 0x66, 0x60, 0x3C, 0x06, 0x66, 0x3C, 0x00];
    // 'i'
    f[(b'i' - 0x20) as usize] = [0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0x18, 0x00];
    // 'g'
    f[(b'g' - 0x20) as usize] = [0x3E, 0x63, 0x60, 0x3E, 0x63, 0x63, 0x3E, 0x00];
    // 'm'
    f[(b'm' - 0x20) as usize] = [0x00, 0x66, 0x7E, 0x7E, 0x60, 0x60, 0x60, 0x00];
    // 'a'
    f[(b'a' - 0x20) as usize] = [0x00, 0x30, 0x78, 0xCC, 0xCC, 0xCC, 0x76, 0x00];
    // '0'..'9'
    f[(b'0' - 0x20) as usize] = [0x3C, 0x66, 0x6E, 0x76, 0x66, 0x66, 0x3C, 0x00];
    f[(b'1' - 0x20) as usize] = [0x18, 0x38, 0x18, 0x18, 0x18, 0x18, 0x7E, 0x00];
    f[(b'2' - 0x20) as usize] = [0x3C, 0x66, 0x06, 0x1C, 0x30, 0x66, 0x7E, 0x00];
    f[(b'3' - 0x20) as usize] = [0x3C, 0x66, 0x06, 0x1C, 0x06, 0x66, 0x3C, 0x00];
    f[(b'4' - 0x20) as usize] = [0x0C, 0x1C, 0x3C, 0x6C, 0x7E, 0x0C, 0x0C, 0x00];
    f[(b'5' - 0x20) as usize] = [0x7E, 0x60, 0x7C, 0x06, 0x06, 0x66, 0x3C, 0x00];
    f[(b'6' - 0x20) as usize] = [0x1C, 0x30, 0x60, 0x7C, 0x66, 0x66, 0x3C, 0x00];
    f[(b'7' - 0x20) as usize] = [0x7E, 0x06, 0x0C, 0x18, 0x30, 0x30, 0x30, 0x00];
    f[(b'8' - 0x20) as usize] = [0x3C, 0x66, 0x66, 0x3C, 0x66, 0x66, 0x3C, 0x00];
    f[(b'9' - 0x20) as usize] = [0x3C, 0x66, 0x66, 0x3E, 0x06, 0x0C, 0x38, 0x00];
    // ' '
    f[0] = [0; 8];
    f
};

/// Draw a string at (x, y) scaled by `scale`.
pub fn draw_text<M: FnMut(usize, u32)>(
    fb: &Framebuffer,
    x: u32,
    y: u32,
    text: &str,
    color: u32,
    scale: u32,
    put: &mut M,
) {
    let mut cx = x;
    for ch in text.chars() {
        let idx = ch as usize;
        if (0x20..0x80).contains(&idx) {
            let glyph = &FONT_8X8[idx - 0x20];
            for (row, bits) in glyph.iter().enumerate() {
                for col in 0..8 {
                    if bits & (0x80 >> col) != 0 {
                        for dy in 0..scale {
                            for dx in 0..scale {
                                let px = cx + col as u32 * scale + dx;
                                let py = y + row as u32 * scale + dy;
                                if fb.contains(px, py) {
                                    put(fb.offset(px, py), color);
                                }
                            }
                        }
                    }
                }
            }
            cx += 8 * scale + scale; // char width + spacing
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot::PixelFormat;
    use alloc::vec::Vec;

    fn fb32() -> Framebuffer {
        Framebuffer {
            info: FramebufferInfo {
                addr: 0xFD00_0000,
                width: 100,
                height: 50,
                pitch: 100 * 4,
                bpp: 32,
                format: PixelFormat::Rgb,
                red_mask: 0x00FF0000,
                green_mask: 0x0000FF00,
                blue_mask: 0x000000FF,
            },
            format: FbFormat::Xrgb8888,
        }
    }

    #[test]
    fn offset_math() {
        let fb = fb32();
        assert_eq!(fb.offset(0, 0), 0);
        assert_eq!(fb.offset(1, 0), 4);
        assert_eq!(fb.offset(0, 1), 400);
    }

    #[test]
    fn encode_colors() {
        let fb = fb32();
        assert_eq!(fb.encode(255, 0, 0), 0xFFFF_0000);
        assert_eq!(fb.encode(0, 255, 0), 0xFF00_FF00);
        assert_eq!(fb.encode(0, 0, 255), 0xFF00_00FF);
    }

    #[test]
    fn fill_rect_clips() {
        let fb = fb32();
        let mut writes: Vec<(usize, u32)> = Vec::new();
        fb.fill_rect(90, 40, 50, 50, 0xDEAD, &mut |o, c| writes.push((o, c)));
        // should clip to 10x10 = 100 pixels
        assert_eq!(writes.len(), 100);
    }

    #[test]
    fn rejects_bad_bpp() {
        let mut info = FramebufferInfo {
            addr: 0,
            width: 10,
            height: 10,
            pitch: 10,
            bpp: 8,
            format: PixelFormat::Indexed,
            red_mask: 0,
            green_mask: 0,
            blue_mask: 0,
        };
        assert!(Framebuffer::from_boot(&info).is_err());
        info.bpp = 32;
        assert!(Framebuffer::from_boot(&info).is_ok());
    }
}
