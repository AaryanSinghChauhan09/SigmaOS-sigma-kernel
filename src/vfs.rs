//! VFS — virtual file system trait plus cpio "newc" initramfs parser.
//!
//! The initramfs is a cpio newc archive unpacked into memory at boot;
//! `Initramfs` implements directory lookup and file read.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use crate::KernelError;

/// A file or directory entry.
#[derive(Debug, Clone)]
pub struct VfsNode {
    pub name: String,
    pub is_dir: bool,
    pub data: Vec<u8>,
    /// child name -> node index into `nodes`
    pub children: BTreeMap<String, usize>,
}

/// A mounted filesystem backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsKind {
    /// In-memory initramfs (cpio newc).
    Initramfs,
    /// FAT32 on a block device.
    Fat32,
    /// Placeholder for future filesystems.
    Other,
}

/// A mount entry: path -> filesystem kind + backing device id.
#[derive(Debug, Clone)]
pub struct MountEntry {
    pub path: String,
    pub kind: FsKind,
    /// Block device index (for device-backed mounts; 0 = none).
    pub dev: u32,
}

/// In-memory filesystem built from an initramfs archive.
pub struct Vfs {
    nodes: Vec<VfsNode>,
    root: usize,
    /// Active mounts keyed by mount path.
    pub mounts: BTreeMap<String, MountEntry>,
}

impl Vfs {
    /// Create an empty VFS with a root directory.
    pub fn new() -> Self {
        let mut mounts = BTreeMap::new();
        mounts.insert("/".into(), MountEntry {
            path: "/".into(),
            kind: FsKind::Initramfs,
            dev: 0,
        });
        Self {
            nodes: vec![VfsNode {
                name: "/".into(),
                is_dir: true,
                data: Vec::new(),
                children: BTreeMap::new(),
            }],
            root: 0,
            mounts,
        }
    }

    /// Mount a filesystem at `path`. Returns error if path is not a dir
    /// or already mounted.
    pub fn mount(&mut self, path: &str, kind: FsKind, dev: u32) -> Result<(), KernelError> {
        let idx = self.lookup(path).ok_or(KernelError::Fs("mount point not found"))?;
        if !self.nodes[idx].is_dir {
            return Err(KernelError::Fs("mount point is not a directory"));
        }
        if self.mounts.contains_key(path) {
            return Err(KernelError::Fs("already mounted"));
        }
        self.mounts.insert(path.to_string(), MountEntry {
            path: path.to_string(),
            kind,
            dev,
        });
        Ok(())
    }

    /// Unmount the filesystem at `path`. Root cannot be unmounted.
    pub fn umount(&mut self, path: &str) -> Result<(), KernelError> {
        if path == "/" {
            return Err(KernelError::Fs("cannot unmount root"));
        }
        if self.mounts.remove(path).is_none() {
            return Err(KernelError::Fs("not mounted"));
        }
        Ok(())
    }

    /// Check if `path` is a mount point.
    pub fn is_mount_point(&self, path: &str) -> bool {
        self.mounts.contains_key(path)
    }

    /// Load a cpio newc archive into the VFS.
    pub fn load_cpio_newc(&mut self, archive: &[u8]) -> Result<(), KernelError> {
        let mut off = 0usize;
        loop {
            if off + 110 > archive.len() {
                return Err(KernelError::Fs("truncated cpio header"));
            }
            let magic = &archive[off..off + 6];
            if magic != b"070701" {
                return Err(KernelError::Fs("not a newc archive"));
            }
            let field = |i: usize| -> Result<u32, KernelError> {
                let s = core::str::from_utf8(&archive[off + 6 + i * 8..off + 6 + (i + 1) * 8])
                    .map_err(|_| KernelError::Fs("bad cpio field"))?;
                u32::from_str_radix(s, 16).map_err(|_| KernelError::Fs("bad cpio hex"))
            };
            let ino = field(0)?;
            let mode = field(1)?;
            let _uid = field(2)?;
            let _gid = field(3)?;
            let nlink = field(4)?;
            let _mtime = field(5)?;
            let filesize = field(6)? as usize;
            let devmajor = field(7)?;
            let devminor = field(8)?;
            let rdevmajor = field(9)?;
            let rdevminor = field(10)?;
            let namesize = field(11)? as usize;
            let check = field(12)?;
            if ino == 0 && check == 0 {
                break; // TRAILER!!!
            }
            let name_start = off + 110;
            let name_end = name_start + namesize;
            if name_end > archive.len() {
                return Err(KernelError::Fs("truncated cpio name"));
            }
            let name_raw = &archive[name_start..name_end];
            let name = core::str::from_utf8(name_raw)
                .map_err(|_| KernelError::Fs("bad cpio name"))?
                .trim_end_matches('\0')
                .to_string();
            if name == "TRAILER!!!" {
                break;
            }
            let _ = (nlink, devmajor, devminor, rdevmajor, rdevminor);

            // data starts after name, padded to 4-byte boundary
            let data_start = (name_end + 3) & !3;
            let data_end = data_start + filesize;
            if data_end > archive.len() {
                return Err(KernelError::Fs("truncated cpio data"));
            }

            let is_dir = (mode & 0o170000) == 0o040000;
            self.insert(&name, is_dir, &archive[data_start..data_end])?;

            off = (data_end + 3) & !3;
        }
        Ok(())
    }

