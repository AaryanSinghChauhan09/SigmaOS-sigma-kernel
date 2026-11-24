//! VirtIO-MMIO transport model — register layout and virtqueue rings.
//!
//! Hardware access goes through `Mmio` trait; queue math is pure and
//! testable.

/// VirtIO MMIO register offsets (from base).
pub mod mmio {
    pub const MAGIC: u32 = 0x00;
    pub const VERSION: u32 = 0x04;
    pub const DEVICE_ID: u32 = 0x08;
    pub const VENDOR_ID: u32 = 0x0C;
    pub const HOST_FEATURES: u32 = 0x10;
    pub const HOST_FEATURES_SEL: u32 = 0x14;
    pub const GUEST_FEATURES: u32 = 0x20;
    pub const GUEST_FEATURES_SEL: u32 = 0x24;
    pub const QUEUE_SEL: u32 = 0x30;
    pub const QUEUE_NUM_MAX: u32 = 0x34;
    pub const QUEUE_NUM: u32 = 0x38;
    pub const QUEUE_READY: u32 = 0x44;
    pub const QUEUE_NOTIFY: u32 = 0x50;
    pub const INTERRUPT_STATUS: u32 = 0x60;
    pub const INTERRUPT_ACK: u32 = 0x64;
    pub const STATUS: u32 = 0x70;
    pub const QUEUE_DESC_LOW: u32 = 0x80;
    pub const QUEUE_DESC_HIGH: u32 = 0x84;
    pub const QUEUE_AVAIL_LOW: u32 = 0x90;
    pub const QUEUE_AVAIL_HIGH: u32 = 0x94;
    pub const QUEUE_USED_LOW: u32 = 0xa0;
    pub const QUEUE_USED_HIGH: u32 = 0xa4;
    pub const CONFIG_GENERATION: u32 = 0xfc;
}

/// Magic value "virt".
pub const VIRTIO_MAGIC: u32 = 0x7472_6976;

/// Device status bits.
pub mod status {
    pub const ACKNOWLEDGE: u32 = 1 << 0;
    pub const DRIVER: u32 = 1 << 1;
    pub const FEATURES_OK: u32 = 1 << 3;
    pub const DRIVER_OK: u32 = 1 << 4;
    pub const DEVICE_NEEDS_RESET: u32 = 1 << 6;
}

/// MMIO register access trait.
pub trait Mmio {
    fn read32(&self, offset: u32) -> u32;
    fn write32(&mut self, offset: u32, value: u32);
}

/// Virtqueue descriptor flags.
pub mod desc_flags {
    pub const NEXT: u32 = 1;
    pub const WRITE: u32 = 2; // device-writable
    pub const INDIRECT: u32 = 4;
}

/// A split virtqueue ring pair (avail + used) state tracker.
///
/// This models the driver side: we hand out descriptor indices and
/// track the used-ring consumption point.
pub struct VirtQueue {
    /// queue size (max descriptors)
    pub size: u16,
    /// next free descriptor head
    pub free_head: u16,
    /// number of descriptors in the current batch
    pub pending: u16,
}

impl VirtQueue {
    /// Create a queue of `size` descriptors (must be power of two).
    pub fn new(size: u16) -> Result<Self, crate::KernelError> {
        if size == 0 || (size & (size - 1)) != 0 {
            return Err(crate::KernelError::InvalidArg("queue size not power of two"));
        }
        Ok(Self { size, free_head: 0, pending: 0 })
    }

    /// Mask for ring wraparound.
    pub fn mask(&self) -> u16 {
        self.size - 1
    }

    /// Allocate the next descriptor index.
    pub fn next_desc(&mut self) -> Option<u16> {
        if self.pending >= self.size {
            return None;
        }
        let idx = self.free_head;
        self.free_head = (self.free_head + 1) & self.mask();
        self.pending += 1;
        Some(idx)
    }

    /// Called when the device consumes `n` used descriptors;
    /// recycles them into the free pool.
    pub fn reclaim(&mut self, n: u16) {
        self.pending = self.pending.saturating_sub(n);
    }

    /// Kick the queue (device polls avail ring).
    /// In real code this writes QUEUE_NOTIFY; here we just mark dirty.
    pub fn kick(&mut self) -> u16 {
        self.pending
    }
}

/// Device IDs we care about.
pub mod device {
    pub const NET: u32 = 1;
    pub const BLOCK: u32 = 2;
    pub const GPU: u32 = 16;
    pub const INPUT: u32 = 18;
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn queue_allocates_and_wraps() {
        let mut q = VirtQueue::new(8).unwrap();
        let mut got = Vec::new();
        for _ in 0..8 {
            got.push(q.next_desc().unwrap());
        }
        assert!(q.next_desc().is_none());
        assert_eq!(got, vec![0, 1, 2, 3, 4, 5, 6, 7]);
        q.reclaim(4);
        assert_eq!(q.next_desc().unwrap(), 0);
    }

    #[test]
    fn queue_rejects_non_pow2() {
        assert!(VirtQueue::new(7).is_err());
        assert!(VirtQueue::new(0).is_err());
    }

    #[test]
    fn magic_check() {
        assert_eq!(VIRTIO_MAGIC, u32::from_le_bytes(*b"virt"));
    }
}
