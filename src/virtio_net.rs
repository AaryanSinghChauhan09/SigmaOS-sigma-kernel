//! VirtIO network device (modern virtio, device id 1).
//!
//! Ethernet packet codec + virtio_net header handling. The header is
//! 12 bytes for modern devices with mergeable buffers; legacy is 10.
//! Transport (queue submission) is abstracted for testability.

use alloc::vec;
use alloc::vec::Vec;

use crate::KernelError;

/// virtio-net header sizes.
pub mod hdr {
    /// Legacy header (10 bytes).
    pub const LEGACY: usize = 10;
    /// Modern header with num_buffers (12 bytes).
    pub const MODERN: usize = 12;
}

/// A parsed virtio-net header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetHeader {
    /// Flags: NEEDS_CSUM(1), DATA_VALID(2), RSC_INFO(4).
    pub flags: u8,
    /// GSO type (0 = none).
    pub gso_type: u8,
    /// Header length for GSO.
    pub hdr_len: u16,
    /// GSO size.
    pub gso_size: u16,
    /// Checksum start offset.
    pub csum_start: u16,
    /// Checksum offset.
    pub csum_offset: u16,
    /// Number of buffers (modern only).
    pub num_buffers: u16,
}

impl NetHeader {
    /// Zeroed header for a plain (non-GSO) packet.
    pub const fn plain() -> Self {
        Self {
            flags: 0,
            gso_type: 0,
            hdr_len: 0,
            gso_size: 0,
            csum_start: 0,
            csum_offset: 0,
            num_buffers: 1,
        }
    }

    /// Encode in modern (12-byte) form.
    pub fn encode_modern(&self) -> [u8; hdr::MODERN] {
        let mut b = [0u8; hdr::MODERN];
        b[0] = self.flags;
        b[1] = self.gso_type;
        b[2..4].copy_from_slice(&self.hdr_len.to_le_bytes());
        b[4..6].copy_from_slice(&self.gso_size.to_le_bytes());
        b[6..8].copy_from_slice(&self.csum_start.to_le_bytes());
        b[8..10].copy_from_slice(&self.csum_offset.to_le_bytes());
        b[10..12].copy_from_slice(&self.num_buffers.to_le_bytes());
        b
    }

    /// Decode a modern header.
    pub fn decode_modern(b: &[u8]) -> Result<Self, KernelError> {
        if b.len() < hdr::MODERN {
            return Err(KernelError::InvalidArg("short net header"));
        }
        Ok(Self {
            flags: b[0],
            gso_type: b[1],
            hdr_len: u16::from_le_bytes(b[2..4].try_into().unwrap()),
            gso_size: u16::from_le_bytes(b[4..6].try_into().unwrap()),
            csum_start: u16::from_le_bytes(b[6..8].try_into().unwrap()),
            csum_offset: u16::from_le_bytes(b[8..10].try_into().unwrap()),
            num_buffers: u16::from_le_bytes(b[10..12].try_into().unwrap()),
        })
    }
}

/// virtio-net device config (subset): MAC + MTU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetConfig {
    /// Device MAC address.
    pub mac: [u8; 6],
    /// Max packet size the device accepts.
    pub mtu: u16,
}

impl NetConfig {
    /// Parse the modern config space: mac[6], status u16,
    /// max_virtqueue_pairs u16, mtu u16.
    pub fn parse(bytes: &[u8]) -> Result<Self, KernelError> {
        if bytes.len() < 12 {
            return Err(KernelError::InvalidArg("net config too small"));
        }
        let mut mac = [0u8; 6];
        mac.copy_from_slice(&bytes[0..6]);
        let mtu = u16::from_le_bytes(bytes[10..12].try_into().unwrap());
        Ok(Self { mac, mtu })
    }

    /// Human-readable MAC.
    pub fn mac_string(&self) -> alloc::string::String {
        alloc::format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.mac[0],
            self.mac[1],
            self.mac[2],
            self.mac[3],
            self.mac[4],
            self.mac[5]
        )
    }
}

/// Ethernet frame helpers.
pub mod eth {
    /// EtherType IPv4.
    pub const IPV4: u16 = 0x0800;
    /// EtherType ARP.
    pub const ARP: u16 = 0x0806;
    /// Broadcast MAC.
    pub const BROADCAST: [u8; 6] = [0xff; 6];

    /// Build an Ethernet header.
    pub fn header(dst: [u8; 6], src: [u8; 6], ethertype: u16) -> [u8; 14] {
        let mut h = [0u8; 14];
        h[0..6].copy_from_slice(&dst);
        h[6..12].copy_from_slice(&src);
        h[12..14].copy_from_slice(&ethertype.to_be_bytes());
        h
    }

