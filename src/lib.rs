//! SigmaOS kernel — no_std operating system core.
//!
//! Modules: boot info parsing (Multiboot2/Limine), physical memory
//! management, VFS with cpio initramfs, 16550 UART, VirtIO-MMIO queues,
//! syscall ABI, and linear framebuffer output.

#![no_std]
#![allow(unsafe_code)]

extern crate alloc;

pub mod boot;
pub mod compositor;
pub mod console;
pub mod elf;
pub mod error;
pub mod fat32;
pub mod fb;
pub mod fb_console;
pub mod mm;
pub mod process;
pub mod ps2;
pub mod syscall;
pub mod theme;
pub mod uart;
pub mod usermem;
pub mod virtio;
pub mod virtio_blk;
pub mod virtio_gpu;
pub mod virtio_mmio;
pub mod virtio_net;
pub mod vfs;
pub mod window_manager;
pub mod agent;

pub use error::KernelError;

/// Kernel version reported via syscall and boot banner.
pub const KERNEL_VERSION: &str = "0.1.0";

/// Higher-half base where the kernel is mapped.
pub const HIGHER_HALF_BASE: u64 = 0xFFFF_8000_0000_0000;

/// Physical load address expected by the boot protocol.
pub const KERNEL_LOAD_ADDR: u64 = 0x100_000; // 1 MiB

#[cfg(test)]
extern crate std;
