//! Driver: the compile-and-run pipeline used by the CLI, tests and (Phase 5)
//! the REPL. Owns `#exe {}` expansion (L7/D014): the block is compiled and
//! executed *at compile time* through the same pipeline with stdout captured;
//! the captured text is re-lexed/re-parsed and spliced in place of the block.

use alloc::{format, string::String, vec::Vec};

use crate::diag::{DiagList, Diagnostic, Pos};
use crate::hr_check::check_unit;
use crate::hr_codegen::compile_unit;
use crate::hr_parser::ast::{Item, Program, Stmt};
use crate::hr_parser::parse_program;
use crate::hr_runtime::{Session, MAX_INCLUDE_DEPTH};

pub const MAX_EXE_DEPTH: u32 = 8;

#[derive(Default)]
pub struct RunOutcome {
    pub diags: DiagList,
    pub aborted: bool,
    pub value: i64,
    /// machine-code bytes produced (for benchmarks/diagnostics)
    pub code_len: usize,
}

/// Compile and run one unit of source. Never panics on user input; failures
/// arrive as diagnostics (+ `aborted` for runtime traps).
pub fn compile_and_run(sess: &mut Session, file: &str, src: &str) -> RunOutcome {
    match compile_only(sess, file, src) {
        Ok((entry, diags, code_len)) => {
            let res = sess.run_entry(entry);
            RunOutcome {
                diags,
                aborted: res.aborted,
                value: res.value,
                code_len,
            }
        }
        Err(diags) => RunOutcome {
            diags,
            aborted: false,
            value: 0,
            code_len: 0,
        },
    }
}

/// Compile without executing; returns (entry, diags-without-errors, code_len).
/// Used by benchmarks, `#exe` sub-units and the Phase 6 hot-patch machinery.
pub fn compile_only(
    sess: &mut Session,
    file: &str,
    src: &str,
) -> Result<(u64, DiagList, usize), DiagList> {
    let mut diags = DiagList::default();
    // TempleOS-RS Phase 5: `#include` is resolved before parsing, because that is
    // what the directive means — the included source *becomes* part of this unit.
    let src = expand_source(sess, file, src, 0, &mut diags);
    if diags.has_errors() {
        return Err(diags);
    }
    let items = expand_program(sess, file, &src, 0, &mut diags);
    if diags.has_errors() {
        return Err(diags);
    }
    compile_items(sess, file, items, diags)
}

// ---------------- #include (TempleOS-RS Phase 5, T5.x) ----------------

/// If `line` is exactly an `#include "path"` directive (surrounded only by
/// whitespace and an optional trailing comment), return the path.
///
/// The shape is deliberately strict: a *statement* about a line's whole content
/// cannot accidentally match inside a string or a comment.
fn include_target(line: &str) -> Option<String> {
    let t = line.trim_start();
    let rest = t.strip_prefix("#include")?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    let path = &rest[..end];
    let tail = rest[end + 1..].trim();
    if !(tail.is_empty() || tail.starts_with("//")) {
        return None;
    }
    Some(String::from(path))
}

/// Read an included file: the session's filesystem first — that is where `/TOS`
/// lives, and it is what lets the OS include its own source — then, on the host
/// only, the real filesystem.
fn read_include(sess: &Session, path: &str) -> Option<String> {
    if let Some(bytes) = sess.rt().fs.read(path) {
        return Some(String::from_utf8_lossy(bytes).into_owned());
    }
    #[cfg(feature = "host")]
    {
        if let Ok(text) = std::fs::read_to_string(path) {
            return Some(text);
        }
    }
    None
}

