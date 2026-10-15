//! D043 (Windows port): assembles `boot.S` into the kernel image without GNU
//! `as`. v1's `build.sh` calls `as --64`; no ELF-capable GNU assembler exists
//! on a stock Windows host, but rustc ships LLVM's integrated assembler, which
//! accepts GAS syntax. Compiling this file with
//! `rustc --target x86_64-unknown-none --emit=obj` yields `boot.o` (ELF64)
//! with byte-identical sections to the `as --64` output.
//!
//! This file is deliberately NOT under `src/`: it is not a module of the
//! kernel crate and must never be compiled into `libhr_kernel.a`.

#![no_std]

// Rust's global_asm! defaults to Intel syntax; boot.S is GAS (AT&T), so the
// dialect must be selected explicitly (this was the first failure mode).
core::arch::global_asm!(include_str!("boot.S"), options(att_syntax));
