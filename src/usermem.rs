//! User page-table construction — builds a ring-3 CR3.
//!
//! The kernel identity-maps the low 1 GiB with 2 MiB supervisor
//! pages. A user process needs its own CR3 that keeps the kernel
//! mapped (supervisor) *and* maps user pages with U/S=1. Modeled
//! on Linux `pgd_alloc`/`pmd_alloc` and the rust-vmm `vm-memory`
//! region model.
//!
//! Physical writes go through `PhysMem` so the lib stays
//! `#![forbid(unsafe_code)]`; the arch layer provides the real
//! identity-mapped implementation.

use crate::error::KernelError;
use crate::mm::FrameAllocator;
use alloc::vec;
use alloc::vec::Vec;

// Page-table entry flags (x86-64).
pub const P_PRESENT: u64 = 1 << 0;
pub const P_WRITE: u64 = 1 << 1;
pub const P_USER: u64 = 1 << 2;
pub const P_PS: u64 = 1 << 7; // page-size bit (2 MiB / 1 GiB)

pub const FRAME: u64 = 4096;
pub const PT_ENTRIES: usize = 512;

/// Raw physical-memory access for page-table construction.
/// The arch layer implements this over identity-mapped pointers.
pub trait PhysMem {
    /// Write a u64 at a physical address.
    fn write_u64(&mut self, phys: u64, val: u64);
    /// Read a u64 at a physical address.
    fn read_u64(&self, phys: u64) -> u64;
}

/// Indices into the 4-level walk for a virtual address.
pub fn indices(vaddr: u64) -> (usize, usize, usize, usize) {
    (
        ((vaddr >> 39) & 0x1ff) as usize, // PML4
        ((vaddr >> 30) & 0x1ff) as usize, // PDPT
        ((vaddr >> 21) & 0x1ff) as usize, // PD
        ((vaddr >> 12) & 0x1ff) as usize, // PT
    )
}

/// A page-table page we allocated (phys addr of its 512 entries).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TablePage {
    pub phys: u64,
}

impl TablePage {
    fn zeroed(phys: u64) -> Self {
        Self { phys }
    }
}

/// A fully built user CR3 plus bookkeeping for teardown.
#[derive(Debug)]
pub struct UserTables {
    /// Physical address to load into CR3.
    pub cr3: u64,
    /// All table pages allocated (for free/teardown).
    pub pages: Vec<TablePage>,
}

fn alloc_table(alloc: &mut FrameAllocator) -> Result<TablePage, KernelError> {
    let phys = alloc.alloc_frame().ok_or(KernelError::OutOfMemory)?;
    Ok(TablePage::zeroed(phys))
}

/// Write `val` into entry `idx` of table page `t`.
fn set_entry(phys_mem: &mut dyn PhysMem, table: u64, idx: usize, val: u64) {
    phys_mem.write_u64(table + idx as u64 * 8, val);
}

/// Read entry `idx` of table page `t`.
fn get_entry(phys_mem: &dyn PhysMem, table: u64, idx: usize) -> u64 {
    phys_mem.read_u64(table + idx as u64 * 8)
}

/// Ensure the PML4→PDPT→PD chain exists for `vaddr`, allocating
/// table pages as needed. Returns the PD page phys for `vaddr`.
fn ensure_pd(
    vaddr: u64,
    pml4: u64,
    alloc: &mut FrameAllocator,
    phys_mem: &mut dyn PhysMem,
) -> Result<u64, KernelError> {
    let (pi, di, _mi, _) = indices(vaddr);

    // PML4[pi] -> PDPT
    let mut pdpt = get_entry(phys_mem, pml4, pi);
    if pdpt & P_PRESENT == 0 {
        let t = alloc_table(alloc)?;
        pdpt = t.phys | P_PRESENT | P_WRITE | P_USER;
        set_entry(phys_mem, pml4, pi, pdpt);
    }
    let pdpt_addr = pdpt & !0xfff;

    // PDPT[di] -> PD
    let mut pd = get_entry(phys_mem, pdpt_addr, di);
    if pd & P_PRESENT == 0 {
        let t = alloc_table(alloc)?;
        pd = t.phys | P_PRESENT | P_WRITE | P_USER;
        set_entry(phys_mem, pdpt_addr, di, pd);
    }
    Ok(pd & !0xfff)
}

