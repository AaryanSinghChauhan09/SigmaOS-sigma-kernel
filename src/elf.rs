//! ELF64 loader — parses an ELF image and produces a load plan.
//!
//! Inspired by Linux `fs/binfmt_elf.c` and the rust-vmm `vm-memory`
//! guest-memory model: the kernel reads the ELF header and program
//! headers, computes where each `PT_LOAD` segment lands, and the
//! caller copies bytes into allocated frames before jumping to
//! `entry`.

use crate::error::KernelError;
use alloc::vec::Vec;

pub const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
pub const ELFCLASS64: u8 = 2;
pub const ELFDATA2LSB: u8 = 1;
pub const ET_EXEC: u16 = 2;
pub const ET_DYN: u16 = 3;
pub const EM_X86_64: u16 = 62;
pub const PT_LOAD: u32 = 1;

const EHDR_SIZE: usize = 64;
const PHDR_SIZE: usize = 56;

/// Parsed ELF64 header fields we care about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfHeader {
    pub entry: u64,
    pub phoff: u64,
    pub phentsize: u16,
    pub phnum: u16,
    pub is_64: bool,
    pub is_le: bool,
    pub machine: u16,
    pub etype: u16,
}

/// One `PT_LOAD` program header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadSegment {
    pub vaddr: u64,
    pub offset: u64,
    pub filesz: u64,
    pub memsz: u64,
    pub flags: u32,
    pub align: u64,
}

impl LoadSegment {
    pub fn is_exec(&self) -> bool { self.flags & 1 != 0 }
    pub fn is_write(&self) -> bool { self.flags & 2 != 0 }
    pub fn is_read(&self) -> bool { self.flags & 4 != 0 }
}

fn u16le(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}
fn u32le(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}
fn u64le(b: &[u8], off: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(a)
}

/// Parse the ELF64 header. Rejects non-ELF, non-64-bit, non-LE,
/// non-x86_64 images.
pub fn parse_header(image: &[u8]) -> Result<ElfHeader, KernelError> {
    if image.len() < EHDR_SIZE || image[..4] != ELF_MAGIC {
        return Err(KernelError::Fs("not an ELF image"));
    }
    if image[4] != ELFCLASS64 || image[5] != ELFDATA2LSB {
        return Err(KernelError::Fs("unsupported ELF class/data"));
    }
    let machine = u16le(image, 18);
    if machine != EM_X86_64 {
        return Err(KernelError::Fs("not x86_64"));
    }
    Ok(ElfHeader {
        entry: u64le(image, 24),
        phoff: u64le(image, 32),
        phentsize: u16le(image, 54),
        phnum: u16le(image, 56),
        is_64: true,
        is_le: true,
        machine,
        etype: u16le(image, 16),
    })
}

/// Iterate `PT_LOAD` program headers.
pub fn load_segments(image: &[u8], hdr: &ElfHeader) -> Result<Vec<LoadSegment>, KernelError> {
    let phoff = hdr.phoff as usize;
    let phentsize = hdr.phentsize as usize;
    if phentsize < PHDR_SIZE {
        return Err(KernelError::Fs("bad phentsize"));
    }
    let mut segs = Vec::new();
    for i in 0..hdr.phnum as usize {
        let off = phoff + i * phentsize;
        if off + PHDR_SIZE > image.len() {
            return Err(KernelError::Fs("program headers out of bounds"));
        }
        let p_type = u32le(image, off);
        if p_type != PT_LOAD {
            continue;
        }
        segs.push(LoadSegment {
            flags: u32le(image, off + 4),
            offset: u64le(image, off + 8),
            vaddr: u64le(image, off + 16),
            filesz: u64le(image, off + 32),
            memsz: u64le(image, off + 40),
            align: u64le(image, off + 48),
        });
    }
    if segs.is_empty() {
        return Err(KernelError::Fs("no PT_LOAD segments"));
    }
    Ok(segs)
}

/// Total span of the loaded image (min vaddr .. max vaddr+memsz).
pub fn image_span(segs: &[LoadSegment]) -> (u64, u64) {
    let lo = segs.iter().map(|s| s.vaddr).min().unwrap_or(0);
    let hi = segs
        .iter()
        .map(|s| s.vaddr + s.memsz)
        .max()
        .unwrap_or(0);
    (lo, hi)
}

