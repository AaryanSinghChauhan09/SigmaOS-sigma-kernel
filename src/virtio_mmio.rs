//! VirtIO-MMIO device driver for block device.

use crate::KernelError;
use crate::virtio::{Mmio, mmio, device, status as virtio_status, VirtQueue, VIRTIO_MAGIC};
use crate::virtio_blk::{BlkConfig, OutHeader, req_type};

/// Simple VirtIO block device with MMIO transport.
pub struct VirtioBlk {
    base: u64,
    queue: VirtQueue,
    buffer: [u8; 512],
}

impl VirtioBlk {
    /// Initialize VirtIO block device at MMIO base.
    pub fn new(base: u64) -> Result<Self, KernelError> {
        let mut mmio = VirtioMmio { base };
        let magic = mmio.read32(mmio::MAGIC);
        if magic != VIRTIO_MAGIC {
            return Err(KernelError::Driver("bad virtio magic"));
        }
        let device_id = mmio.read32(mmio::DEVICE_ID);
        if device_id != device::BLOCK {
            return Err(KernelError::Driver("not a block device"));
        }
        mmio.write32(mmio::STATUS, virtio_status::ACKNOWLEDGE);
        mmio.write32(mmio::STATUS, virtio_status::DRIVER);
        mmio.write32(mmio::QUEUE_SEL, 0);
        let _max_size = mmio.read32(mmio::QUEUE_NUM_MAX);
        mmio.write32(mmio::QUEUE_NUM, 8);
        mmio.write32(mmio::QUEUE_READY, 1);
        let mut cfg_data = [0u8; 8];
        let cfg_offset = 0x100u32;
        for i in 0..8 {
            cfg_data[i] = mmio.read32(cfg_offset + i as u32 * 4) as u8;
        }
        let _config = BlkConfig::parse_capacity(&cfg_data)?;
        Ok(Self {
            base,
            queue: VirtQueue::new(8)?,
            buffer: [0u8; 512],
        })
    }

    /// Read one sector (simplified polling).
    pub fn read_sector(&mut self, sector: u64) -> Result<[u8; 512], KernelError> {
        let mut mmio = VirtioMmio { base: self.base };
        let header = OutHeader { req_type: req_type::READ, sector };
        let enc = header.encode();
        let desc_idx = self.queue.next_desc().ok_or(KernelError::Driver("queue full"))?;
        let desc_addr = self.base + mmio::QUEUE_DESC_LOW as u64;
        mmio.write32((desc_addr + desc_idx as u64 * 16) as u32, enc[0] as u32);
        mmio.write32((desc_addr + desc_idx as u64 * 16 + 4) as u32, enc[4] as u32);
        mmio.write32((desc_addr + desc_idx as u64 * 16 + 8) as u32, enc[8] as u32);
        mmio.write32((desc_addr + desc_idx as u64 * 16 + 12) as u32, enc[12] as u32);
        mmio.write32(mmio::QUEUE_NOTIFY, 0);
        for _ in 0..10000 {
            let status = mmio.read32(mmio::INTERRUPT_STATUS);
            if status != 0 {
                mmio.write32(mmio::INTERRUPT_ACK, status);
                break;
            }
        }
        Ok(self.buffer)
    }
}

struct VirtioMmio {
    base: u64,
}

impl Mmio for VirtioMmio {
    fn read32(&self, offset: u32) -> u32 {
        unsafe {
            ((self.base + offset as u64) as *const u32).read_volatile()
        }
    }

    fn write32(&mut self, offset: u32, value: u32) {
        unsafe {
            ((self.base + offset as u64) as *mut u32).write_volatile(value)
        }
    }
}
