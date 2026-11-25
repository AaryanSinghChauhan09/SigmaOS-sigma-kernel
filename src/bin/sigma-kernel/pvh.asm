/* SigmaOS PVH entry — 32-bit stub that enters long mode.
 *
 * QEMU's linuxboot loads the ELF, reads the Xen ELF note for the
 * 32-bit entry address, sets %ebx = hvm_start_info physical address,
 * and jumps here in 32-bit protected mode (flat segments).
 *
 * We build page tables (identity map, 1 GiB), enable PAE+PG, far-jump
 * to 64-bit code, then call the Rust kernel entry.
 */

.section .text.pvh32
.code32
.global pvh32_entry
pvh32_entry:
    cli
    mov $stack_top, %esp

    lgdt pvh_gdt_ptr
    mov $0x10, %ax              /* data selector */
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %ss

    /* CR0.PE */
    mov %cr0, %eax
    or $0x1, %eax
    mov %eax, %cr0

    /* CR4.PAE */
    mov %cr4, %eax
    or $0x20, %eax
    mov %eax, %cr4

    /* Page tables: PML4[0] -> PDPT, PDPT[0] -> PD, PD[i] -> 2 MiB pages */
    mov $pml4, %edi
    mov $pdpt, %eax
    or $0x3, %eax               /* P | RW */
    mov %eax, (%edi)
    mov $pdpt, %edi
    mov $pd, %eax
    or $0x3, %eax
    mov %eax, (%edi)

    mov $pd, %edi
    xor %ecx, %ecx
1:
    mov %ecx, %eax
    shl $21, %eax               /* i * 2 MiB */
    or $0x83, %eax              /* P | RW | PS */
    mov %eax, pd(,%ecx,8)
    inc %ecx
    cmp $512, %ecx
    jne 1b

    /* CR3 = pdpt. In 32-bit PAE mode the walk is PDPT->PD (no PML4);
     * after the far jump to 64-bit, the same memory reinterprets as
     * PML4->PDPT and PD[0]=0x83 becomes a 1 GiB page — identity map
     * works in both modes with one table. */
    /* EFER.LME (bit 8) — required for 64-bit mode */
    mov $0xc0000080, %ecx
    rdmsr
    or $0x101, %eax
    wrmsr

    mov $pdpt, %eax
    mov %eax, %cr3

    /* CR0.PG (keep PE set) */
    mov %cr0, %eax
    or $0x80000000, %eax
    mov %eax, %cr0

    /* Far jump into 64-bit code segment (selector 0x18). */
    mov $0x3fd, %dx
    mov $'A', %al
1:  in %dx, %al
    test $0x20, %al
    jz 1b
    mov $'A', %al
    mov $0x3f8, %dx
    out %al, %dx
    ljmp $0x18, $pvh64_entry

.code64
pvh64_entry:
    mov $0x3fd, %dx
    mov $'B', %al
2:  in %dx, %al
    test $0x20, %al
    jz 2b
    mov $'B', %al
    mov $0x3f8, %dx
    out %al, %dx
    mov $0x10, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %ss
    movabs $stack_top, %rsp

    /* %ebx held hvm_start_info paddr; stash it before it is lost. */
    mov %ebx, %eax
    movabs $saved_info, %rcx
    mov %rax, (%rcx)

    /* Build a 32-entry IDT -> isr_common so faults print to serial
     * instead of triple-faulting through the null IDT. */
    mov $idt_table, %rdi
    xor %ecx, %ecx
build_idt:
    mov isr_table(,%rcx,8), %rax
    mov %ax, (%rdi)             /* offset[15:0] */
    movw $0x18, 2(%rdi)         /* code64 selector */
    movb $0, 4(%rdi)            /* IST */
    movb $0x8e, 5(%rdi)         /* P=1 DPL=0 type=interrupt gate */
    mov %rax, %rdx
    shr $16, %rdx
    mov %dx, 6(%rdi)            /* offset[31:16] */
    shr $16, %rdx
    mov %edx, 8(%rdi)           /* offset[63:32] (0: addrs < 4 GiB) */
    movl $0, 12(%rdi)
    add $16, %rdi
    inc %ecx
    cmp $32, %ecx
    jb build_idt
    lidt idt_ptr

    /* Syscall MSRs: STAR packs kernel CS (syscall) + user base
     * (sysret); LSTAR = entry stub; SFMASK = 0 (keep rflags). */
    mov $0xc0000081, %ecx      /* STAR */
    /* STAR = (0x18<<32) | (0x20<<48) */
    mov $0x18, %eax
    shl $32, %rax
    mov $0x20, %rbx
    shl $48, %rbx
    or %rbx, %rax
    wrmsr
    mov $0xc0000082, %ecx      /* LSTAR */
    movabs $syscall_entry, %rax
    wrmsr
    xor %eax, %eax
    mov $0xc0000084, %ecx      /* SFMASK */
    wrmsr

    movabs $saved_info, %rdi
    mov (%rdi), %rdi            /* rdi = hvm_start_info paddr */
    movabs $pvh_kernel_entry, %rax
    call *%rax                  /* -> ! */