/// Compute the highest mapped address for a segment list — used to
/// size the user page tables / allocate frames.
pub fn highest_addr(segs: &[LoadSegment]) -> u64 {
    image_span(segs).1
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// Build a minimal valid ELF64 header + one PT_LOAD phdr.
    fn elf64(entry: u64, vaddr: u64, filesz: u64, memsz: u64) -> Vec<u8> {
        let mut e = vec![0u8; 4096];
        e[0..4].copy_from_slice(&ELF_MAGIC);
        e[4] = ELFCLASS64;
        e[5] = ELFDATA2LSB;
        e[16..18].copy_from_slice(&ET_EXEC.to_le_bytes());
        e[18..20].copy_from_slice(&EM_X86_64.to_le_bytes());
        e[24..32].copy_from_slice(&entry.to_le_bytes());
        e[32..40].copy_from_slice(&64u64.to_le_bytes()); // phoff
        e[54..56].copy_from_slice(&56u16.to_le_bytes()); // phentsize
        e[56..58].copy_from_slice(&1u16.to_le_bytes()); // phnum
        // phdr at 64
        e[64..68].copy_from_slice(&PT_LOAD.to_le_bytes());
        e[68..72].copy_from_slice(&5u32.to_le_bytes()); // R+X
        e[72..80].copy_from_slice(&0x1000u64.to_le_bytes()); // offset
        e[80..88].copy_from_slice(&vaddr.to_le_bytes());
        e[88..96].copy_from_slice(&0x1000u64.to_le_bytes()); // vaddr hi? no—paddr
        e[96..104].copy_from_slice(&filesz.to_le_bytes());   // p_filesz @+32
        e[104..112].copy_from_slice(&memsz.to_le_bytes());   // p_memsz @+40
        e[112..120].copy_from_slice(&0x1000u64.to_le_bytes()); // p_align @+48
        e
    }

    #[test]
    fn parses_header() {
        let img = elf64(0x400000, 0x400000, 0x100, 0x200);
        let h = parse_header(&img).unwrap();
        assert_eq!(h.entry, 0x400000);
        assert_eq!(h.phnum, 1);
        assert_eq!(h.machine, EM_X86_64);
    }

    #[test]
    fn rejects_non_elf() {
        assert!(parse_header(b"not elf").is_err());
        assert!(parse_header(&[]).is_err());
    }

    #[test]
    fn rejects_wrong_class() {
        let mut img = elf64(0, 0, 1, 1);
        img[4] = 1; // ELFCLASS32
        assert!(parse_header(&img).is_err());
    }

    #[test]
    fn extracts_load_segment() {
        let img = elf64(0x401000, 0x400000, 0x100, 0x200);
        let h = parse_header(&img).unwrap();
        let segs = load_segments(&img, &h).unwrap();
        assert_eq!(segs.len(), 1);
        let s = &segs[0];
        assert_eq!(s.vaddr, 0x400000);
        assert_eq!(s.filesz, 0x100);
        assert_eq!(s.memsz, 0x200);
        assert!(s.is_exec() && s.is_read());
        assert!(!s.is_write());
    }

    #[test]
    fn image_span_and_highest() {
        let segs = vec![
            LoadSegment { vaddr: 0x400000, offset: 0, filesz: 0x100, memsz: 0x100, flags: 5, align: 0x1000 },
            LoadSegment { vaddr: 0x401000, offset: 0, filesz: 0x80, memsz: 0x200, flags: 6, align: 0x1000 },
        ];
        let (lo, hi) = image_span(&segs);
        assert_eq!(lo, 0x400000);
        assert_eq!(hi, 0x401200);
        assert_eq!(highest_addr(&segs), 0x401200);
    }

    #[test]
    fn no_load_segments_is_error() {
        let mut img = elf64(0, 0, 1, 1);
        img[64..68].copy_from_slice(&2u32.to_le_bytes()); // PT_INTERP not LOAD
        let h = parse_header(&img).unwrap();
        assert!(load_segments(&img, &h).is_err());
    }
}
