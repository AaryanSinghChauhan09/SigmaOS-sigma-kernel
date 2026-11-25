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
    cmp $64, %ecx
    jb build_idt
    lidt idt_ptr

    /* Syscall MSRs: STAR packs kernel CS (syscall) + user base
     * (sysret); LSTAR = entry stub; SFMASK = 0 (keep rflags). */
    /* STAR[63:32] -> EDX, STAR[31:0] -> EAX for wrmsr.
     * STAR = (syscall_base 0x18)<<32 | (sysret_base 0x20)<<48:
     *   syscall -> CS 0x18 SS 0x20 ; sysret -> CS 0x33 SS 0x2b. */
    mov $0xc0000081, %ecx      /* STAR */
    mov $0x00200018, %edx
    xor %eax, %eax
    wrmsr
    mov $0xc0000082, %ecx      /* LSTAR */
    movabs $syscall_entry, %rax
    mov %rax, %rdx
    shr $32, %rdx              /* edx:eax = entry address */
    wrmsr
    mov $0xc0000084, %ecx      /* SFMASK: clear IF on syscall entry */
    mov $0x200, %eax
    xor %edx, %edx
    wrmsr

    /* TSS: 104-byte struct; RSP0 = kernel stack for ring3->kernel
     * interrupt stack switch. Descriptor built at runtime into the
     * GDT slot at 0x38. */
    movabs $stack_top, %rax
    mov %rax, tss+4             /* TSS.RSP0 */
    movw $104, tss+102          /* IOPB offset: no I/O bitmap */
    lea tss(%rip), %rax
    mov %ax, gdt_tss+0          /* limit[15:0] = 103 */
    movw $103, gdt_tss
    mov %ax, gdt_tss+2          /* base[15:0] */
    shr $16, %rax
    mov %al, gdt_tss+4          /* base[23:16] */
    movb $0x89, gdt_tss+5       /* P=1 type=9 (64-bit TSS avail) */
    movb $0, gdt_tss+6          /* limit[19:16]|flags = 0 */
    movb $0, gdt_tss+7
    shr $8, %rax
    mov %al, gdt_tss+8          /* base[31:24] */
    /* base[63:32] = 0 (tss < 4 GiB); bytes 9-11 reserved = 0 */
    mov $0x38, %ax
    ltr %ax

    /* PIT channel 0 ~100 Hz for the timer IRQ (vector 32). */
    mov $0x43, %dx
    mov $0x36, %al              /* ch0, lobyte/hibyte, mode 3 */
    out %al, %dx
    mov $0x40, %dx
    mov $0x99, %al              /* divisor 1193 lo ~1kHz */
    out %al, %dx
    mov $0x04, %al              /* hi */
    out %al, %dx

    /* Remap 8259 PICs: IRQ0-7 -> vectors 0x20-0x27 (master),
     * IRQ8-15 -> 0x28-0x2f (slave). Without this IRQ0 arrives
     * at vector 8 (#DF) under the BIOS default mapping. */
    mov $0x20, %dx
    mov $0x11, %al              /* ICW1: init, edge, cascade */
    out %al, %dx
    mov $0x21, %dx
    mov $0x20, %al              /* ICW2: vector offset 0x20 */
    out %al, %dx
    mov $0x21, %dx
    mov $0x04, %al              /* ICW3: slave on IRQ2 */
    out %al, %dx
    mov $0x21, %dx
    mov $0x01, %al              /* ICW4: 8086 mode */
    out %al, %dx
    mov $0xa0, %dx
    mov $0x11, %al
    out %al, %dx
    mov $0xa1, %dx
    mov $0x28, %al              /* slave offset 0x28 */
    out %al, %dx
    mov $0xa1, %dx
    mov $0x02, %al
    out %al, %dx
    mov $0xa1, %dx
    mov $0x01, %al
    out %al, %dx
    /* Unmask IRQ0 (timer) on master; leave slave masked. */
    mov $0x21, %dx
    mov $0xfe, %al              /* OCW1: IRQ0 enabled */
    out %al, %dx
    mov $0xa1, %dx
    mov $0xff, %al
    out %al, %dx

    /* Local APIC timer via x2APIC MSRs (Linux lapic timer path;
     * the legacy PIT->PIC route only delivered one IRQ in QEMU).
     * APIC_BASE: bit11 EN | bit10 EXTD(x2apic) = 0xc00.
     * Timer vector 0x30 (outside the PIC's 0x20-0x2f range). */
    mov $0x1b, %ecx            /* IA32_APIC_BASE */
    rdmsr
    or $0xc00, %eax
    wrmsr
    mov $0x80f, %ecx            /* SPURIOUS_VECTOR: en|vec 0xff */
    xor %eax, %eax
    xor %edx, %edx
    mov $0x1ff, %eax
    wrmsr
    mov $0x3e0, %ecx            /* DIVIDE_CONTROL: /16 */
    xor %eax, %eax
    xor %edx, %edx
    xor %eax, %eax
    xor %edx, %edx
    wrmsr
    mov $0x832, %ecx            /* LVT_TIMER: vec 0x30 | periodic */
    xor %eax, %eax
    xor %edx, %edx
    mov $0x20030, %eax
    wrmsr
    mov $0x838, %ecx            /* INITIAL_COUNT */
    xor %eax, %eax
    xor %edx, %edx
    mov $0x100000, %eax
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

