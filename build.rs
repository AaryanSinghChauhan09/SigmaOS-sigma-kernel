//! Generates boot headers at build time via sigma-bootloader.
//! Emits:
//!   mb2header.bin — Multiboot2 header (for modern bootloaders)
//!
//! NB: no Multiboot1 header is emitted — MB1 only supports 32-bit
//! kernels, and QEMU's -kernel multiboot path rejects x86-64 ELFs.
//! QEMU boots this image via the PVH Xen note instead.
//! Both are padded to fixed sizes so the kernel can declare arrays.

fn main() {
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    let dir = std::path::Path::new(&out);

    // MB2 header follows in .multiboot2.
    let mb2 = sigma_bootloader::Mb2Header::sigmaos_default(0x100000, &[]).expect("mb2 header");
    let mut mb2_bytes = mb2.to_bytes();
    mb2_bytes.resize(512, 0);
    std::fs::write(dir.join("mb2header.bin"), &mb2_bytes).expect("write mb2header.bin");

    println!("cargo:rerun-if-changed=build.rs");
}
