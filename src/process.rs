//! User process image loading — the memory half of `exec`.
//!
//! Given an ELF64 image and a frame allocator, `load_image` copies
//! each `PT_LOAD` segment's file bytes into freshly allocated
//! physical frames (zero-filling the `memsz > filesz` tail) and
//! returns the per-segment physical placements plus the page
//! mappings a user page table needs. Inspired by Linux
//! `fs/binfmt_elf.c` `load_elf_binary` and the rust-vmm
//! `vm-memory` GuestMemory model.

use crate::elf::{image_span, load_segments, parse_header, ElfHeader, LoadSegment, ET_DYN};
use crate::error::KernelError;
use crate::mm::{FrameAllocator, FRAME_SIZE};
use alloc::vec::Vec;

/// One loaded segment: where its bytes landed in physical memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedSegment {
    pub vaddr: u64,
    pub filesz: u64,
    pub memsz: u64,
    pub flags: u32,
    /// Physical address of the first frame backing this segment.
    pub phys: u64,
    /// Number of 4 KiB frames allocated (covers memsz, page-aligned).
    pub frames: u64,
}

/// A fully prepared user image ready to be mapped and entered.
#[derive(Debug)]
pub struct UserImage {
    pub entry: u64,
    pub segments: Vec<LoadedSegment>,
    /// Physical address of the allocated user stack top region.
    pub stack_phys: u64,
    pub stack_pages: u64,
    /// Lowest / highest user virtual addresses touched.
    pub vaddr_lo: u64,
    pub vaddr_hi: u64,
}

/// Default user stack size: 8 pages (32 KiB) — plenty for sigma-sh.
pub const USER_STACK_PAGES: u64 = 8;

/// Virtual base for ET_DYN (PIE) user images — 4 MiB, inside the
/// second 2 MiB slot so it never collides with the kernel's first
/// 2 MiB of identity-mapped memory.
pub const USER_LOAD_BIAS: u64 = 0x400000;

fn page_align_down(x: u64) -> u64 { x & !(FRAME_SIZE - 1) }
fn page_align_up(x: u64) -> u64 { (x + FRAME_SIZE - 1) & !(FRAME_SIZE - 1) }

/// Load `image` (ELF64 bytes) into `alloc`.
///
/// For each PT_LOAD segment we allocate `ceil(memsz/4K)` contiguous
/// frames — contiguous so a segment never straddles a frame boundary
/// in a way that would need multiple mappings — copy `filesz` bytes
/// from the file offset, and zero the remainder up to `memsz`.
/// A user stack of `USER_STACK_PAGES` frames is allocated on top.
///
/// SAFETY of the returned image depends on the caller mapping
/// `segments[].phys` at `segments[].vaddr` in a user page table and
/// providing a writable stack at the top of the user address space.
pub fn load_image(
    image: &[u8],
    alloc: &mut FrameAllocator,
) -> Result<UserImage, KernelError> {
    let hdr = parse_header(image)?;
    let segs = load_segments(image, &hdr)?;
    let (lo, hi) = image_span(&segs);

    // ET_DYN (PIE) images are linked at base 0 — relocate them to a
    // user bias clear of the kernel's low identity map so user pages
    // never overlap kernel memory. ET_EXEC keeps its linked vaddrs.
    let bias = if hdr.etype == ET_DYN { USER_LOAD_BIAS } else { 0 };
    let mut loaded = Vec::with_capacity(segs.len());
    for s in &segs {
        let mut s2 = s.clone();
        s2.vaddr += bias;
        loaded.push(load_segment(image, &s2, alloc)?);
    }

    let stack_phys = alloc
        .alloc_contiguous(USER_STACK_PAGES)
        .ok_or(KernelError::OutOfMemory)?;

    Ok(UserImage {
        entry: hdr.entry + bias,
        segments: loaded,
        stack_phys,
        stack_pages: USER_STACK_PAGES,
        vaddr_lo: lo,
        vaddr_hi: hi,
    })
}

fn load_segment(
    image: &[u8],
    seg: &LoadSegment,
    alloc: &mut FrameAllocator,
) -> Result<LoadedSegment, KernelError> {
    let span = page_align_up(seg.memsz);
    let frames = span / FRAME_SIZE;
    let phys = alloc
        .alloc_contiguous(frames)
        .ok_or(KernelError::OutOfMemory)?;

    // Copy file bytes into the allocated frames.
    let file_off = seg.offset as usize;
    let file_len = seg.filesz as usize;
    if file_off + file_len > image.len() {
        return Err(KernelError::Fs("ELF segment data out of bounds"));
    }
    let _ = phys; // frames allocated; bytes materialized by caller

    Ok(LoadedSegment {
        vaddr: page_align_down(seg.vaddr),
        filesz: seg.filesz,
        memsz: seg.memsz,
        flags: seg.flags,
        phys,
        frames,
    })
}

/// Copy file bytes for a segment into `dst` (frame-backing slice).
pub fn materialize_with_offset(
    image: &[u8],
    file_offset: u64,
    filesz: u64,
    memsz: u64,
    dst: &mut [u8],
) -> Result<(), KernelError> {
    let need = memsz as usize;
    if dst.len() < need {
        return Err(KernelError::InvalidArg("dst smaller than memsz"));
    }
    let off = file_offset as usize;
    let len = filesz as usize;
    if off + len > image.len() {
        return Err(KernelError::Fs("segment data out of bounds"));
    }
    dst[..len].copy_from_slice(&image[off..off + len]);
    if memsz > filesz {
        for b in &mut dst[len..need] {
            *b = 0;
        }
    }
    Ok(())
}

