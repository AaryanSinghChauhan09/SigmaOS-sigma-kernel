//! Physical memory management — 4 KiB frame allocator over a bitmap.
//!
//! The allocator is seeded from Multiboot2 usable regions. All state
//! is `alloc`-backed so it is testable on host.

use alloc::vec::Vec;
use crate::boot::MemRegion;
use crate::KernelError;

/// Frame size in bytes.
pub const FRAME_SIZE: u64 = 4096;

/// One contiguous usable memory area.
#[derive(Debug, Clone, Copy)]
struct Area {
    start_frame: u64,
    frames: u64,
}

/// Bitmap frame allocator.
pub struct FrameAllocator {
    areas: Vec<Area>,
    /// bitmap: 1 bit per frame across all areas, 1 = free
    bitmap: Vec<u64>,
    total_frames: u64,
    free_frames: u64,
}

impl FrameAllocator {
    /// Build an allocator covering the given memory regions.
    /// Only regions fully inside `usable` range and aligned are used.
    pub fn new(regions: &[MemRegion]) -> Result<Self, KernelError> {
        let mut areas = Vec::new();
        let mut total = 0u64;
        for r in regions {
            if r.mem_type != 1 || r.len == 0 {
                continue;
            }
            let start = (r.start + FRAME_SIZE - 1) / FRAME_SIZE * FRAME_SIZE;
            let end = (r.start + r.len) / FRAME_SIZE * FRAME_SIZE;
            if end <= start {
                continue;
            }
            let frames = (end - start) / FRAME_SIZE;
            areas.push(Area { start_frame: start / FRAME_SIZE, frames });
            total += frames;
        }
        if total == 0 {
            return Err(KernelError::OutOfMemory);
        }
        let words = ((total + 63) / 64) as usize;
        // all bits set = free; trailing bits beyond total are cleared
        let mut bitmap = alloc::vec![u64::MAX; words];
        let rem = total % 64;
        if rem != 0 {
            let last = words - 1;
            bitmap[last] = (1u64 << rem) - 1;
        }
        Ok(Self {
            areas,
            bitmap,
            total_frames: total,
            free_frames: total,
        })
    }

    /// Allocate a single frame. Returns its physical address.
    pub fn alloc_frame(&mut self) -> Option<u64> {
        for (wi, word) in self.bitmap.iter_mut().enumerate() {
            if *word != 0 {
                let bit = word.trailing_zeros() as usize;
                *word &= !(1u64 << bit);
                let idx = wi * 64 + bit;
                self.free_frames -= 1;
                return Some(self.frame_to_phys(idx as u64));
            }
        }
        None
    }

    /// Free a frame by physical address.
    pub fn free_frame(&mut self, phys: u64) {
        if let Some(idx) = self.phys_to_frame(phys) {
            let wi = (idx / 64) as usize;
            let bit = (idx % 64) as u32;
            if self.bitmap[wi] & (1u64 << bit) == 0 {
                self.bitmap[wi] |= 1u64 << bit;
                self.free_frames += 1;
            }
        }
    }

    /// Allocate `n` contiguous frames.
    pub fn alloc_contiguous(&mut self, n: u64) -> Option<u64> {
        let mut run_start: Option<u64> = None;
        let mut run = 0u64;
        for idx in 0..self.total_frames {
            let free = self.is_free(idx);
            if free {
                if run_start.is_none() {
                    run_start = Some(idx);
                }
                run += 1;
                if run == n {
                    let start = run_start.unwrap();
                    for i in start..start + n {
                        self.clear_bit(i);
                    }
                    self.free_frames -= n;
                    return Some(self.frame_to_phys(start));
                }
            } else {
                run_start = None;
                run = 0;
            }
        }
        None
    }

    /// Total frames managed.
    pub fn total_frames(&self) -> u64 {
        self.total_frames
    }

    /// Free frames remaining.
    pub fn free_frames(&self) -> u64 {
        self.free_frames
    }

    fn frame_to_phys(&self, idx: u64) -> u64 {
        // map global frame index -> physical address across areas
        let mut base = 0u64;
        for a in &self.areas {
            if idx < base + a.frames {
                return (a.start_frame + (idx - base)) * FRAME_SIZE;
            }
            base += a.frames;
        }
        0
    }

    fn phys_to_frame(&self, phys: u64) -> Option<u64> {
        let frame = phys / FRAME_SIZE;
        let mut base = 0u64;
        for a in &self.areas {
            if frame >= a.start_frame && frame < a.start_frame + a.frames {
                return Some(base + (frame - a.start_frame));
            }
            base += a.frames;
        }
        None
    }

    fn is_free(&self, idx: u64) -> bool {
        let wi = (idx / 64) as usize;
        let bit = (idx % 64) as u32;
        self.bitmap[wi] & (1u64 << bit) != 0
    }

    fn clear_bit(&mut self, idx: u64) {
        let wi = (idx / 64) as usize;
        let bit = (idx % 64) as u32;
        self.bitmap[wi] &= !(1u64 << bit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(start: u64, len: u64) -> MemRegion {
        MemRegion { start, len, mem_type: 1 }
    }

    #[test]
    fn alloc_and_free() {
        let mut a = FrameAllocator::new(&[region(0x100000, 0x10000)]).unwrap();
        assert_eq!(a.total_frames(), 16);
        let p = a.alloc_frame().unwrap();
        assert!(p >= 0x100000 && p < 0x110000);
        assert_eq!(a.free_frames(), 15);
        a.free_frame(p);
        assert_eq!(a.free_frames(), 16);
    }

    #[test]
    fn contiguous_alloc() {
        let mut a = FrameAllocator::new(&[region(0x100000, 0x10000)]).unwrap();
        let p = a.alloc_contiguous(4).unwrap();
        assert_eq!(p % FRAME_SIZE, 0);
        assert_eq!(a.free_frames(), 12);
    }

    #[test]
    fn skips_non_usable() {
        let regs = [
            MemRegion { start: 0, len: 0x1000, mem_type: 2 },
            region(0x100000, 0x10000),
        ];
        let a = FrameAllocator::new(&regs).unwrap();
        assert_eq!(a.total_frames(), 16);
    }

    #[test]
    fn empty_regions_error() {
        assert!(FrameAllocator::new(&[]).is_err());
    }

    #[test]
    fn double_free_is_idempotent() {
        let mut a = FrameAllocator::new(&[region(0x100000, 0x10000)]).unwrap();
        let p = a.alloc_frame().unwrap();
        a.free_frame(p);
        a.free_frame(p);
        assert_eq!(a.free_frames(), 16);
    }
}
