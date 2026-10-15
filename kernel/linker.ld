ENTRY(_start)
SECTIONS
{
    . = 1M;
    _kernel_start = .;
    .text : ALIGN(4K)
    {
        *(.multiboot)
        *(.text*)
    }
    .rodata : ALIGN(4K) { *(.rodata*) }
    .eh_frame : ALIGN(4K) { *(.eh_frame*) }
    .gcc_except_table : { *(.gcc_except_table*) }
    .data : ALIGN(4K) { *(.data*) }
    .data.rel.ro : { *(.data.rel.ro*) }
    /* GOT MUST be inside the loaded image: the multiboot a.out kludge only
       copies [load_addr, load_end) — orphaned .got was the K4 boot crash. */
    .got : { *(.got*) }
    .got.plt : { *(.got.plt*) }
    _kernel_load_end = .;
    /* NOBITS, deliberately OUTSIDE the [_kernel_bss_start, _kernel_bss_end)
       zero range: boot page tables + kernel stack (see boot.S). */
    .boot_tables : ALIGN(4K) { *(.boot_tables) }
    .bss : ALIGN(4K)
    {
        _kernel_bss_start = .;
        *(.bss*)
        *(COMMON)
    }
    _kernel_bss_end = .;
    /DISCARD/ : { *(.comment*) *(.note*) *(.debug*) *(.rela*) }
}
