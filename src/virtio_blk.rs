//! VirtIO block device (modern virtio, device id 2).
//!
//! Models the request protocol: 16-byte out-header + data + 1-byte
//! status, submitted as a 3-descriptor chain on queue 0. Transport is
//! abstracted behind `BlkTransport` so protocol logic is testable
//! without MMIO.

use alloc::vec::Vec;

use crate::KernelError;

/// Request types (virtio_blk spec).
pub mod req_type {
    pub const READ: u32 = 0;
    pub const WRITE: u32 = 1;
    pub const FLUSH: u32 = 4;
    pub const GET_ID: u32 = 8;
    pub const DISCARD: u32 = 11;
    pub const WRITE_ZEROES: u32 = 13;
}

/// virtio-blk config space (subset): capacity in 512-byte sectors.
#[derive(Debug, Clone)]
pub struct BlkConfig {
    /// Capacity in sectors.
    pub capacity_sectors: u64,
    /// Sector size (always 512 for virtio-blk).
    pub sector_size: u32,
}

impl BlkConfig {
    /// Parse capacity (first 8 bytes of config space, little-endian).
    pub fn parse_capacity(bytes: &[u8]) -> Result<Self, KernelError> {
        if bytes.len() < 8 {
            return Err(KernelError::InvalidArg("blk config too small"));
        }
        let capacity_sectors = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        Ok(Self { capacity_sectors, sector_size: 512 })
    }

    /// Total capacity in bytes.
    pub fn capacity_bytes(&self) -> u64 {
        self.capacity_sectors.saturating_mul(self.sector_size as u64)
    }
}

/// A block request header (device-readable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutHeader {
    pub req_type: u32,
    pub sector: u64,
}

impl OutHeader {
    /// Encode to the 16-byte wire format.
    pub fn encode(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0..4].copy_from_slice(&self.req_type.to_le_bytes());
        // bytes 4..8 reserved = 0
        b[8..16].copy_from_slice(&self.sector.to_le_bytes());
        b
    }

    /// Decode from wire bytes.
    pub fn decode(b: &[u8]) -> Result<Self, KernelError> {
        if b.len() < 16 {
            return Err(KernelError::InvalidArg("short blk header"));
        }
        Ok(Self {
            req_type: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            sector: u64::from_le_bytes(b[8..16].try_into().unwrap()),
        })
    }
}

/// Status byte values (device-writable).
pub mod status {
    pub const OK: u8 = 0;
    pub const IOERR: u8 = 1;
    pub const UNSUP: u8 = 2;
}

/// Interpret a status byte.
pub fn check_status(s: u8) -> Result<(), KernelError> {
    match s {
        status::OK => Ok(()),
        status::IOERR => Err(KernelError::Driver("virtio-blk IO error")),
        status::UNSUP => Err(KernelError::Driver("virtio-blk unsupported op")),
        _ => Err(KernelError::Driver("virtio-blk bad status")),
    }
}

/// Sector-level block device.
///
/// Implementations perform the actual queue submission; this trait is
/// the contract both the MMIO driver and tests satisfy.
pub trait BlkTransport {
    /// Read `len` bytes starting at `sector` into `buf`.
    /// `buf.len()` must be a multiple of 512.
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), KernelError>;
    /// Write `data` starting at `sector`.
    fn write(&mut self, sector: u64, data: &[u8]) -> Result<(), KernelError>;
    /// Flush device caches.
    fn flush(&mut self) -> Result<(), KernelError>;
    /// Device serial (16 bytes ASCII, may be empty).
    fn serial(&self) -> &[u8];
}

/// Sector-addressable block device wrapper enforcing bounds.
pub struct BlockDevice<T> {
    pub transport: T,
    pub config: BlkConfig,
}

impl<T: BlkTransport> BlockDevice<T> {
    pub fn new(transport: T, config: BlkConfig) -> Self {
        Self { transport, config }
    }

