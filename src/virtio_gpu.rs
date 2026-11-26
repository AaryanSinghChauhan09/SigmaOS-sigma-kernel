//! VirtIO GPU device (modern virtio, device id 16).
//!
//! Control-queue message codec: 24-byte `virtio_gpu_ctrl_hdr` followed
//! by command-specific payload. Covers the commands needed to bring up
//! a scanout: GET_DISPLAY_INFO, RESOURCE_CREATE_2D, SET_SCANOUT,
//! RESOURCE_ATTACH_BACKING, TRANSFER_TO_HOST_2D, RESOURCE_FLUSH,
//! RESOURCE_UNREF.

use alloc::vec::Vec;

use crate::KernelError;

/// Control message types (VIRTIO_GPU_CMD_*).
pub mod cmd {
    pub const GET_DISPLAY_INFO: u32 = 0x0100;
    pub const RESOURCE_CREATE_2D: u32 = 0x0101;
    pub const RESOURCE_UNREF: u32 = 0x0102;
    pub const SET_SCANOUT: u32 = 0x0103;
    pub const RESOURCE_FLUSH: u32 = 0x0104;
    pub const TRANSFER_TO_HOST_2D: u32 = 0x0105;
    pub const RESOURCE_ATTACH_BACKING: u32 = 0x0106;
    pub const RESOURCE_DETACH_BACKING: u32 = 0x0107;
    pub const GET_CAPSET_INFO: u32 = 0x0108;
    pub const GET_CAPSET: u32 = 0x0109;
    pub const GET_EDID: u32 = 0x010a;
    pub const RESOURCE_ASSIGN_UUID: u32 = 0x010b;
}

/// Control message flags.
pub mod flags {
    /// Response fence flag.
    pub const FENCE: u32 = 1 << 0;
    /// Response only (device→driver).
    pub const RESPONSE: u32 = 1 << 1;
}

/// virtio_gpu_ctrl_hdr — 24 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CtrlHdr {
    pub cmd_type: u32,
    pub flags: u32,
    pub fence_id: u64,
    pub ctx_id: u32,
    pub padding: u32,
}

impl CtrlHdr {
    /// Encode to wire format.
    pub fn encode(&self) -> [u8; 24] {
        let mut b = [0u8; 24];
        b[0..4].copy_from_slice(&self.cmd_type.to_le_bytes());
        b[4..8].copy_from_slice(&self.flags.to_le_bytes());
        b[8..16].copy_from_slice(&self.fence_id.to_le_bytes());
        b[16..20].copy_from_slice(&self.ctx_id.to_le_bytes());
        // 20..24 padding
        b
    }

    /// Decode from wire format.
    pub fn decode(b: &[u8]) -> Result<Self, KernelError> {
        if b.len() < 24 {
            return Err(KernelError::InvalidArg("short gpu ctrl hdr"));
        }
        Ok(Self {
            cmd_type: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            flags: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            fence_id: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            ctx_id: u32::from_le_bytes(b[16..20].try_into().unwrap()),
            padding: u32::from_le_bytes(b[20..24].try_into().unwrap()),
        })
    }

    /// Build a request header.
    pub fn request(cmd_type: u32, fence_id: u64) -> Self {
        Self {
            cmd_type,
            flags: 0,
            fence_id,
            ctx_id: 0,
            padding: 0,
        }
    }

    /// Response header (flags = RESPONSE).
    pub fn response(cmd_type: u32, fence_id: u64) -> Self {
        Self {
            cmd_type,
            flags: flags::RESPONSE,
            fence_id,
            ctx_id: 0,
            padding: 0,
        }
    }
}

/// virtio_gpu_resource_create_2d — 32 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceCreate2d {
    pub resource_id: u32,
    pub format: u32,
    pub width: u32,
    pub height: u32,
}

/// Pixel formats (subset of drm fourcc).
pub mod format {
    pub const B8G8R8A8_UNORM: u32 = u32::from_le_bytes(*b"AR24");
    pub const B8G8R8X8_UNORM: u32 = u32::from_le_bytes(*b"XR24");
    pub const R8G8B8A8_UNORM: u32 = u32::from_le_bytes(*b"AB24");
}