    /// Parse destination MAC and ethertype from a frame.
    pub fn parse(frame: &[u8]) -> Option<([u8; 6], [u8; 6], u16)> {
        if frame.len() < 14 {
            return None;
        }
        let mut dst = [0u8; 6];
        let mut src = [0u8; 6];
        dst.copy_from_slice(&frame[0..6]);
        src.copy_from_slice(&frame[6..12]);
        let et = u16::from_be_bytes([frame[12], frame[13]]);
        Some((dst, src, et))
    }
}

/// Packet codec: wraps/unwraps virtio-net headers around Ethernet
/// frames for TX/RX descriptor buffers.
pub struct NetCodec {
    /// Whether the device uses the modern 12-byte header.
    pub modern: bool,
}

impl NetCodec {
    pub fn new(modern: bool) -> Self {
        Self { modern }
    }

    /// Header size for this device.
    pub fn header_len(&self) -> usize {
        if self.modern {
            hdr::MODERN
        } else {
            hdr::LEGACY
        }
    }

    /// Build a TX buffer: virtio-net header + ethernet frame.
    pub fn encode_tx(&self, frame: &[u8]) -> Result<Vec<u8>, KernelError> {
        let h = NetHeader::plain();
        let hdr = if self.modern {
            h.encode_modern().to_vec()
        } else {
            // legacy: first 10 bytes (num_buffers absent)
            h.encode_modern()[..hdr::LEGACY].to_vec()
        };
        let mut out = hdr;
        out.extend_from_slice(frame);
        Ok(out)
    }

    /// Parse an RX buffer into (header, frame).
    pub fn decode_rx<'a>(&self, buf: &'a [u8]) -> Result<(NetHeader, &'a [u8]), KernelError> {
        let n = self.header_len();
        if buf.len() < n {
            return Err(KernelError::InvalidArg("rx buffer too small"));
        }
        let header = if self.modern {
            NetHeader::decode_modern(&buf[..n])?
        } else {
            let m = NetHeader::decode_modern(&[buf[..10].to_vec(), vec![0, 0]].concat())?;
            m
        };
        Ok((header, &buf[n..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn header_roundtrip() {
        let h = NetHeader {
            flags: 2,
            gso_type: 0,
            hdr_len: 0,
            gso_size: 0,
            csum_start: 40,
            csum_offset: 16,
            num_buffers: 3,
        };
        let enc = h.encode_modern();
        assert_eq!(enc.len(), 12);
        assert_eq!(NetHeader::decode_modern(&enc).unwrap(), h);
    }

    #[test]
    fn config_parse_mac() {
        let mut b = vec![0u8; 12];
        b[0..6].copy_from_slice(&[0x52, 0x54, 0, 1, 2, 3]);
        b[10..12].copy_from_slice(&1500u16.to_be_bytes());
        // note: config fields after mac are host-endian? spec says
        // little-endian for modern; test uses LE
        let mut b2 = vec![0u8; 12];
        b2[0..6].copy_from_slice(&[0x52, 0x54, 0, 1, 2, 3]);
        b2[10..12].copy_from_slice(&1500u16.to_le_bytes());
        let cfg = NetConfig::parse(&b2).unwrap();
        assert_eq!(cfg.mac, [0x52, 0x54, 0, 1, 2, 3]);
        assert_eq!(cfg.mtu, 1500);
        let _ = b;
    }

    #[test]
    fn codec_roundtrip() {
        let c = NetCodec::new(true);
        let frame = eth::header([1, 2, 3, 4, 5, 6], [6, 5, 4, 3, 2, 1], eth::IPV4);
        let mut pkt = frame.to_vec();
        pkt.extend_from_slice(&[0xaa; 46]); // pad to min eth size
        let tx = c.encode_tx(&pkt).unwrap();
        assert_eq!(tx.len(), 12 + pkt.len());
        let (hdr, payload) = c.decode_rx(&tx).unwrap();
        assert_eq!(hdr.num_buffers, 1);
        assert_eq!(payload, pkt.as_slice());
    }

    #[test]
    fn legacy_header() {
        let c = NetCodec::new(false);
        assert_eq!(c.header_len(), 10);
        let tx = c.encode_tx(&[0u8; 60]).unwrap();
        assert_eq!(tx.len(), 70);
    }

    #[test]
    fn eth_parse() {
        let f = eth::header(eth::BROADCAST, [1; 6], eth::ARP);
        let (d, s, t) = eth::parse(&f).unwrap();
        assert_eq!(d, eth::BROADCAST);
        assert_eq!(s, [1; 6]);
        assert_eq!(t, eth::ARP);
        assert!(eth::parse(&[0u8; 5]).is_none());
    }
}