/* ---- exception stubs: eax = vector, jmp common ---- */
.macro ISR n
isr\n:
    mov $\n, %eax
    jmp isr_common
.endm
.irp n,0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
ISR \n
.endr

/* cl = char -> COM1 (polls THR-empty) */
ser_putc:
    push %rax
    push %rdx
    mov $0x3fd, %dx
1:  in %dx, %ax               /* ax = LSR:AL(3fd) + IER:AH(3fe) */
    test $0x20, %al
    jz 1b
    mov $0x3f8, %dx
    mov %cl, %al
    out %al, %dx
    pop %rdx
    pop %rax
    ret

/* Common exception handler: prints "E<vec hex>!" then halts.
 * eax = vector (set by stub). Error-code stack layout ignored. */
isr_common:
    cli
    mov %eax, %r8d
    mov $'E', %cl
    call ser_putc
    mov %r8d, %eax
    shr $4, %eax
    mov %al, %cl
    and $0xf, %cl
    cmp $10, %cl
    jb 2f
    add $('a'-10), %cl
    jmp 3f
2:  add $48, %cl
3:  call ser_putc
    mov %r8d, %eax
    mov %al, %cl
    and $0xf, %cl
    cmp $10, %cl
    jb 4f
    add $('a'-10), %cl
    jmp 5f
4:  add $48, %cl
5:  call ser_putc
    mov $'@', %cl
    call ser_putc
    /* RIP: error-code vectors have ERR at [rsp], RIP at [rsp+8];
     * others have RIP at [rsp]. */
    mov %rsp, %r9
    mov %r8d, %eax
    cmp $8, %eax
    je 7f
    cmp $10, %eax
    je 7f
    cmp $11, %eax
    je 7f
    cmp $12, %eax
    je 7f
    cmp $13, %eax
    je 7f
    cmp $14, %eax
    je 7f
    cmp $17, %eax
    je 7f
    cmp $21, %eax
    je 7f
    cmp $29, %eax
    je 7f
    cmp $30, %eax
    je 7f
    jmp 8f
7:  mov 8(%r9), %r10
    jmp 9f
8:  mov (%r9), %r10
9:  mov %r10d, %eax
    mov $28, %r11d
10: mov %eax, %edx
    mov %r11d, %ecx
    shr %cl, %edx
    and $0xf, %edx
    mov %edx, %ecx
    cmp $10, %ecx
    jb 11f
    add $87, %cl
    jmp 12f
11: add $48, %cl
12: call ser_putc
    sub $4, %r11d
    jns 10b
    mov $10, %cl
    call ser_putc
    /* dump 8 qwords from rsp as "<r11=val>" markers — actually print
     * rsp then 4 stack qwords low-32 bits each, space separated */
    /* for #PF (vec 14) dump CR2 so we see the faulting data addr */
    cmp $14, %r8d
    jne 90f
    mov %cr2, %r10
    mov $'C', %cl
    call ser_putc
    mov $16, %r11d
91: mov %r10, %rdx
    mov %r11d, %ecx
    shr %cl, %rdx
    and $0xf, %edx
    mov %edx, %ecx
    cmp $10, %ecx
    jb 92f
    add $87, %cl
    jmp 93f
92: add $48, %cl
93: call ser_putc
    sub $4, %r11d
    jns 91b
    mov $' ', %cl
    call ser_putc
90:
    mov $'S', %cl
    call ser_putc
    mov %r9, %r10           /* r9 = rsp at entry */
    mov $16, %r11d
