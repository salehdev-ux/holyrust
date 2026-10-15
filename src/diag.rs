//! Shared diagnostic type. Rule (SPEC Part 7.5): every failure path produces a
//! Diagnostic, never a panic.

use alloc::{format, string::{String, ToString}, vec::Vec};
use core::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pos {
    pub line: u32,   // 1-based
    pub col: u32,    // 1-based
    pub offset: u32, // byte offset
}

impl fmt::Display for Pos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub pos: Pos,
    pub code: String,
    pub message: String,
    pub file: String,
    pub suggestion: Option<String>,
}

impl Diagnostic {
    pub fn error(file: &str, pos: Pos, code: &str, message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Error,
            pos,
            code: code.to_string(),
            message: message.into(),
            file: file.to_string(),
            suggestion: None,
        }
    }
    pub fn with_suggestion(mut self, s: impl Into<String>) -> Self {
        self.suggestion = Some(s.into());
        self
    }
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        };
        write!(f, "{}[{}]: {} ({} {})", sev, self.code, self.message, self.file, self.pos)?;
        if let Some(s) = &self.suggestion {
            write!(f, "\n  suggestion: {}", s)?;
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct DiagList {
    pub items: Vec<Diagnostic>,
}

impl DiagList {
    pub fn push(&mut self, d: Diagnostic) {
        self.items.push(d);
    }
    pub fn has_errors(&self) -> bool {
        self.items.iter().any(|d| d.is_error())
    }
    pub fn render(&self, src: &str) -> String {
        let mut out = String::new();
        for d in &self.items {
            out.push_str(&format!("{}\n", d));
            // source-line snippet with caret
            if let Some(line) = src.lines().nth(d.pos.line.saturating_sub(1) as usize) {
                out.push_str(&format!("  | {}\n", line));
                let c = (d.pos.col.saturating_sub(1)) as usize;
                out.push_str(&format!("  | {}^\n", " ".repeat(c.min(line.chars().count()))));
            }
        }
        out
    }
}