/// Map a 2 MiB supervisor page (kernel identity) at `vaddr`->`phys`.
pub fn map_2m_supervisor(
    vaddr: u64,
    phys: u64,
    pml4: u64,
    alloc: &mut FrameAllocator,
    phys_mem: &mut dyn PhysMem,
) -> Result<(), KernelError> {
    let (_, _, mi, _) = indices(vaddr);
    let pd = ensure_pd(vaddr, pml4, alloc, phys_mem)?;
    let paddr = phys & !((2 << 20) - 1);
    set_entry(phys_mem, pd, mi, paddr | P_PRESENT | P_WRITE | P_PS);
    Ok(())
}

/// Map one 4K user page: `vaddr` -> `phys`, U/S=1, RW.
pub fn map_4k_user(
    vaddr: u64,
    phys: u64,
    pml4: u64,
    alloc: &mut FrameAllocator,
    phys_mem: &mut dyn PhysMem,
) -> Result<(), KernelError> {
    let (_, _, mi, ti) = indices(vaddr);
    let pd = ensure_pd(vaddr, pml4, alloc, phys_mem)?;

    // PD[mi] must point to a PT (not a 2 MiB page). If absent or a
    // large page, we only support the case where it is absent —
    // callers must not mix 2 MiB and 4K maps in the same PD entry.
    let pd_entry = get_entry(phys_mem, pd, mi);
    let pt_addr = if pd_entry & P_PRESENT == 0 {
        let t = alloc_table(alloc)?;
        let a = t.phys | P_PRESENT | P_WRITE | P_USER;
        set_entry(phys_mem, pd, mi, a);
        t.phys
    } else if pd_entry & P_PS == 0 {
        pd_entry & !0xfff
    } else {
        return Err(KernelError::InvalidArg(
            "cannot split 2 MiB page for 4K user map",
        ));
    };

    let flags = P_PRESENT | P_WRITE | P_USER;
    set_entry(phys_mem, pt_addr, ti, (phys & !0xfff) | flags);
    Ok(())
}

/// Build a complete user CR3 for `mappings` — a list of
/// `(vaddr, phys, writable)` 4K user pages — while keeping the
/// kernel's low identity map supervisor-only.
///
/// Strategy (mirrors Linux's two-region pgd):
/// - PML4[0] -> PDPT covering the low 512 GiB.
/// - For every 2 MiB slot in the low 1 GiB we install a supervisor
///   identity 2 MiB page *except* slots containing user pages, which
///   get a PT of 4K user mappings (kernel pages in the same slot stay
///   supervisor by mapping them explicitly).
///
/// To keep this first cut tractable we require user pages to live
/// in 2 MiB slots disjoint from the kernel's first 2 MiB — the
/// loader picks a bias (e.g. 4 MiB) so this holds.
pub fn build_user_tables(
    user_pages: &[(u64, u64)], // (vaddr_page, phys_page) 4K-aligned
    kernel_top_2m: u64,        // kernel occupies [0, kernel_top_2m)
    alloc: &mut FrameAllocator,
    phys_mem: &mut dyn PhysMem,
) -> Result<UserTables, KernelError> {
    let mut tables = Vec::new();
    let pml4_t = alloc_table(alloc)?;
    tables.push(pml4_t);
    let pml4 = pml4_t.phys;

    // PML4[0] -> PDPT for low half
    let pdpt_t = alloc_table(alloc)?;
    tables.push(pdpt_t);
    set_entry(
        phys_mem,
        pml4,
        0,
        pdpt_t.phys | P_PRESENT | P_WRITE | P_USER,
    );
    let pdpt = pdpt_t.phys;

    // PDPT[0] -> PD covering 0..1 GiB
    let pd_t = alloc_table(alloc)?;
    tables.push(pd_t);
    set_entry(phys_mem, pdpt, 0, pd_t.phys | P_PRESENT | P_WRITE | P_USER);
    let pd = pd_t.phys;

    // Group user pages by 2 MiB slot.
    let mut slots: Vec<(u64, Vec<(u64, u64)>)> = Vec::new();
    for &(v, p) in user_pages {
        let slot = v & !((2 << 20) - 1);
        match slots.iter_mut().find(|(s, _)| *s == slot) {
            Some((_, v2)) => v2.push((v, p)),
            None => slots.push((slot, vec![(v, p)])),
        }
    }

    for (slot, pages) in &slots {
        // A slot that overlaps the kernel's first 2 MiB cannot be a
        // clean user region — reject rather than silently corrupt.
        if *slot < kernel_top_2m {
            return Err(KernelError::InvalidArg("user pages overlap kernel slot"));
        }
        // PT for this slot's 512 4K pages.
        let pt_t = alloc_table(alloc)?;
        tables.push(pt_t);
        let (_, _, mi, _) = indices(*slot);
        set_entry(phys_mem, pd, mi, pt_t.phys | P_PRESENT | P_WRITE | P_USER);
        for &(v, p) in pages {
            let (_, _, _, ti) = indices(v);
            set_entry(
                phys_mem,
                pt_t.phys,
                ti,
                (p & !0xfff) | P_PRESENT | P_WRITE | P_USER,
            );
        }
    }

    // Fill remaining low-1GiB PD slots with supervisor 2 MiB identity
    // pages so the kernel stays mapped under the user CR3.
    for mi in 0..PT_ENTRIES {
        let vaddr = (mi as u64) << 21;
        if get_entry(phys_mem, pd, mi) & P_PRESENT != 0 {
            continue; // already a user PT
        }
        // skip non-canonical holes: only map 0..1GiB identity
        set_entry(phys_mem, pd, mi, vaddr | P_PRESENT | P_WRITE | P_PS);
    }

    Ok(UserTables {
        cr3: pml4,
        pages: tables,
    })
}