    /// Insert a path like "/bin/sh" or "bin/" creating parents.
    fn insert(&mut self, path: &str, is_dir: bool, data: &[u8]) -> Result<(), KernelError> {
        let trimmed = path.trim_matches('/');
        if trimmed.is_empty() {
            return Ok(());
        }
        let mut cur = self.root;
        let parts: Vec<&str> = trimmed.split('/').collect();
        for (i, part) in parts.iter().enumerate() {
            let last = i == parts.len() - 1;
            let want_dir = !last || is_dir || part.ends_with('.');
            if let Some(&next) = self.nodes[cur].children.get(*part) {
                cur = next;
                continue;
            }
            let node = VfsNode {
                name: (*part).to_string(),
                is_dir: want_dir,
                data: Vec::new(),
                children: BTreeMap::new(),
            };
            let idx = self.nodes.len();
            self.nodes.push(node);
            self.nodes[cur].children.insert((*part).to_string(), idx);
            cur = idx;
        }
        if is_dir {
            self.nodes[cur].is_dir = true;
        } else {
            self.nodes[cur].data = data.to_vec();
            self.nodes[cur].is_dir = false;
        }
        Ok(())
    }

    /// Look up a node by absolute or relative path.
    pub fn lookup(&self, path: &str) -> Option<usize> {
        let trimmed = path.trim_start_matches('/');
        if trimmed.is_empty() {
            return Some(self.root);
        }
        let mut cur = self.root;
        for part in trimmed.split('/') {
            if part.is_empty() || part == "." {
                continue;
            }
            if part == ".." {
                // no parent pointers; stay (initramfs is flat-ish)
                continue;
            }
            cur = *self.nodes[cur].children.get(part)?;
        }
        Some(cur)
    }

    /// Read a file's bytes.
    pub fn read(&self, path: &str) -> Option<&[u8]> {
        let idx = self.lookup(path)?;
        let n = &self.nodes[idx];
        if n.is_dir {
            None
        } else {
            Some(&n.data)
        }
    }

    /// List children names of a directory.
    pub fn list(&self, path: &str) -> Option<Vec<String>> {
        let idx = self.lookup(path)?;
        let n = &self.nodes[idx];
        if !n.is_dir {
            return None;
        }
        Some(n.children.keys().cloned().collect())
    }

    /// Check if a path exists.
    pub fn exists(&self, path: &str) -> bool {
        self.lookup(path).is_some()
    }

    /// Number of nodes in the VFS.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
}

/// Build a minimal cpio newc archive in memory (for tests/tools).
pub fn cpio_newc_write(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut ino = 1u32;
    for (name, data) in entries {
        let namesize = name.len() + 1;
        let header = [
            format!("{:08x}", ino),
            "000081a4".to_string(), // regular file 0644
            "00000000".to_string(),
            "00000000".to_string(),
            "00000001".to_string(),
            "00000000".to_string(),
            format!("{:08x}", data.len()),
            "00000000".to_string(),
            "00000000".to_string(),
            "00000000".to_string(),
            "00000000".to_string(),
            format!("{:08x}", namesize),
            "00000000".to_string(),
        ]
        .concat();
        out.extend_from_slice(b"070701");
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        ino += 1;
    }
    // trailer
    let trailer = [
        "00000000", "00000000", "00000000", "00000000", "00000001",
        "00000000", "00000000", "00000000", "00000000", "00000000",
        "00000000", "0000000b", "00000000",
    ]
    .concat();
    out.extend_from_slice(b"070701");
    out.extend_from_slice(trailer.as_bytes());
    out.extend_from_slice(b"TRAILER!!!\0");
    while out.len() % 512 != 0 {
        out.push(0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_cpio() {
        let archive = cpio_newc_write(&[
            ("/bin/sh", b"#!/bin/sh\n"),
            ("/etc/hostname", b"sigmaos\n"),
        ]);
        let mut vfs = Vfs::new();
        vfs.load_cpio_newc(&archive).unwrap();
        assert_eq!(vfs.read("/bin/sh"), Some(&b"#!/bin/sh\n"[..]));
        assert_eq!(vfs.read("/etc/hostname"), Some(&b"sigmaos\n"[..]));
        assert!(vfs.exists("/bin"));
        assert!(vfs.list("/").unwrap().contains(&"bin".to_string()));
    }

    #[test]
    fn rejects_non_cpio() {
        let mut vfs = Vfs::new();
        assert!(vfs.load_cpio_newc(b"not an archive").is_err());
    }

    #[test]
    fn missing_file_returns_none() {
        let vfs = Vfs::new();
        assert!(vfs.read("/nope").is_none());
        assert!(!vfs.exists("/nope"));
    }
}