13: mov (%r10), %eax
    mov %eax, %edx
    shr $28, %edx           /* placeholder — print via nibble loop */
    /* reuse digit printer: print eax as 8 hex */
    mov %eax, %r12d
    mov $28, %r13d
14: mov %r12d, %eax
    mov %eax, %edx
    mov %r13d, %ecx
    shr %cl, %edx
    and $0xf, %edx
    mov %edx, %ecx
    cmp $10, %ecx
    jb 15f
    add $87, %cl
    jmp 16f
15: add $48, %cl
16: call ser_putc
    sub $4, %r13d
    jns 14b
    mov $' ', %cl
    call ser_putc
    add $8, %r10
    dec %r11d
    jnz 13b
    mov $10, %cl
    call ser_putc
6:  cli
    hlt
    jmp 6b

/* Xen ELF note: entry type 18 = XEN_ELFNOTE_PHYS32_ENTRY */
.section .note.Xen,"a"
.align 4
.long 4                      /* namesz */
.long 4                      /* descsz */
.long 18                     /* type */
.asciz "Xen"
.align 4
.long pvh32_entry            /* 32-bit entry physical address */
.align 4

.section .data
.align 8
pvh_gdt:
    .quad 0                    /* 0x00: null */
    .quad 0x00cf9a000000ffff   /* 0x08: 32-bit code, base 0, 4 GiB */
    .quad 0x00cf92000000ffff   /* 0x10: 32-bit data */
    .quad 0x00af9a000000ffff   /* 0x18: 64-bit kernel code, L=1, DPL=0 */
    .quad 0x00af92000000ffff   /* 0x20: 64-bit kernel data, DPL=0 */
    .quad 0x00cff2000000ffff   /* 0x28: 64-bit user data, DPL=3 */
    .quad 0x00affa000000ffff   /* 0x30: 64-bit user code, L=1, DPL=3 */
pvh_gdt_end:
pvh_gdt_ptr:
    .word pvh_gdt_end - pvh_gdt - 1
    .long pvh_gdt
    .long 0
.align 8
saved_info: .quad 0

/* isr_table[i] = address of isr<i> stub */
.section .data
.align 8
isr_table:
.irp n,0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
    .quad isr\n
.endr
idt_ptr:
    .word 32*16 - 1
    .long idt_table
    .long 0

.section .bss
.align 16
idt_table: .skip 32*16

.section .bss
.align 4096
pml4:   .skip 4096
pdpt:   .skip 4096
pd:     .skip 4096
.align 16
stack_bottom:
    .skip 65536
stack_top:

/* ---- ring-3 transition + syscall entry ----
 * User CS=0x33 SS=0x2b (GDT idx6/idx5 | RPL3).
 * STAR = (kcs<<32)|(ucs_base<<48): syscall -> CS 0x18 SS 0x20,
 * sysret -> CS (0x20+16)|3=0x33, SS (0x20+8)|3=0x2b.
 */
.section .text
.code64
.global enter_user
enter_user:
    /* rdi=user rip, rsi=user rsp, rdx=user cr3 */
    mov %rdx, %cr3
    push $0x2b             /* user ss */
    push %rsi              /* user rsp */
    push $0x2              /* rflags: IF=0 (no IRQs yet) */
    push $0x33             /* user cs */
    push %rdi              /* user rip */
    iretq

.global syscall_entry
syscall_entry:
    /* syscall leaves RSP=user rsp; switch to kernel stack.
     * rcx=user rip, r11=user rflags (clobbered by syscall). */
    mov %rsp, user_sp
    movabs $stack_top, %rsp
    mov %rax, sf_rax
    mov %rcx, sf_rcx
    mov %r11, sf_r11
    mov %rdi, sf_rdi
    mov %rsi, sf_rsi
    mov %rdx, sf_rdx
    mov %r10, sf_r10
    mov %r8,  sf_r8
    mov %r9,  sf_r9
    call syscall_trap      /* returns result in rax */
    mov user_sp, %rsp
    mov sf_rcx, %rcx
    mov sf_r11, %r11
    sysretq

.section .bss
.align 8
user_sp:   .quad 0
sf_rax:    .quad 0
sf_rcx:    .quad 0
sf_r11:    .quad 0
sf_rdi:    .quad 0
sf_rsi:    .quad 0
sf_rdx:    .quad 0
sf_r10:    .quad 0
sf_r8:     .quad 0
sf_r9:     .quad 0
