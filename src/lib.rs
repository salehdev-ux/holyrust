//! HolyRust compiler + runtime (SPEC.md Part 2.2 components).
//! Each module maps 1:1 to a spec component.
//!
//! Dual-mode (P6b): with the default `host` feature this builds the normal
//! std host compiler/REPL. With `--no-default-features` the whole library is
//! `#![no_std] + alloc` — this is what the Phase 6 kernel links to run the
//! compiler INSIDE the kernel (SPEC Phase 6 Task 2).

#![cfg_attr(not(feature = "host"), no_std)]
#[macro_use]
extern crate alloc;

/// Vec-backed map replacing HashMap (kernel has no SipHash); see module docs.
pub mod compat;
/// Pipeline diagnostics shared by all components.
pub mod diag;
/// Output hooks (host: std streams; kernel: VGA+serial).
pub mod hr_out;

pub mod hr_lexer;
pub mod hr_parser;
pub mod hr_check;
pub mod hr_codegen;
pub mod hr_runtime;
/// Driver: source → expanded AST → typed AST → machine code → execution.
pub mod pipeline;

#[cfg(feature = "host")]
pub mod hr_shell;