/// Splice every `#include` recursively, depth-capped like `#exe` (D014).
pub fn expand_source(
    sess: &mut Session,
    file: &str,
    src: &str,
    depth: u32,
    diags: &mut DiagList,
) -> String {
    if !src.contains("#include") {
        return String::from(src);
    }
    if depth > MAX_INCLUDE_DEPTH {
        diags.push(Diagnostic::error(
            file,
            Pos::default(),
            "C093",
            format!("#include nesting too deep (limit {})", MAX_INCLUDE_DEPTH),
        ));
        return String::from(src);
    }
    let mut out = String::new();
    for (idx, line) in src.split('\n').enumerate() {
        match include_target(line) {
            Some(path) => match read_include(sess, &path) {
                Some(text) => {
                    let sub = format!("{}#include({})", file, path);
                    out.push_str(&expand_source(sess, &sub, &text, depth + 1, diags));
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                None => diags.push(Diagnostic::error(
                    file,
                    Pos { line: idx as u32 + 1, col: 1, offset: 0 },
                    "C094",
                    format!("#include — cannot find `{}`", path),
                )),
            },
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

/// Compile pre-parsed items (hot-patching compiles single function defs).
pub fn compile_items(
    sess: &mut Session,
    file: &str,
    items: Vec<Item>,
    mut diags: DiagList,
) -> Result<(u64, DiagList, usize), DiagList> {
    let prog = Program { items };
    let (typed, cdiags) = check_unit(sess, file, &prog);
    diags.items.extend(cdiags.items);
    if diags.has_errors() {
        return Err(diags);
    }
    match compile_unit(sess, &typed) {
        Ok(cu) => Ok((cu.entry, diags, cu.code_len)),
        Err(d) => {
            diags.items.extend(d.items);
            Err(diags)
        }
    }
}

/// Parse + expand all top-level `#exe` blocks (recursively, depth-capped).
pub fn expand_program(
    sess: &mut Session,
    file: &str,
    src: &str,
    depth: u32,
    diags: &mut DiagList,
) -> Vec<Item> {
    let (prog, parse_diags) = parse_program(file, src);
    diags.items.extend(parse_diags.items);
    if diags.has_errors() {
        return prog.items;
    }
    expand_items(sess, file, prog.items, depth, diags)
}

fn expand_items(
    sess: &mut Session,
    file: &str,
    items: Vec<Item>,
    depth: u32,
    diags: &mut DiagList,
) -> Vec<Item> {
    let has_exe = items
        .iter()
        .any(|it| matches!(it, Item::Stmt(Stmt::Exe(..))));
    if !has_exe || diags.has_errors() {
        return items;
    }
    if depth > MAX_EXE_DEPTH {
        diags.push(Diagnostic::error(
            file,
            Pos::default(),
            "C091",
            format!("#exe nesting too deep (limit {})", MAX_EXE_DEPTH),
        ));
        return items;
    }
    let mut out = Vec::new();
    for it in items {
        match it {
            Item::Stmt(Stmt::Exe(body, sp)) => {
                let subfile = format!("{}#exe@{}", file, sp.pos);
                let sub_prog = block_to_program(*body);
                if let Some(text) = run_captured(sess, &subfile, &sub_prog, diags) {
                    let sub_items = expand_program(sess, &subfile, &text, depth + 1, diags);
                    out.extend(sub_items);
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn block_to_program(body: Stmt) -> Program {
    match body {
        Stmt::Block(stmts, _) => Program {
            items: stmts.into_iter().map(Item::Stmt).collect(),
        },
        other => Program {
            items: vec![Item::Stmt(other)],
        },
    }
}

/// Compile+run a sub-program with stdout captured; returns the captured text.
fn run_captured(
    sess: &mut Session,
    file: &str,
    prog: &Program,
    diags: &mut DiagList,
) -> Option<String> {
    let (typed, cdiags) = check_unit(sess, file, prog);
    diags.items.extend(cdiags.items);
    if diags.has_errors() {
        return None;
    }
    let compiled = compile_unit(sess, &typed);
    let cu = match compiled {
        Ok(cu) => cu,
        Err(d) => {
            diags.items.extend(d.items);
            return None;
        }
    };
    let outer_capture = sess.rt_mut().capture.take();
    sess.rt_mut().capture = Some(Vec::new());
    let res = sess.run_entry(cu.entry);
    let cap = sess.rt_mut().capture.take().unwrap_or_default();
    sess.rt_mut().capture = outer_capture;
    if res.aborted {
        diags.push(Diagnostic::error(
            file,
            Pos::default(),
            "C092",
            "#exe block trapped at compile time (see runtime diagnostic) — nothing spliced",
        ));
        return None;
    }
    Some(String::from_utf8_lossy(&cap).into_owned())
}