/* ---- IRQ stubs (vectors >= 32): must preserve user rax, which
 * `mov $vec,%eax` would otherwise destroy. Save rax first, load
 * the vector, jump to irq_common which restores rax before iretq. */
.macro ISR_IRQ n
isr\n:
    push %rax
    mov $\n, %eax
    jmp irq_common
.endm
.irp n,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63
ISR_IRQ \n
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
    cmp $32, %eax          /* vector in eax; regs still pristine */
    jae 80f                /* IRQs: skip exception dump, EOI+iret */
    mov %eax, %r8d
    cmp $13, %eax            /* #GP: dump error code at (%rsp) */
    jne 86f
    mov $'G', %cl
    call ser_putc
    mov $'P', %cl
    call ser_putc
    mov (%rsp), %eax         /* error code */
    mov %eax, %r12d
    mov $28, %r13d
87: mov %r12d, %eax
    mov %eax, %edx
    mov %r13d, %ecx
    shr %cl, %edx
    and $0xf, %edx
    mov %edx, %ecx
    cmp $10, %ecx
    jb 88f
    add $87, %cl
    jmp 89f
88: add $48, %cl
89: call ser_putc
    sub $4, %r13d
    jns 87b
    mov $10, %cl
    call ser_putc
86: cmp $32, %eax          /* IRQs: skip exception dump, EOI+iret */
    jae 80f
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
6:  cmp $32, %r8d
    jae 80f                /* IRQ: EOI + return to context */
    cli
81: hlt
    jmp 81b
80: /* IRQ path: vector still in eax. Save every GPR (iretq
     * restores none), count ticks, dot every 64, EOI, return. */
    mov %eax, irq_vec
    push %rax
    push %rcx
    push %rdx
    push %rsi
    push %rdi
    push %rbx
    push %rbp
    push %r8
    push %r9
    push %r10
    push %r11
    push %r12
    push %r13
    push %r14
    push %r15
    movabs $tick_ctr, %rax
    incq (%rax)
    mov (%rax), %rdx
82: /* EOI: vec 0x30 -> x2APIC MSR 0x80b; PIC IRQs -> 0x20/0xa0 */
    mov irq_vec, %eax
    cmp $0x30, %eax
    je 83f
    mov $0x20, %al
    mov $0x20, %dx
    out %al, %dx           /* master PIC EOI */
    cmp $0x28, %eax        /* slave IRQs (vec >= 0x28) need both */
    jb 84f
    mov $0x20, %al
    mov $0xa0, %dx
    out %al, %dx           /* slave PIC EOI */
    jmp 84f
83: xor %eax, %eax        /* x2APIC EOI */
    xor %edx, %edx
    mov $0x80b, %ecx
    wrmsr
84: pop %r15
    pop %r14
    pop %r13
    pop %r12
    pop %r11
    pop %r10
    pop %r9
    pop %r8
    pop %rbp
    pop %rbx
    pop %rdi
    pop %rsi
    pop %rdx
    pop %rcx
    pop %rax
    iretq
    jmp 6b

/* IRQ common handler — entered from ISR_IRQ stubs (vec >= 32) with
 * eax = vector and user rax already saved on the stack by the stub.
 * Stack at entry: [user_rax][rip][cs][rflags][rsp][ss].
 * Saves the other 14 GPRs, counts ticks, EOI, restores everything
 * (including user rax), iretq back to the interrupted context. */
irq_common:
    cli
    mov %eax, irq_vec
    push %rcx
    push %rdx
    push %rsi
    push %rdi
    push %rbx
    push %rbp
    push %r8
    push %r9
    push %r10
    push %r11
    push %r12
    push %r13
    push %r14
    push %r15
    movabs $tick_ctr, %r10
    incq (%r10)
