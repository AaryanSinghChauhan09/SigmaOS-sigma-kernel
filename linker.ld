/* SigmaOS kernel linker script — Multiboot2, loaded at 1 MiB. */
OUTPUT_FORMAT("elf64-x86-64")
ENTRY(_start)

SECTIONS {
    . = 0x100000;

    /* ELF notes (PVH Xen note lives here -> PT_NOTE segment). */
    .notes : ALIGN(4) {
        KEEP(*(.note.Xen))
    }

    /* Multiboot2 header must be in the first 32 KiB of the file. */
    .multiboot2 : ALIGN(8) {
        KEEP(*(.multiboot2))
    }

    .text : ALIGN(0x1000) {
        *(.text .text.*)
    }

    .rodata : ALIGN(0x1000) {
        *(.rodata .rodata.*)
    }

    /* GOT must be kept: rust-lld emits GOT-indirect calls (memcpy,
     * memset, alloc shims) even for -no-pie static links. */
    .got : ALIGN(8) {
        *(.got .got.plt)
    }

    .data : ALIGN(0x1000) {
        *(.data .data.*)
    }

    .bss : ALIGN(0x1000) {
        *(COMMON)
        *(.bss .bss.*)
    }

    /DISCARD/ : {
        *(.comment)
        *(.eh_frame*)
        *(.dynsym)
        *(.dynstr)
        *(.hash)
        *(.gnu.hash)
        *(.rela.dyn)
        *(.rela.plt)
        *(.dynamic)
    }
}
