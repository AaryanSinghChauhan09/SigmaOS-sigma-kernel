//! Bootable ISO installer stub.
//!
//! Placeholder for future ISO installer implementation.

use alloc::string::String;

/// ISO installer configuration.
#[derive(Debug, Clone)]
pub struct IsoConfig {
    pub version: String,
    pub kernel_path: String,
    pub initramfs_path: String,
    pub output_path: String,
}

impl IsoConfig {
    /// Create a new ISO configuration.
    pub fn new(version: &str) -> Self {
        Self {
            version: String::from(version),
            kernel_path: String::from("target/kernel.elf"),
            initramfs_path: String::from("target/initramfs.cpio"),
            output_path: String::from("sigmaos.iso"),
        }
    }

    /// Build the ISO image (stub).
    pub fn build(&self) -> Result<String, ()> {
        // Full implementation requires GRUB configuration, multi-boot header setup, kernel/initramfs packaging, ISO 9660 filesystem creation, bootable image generation
        Err(())
    }

    /// Install to disk (stub).
    pub fn install(&self, _device: &str) -> Result<(), ()> {
        // Full implementation requires disk partitioning, filesystem creation, bootloader installation, system files extraction
        Err(())
    }
}