irq_eoi:
    mov irq_vec, %eax        /* vector for EOI routing */
    cmp $0x30, %eax          /* LAPIC timer -> x2APIC EOI MSR */
    je irq_lapic_eoi
    mov $0x20, %al           /* master PIC EOI (al=0x20, ah=0) */
    mov $0x20, %dx
    out %al, %dx
    mov irq_vec, %eax        /* reload: al was clobbered above */
    cmp $0x28, %eax          /* slave IRQs need slave+master EOI */
    jb irq_done
    mov $0x20, %al
    mov $0xa0, %dx
    out %al, %dx
    jmp irq_done
irq_lapic_eoi:
    xor %eax, %eax
    xor %edx, %edx
    mov $0x80b, %ecx
    wrmsr
irq_done:
    pop %r15
    pop %r14
    pop %r13
    pop %r12
    pop %r11
    pop %r10
    pop %r9
    pop %r8
    pop %rbp
    pop %rbx
    pop %rdi
    pop %rsi
    pop %rdx
    pop %rcx
    pop %rax                 /* user rax saved by the ISR_IRQ stub */
    iretq

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
    .quad 0                    /* 0x38: TSS descriptor (filled at runtime) */
    .quad 0
pvh_gdt_end:
gdt_tss = pvh_gdt + 0x38        /* TSS descriptor slot (idx7/8) */
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
.irp n,0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63
    .quad isr\n
.endr
idt_ptr:
    .word 64*16 - 1
    .long idt_table
    .long 0

.section .bss
.align 16
idt_table: .skip 64*16

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
    push $0x202            /* rflags: IF=1 (timer IRQs on) */
    push $0x33             /* user cs */
    push %rdi              /* user rip */
    iretq

/* Per-process user context — 17 u64 slots, written by syscall_entry:
 *   0 rax(nr/result) 1 rip(rcx) 2 rdx 3 rsi 4 rdi 5 r8 6 r9 7 r10
 *   8 rbx 9 rbp 10 r12 11 r13 12 r14 13 r15 14 user_sp 15 rflags(r11)
 *   16 cr3
 * cur_ctx -> current ctx; next_ctx/switch_req implement SPAWN/EXIT
 * context switches (Linux context_switch analogue). */
.global cur_ctx
.global next_ctx
.global switch_req
.global tmp_r11

.global syscall_entry
syscall_entry:
    cli
    mov %r11, tmp_r11
    movabs $cur_ctx, %r11
    mov (%r11), %r11          /* r11 = current ctx base */
    mov %rsp, 112(%r11)       /* ctx.user_sp (rsp still user) */
    movabs $stack_top, %rsp   /* switch to kernel stack */
    mov %rax, 0(%r11)
    mov %rcx, 8(%r11)
    mov %rdx, 16(%r11)
    mov %rsi, 24(%r11)
    mov %rdi, 32(%r11)
    mov %r8,  40(%r11)
    mov %r9,  48(%r11)
    mov %r10, 56(%r11)
    mov %rbx, 64(%r11)
    mov %rbp, 72(%r11)
    mov %r12, 80(%r11)
    mov %r13, 88(%r11)
    mov %r14, 96(%r11)
    mov %r15, 104(%r11)
    mov %cr3, %rax
    mov %rax, 128(%r11)       /* ctx.cr3 */
    mov %r11, %rbx            /* rbx = ctx ptr (user rbx saved) */
    mov tmp_r11, %rax
    mov %rax, 120(%r11)       /* ctx.rflags */
    call syscall_trap         /* rax = result; may set switch_req */
    mov %rax, 0(%rbx)         /* ctx.rax = result */
    movabs $switch_req, %rax
    cmpq $0, (%rax)
    je 1f
    movabs $next_ctx, %rax
    mov (%rax), %r11          /* target ctx */
    jmp 2f
1:  mov %rbx, %r11           /* own ctx */
2:  mov 128(%r11), %rax
    mov %rax, %cr3            /* target address space */
    mov 0(%r11), %rax
    mov 8(%r11), %rcx
    mov 16(%r11), %rdx
    mov 24(%r11), %rsi
    mov 32(%r11), %rdi
    mov 40(%r11), %r8
    mov 48(%r11), %r9
    mov 56(%r11), %r10
    mov 64(%r11), %rbx
    mov 72(%r11), %rbp
    mov 80(%r11), %r12
    mov 88(%r11), %r13
    mov 96(%r11), %r14
    mov 104(%r11), %r15
    mov 112(%r11), %rsp       /* user rsp */
    mov 120(%r11), %r11       /* user rflags (last reg load) */
    sysretq

.section .bss
.align 8
cur_ctx:    .quad 0
next_ctx:   .quad 0
switch_req: .quad 0
tmp_r11:    .quad 0
tick_ctr:   .quad 0
irq_vec:    .long 0
tss:       .skip 104