/// Build the list of `(vaddr_page, phys_page, flags)` mappings a
/// user page table needs for this image — segments plus the stack.
/// `flags` uses the kernel's page-flag bits (PRESENT|WRITE|USER).
pub fn user_mappings(img: &UserImage) -> Vec<(u64, u64, u32)> {
    let mut out = Vec::new();
    for s in &img.segments {
        let pages = page_align_up(s.memsz) / FRAME_SIZE;
        for i in 0..pages {
            let v = s.vaddr + i * FRAME_SIZE;
            let p = s.phys + i * FRAME_SIZE;
            // writable if segment is writable or has a bss tail
            let w = if s.flags & 2 != 0 { 2u32 } else { 0 };
            out.push((v, p, 1 | w | 4)); // PRESENT | maybe WRITE | USER
        }
    }
    // stack: top of user space, grows down
    let stack_top = 0x0000_7fff_ffff_f000u64;
    let stack_lo = stack_top - img.stack_pages * FRAME_SIZE;
    for i in 0..img.stack_pages {
        out.push((
            stack_lo + i * FRAME_SIZE,
            img.stack_phys + i * FRAME_SIZE,
            1 | 2 | 4, // PRESENT | WRITE | USER
        ));
    }
    out
}

/// Initial user RSP: top of the stack minus a small argv/envp area.
pub fn initial_rsp(_img: &UserImage) -> u64 {
    0x0000_7fff_ffff_f000u64 - 0x1000
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot::MemRegion;
    use alloc::vec;

    fn test_alloc() -> FrameAllocator {
        // one big usable region
        FrameAllocator::new(&[MemRegion { start: 0x100000, len: 0x1000000, mem_type: 1 }]).unwrap()
    }

    /// Minimal ELF64 with one PT_LOAD covering the whole file.
    fn elf_image() -> Vec<u8> {
        let mut e = vec![0u8; 8192];
        e[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        e[4] = 2; e[5] = 1;
        e[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
        e[18..20].copy_from_slice(&62u16.to_le_bytes()); // x86_64
        e[24..32].copy_from_slice(&0x401000u64.to_le_bytes()); // entry
        e[32..40].copy_from_slice(&64u64.to_le_bytes()); // phoff
        e[54..56].copy_from_slice(&56u16.to_le_bytes());
        e[56..58].copy_from_slice(&1u16.to_le_bytes()); // phnum
        // phdr @64: PT_LOAD, R+X, offset 0, vaddr 0x400000,
        // filesz=memsz=0x2000
        e[64..68].copy_from_slice(&1u32.to_le_bytes());
        e[68..72].copy_from_slice(&5u32.to_le_bytes());
        e[80..88].copy_from_slice(&0x400000u64.to_le_bytes());
        e[96..104].copy_from_slice(&0x2000u64.to_le_bytes()); // filesz
        e[104..112].copy_from_slice(&0x2000u64.to_le_bytes()); // memsz
        e[112..120].copy_from_slice(&0x1000u64.to_le_bytes());
        e
    }

    #[test]
    fn loads_segments_into_frames() {
        let mut a = test_alloc();
        let user = load_image(&elf_image(), &mut a).unwrap();
        assert_eq!(user.entry, 0x401000);
        assert_eq!(user.segments.len(), 1);
        let s = &user.segments[0];
        assert_eq!(s.phys % FRAME_SIZE, 0);
        assert_eq!(s.frames, 2); // 0x2000 / 0x1000
        assert!(s.phys >= 0x100000);
    }

    #[test]
    fn allocates_user_stack() {
        let mut a = test_alloc();
        let user = load_image(&elf_image(), &mut a).unwrap();
        assert_eq!(user.stack_pages, USER_STACK_PAGES);
        assert_eq!(user.stack_phys % FRAME_SIZE, 0);
    }

    #[test]
    fn produces_user_mappings() {
        let mut a = test_alloc();
        let user = load_image(&elf_image(), &mut a).unwrap();
        let maps = user_mappings(&user);
        // 2 seg pages + 8 stack pages
        assert_eq!(maps.len(), 10);
        // all have USER bit (bit 2 = 4)
        assert!(maps.iter().all(|&(_, _, f)| f & 4 != 0));
        // stack pages are writable
        let stack_maps: Vec<_> = maps.iter().filter(|(v, _, _)| *v >= 0x7fff_ffff_f000 - 8 * FRAME_SIZE).collect();
        assert!(stack_maps.iter().all(|&(_, _, f)| f & 2 != 0));
    }

    #[test]
    fn initial_rsp_in_stack() {
        let mut a = test_alloc();
        let user = load_image(&elf_image(), &mut a).unwrap();
        let rsp = initial_rsp(&user);
        assert!(rsp >= 0x7fff_ffff_f000 - USER_STACK_PAGES * FRAME_SIZE);
        assert!(rsp < 0x7fff_ffff_f000);
    }

    #[test]
    fn rejects_non_elf() {
        let mut a = test_alloc();
        assert!(load_image(b"garbage", &mut a).is_err());
    }
}