    /// Read one sector.
    pub fn read_sector(&mut self, sector: u64) -> Result<[u8; 512], KernelError> {
        if sector >= self.config.capacity_sectors {
            return Err(KernelError::InvalidArg("sector out of range"));
        }
        let mut buf = [0u8; 512];
        self.transport.read(sector, &mut buf)?;
        Ok(buf)
    }

    /// Read `n` sectors.
    pub fn read_sectors(&mut self, sector: u64, n: u64) -> Result<Vec<u8>, KernelError> {
        if sector.checked_add(n).map_or(true, |e| e > self.config.capacity_sectors) {
            return Err(KernelError::InvalidArg("read past end"));
        }
        let mut buf = alloc::vec![0u8; (n * 512) as usize];
        self.transport.read(sector, &mut buf)?;
        Ok(buf)
    }

    /// Write one sector.
    pub fn write_sector(&mut self, sector: u64, data: &[u8; 512]) -> Result<(), KernelError> {
        if sector >= self.config.capacity_sectors {
            return Err(KernelError::InvalidArg("sector out of range"));
        }
        self.transport.write(sector, data)
    }

    /// Byte offset of a sector (for callers building disk images).
    pub fn sector_offset(&self, sector: u64) -> u64 {
        sector * self.config.sector_size as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    struct FakeTransport {
        backing: Vec<u8>,
        flushed: bool,
    }
    impl BlkTransport for FakeTransport {
        fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), KernelError> {
            let off = sector as usize * 512;
            buf.copy_from_slice(&self.backing[off..off + buf.len()]);
            Ok(())
        }
        fn write(&mut self, sector: u64, data: &[u8]) -> Result<(), KernelError> {
            let off = sector as usize * 512;
            self.backing[off..off + data.len()].copy_from_slice(data);
            Ok(())
        }
        fn flush(&mut self) -> Result<(), KernelError> {
            self.flushed = true;
            Ok(())
        }
        fn serial(&self) -> &[u8] {
            b"SIGMA-TEST-DISK"
        }
    }

    fn dev() -> BlockDevice<FakeTransport> {
        let cfg = BlkConfig { capacity_sectors: 4, sector_size: 512 };
        BlockDevice::new(
            FakeTransport { backing: vec![0u8; 4 * 512], flushed: false },
            cfg,
        )
    }

    #[test]
    fn header_roundtrip() {
        let h = OutHeader { req_type: req_type::READ, sector: 0xdead_beef };
        let enc = h.encode();
        assert_eq!(enc[0], 0); // type LE
        assert_eq!(u64::from_le_bytes(enc[8..16].try_into().unwrap()), 0xdead_beef);
        assert_eq!(OutHeader::decode(&enc).unwrap(), h);
    }

    #[test]
    fn config_parse() {
        let cfg = BlkConfig::parse_capacity(&8u64.to_le_bytes()).unwrap();
        assert_eq!(cfg.capacity_sectors, 8);
        assert_eq!(cfg.capacity_bytes(), 4096);
    }

    #[test]
    fn sector_io_bounds() {
        let mut d = dev();
        assert!(d.read_sector(4).is_err());
        let mut s = [0u8; 512];
        for i in 0..512 {
            s[i] = (i % 256) as u8;
        }
        d.write_sector(1, &s).unwrap();
        assert_eq!(d.read_sector(1).unwrap(), s);
    }

    #[test]
    fn read_sectors_range() {
        let mut d = dev();
        let data = d.read_sectors(0, 2).unwrap();
        assert_eq!(data.len(), 1024);
        assert!(d.read_sectors(3, 2).is_err()); // 3+2 > 4
    }

    #[test]
    fn status_mapping() {
        assert!(check_status(status::OK).is_ok());
        assert!(check_status(status::IOERR).is_err());
        assert!(check_status(status::UNSUP).is_err());
        assert!(check_status(99).is_err());
    }
}
