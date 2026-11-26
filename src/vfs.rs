//! VFS — virtual file system trait plus cpio "newc" initramfs parser.
//!
//! The initramfs is a cpio newc archive unpacked into memory at boot;
//! `Initramfs` implements directory lookup and file read.

use crate::KernelError;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

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
        mounts.insert(
            "/".into(),
            MountEntry {
                path: "/".into(),
                kind: FsKind::Initramfs,
                dev: 0,
            },
        );
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
        let idx = self
            .lookup(path)
            .ok_or(KernelError::Fs("mount point not found"))?;
        if !self.nodes[idx].is_dir {
            return Err(KernelError::Fs("mount point is not a directory"));
        }
        if self.mounts.contains_key(path) {
            return Err(KernelError::Fs("already mounted"));
        }
        self.mounts.insert(
            path.to_string(),
            MountEntry {
                path: path.to_string(),
                kind,
                dev,
            },
        );
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

    /// Whether `path` resolves to a directory.
    pub fn is_dir(&self, path: &str) -> bool {
        self.lookup(path).is_some_and(|i| self.nodes[i].is_dir)
    }

    /// Size in bytes of the file at `path` (None if missing or a dir).
    pub fn file_len(&self, path: &str) -> Option<u64> {
        let idx = self.lookup(path)?;
        let n = &self.nodes[idx];
        (!n.is_dir).then_some(n.data.len() as u64)
    }

    /// Create an empty file at `path`. Parent directories must already
    /// exist. Errors if the path exists or a parent is missing/not a dir.
    pub fn create_file(&mut self, path: &str) -> Result<(), KernelError> {
        let trimmed = path.trim_matches('/');
        if trimmed.is_empty() {
            return Err(KernelError::Fs("invalid path"));
        }
        let mut cur = self.root;
        let parts: Vec<&str> = trimmed.split('/').collect();
        for (i, part) in parts.iter().enumerate() {
            let last = i == parts.len() - 1;
            match self.nodes[cur].children.get(*part) {
                Some(&next) => {
                    if last {
                        return Err(KernelError::Fs("file exists"));
                    }
                    if !self.nodes[next].is_dir {
                        return Err(KernelError::Fs("not a directory"));
                    }
                    cur = next;
                }
                None => {
                    if last {
                        let idx = self.nodes.len();
                        self.nodes.push(VfsNode {
                            name: (*part).to_string(),
                            is_dir: false,
                            data: Vec::new(),
                            children: BTreeMap::new(),
                        });
                        self.nodes[cur].children.insert((*part).to_string(), idx);
                        return Ok(());
                    }
                    return Err(KernelError::Fs("no such directory"));
                }
            }
        }
        Err(KernelError::Fs("invalid path"))
    }

    /// Empty the file at `path`.
    pub fn truncate(&mut self, path: &str) -> Result<(), KernelError> {
        let idx = self.lookup(path).ok_or(KernelError::Fs("no such file"))?;
        if self.nodes[idx].is_dir {
            return Err(KernelError::Fs("is a directory"));
        }
        self.nodes[idx].data.clear();
        Ok(())
    }

    /// Resolve `path` to (parent node index, final component name).
    /// The final component must not exist yet is NOT required — the
    /// parent must exist and be a directory.
    fn resolve_parent<'a>(&self, path: &'a str) -> Option<(usize, &'a str)> {
        let trimmed = path.trim_matches('/');
        if trimmed.is_empty() {
            return None;
        }
        let mut parts = trimmed.rsplitn(2, '/');
        let name = parts.next()?;
        let parent_path = parts.next();
        let parent = match parent_path {
            Some(p) if !p.is_empty() => self.lookup(&format!("/{}", p))?,
            _ => self.root,
        };
        if !self.nodes[parent].is_dir || name.is_empty() {
            return None;
        }
        Some((parent, name))
    }

    /// Create an empty directory at `path`. Parent must exist.
    pub fn create_dir(&mut self, path: &str) -> Result<(), KernelError> {
        let Some((parent, name)) = self.resolve_parent(path) else {
            return Err(KernelError::Fs("no such directory"));
        };
        if self.nodes[parent].children.contains_key(name) {
            return Err(KernelError::Fs("file exists"));
        }
        let idx = self.nodes.len();
        self.nodes.push(VfsNode {
            name: name.to_string(),
            is_dir: true,
            data: Vec::new(),
            children: BTreeMap::new(),
        });
        self.nodes[parent].children.insert(name.to_string(), idx);
        Ok(())
    }

    /// Remove the file at `path`. Fails for directories and root.
    pub fn unlink(&mut self, path: &str) -> Result<(), KernelError> {
        let Some((parent, name)) = self.resolve_parent(path) else {
            return Err(KernelError::Fs("no such file"));
        };
        let Some(&idx) = self.nodes[parent].children.get(name) else {
            return Err(KernelError::Fs("no such file"));
        };
        if self.nodes[idx].is_dir {
            return Err(KernelError::Fs("is a directory"));
        }
        self.nodes[parent].children.remove(name);
        Ok(())
    }

    /// Remove the empty directory at `path`. Fails for non-empty dirs
    /// and root.
    pub fn remove_dir(&mut self, path: &str) -> Result<(), KernelError> {
        let Some((parent, name)) = self.resolve_parent(path) else {
            return Err(KernelError::Fs("no such directory"));
        };
        let Some(&idx) = self.nodes[parent].children.get(name) else {
            return Err(KernelError::Fs("no such directory"));
        };
        if !self.nodes[idx].is_dir {
            return Err(KernelError::Fs("not a directory"));
        }
        if !self.nodes[idx].children.is_empty() {
            return Err(KernelError::Fs("directory not empty"));
        }
        self.nodes[parent].children.remove(name);
        Ok(())
    }

    /// Write `data` at byte offset `off` in the file at `path`, growing
    /// the file as needed. Sparse gaps are zero-filled. Returns the
    /// number of bytes written.
    pub fn write_at(&mut self, path: &str, data: &[u8], off: u64) -> Result<usize, KernelError> {
        let idx = self.lookup(path).ok_or(KernelError::Fs("no such file"))?;
        if self.nodes[idx].is_dir {
            return Err(KernelError::Fs("is a directory"));
        }
        let end = off as usize + data.len();
        if self.nodes[idx].data.len() < end {
            self.nodes[idx].data.resize(end, 0);
        }
        self.nodes[idx].data[off as usize..end].copy_from_slice(data);
        Ok(data.len())
    }

    /// Number of nodes in the VFS.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the VFS has no nodes.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl Default for Vfs {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a minimal cpio newc archive in memory (for tests/tools).
