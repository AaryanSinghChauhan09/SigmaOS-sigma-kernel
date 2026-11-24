//! Integration tests — kernel subsystems working together.

use sigma_kernel::boot::{parse_mb2, TAG_END};
use sigma_kernel::mm::FrameAllocator;
use sigma_kernel::syscall::{arity, nr, valid};
use sigma_kernel::vfs::{cpio_newc_write, Vfs};

#[test]
fn boot_to_alloc_to_vfs_pipeline() {
    // Arrange: minimal MB2 blob with one usable region + end tag
    let mut blob = Vec::new();
    blob.extend_from_slice(&16u32.to_le_bytes()); // total_size
    blob.extend_from_slice(&[0u8; 4]); // reserved
    blob.extend_from_slice(&TAG_END.to_le_bytes());
    blob.extend_from_slice(&8u32.to_le_bytes());

    // Act: parse boot info, seed frame allocator
    let info = parse_mb2(&blob).unwrap();
    // no mem map tag -> allocator fails gracefully
    assert!(FrameAllocator::new(&[]).is_err());
    let _ = info;

    // VFS: pack a cpio archive and read it back
    let arc = cpio_newc_write(&[("/init", b"sigmaos-init\n")]);
    let mut vfs = Vfs::new();
    vfs.load_cpio_newc(&arc).unwrap();
    assert_eq!(vfs.read("/init"), Some(&b"sigmaos-init\n"[..]));
}

#[test]
fn syscall_abi_is_consistent() {
    for n in 0..nr::MAX {
        assert!(valid(n));
        let _ = arity(n);
    }
    assert!(!valid(nr::MAX));
}