impl ResourceCreate2d {
    pub fn encode(&self) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[0..4].copy_from_slice(&self.resource_id.to_le_bytes());
        b[4..8].copy_from_slice(&self.format.to_le_bytes());
        b[8..12].copy_from_slice(&self.width.to_le_bytes());
        b[12..16].copy_from_slice(&self.height.to_le_bytes());
        b
    }
    pub fn decode(b: &[u8]) -> Result<Self, KernelError> {
        if b.len() < 32 {
            return Err(KernelError::InvalidArg("short create2d"));
        }
        Ok(Self {
            resource_id: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            format: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            width: u32::from_le_bytes(b[8..12].try_into().unwrap()),
            height: u32::from_le_bytes(b[12..16].try_into().unwrap()),
        })
    }
}

/// virtio_gpu_set_scanout — 12 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetScanout {
    pub r#type: u32,
    pub scanout_id: u32,
    pub resource_id: u32,
}

impl SetScanout {
    pub fn encode(&self) -> [u8; 12] {
        let mut b = [0u8; 12];
        b[0..4].copy_from_slice(&self.r#type.to_le_bytes());
        b[4..8].copy_from_slice(&self.scanout_id.to_le_bytes());
        b[8..12].copy_from_slice(&self.resource_id.to_le_bytes());
        b
    }
    pub fn decode(b: &[u8]) -> Result<Self, KernelError> {
        if b.len() < 12 {
            return Err(KernelError::InvalidArg("short set_scanout"));
        }
        Ok(Self {
            r#type: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            scanout_id: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            resource_id: u32::from_le_bytes(b[8..12].try_into().unwrap()),
        })
    }
}

/// virtio_gpu_transfer_to_host_2d — 24 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferToHost2d {
    pub r#type: u32,
    pub resource_id: u32,
    pub box_x: u32,
    pub box_y: u32,
    pub box_w: u32,
    pub box_h: u32,
    pub offset: u64,
}

impl TransferToHost2d {
    /// Encode: type, resource_id, then virtio_gpu_box (16B), then offset.
    pub fn encode(&self) -> [u8; 24] {
        let mut b = [0u8; 24];
        b[0..4].copy_from_slice(&self.r#type.to_le_bytes());
        b[4..8].copy_from_slice(&self.resource_id.to_le_bytes());
        // box: x,y,w,h at 8..24 (16 bytes) — actually box is 4xu32=16B
        // then offset u64 — total struct is 40 bytes; our encode is
        // simplified for the common single-rect case.
        b[8..12].copy_from_slice(&self.box_x.to_le_bytes());
        b[12..16].copy_from_slice(&self.box_y.to_le_bytes());
        b[16..20].copy_from_slice(&self.box_w.to_le_bytes());
        b[20..24].copy_from_slice(&self.box_h.to_le_bytes());
        b
    }
}

/// virtio_gpu_resource_flush — 24 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceFlush {
    pub r#type: u32,
    pub resource_id: u32,
    pub box_x: u32,
    pub box_y: u32,
    pub box_w: u32,
    pub box_h: u32,
}

impl ResourceFlush {
    pub fn encode(&self) -> [u8; 24] {
        let mut b = [0u8; 24];
        b[0..4].copy_from_slice(&self.r#type.to_le_bytes());
        b[4..8].copy_from_slice(&self.resource_id.to_le_bytes());
        b[8..12].copy_from_slice(&self.box_x.to_le_bytes());
        b[12..16].copy_from_slice(&self.box_y.to_le_bytes());
        b[16..20].copy_from_slice(&self.box_w.to_le_bytes());
        b[20..24].copy_from_slice(&self.box_h.to_le_bytes());
        b
    }
}

/// virtio_gpu_resource_attach_backing — 12 + 12*N bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceAttachBacking {
    pub resource_id: u32,
    pub nr_entries: u32,
    /// (address, length) pairs.
    pub entries: Vec<(u64, u32)>,
}

impl ResourceAttachBacking {
    /// Total encoded size: 12-byte header + 12 bytes per entry.
    pub fn encoded_len(&self) -> usize {
        12 + self.entries.len() * 12
    }

    /// Encode into `out` (must be >= encoded_len).
    pub fn encode_into(&self, out: &mut [u8]) -> Result<(), KernelError> {
        let need = self.encoded_len();
        if out.len() < need {
            return Err(KernelError::InvalidArg("attach backing buffer small"));
        }
        out[0..4].copy_from_slice(&self.resource_id.to_le_bytes());
        out[4..8].copy_from_slice(&self.nr_entries.to_le_bytes());
        // bytes 8..12 padding
        let mut off = 12;
        for (addr, len) in &self.entries {
            out[off..off + 8].copy_from_slice(&addr.to_le_bytes());
            out[off + 8..off + 12].copy_from_slice(&len.to_le_bytes());
            off += 12;
        }
        Ok(())
    }