pub fn cpio_newc_write(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (ino, &(name, data)) in (1u32..).zip(entries.iter()) {
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
    }
    // trailer
    let trailer = [
        "00000000", "00000000", "00000000", "00000000", "00000001", "00000000", "00000000",
        "00000000", "00000000", "00000000", "00000000", "0000000b", "00000000",
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
        let archive =
            cpio_newc_write(&[("/bin/sh", b"#!/bin/sh\n"), ("/etc/hostname", b"sigmaos\n")]);
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

    #[test]
    fn write_at_creates_and_updates_file() {
        let mut vfs = Vfs::new();
        vfs.create_file("/tmp/x").unwrap_err(); // /tmp missing
        vfs.create_file("/log").unwrap();
        assert_eq!(vfs.file_len("/log"), Some(0));
        vfs.write_at("/log", b"hello", 0).unwrap();
        assert_eq!(vfs.read("/log"), Some(&b"hello"[..]));
        vfs.write_at("/log", b"!", 5).unwrap();
        assert_eq!(vfs.read("/log"), Some(&b"hello!"[..]));
        // sparse write zero-fills the gap
        vfs.write_at("/log", b"Z", 8).unwrap();
        assert_eq!(vfs.read("/log"), Some(&b"hello!\0\0Z"[..]));
    }

    #[test]
    fn create_file_rejects_existing_and_dir() {
        let archive = cpio_newc_write(&[("/a/b", b"data")]);
        let mut vfs = Vfs::new();
        vfs.load_cpio_newc(&archive).unwrap();
        assert!(vfs.create_file("/a/b").is_err()); // exists
        assert!(vfs.create_file("/a").is_err()); // is a dir path -> exists anyway
        vfs.create_file("/a/c").unwrap();
        assert_eq!(vfs.file_len("/a/c"), Some(0));
        assert!(vfs.write_at("/a", b"x", 0).is_err()); // is a dir
    }

    #[test]
    fn mkdir_unlink_rmdir() {
        let mut vfs = Vfs::new();
        vfs.create_dir("/d").unwrap();
        assert!(vfs.is_dir("/d"));
        assert!(vfs.create_dir("/d").is_err()); // exists
        assert!(vfs.create_dir("/d/e/f").is_err()); // /d/e missing
        vfs.create_dir("/d/e").unwrap();
        vfs.create_file("/d/e/f").unwrap();
        assert!(vfs.remove_dir("/d/e").is_err()); // not empty
        vfs.unlink("/d/e/f").unwrap();
        assert!(!vfs.exists("/d/e/f"));
        vfs.remove_dir("/d/e").unwrap();
        vfs.remove_dir("/d").unwrap();
        assert!(!vfs.exists("/d"));
        assert!(vfs.unlink("/d").is_err()); // gone
    }

    #[test]
    fn unlink_rejects_dirs() {
        let mut vfs = Vfs::new();
        vfs.create_dir("/dir").unwrap();
        assert!(vfs.unlink("/dir").is_err());
        assert!(vfs.is_dir("/dir"));
    }

    #[test]
    fn truncate_empties_file() {
        let mut vfs = Vfs::new();
        vfs.create_file("/f").unwrap();
        vfs.write_at("/f", b"data", 0).unwrap();
        vfs.truncate("/f").unwrap();
        assert_eq!(vfs.file_len("/f"), Some(0));
        assert_eq!(vfs.read("/f"), Some(&b""[..]));
    }
}