/// Map a single 4K user page `vaddr`->`phys` into an existing user
/// CR3, allocating/splitting page-table levels as needed.
///
/// If the covering PD entry is a 2 MiB supervisor page it is split
/// into a PT of 512 supervisor 4K entries first (leaves keep
/// supervisor-only flags so user access still faults there).
/// Modeled on Linux `pmd_install` + `pte_alloc_one`.
pub fn map_user_page(
    pml4: u64,
    vaddr: u64,
    phys: u64,
    alloc: &mut FrameAllocator,
    phys_mem: &mut dyn PhysMem,
) -> Result<(), KernelError> {
    let (_, _, mi, ti) = indices(vaddr);
    let pd = ensure_pd(vaddr, pml4, alloc, phys_mem)?;

    // PD[mi]: 2 MiB supervisor page -> split into a PT.
    let mut pd_e = get_entry(phys_mem, pd, mi);
    if pd_e & P_PRESENT == 0 {
        let pt_t = alloc_table(alloc)?;
        pd_e = pt_t.phys | P_PRESENT | P_WRITE | P_USER;
        set_entry(phys_mem, pd, mi, pd_e);
    } else if pd_e & P_PS != 0 {
        let slot_base = (pd_e & !0xfff) & !((2 << 20) - 1);
        let pt_t = alloc_table(alloc)?;
        for i in 0..PT_ENTRIES {
            let leaf = (slot_base + i as u64 * FRAME) | P_PRESENT | P_WRITE;
            set_entry(phys_mem, pt_t.phys, i, leaf);
        }
        pd_e = pt_t.phys | P_PRESENT | P_WRITE | P_USER;
        set_entry(phys_mem, pd, mi, pd_e);
    }
    let pt = pd_e & !0xfff;

    set_entry(
        phys_mem,
        pt,
        ti,
        (phys & !0xfff) | P_PRESENT | P_WRITE | P_USER,
    );
    Ok(())
}

