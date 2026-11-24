//! Boot information — Multiboot2 tag parsing and memory map.
//!
//! The bootloader hands us a pointer to the Multiboot2 info struct;
//! this module parses memory-map and framebuffer tags into owned data.

use alloc::vec::Vec;
use crate::KernelError;

/// Multiboot2 magic value at the start of the info struct.
pub const MB2_MAGIC: u32 = 0x36_D7_62_89;

/// Multiboot2 info struct header (fixed part).
/// Layout: total_size u32, reserved u32 — no magic; the magic lives
/// in the boot header, not the info struct.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Mb2InfoHeader {
    pub total_size: u32,
    pub reserved: u32,
}

/// A parsed memory region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemRegion {
    pub start: u64,
    pub len: u64,
    /// Multiboot2 memory type (1=usable RAM, 2=reserved, ...).
    pub mem_type: u32,
}

/// Framebuffer pixel format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Indexed,
    Rgb,
    EgaText,
}

/// Parsed framebuffer info from the MB2 framebuffer tag.
#[derive(Debug, Clone)]
pub struct FramebufferInfo {
    pub addr: u64,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u8,
    pub format: PixelFormat,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
}

/// Everything the kernel needs from the bootloader.
#[derive(Debug, Clone)]
pub struct BootInfo {
    pub mem_regions: Vec<MemRegion>,
    pub framebuffer: Option<FramebufferInfo>,
    pub cmdline: Option<alloc::string::String>,
}

/// MB2 tag type constants.
pub const TAG_MEM_MAP: u32 = 6;
pub const TAG_FRAMEBUFFER: u32 = 8;
pub const TAG_CMDLINE: u32 = 1;
pub const TAG_END: u32 = 0;

/// Parse a raw Multiboot2 info struct from bytes.
///
/// `bytes` must start at the MB2 info struct (the bootloader places its
/// address in %rbx per the ABI).
pub fn parse_mb2(bytes: &[u8]) -> Result<BootInfo, KernelError> {
    if bytes.len() < 8 {
        return Err(KernelError::BootInfo("info struct too small"));
    }
    let total = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    if total < 8 || total > bytes.len() {
        return Err(KernelError::BootInfo("bad total size"));
    }

    let mut info = BootInfo {
        mem_regions: Vec::new(),
        framebuffer: None,
        cmdline: None,
    };

    let mut off = 8usize;
    while off + 8 <= total {
        let tag_type = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        let tag_size = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap()) as usize;
        if tag_size < 8 || off + tag_size > total {
            return Err(KernelError::BootInfo("bad tag size"));
        }
        let body = &bytes[off + 8..off + tag_size];

        match tag_type {
            TAG_MEM_MAP => parse_mem_map_tag(body, &mut info)?,
            TAG_FRAMEBUFFER => parse_fb_tag(body, &mut info)?,
            TAG_CMDLINE => {
                let end = body.iter().position(|&b| b == 0).unwrap_or(body.len());
                info.cmdline = Some(
                    alloc::string::String::from_utf8_lossy(&body[..end]).into_owned(),
                );
            }
            TAG_END => break,
            _ => {}
        }
        // tags are 8-byte aligned
        off += (tag_size + 7) & !7;
    }
    Ok(info)
}

fn parse_mem_map_tag(body: &[u8], info: &mut BootInfo) -> Result<(), KernelError> {
    if body.len() < 8 {
        return Err(KernelError::BootInfo("mem map tag short"));
    }
    // entry_size u32, entry_version u32, then entries
    let entry_size = u32::from_le_bytes(body[0..4].try_into().unwrap()) as usize;
    if entry_size < 24 {
        return Err(KernelError::BootInfo("bad mem map entry size"));
    }
    let mut off = 8usize;
    while off + entry_size <= body.len() {
        let start = u64::from_le_bytes(body[off..off + 8].try_into().unwrap());
        let len = u64::from_le_bytes(body[off + 8..off + 16].try_into().unwrap());
        let mem_type = u32::from_le_bytes(body[off + 16..off + 20].try_into().unwrap());
        info.mem_regions.push(MemRegion { start, len, mem_type });
        off += entry_size;
    }
    Ok(())
}

