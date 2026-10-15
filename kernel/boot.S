/* HolyRust kernel — multiboot1 boot + long-mode bring-up (K1 smoke stage).
 * Evidence channel: serial COM1 (0x3F8) at every step + VGA marker at the end. */
    .section .multiboot
    .align 4
    .set MB_MAGIC,    0x1BADB002
    .set MB_FLAGS,    0x00010003        /* a.out kludge (bit16) + page-align + mem info */
    .set MB_CHECKSUM, -(MB_MAGIC + MB_FLAGS)
mb_header:
    .long MB_MAGIC
    .long MB_FLAGS
    .long MB_CHECKSUM
    /* a.out kludge: flat-binary load info (QEMU -kernel raw multiboot path) */
    .long mb_header                     /* header_addr  */
    .long _kernel_start                 /* load_addr    */
    .long _kernel_load_end              /* load_end_addr (data end, excl. bss) */
    .long _kernel_bss_end               /* bss_end_addr */
    .long _start                        /* entry_addr   */

    .section .rodata
    .align 8
gdt64:
    .quad 0x0000000000000000            /* null */
gdt64_code:
    .quad 0x00209A0000000000            /* 64-bit code, ring0 */
gdt64_data:
    .quad 0x0000920000000000            /* 64-bit data */
gdt64_ptr:
    .word gdt64_ptr - gdt64 - 1
    .quad gdt64

    /* Boot page tables + big kernel stack live OUTSIDE .bss: the explicit
     * bss-zero loop must never touch the tables backing its own writes
     * (this was the S2.1 triple-fault). Loader zeroing is fine but the
     * in-kernel loop range is [_kernel_bss_start, _kernel_bss_end). */
    .section .boot_tables, "aw", @nobits
    .align 4096
pml4:   .skip 4096
pdpt:   .skip 4096
pd:     .skip 4096
    .align 16
kstack_bottom: .skip 8388608      /* 8 MiB kernel+guest+compiler stack */
    .globl kstack_bottom
kstack_top:
    .globl kstack_top

    .section .bss
    .align 16
stack_bottom: .skip 65536
stack_top:

    .section .text
    .code32
    .globl _start
_start:
    mov  $stack_top, %esp
    /* --- serial trace: "S1" --- */
    push $msg_s1
    call serial_puts
    add  $4, %esp

    /* --- build identity page tables: first 1 GiB in 2 MiB pages --- */
    mov  $pml4, %edi
    xor  %eax, %eax
    mov  $4096, %ecx
    cld
1:  stosl
    dec  %ecx
    jnz  1b                               /* zero pml4+pdpt+pd (12 KiB) */

    mov  $pdpt, %eax
    or   $0x3, %eax                       /* present|writable */
    mov  %eax, pml4                       /* PML4[0] -> pdpt */

    mov  $pd, %eax
    or   $0x3, %eax
    mov  %eax, pdpt                       /* PDPT[0] -> pd */

    /* PD: 512 x 2MiB pages, present|write|PS */
    mov  $pd, %edi
    mov  $0x00000083, %eax                /* 0 MiB base */
    mov  $512, %ecx
2:  mov  %eax, (%edi)                     /* low half: addr|flags */
    movl $0, 4(%edi)                      /* high half: MUST be zero */
    add  $0x200000, %eax
    add  $8, %edi
    dec  %ecx
    jnz  2b

    /* --- enable PAE + SSE state + long mode + paging --- */
    /* CR4.PAE (0x20)          : paging with 4-level page tables
       CR4.OSFXSR (0x200)      : the OS supports FXSAVE/FXRSTOR, which is what
                                 makes SSE instructions legal. Without it every
                                 F64 arithmetic instruction the JIT emits faults
                                 (#UD) — the kernel dies the moment guest code
                                 does floating-point maths, while F64 values that
                                 are only copied as bit patterns keep working.
       CR4.OSXMMEXCPT (0x400)  : unmasked SSE exceptions go to #XF rather than
                                 #UD, so a real floating-point fault is at least
                                 distinguishable from an illegal opcode. */
    mov  %cr4, %eax
    or   $0x620, %eax
    mov  %eax, %cr4

    mov  $pml4, %eax
    mov  %eax, %cr3

    mov  $0xC0000080, %ecx                /* EFER */
    rdmsr
    or   $0x100, %eax                     /* EFER.LME */
    wrmsr

    mov  %cr0, %eax
    or   $0x80000000, %eax                /* CR0.PG */
    mov  %eax, %cr0

    push $msg_s15
    call serial_puts
    add  $4, %esp

    /* --- load 64-bit GDT and far-jump to long mode --- */
    lgdt gdt64_ptr
    ljmp $0x08, $long_mode_entry

    .code64
long_mode_entry:
    xor  %ax, %ax
    mov  $0x10, %ax
    mov  %ax, %ds
    mov  %ax, %es
    mov  %ax, %fs
    mov  %ax, %gs
    mov  %ax, %ss
    mov  $stack_top, %rsp

    mov  $msg_s2, %rsi
    call serial_puts64

    mov  $msg_s21, %rsi
    call serial_puts64
    /* --- zero .bss explicitly (belt & braces: do not trust the loader) --- */
    mov  $_kernel_bss_start, %rdi
    mov  $_kernel_bss_end, %rcx
    sub  %rdi, %rcx
    shr  $3, %rcx
    xor  %eax, %eax
    cld
    rep  stosq

    mov  $msg_s22, %rsi
    call serial_puts64
    /* --- switch to the big kernel stack and enter Rust (kmain) --- */
    mov  $kstack_top, %rsp
    xor  %rbp, %rbp
    call kmain

    /* kmain must not return; park if it does */
halt_loop:
    cli
    hlt
    jmp  halt_loop

    /* --- serial helpers (COM1 = 0x3F8, polled) --- */
    .code32
serial_puts:                              /* cdecl: [esp+4] = msg */
    mov  4(%esp), %esi
sp32_loop:
    movzbl (%esi), %eax
    test %al, %al
    jz   sp32_ret
    mov  $0x3FD, %dx
sp32_wait:
    in   (%dx), %al
    test $0x20, %al
    jz   sp32_wait
    mov  $0x3F8, %dx
    mov  0(%esi), %al
    out  %al, (%dx)
    inc  %esi
    jmp  sp32_loop
sp32_ret:
    ret

    .code64
serial_puts64:                            /* rsi = msg */
sp64_loop:
    movzbl (%rsi), %eax
    test %al, %al
    jz   sp64_ret
    mov  $0x3FD, %dx
sp64_wait:
    in   (%dx), %al
    test $0x20, %al
    jz   sp64_wait
    mov  %eax, %ecx                       /* save data (al clobbered by in) */
    movzbl (%rsi), %eax
    mov  $0x3F8, %dx
    out  %al, (%dx)
    inc  %rsi
    jmp  sp64_loop
sp64_ret:
    ret

    .section .rodata
msg_s1: .asciz "S1:32bit-entry\n"
msg_s15: .asciz "S1.5:paging-on\n"
msg_s2: .asciz "S2:64bit-longmode\n"
msg_s21: .asciz "S2.1:pre-bss-zero\n"
msg_s22: .asciz "S2.2:pre-kmain\n"
msg_ok: .asciz "BOOT-OK-64BIT\n"