/// Walk a user CR3 for `vaddr` and return `(pte_slot_phys, entry)`
/// for a present 4K leaf, or `None` when any level is absent, the
/// covering PD entry is a 2 MiB supervisor page, or the leaf itself
/// is not present. Used by MUNMAP — callers must check `P_USER`
/// before freeing so supervisor/kernel pages are never released.
pub fn user_pte(pml4: u64, vaddr: u64, phys_mem: &dyn PhysMem) -> Option<(u64, u64)> {
    let (pi, di, mi, ti) = indices(vaddr);
    let pdpt_e = get_entry(phys_mem, pml4, pi);
    if pdpt_e & P_PRESENT == 0 {
        return None;
    }
    let pd_e = get_entry(phys_mem, pdpt_e & !0xfff, di);
    if pd_e & P_PRESENT == 0 {
        return None;
    }
    // PD[mi]: 2 MiB supervisor page -> not a 4K user leaf.
    let pde = get_entry(phys_mem, pd_e & !0xfff, mi);
    if pde & P_PRESENT == 0 || pde & P_PS != 0 {
        return None;
    }
    let pte_addr = (pde & !0xfff) + ti as u64 * 8;
    let e = phys_mem.read_u64(pte_addr);
    if e & P_PRESENT == 0 {
        return None;
    }
    Some((pte_addr, e))
}