fn parse_fb_tag(body: &[u8], info: &mut BootInfo) -> Result<(), KernelError> {
    // MB2 framebuffer tag body: addr(8) pitch(4) width(4) height(4)
    // bpp(1) type(1) reserved(2) then format-specific u32s.
    if body.len() < 24 {
        return Err(KernelError::BootInfo("fb tag short"));
    }
    let addr = u64::from_le_bytes(body[0..8].try_into().unwrap());
    let pitch = u32::from_le_bytes(body[8..12].try_into().unwrap());
    let width = u32::from_le_bytes(body[12..16].try_into().unwrap());
    let height = u32::from_le_bytes(body[16..20].try_into().unwrap());
    let bpp = body[20];
    let ftype = body[21];
    let (format, red, green, blue) = match ftype {
        0 => (PixelFormat::Indexed, 0, 0, 0),
        1 => (PixelFormat::Rgb, 0, 0, 0),
        2 => {
            if body.len() < 36 {
                return Err(KernelError::BootInfo("fb tag short"));
            }
            (
                PixelFormat::EgaText,
                u32::from_le_bytes(body[24..28].try_into().unwrap()),
                u32::from_le_bytes(body[28..32].try_into().unwrap()),
                u32::from_le_bytes(body[32..36].try_into().unwrap()),
            )
        }
        _ => (PixelFormat::Indexed, 0, 0, 0),
    };
    info.framebuffer = Some(FramebufferInfo {
        addr,
        width,
        height,
        pitch,
        bpp,
        format,
        red_mask: red,
        green_mask: green,
        blue_mask: blue,
    });
    Ok(())
}

/// Total usable RAM (type 1 regions) in bytes.
pub fn usable_ram(info: &BootInfo) -> u64 {
    info.mem_regions
        .iter()
        .filter(|r| r.mem_type == 1)
        .map(|r| r.len)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal MB2 info blob: header + end tag.
    fn mb2_blob(tags: &[(u32, &[u8])]) -> alloc::vec::Vec<u8> {
        let mut out = alloc::vec::Vec::new();
        let mut body_len = 8usize; // end tag
        for (t, b) in tags {
            body_len += (8 + b.len() + 7) & !7;
            let _ = t;
        }
        out.extend_from_slice(&(body_len as u32).to_le_bytes());
        out.extend_from_slice(&[0u8; 4]);
        for (t, b) in tags {
            out.extend_from_slice(&t.to_le_bytes());
            out.extend_from_slice(&((8 + b.len()) as u32).to_le_bytes());
            out.extend_from_slice(b);
            while out.len() % 8 != 0 {
                out.push(0);
            }
        }
        out.extend_from_slice(&TAG_END.to_le_bytes());
        out.extend_from_slice(&8u32.to_le_bytes());
        out
    }

    #[test]
    fn parses_end_only() {
        let blob = mb2_blob(&[]);
        let info = parse_mb2(&blob).unwrap();
        assert!(info.mem_regions.is_empty());
        assert!(info.framebuffer.is_none());
    }

    #[test]
    fn rejects_bad_total() {
        let mut blob = mb2_blob(&[]);
        blob[0] = 0;
        blob[1] = 0;
        blob[2] = 0;
        blob[3] = 4; // total_size = 4 < 8
        assert!(parse_mb2(&blob).is_err());
    }

    #[test]
    fn parses_mem_map() {
        // entry_size=24, version=0, one entry: start=0x100000 len=0x100000 type=1
        let mut body = alloc::vec::Vec::new();
        body.extend_from_slice(&24u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&0x100000u64.to_le_bytes());
        body.extend_from_slice(&0x100000u64.to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&[0u8; 4]); // pad to 24
        let blob = mb2_blob(&[(TAG_MEM_MAP, &body)]);
        let info = parse_mb2(&blob).unwrap();
        assert_eq!(info.mem_regions.len(), 1);
        assert_eq!(usable_ram(&info), 0x100000);
    }

    #[test]
    fn parses_framebuffer() {
        let mut body = alloc::vec::Vec::new();
        body.extend_from_slice(&0xFD00_0000u64.to_le_bytes()); // addr
        body.extend_from_slice(&1920u32.to_le_bytes()); // pitch
        body.extend_from_slice(&1920u32.to_le_bytes()); // width
        body.extend_from_slice(&1080u32.to_le_bytes()); // height
        body.push(32); // bpp
        body.push(1); // type RGB
        body.extend_from_slice(&[0u8; 2]);
        let blob = mb2_blob(&[(TAG_FRAMEBUFFER, &body)]);
        let info = parse_mb2(&blob).unwrap();
        let fb = info.framebuffer.unwrap();
        assert_eq!((fb.width, fb.height, fb.bpp), (1920, 1080, 32));
        assert_eq!(fb.format, PixelFormat::Rgb);
    }

    #[test]
    fn parses_cmdline() {
        let blob = mb2_blob(&[(TAG_CMDLINE, b"quiet loglevel=3\0")]);
        let info = parse_mb2(&blob).unwrap();
        assert_eq!(info.cmdline.as_deref(), Some("quiet loglevel=3"));
    }
}