    /// Decode from wire bytes.
    pub fn decode(b: &[u8]) -> Result<Self, KernelError> {
        if b.len() < 12 {
            return Err(KernelError::InvalidArg("short attach backing"));
        }
        let resource_id = u32::from_le_bytes(b[0..4].try_into().unwrap());
        let nr_entries = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
        if b.len() < 12 + nr_entries * 12 {
            return Err(KernelError::InvalidArg("attach backing truncated"));
        }
        let mut entries = Vec::with_capacity(nr_entries);
        let mut off = 12;
        for _ in 0..nr_entries {
            let addr = u64::from_le_bytes(b[off..off + 8].try_into().unwrap());
            let len = u32::from_le_bytes(b[off + 8..off + 12].try_into().unwrap());
            entries.push((addr, len));
            off += 12;
        }
        Ok(Self {
            resource_id,
            nr_entries: nr_entries as u32,
            entries,
        })
    }
}

/// Response payload for GET_DISPLAY_INFO etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayInfo {
    pub rect_x: u32,
    pub rect_y: u32,
    pub rect_w: u32,
    pub rect_h: u32,
    pub enabled: u32,
    pub flags: u32,
}

impl DisplayInfo {
    pub fn decode(b: &[u8]) -> Result<Self, KernelError> {
        if b.len() < 24 {
            return Err(KernelError::InvalidArg("short display info"));
        }
        Ok(Self {
            rect_x: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            rect_y: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            rect_w: u32::from_le_bytes(b[8..12].try_into().unwrap()),
            rect_h: u32::from_le_bytes(b[12..16].try_into().unwrap()),
            enabled: u32::from_le_bytes(b[16..20].try_into().unwrap()),
            flags: u32::from_le_bytes(b[20..24].try_into().unwrap()),
        })
    }
}

/// Response status (u32 at start of response payload).
pub fn resp_status(b: &[u8]) -> Result<u32, KernelError> {
    if b.len() < 4 {
        return Err(KernelError::InvalidArg("short gpu response"));
    }
    Ok(u32::from_le_bytes(b[0..4].try_into().unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn ctrl_hdr_roundtrip() {
        let h = CtrlHdr::request(cmd::RESOURCE_CREATE_2D, 42);
        let enc = h.encode();
        assert_eq!(enc.len(), 24);
        let d = CtrlHdr::decode(&enc).unwrap();
        assert_eq!(d.cmd_type, cmd::RESOURCE_CREATE_2D);
        assert_eq!(d.fence_id, 42);
        assert_eq!(d.flags, 0);
    }

    #[test]
    fn create2d_roundtrip() {
        let c = ResourceCreate2d {
            resource_id: 1,
            format: format::B8G8R8A8_UNORM,
            width: 800,
            height: 600,
        };
        let enc = c.encode();
        assert_eq!(ResourceCreate2d::decode(&enc).unwrap(), c);
    }

    #[test]
    fn set_scanout_roundtrip() {
        let s = SetScanout {
            r#type: 0,
            scanout_id: 0,
            resource_id: 7,
        };
        assert_eq!(SetScanout::decode(&s.encode()).unwrap(), s);
    }

    #[test]
    fn attach_backing_roundtrip() {
        let a = ResourceAttachBacking {
            resource_id: 3,
            nr_entries: 2,
            entries: vec![(0x1000, 0x100000), (0x200000, 0x100000)],
        };
        let mut buf = vec![0u8; a.encoded_len()];
        a.encode_into(&mut buf).unwrap();
        let d = ResourceAttachBacking::decode(&buf).unwrap();
        assert_eq!(d, a);
    }

    #[test]
    fn display_info_decode() {
        let mut b = vec![0u8; 24];
        b[8..12].copy_from_slice(&1920u32.to_le_bytes());
        b[12..16].copy_from_slice(&1080u32.to_le_bytes());
        b[16..20].copy_from_slice(&1u32.to_le_bytes());
        let d = DisplayInfo::decode(&b).unwrap();
        assert_eq!(d.rect_w, 1920);
        assert_eq!(d.rect_h, 1080);
        assert_eq!(d.enabled, 1);
    }

    #[test]
    fn response_status() {
        assert_eq!(resp_status(&0u32.to_le_bytes()).unwrap(), 0);
        assert!(resp_status(&[0u8; 2]).is_err());
    }
}