/// Free all page-table pages for a user CR3 (PML4, PDPT, PD, PTs).
/// Does NOT free user data frames — those are the caller's responsibility.
/// (Linux `pgd_free` + `pud_free` + `pmd_free` + `pte_free` analogue.)
pub fn free_user_tables(pml4: u64, alloc: &mut FrameAllocator, phys_mem: &mut dyn PhysMem) {
    // Walk PML4[0] -> PDPT[0] -> PD and free all table pages.
    let pdpt_e = get_entry(phys_mem, pml4, 0);
    if pdpt_e & P_PRESENT == 0 {
        return;
    }
    let pdpt = pdpt_e & !0xfff;
    let pd_e = get_entry(phys_mem, pdpt, 0);
    if pd_e & P_PRESENT == 0 {
        return;
    }
    let pd = pd_e & !0xfff;
    // Free all PTs pointed to by PD entries that are PTs (not 2MiB pages).
    for mi in 0..PT_ENTRIES {
        let pde = get_entry(phys_mem, pd, mi);
        if pde & P_PRESENT != 0 && pde & P_PS == 0 {
            // It's a PT page — free it.
            let pt = pde & !0xfff;
            alloc.free_frame(pt);
        }
    }
    // Free PD, PDPT, PML4.
    alloc.free_frame(pd);
    alloc.free_frame(pdpt);
    alloc.free_frame(pml4);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot::MemRegion;
    use alloc::vec;
    use alloc::vec::Vec;
    use std::collections::HashMap;

    /// In-memory PhysMem over a HashMap for tests.
    struct MockMem {
        map: HashMap<u64, u64>,
    }
    impl PhysMem for MockMem {
        fn write_u64(&mut self, phys: u64, val: u64) {
            self.map.insert(phys, val);
        }
        fn read_u64(&self, phys: u64) -> u64 {
            *self.map.get(&phys).unwrap_or(&0)
        }
    }

    #[test]
    fn map_user_page_splits_supervisor_slot() {
        let mut alloc = test_alloc();
        let mut mem = MockMem {
            map: HashMap::new(),
        };
        // Minimal CR3: PML4[0]->PDPT->PD with one supervisor 2MiB slot.
        let pml4 = alloc.alloc_frame().unwrap();
        mem.write_u64(pml4, 0); // PML4[0] empty for now
                                // Build via build_user_tables with no user pages.
        let t = build_user_tables(&[], 0x200000, &mut alloc, &mut mem).unwrap();
        // vaddr 0x300000 is inside a supervisor 2MiB slot (< kernel_top? no,
        // kernel_top=0x200000 so slot 0x200000..0x400000 IS kernel).
        // Use 0x800000 (slot 0x800000, supervisor identity).
        let phys = alloc.alloc_frame().unwrap();
        map_user_page(t.cr3, 0x800000, phys, &mut alloc, &mut mem).unwrap();
        // Walk: PML4[0]->PDPT[0]->PD[mi=4]->PT[ti=0] should be present+user.
        let pdpt = mem.read_u64(t.cr3);
        let pd = mem.read_u64(pdpt & !0xfff);
        let mi = (0x800000usize >> 21) & 0x1ff;
        let pde = mem.read_u64((pd & !0xfff) + mi as u64 * 8);
        assert_ne!(pde & P_PRESENT, 0);
        assert_eq!(pde & P_PS, 0, "slot should now be a PT, not 2MiB page");
        let pt = pde & !0xfff;
        let ti = (0x800000usize >> 12) & 0x1ff;
        let pte = mem.read_u64(pt + ti as u64 * 8);
        assert_ne!(pte & (P_PRESENT | P_WRITE | P_USER), 0);
        assert_eq!(pte & !0xfff, phys & !0xfff);
    }

    #[test]
    fn user_pte_finds_only_user_leaves() {
        let mut alloc = test_alloc();
        let mut mem = MockMem {
            map: HashMap::new(),
        };
        let t = build_user_tables(&[], 0x200000, &mut alloc, &mut mem).unwrap();
        let phys = alloc.alloc_frame().unwrap();
        map_user_page(t.cr3, 0x800000, phys, &mut alloc, &mut mem).unwrap();

        // Present user leaf -> Some with U/S set.
        let (slot, e) = user_pte(t.cr3, 0x800000, &mem).unwrap();
        assert_ne!(e & P_USER, 0);
        assert_eq!(e & !0xfff, phys & !0xfff);
        // Clearing through the returned slot makes it absent.
        mem.write_u64(slot, 0);
        assert!(user_pte(t.cr3, 0x800000, &mem).is_none());
        // Vaddr in a different, untouched supervisor 2MiB slot -> None.
        assert!(user_pte(t.cr3, 0xA00000, &mem).is_none());
        // Kernel identity slot -> None (never a user leaf).
        assert!(user_pte(t.cr3, 0x100000, &mem).is_none());
    }

    fn test_alloc() -> FrameAllocator {
        FrameAllocator::new(&[MemRegion {
            start: 0x100000,
            len: 0x4000000,
            mem_type: 1,
        }])
        .unwrap()
    }

    #[test]
    fn indices_split_correctly() {
        // vaddr 0x401234 -> PML4 0, PDPT 0, PD 2, PT 1
        let (pi, di, mi, ti) = indices(0x401234);
        assert_eq!((pi, di, mi, ti), (0, 0, 2, 1));
    }

    #[test]
    fn builds_cr3_and_maps_user_pages() {
        let mut a = test_alloc();
        let mut mem = MockMem {
            map: HashMap::new(),
        };
        // user page at vaddr 0x400000 -> phys 0x800000
        let user_pages = vec![(0x400000u64, 0x800000u64)];
        let t = build_user_tables(&user_pages, 0x200000, &mut a, &mut mem).unwrap();
        assert_ne!(t.cr3, 0);
        assert!(t.cr3 % FRAME == 0);
        // PML4[0] present
        assert_ne!(mem.read_u64(t.cr3) & P_PRESENT, 0);
        // walk to the user PT and check the 4K entry has U/S
        let pdpt = mem.read_u64(t.cr3) & !0xfff;
        let pd = mem.read_u64(pdpt) & !0xfff;
        let pt = mem.read_u64(pd + 2 * 8) & !0xfff;
        let e = mem.read_u64(pt + 0 * 8);
        assert_eq!(e & !0xfff, 0x800000);
        assert_ne!(e & P_USER, 0);
        assert_ne!(e & P_PRESENT, 0);
    }

    #[test]
    fn kernel_slots_stay_supervisor() {
        let mut a = test_alloc();
        let mut mem = MockMem {
            map: HashMap::new(),
        };
        let t = build_user_tables(&[(0x400000, 0x800000)], 0x200000, &mut a, &mut mem).unwrap();
        let pdpt = mem.read_u64(t.cr3) & !0xfff;
        let pd = mem.read_u64(pdpt) & !0xfff;
        // PD[0] = 2 MiB supervisor identity page for kernel slot
        let e0 = mem.read_u64(pd);
        assert_ne!(e0 & P_PS, 0);
        assert_eq!(e0 & P_USER, 0);
        assert_eq!(e0 & !0xfff, 0);
    }

    #[test]
    fn rejects_kernel_overlap() {
        let mut a = test_alloc();
        let mut mem = MockMem {
            map: HashMap::new(),
        };
        // user page inside kernel's first 2 MiB
        let r = build_user_tables(&[(0x100000, 0x900000)], 0x200000, &mut a, &mut mem);
        assert!(r.is_err());
    }
}
