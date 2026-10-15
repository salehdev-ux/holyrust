//! Component A — `hr_lexer` (SPEC §2.2A).
//! UTF-8 source → token stream with (line, col) positions. Zero external deps.
//! Hard rule: never panic on any input; every failure path yields a Diagnostic
//! and lexing continues (SPEC Part 7.5).

use alloc::{format, string::String, vec::Vec};

use crate::diag::{DiagList, Diagnostic, Pos};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kw {
    If,
    Else,
    While,
    Do,
    For,
    Switch,
    Case,
    Default,
    Break,
    Continue,
    Return,
    Try,
    Catch,
    Throw,
    Unsafe,
    Class,
    Mut,
    // Types (L1) + built-in Str (D007)
    U0,
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F64,
    Str,
}

impl Kw {
    pub fn name(self) -> &'static str {
        use Kw::*;
        match self {
            If => "if",
            Else => "else",
            While => "while",
            Do => "do",
            For => "for",
            Switch => "switch",
            Case => "case",
            Default => "default",
            Break => "break",
            Continue => "continue",
            Return => "return",
            Try => "try",
            Catch => "catch",
            Throw => "throw",
            Unsafe => "unsafe",
            Class => "class",
            Mut => "mut",
            U0 => "U0",
            U8 => "U8",
            U16 => "U16",
            U32 => "U32",
            U64 => "U64",
            I8 => "I8",
            I16 => "I16",
            I32 => "I32",
            I64 => "I64",
            F64 => "F64",
            Str => "Str",
        }
    }
    pub fn from_ident(s: &str) -> Option<Kw> {
        use Kw::*;
        Some(match s {
            "if" => If,
            "else" => Else,
            "while" => While,
            "do" => Do,
            "for" => For,
            "switch" => Switch,
            "case" => Case,
            "default" => Default,
            "break" => Break,
            "continue" => Continue,
            "return" => Return,
            "try" => Try,
            "catch" => Catch,
            "throw" => Throw,
            "unsafe" => Unsafe,
            "class" => Class,
            "mut" => Mut,
            "U0" => U0,
            "U8" => U8,
            "U16" => U16,
            "U32" => U32,
            "U64" => U64,
            "I8" => I8,
            "I16" => I16,
            "I32" => I32,
            "I64" => I64,
            "F64" => F64,
            "Str" => Str,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    IntLit(i64),   // bit-pattern; unsigned contexts reinterpret (LANG_SPEC §3)
    FloatLit(f64),
    StrLit(String),
    CharLit(i64), // single byte value, zero-extended
    Ident(String),
    Kw(Kw),
    // punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Semi,
    Comma,
    Colon,
    Dot,
    DotDotDot, // `...` case range (L6) AND variadic marker (L3); parser disambiguates
    // operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    AmpAmp,
    Pipe,
    PipePipe,
    Caret,
    Tilde,
    Bang,
    ShiftL,
    ShiftR,
    Eq,
    EqEq,
    BangEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,
    ShiftLEq,
    ShiftREq,
    PlusPlus,
    MinusMinus,
    Hash,    // `#`  → #exe (L7)
    HashBang, // `#!` → #![ownership(...)] (§2.3)
    Eof,
}

impl Tok {
    /// Stable short rendering used by golden tests and `--dump-tokens`.
    pub fn name(&self) -> String {
        match self {
            Tok::IntLit(v) => format!("IntLit({})", v),
            Tok::FloatLit(v) => format!("FloatLit({:?})", v),
            Tok::StrLit(s) => format!("StrLit({:?})", s),
            Tok::CharLit(c) => format!("CharLit({})", c),
            Tok::Ident(s) => format!("Ident({})", s),
            Tok::Kw(k) => format!("Kw({})", k.name()),
            Tok::LParen => "(".into(),
            Tok::RParen => ")".into(),
            Tok::LBrace => "{".into(),
            Tok::RBrace => "}".into(),
            Tok::LBracket => "[".into(),
            Tok::RBracket => "]".into(),
            Tok::Semi => ";".into(),
            Tok::Comma => ",".into(),
            Tok::Colon => ":".into(),
            Tok::Dot => ".".into(),
            Tok::DotDotDot => "...".into(),
            Tok::Plus => "+".into(),
            Tok::Minus => "-".into(),
            Tok::Star => "*".into(),
            Tok::Slash => "/".into(),
            Tok::Percent => "%".into(),
            Tok::Amp => "&".into(),
            Tok::AmpAmp => "&&".into(),
            Tok::Pipe => "|".into(),
            Tok::PipePipe => "||".into(),
            Tok::Caret => "^".into(),
            Tok::Tilde => "~".into(),
            Tok::Bang => "!".into(),
            Tok::ShiftL => "<<".into(),
            Tok::ShiftR => ">>".into(),
            Tok::Eq => "=".into(),
            Tok::EqEq => "==".into(),
            Tok::BangEq => "!=".into(),
            Tok::Lt => "<".into(),
            Tok::LtEq => "<=".into(),
            Tok::Gt => ">".into(),
            Tok::GtEq => ">=".into(),
            Tok::PlusEq => "+=".into(),
            Tok::MinusEq => "-=".into(),
            Tok::StarEq => "*=".into(),
            Tok::SlashEq => "/=".into(),
            Tok::PercentEq => "%=".into(),
            Tok::AmpEq => "&=".into(),
            Tok::PipeEq => "|=".into(),
            Tok::CaretEq => "^=".into(),
            Tok::ShiftLEq => "<<=".into(),
            Tok::ShiftREq => ">>=".into(),
            Tok::PlusPlus => "++".into(),
            Tok::MinusMinus => "--".into(),
            Tok::Hash => "#".into(),
            Tok::HashBang => "#!".into(),
            Tok::Eof => "EOF".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub pos: Pos,     // start (1-based line/col, byte offset)
    pub end_off: u32, // exclusive end byte offset
}

pub struct Lexer<'a> {
    file: &'a str,
    chars: Vec<char>,
    // byte_off[i] = byte offset of chars[i]; byte_off[len] = total len
    byte_off: Vec<u32>,
    i: usize,
    line: u32,
    line_start: usize, // char index of current line start
    pub diags: DiagList,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}
fn is_ident_cont(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}
fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}

impl<'a> Lexer<'a> {
    pub fn new(file: &'a str, src: &str) -> Self {
        let chars: Vec<char> = src.chars().collect();
        let mut byte_off = Vec::with_capacity(chars.len() + 1);
        let mut acc: u32 = 0;
        for ch in &chars {
            byte_off.push(acc);
            acc = acc.saturating_add(ch.len_utf8() as u32);
        }
        byte_off.push(acc);
        Lexer {
            file,
            chars,
            byte_off,
            i: 0,
            line: 1,
            line_start: 0,
            diags: DiagList::default(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.i).copied()
    }
    fn peek2(&self) -> Option<char> {
        self.chars.get(self.i + 1).copied()
    }
    fn peek_at(&self, k: usize) -> Option<char> {
        self.chars.get(self.i + k).copied()
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.chars.get(self.i).copied();
        if c == Some('\n') {
            self.line = self.line.saturating_add(1);
            self.line_start = self.i + 1;
        }
        if c.is_some() {
            self.i += 1;
        }
        c
    }
    fn pos(&self) -> Pos {
        let idx = self.i.min(self.byte_off.len().saturating_sub(1));
        let col = (idx - self.line_start) as u32 + 1;
        Pos {
            line: self.line,
            col,
            offset: self.byte_off.get(idx).copied().unwrap_or(0),
        }
    }

    fn err(&mut self, pos: Pos, code: &str, msg: impl Into<String>) {
        self.diags
            .push(Diagnostic::error(self.file, pos, code, msg));
    }
    fn err_sugg(&mut self, pos: Pos, code: &str, msg: impl Into<String>, sugg: impl Into<String>) {
        self.diags
            .push(Diagnostic::error(self.file, pos, code, msg).with_suggestion(sugg));
    }

    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('/') if self.peek2() == Some('/') => {
                    while let Some(c) = self.peek() {
                        self.bump();
                        if c == '\n' {
                            break;
                        }
                    }
                }
                Some('/') if self.peek2() == Some('*') => {
                    let start = self.pos();
                    self.bump();
                    self.bump();
                    loop {
                        match self.peek() {
                            None => {
                                self.err(start, "L001", "unterminated block comment");
                                break;
                            }
                            Some('*') if self.peek2() == Some('/') => {
                                self.bump();
                                self.bump();
                                break;
                            }
                            Some(_) => {
                                self.bump();
                            }
                        }
                    }
                }
                _ => break,
            }
        }
    }

    /// Lex a string or char body starting AFTER the opening quote.
    /// Returns decoded bytes-as-string plus (for chars) the single value.
    fn lex_quoted(&mut self, quote: char, start: Pos) -> Option<String> {
        let mut out = String::new();
        loop {
            match self.peek() {
                None => {
                    self.err(
                        start,
                        "L002",
                        format!("unterminated {} literal", if quote == '"' { "string" } else { "char" }),
                    );
                    return Some(out);
                }
                Some('\n') => {
                    self.err(
                        start,
                        "L002",
                        format!("unterminated {} literal (newline before closing {})", if quote == '"' { "string" } else { "char" }, quote),
                    );
                    return Some(out);
                }
                Some(c) if c == quote => {
                    self.bump();
                    return Some(out);
                }
                Some('\\') => {
                    self.bump();
                    match self.peek() {
                        None => {
                            self.err(start, "L003", "unterminated escape at end of input");
                            return Some(out);
                        }
                        Some(e) => {
                            self.bump();
                            match e {
                                'n' => out.push('\n'),
                                't' => out.push('\t'),
                                'r' => out.push('\r'),
                                '0' => out.push('\0'),
                                '\\' => out.push('\\'),
                                '"' => out.push('"'),
                                '\'' => out.push('\''),
                                'x' => {
                                    let mut v: u32 = 0;
                                    let mut n = 0;
                                    while n < 2 {
                                        match self.peek() {
                                            Some(h) if h.is_ascii_hexdigit() => {
                                                v = v
                                                    .wrapping_mul(16)
                                                    .wrapping_add(h.to_digit(16).unwrap_or(0));
                                                self.bump();
                                                n += 1;
                                            }
                                            _ => break,
                                        }
                                    }
                                    if n < 2 {
                                        self.err(start, "L004", "invalid \\x escape (expected 2 hex digits)");
                                        out.push((v as u8) as char);
                                    } else {
                                        out.push((v as u8) as char);
                                    }
                                }
                                other => {
                                    let p = self.pos();
                                    self.err_sugg(p, "L005", format!("unknown escape '\\{}'", other), "valid escapes: \\n \\t \\r \\0 \\\\ \\\" \\' \\xHH");
                                    out.push(other);
                                }
                            }
                        }
                    }
                }
                Some(c) => {
                    self.bump();
                    out.push(c);
                }
            }
        }
    }

    fn lex_number(&mut self, start: Pos) -> Tok {
        // hex / binary
        if self.peek() == Some('0') && matches!(self.peek2(), Some('x') | Some('X')) {
            self.bump();
            self.bump();
            let mut v: u64 = 0;
            let mut n = 0;
            while let Some(c) = self.peek() {
                if c.is_ascii_hexdigit() {
                    v = v.wrapping_mul(16).wrapping_add(c.to_digit(16).unwrap_or(0) as u64);
                    self.bump();
                    n += 1;
                } else {
                    break;
                }
            }
            if n == 0 {
                self.err(start, "L010", "hex literal with no digits");
                return Tok::IntLit(0);
            }
            self.after_number(start);
            return Tok::IntLit(v as i64);
        }
        if self.peek() == Some('0') && matches!(self.peek2(), Some('b') | Some('B')) {
            self.bump();
            self.bump();
            let mut v: u64 = 0;
            let mut n = 0;
            while let Some(c) = self.peek() {
                if c == '0' || c == '1' {
                    v = v.wrapping_mul(2).wrapping_add(c.to_digit(2).unwrap_or(0) as u64);
                    self.bump();
                    n += 1;
                } else {
                    break;
                }
            }
            if n == 0 {
                self.err(start, "L011", "binary literal with no digits");
                return Tok::IntLit(0);
            }
            self.after_number(start);
            return Tok::IntLit(v as i64);
        }
        // decimal int / float
        let mut digits = String::new();
        while let Some(c) = self.peek() {
            if is_digit(c) {
                digits.push(c);
                self.bump();
            } else {
                break;
            }
        }
        // float: `.` only joins when followed by a digit (resolves `4...8` vs `4.5`)
        let mut is_float = false;
        if self.peek() == Some('.') && is_digit(self.peek2().unwrap_or(' ')) {
            is_float = true;
            digits.push('.');
            self.bump();
            while let Some(c) = self.peek() {
                if is_digit(c) {
                    digits.push(c);
                    self.bump();
                } else {
                    break;
                }
            }
        }
        // exponent
        if matches!(self.peek(), Some('e') | Some('E')) {
            let save_i = self.i;
            self.bump();
            let mut exp = String::new();
            if matches!(self.peek(), Some('+') | Some('-')) {
                exp.push(self.bump().unwrap_or('+'));
            }
            let mut ndig = 0;
            while let Some(c) = self.peek() {
                if is_digit(c) {
                    exp.push(c);
                    self.bump();
                    ndig += 1;
                } else {
                    break;
                }
            }
            if ndig == 0 {
                // not an exponent after all (e.g. `1e` → IntLit(1) + Ident(e)); rewind
                self.i = save_i;
            } else {
                is_float = true;
                digits.push('e');
                digits.push_str(&exp);
            }
        }
        self.after_number(start);
        if is_float {
            match digits.parse::<f64>() {
                Ok(v) => Tok::FloatLit(v),
                Err(_) => {
                    // overflow to inf / parse issue — diagnose, keep going
                    self.err(start, "L012", format!("invalid float literal '{}'", digits));
                    Tok::FloatLit(f64::INFINITY)
                }
            }
        } else {
            match digits.parse::<u128>() {
                Ok(v) => {
                    if v <= i64::MAX as u128 {
                        Tok::IntLit(v as i64)
                    } else if v <= u64::MAX as u128 {
                        // allowed as bit pattern for unsigned contexts (LANG_SPEC §3)
                        Tok::IntLit(v as u64 as i64)
                    } else {
                        self.err_sugg(
                            start,
                            "L013",
                            format!("integer literal {} overflows U64", v),
                            "largest supported literal is 18446744073709551615",
                        );
                        Tok::IntLit(i64::MAX)
                    }
                }
                Err(_) => {
                    self.err(start, "L013", "integer literal overflow");
                    Tok::IntLit(i64::MAX)
                }
            }
        }
    }

    /// After a numeric literal: reject glued identifier chars (`123abc`).
    fn after_number(&mut self, start: Pos) {
        if let Some(c) = self.peek() {
            if is_ident_start(c) {
                let p = self.pos();
                self.err(
                    p,
                    "L014",
                    format!("unexpected character '{}' after numeric literal", c),
                );
                let _ = start;
                while let Some(c2) = self.peek() {
                    if is_ident_cont(c2) {
                        self.bump();
                    } else {
                        break;
                    }
                }
            }
        }
    }

    fn lex_ident(&mut self, start: Pos) -> Tok {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if is_ident_cont(c) {
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }
        let _ = start;
        if let Some(k) = Kw::from_ident(&s) {
            Tok::Kw(k)
        } else {
            Tok::Ident(s)
        }
    }

    /// Main entry: lex the whole input. Always returns a stream ending in exactly one Eof.
    pub fn tokenize_all(&mut self) -> Vec<Token> {
        let mut out = Vec::new();
        loop {
            self.skip_trivia();
            let start_idx = self.i;
            let start = self.pos();
            let Some(c) = self.peek() else {
                out.push(Token {
                    tok: Tok::Eof,
                    pos: start,
                    end_off: self.byte_off.last().copied().unwrap_or(0),
                });
                return out;
            };
            let tok = match c {
                '"' => {
                    self.bump();
                    let body = self.lex_quoted('"', start).unwrap_or_default();
                    Tok::StrLit(body)
                }
                '\'' => {
                    self.bump();
                    let body = self.lex_quoted('\'', start).unwrap_or_default();
                    let mut bytes = body.chars();
                    let first = bytes.next();
                    if bytes.next().is_some() {
                        self.err(start, "L020", "char literal with more than one character");
                    }
                    if first.is_none() {
                        self.err(start, "L021", "empty char literal");
                    }
                    Tok::CharLit(first.map(|ch| (ch as u32) as i64).unwrap_or(0))
                }
                '0'..='9' => self.lex_number(start),
                'a'..='z' | 'A'..='Z' | '_' => self.lex_ident(start),
                '(' => { self.bump(); Tok::LParen }
                ')' => { self.bump(); Tok::RParen }
                '{' => { self.bump(); Tok::LBrace }
                '}' => { self.bump(); Tok::RBrace }
                '[' => { self.bump(); Tok::LBracket }
                ']' => { self.bump(); Tok::RBracket }
                ';' => { self.bump(); Tok::Semi }
                ',' => { self.bump(); Tok::Comma }
                ':' => { self.bump(); Tok::Colon }
                '~' => { self.bump(); Tok::Tilde }
                '.' => {
                    if self.peek2() == Some('.') && self.peek_at(2) == Some('.') {
                        self.bump(); self.bump(); self.bump();
                        Tok::DotDotDot
                    } else if self.peek2() == Some('.') {
                        // `..` typo → recover as `...` with a suggestion
                        self.err_sugg(start, "L030", "found `..`", "did you mean the range operator `...`?");
                        self.bump(); self.bump();
                        Tok::DotDotDot
                    } else {
                        self.bump();
                        Tok::Dot
                    }
                }
                '+' => { self.bump(); match self.peek() { Some('=') => { self.bump(); Tok::PlusEq } Some('+') => { self.bump(); Tok::PlusPlus } _ => Tok::Plus } }
                '-' => { self.bump(); match self.peek() { Some('=') => { self.bump(); Tok::MinusEq } Some('-') => { self.bump(); Tok::MinusMinus } _ => Tok::Minus } }
                '*' => { self.bump(); match self.peek() { Some('=') => { self.bump(); Tok::StarEq } _ => Tok::Star } }
                '%' => { self.bump(); match self.peek() { Some('=') => { self.bump(); Tok::PercentEq } _ => Tok::Percent } }
                '^' => { self.bump(); match self.peek() { Some('=') => { self.bump(); Tok::CaretEq } _ => Tok::Caret } }
                '/' => {
                    // comments already handled in skip_trivia
                    self.bump();
                    match self.peek() { Some('=') => { self.bump(); Tok::SlashEq } _ => Tok::Slash }
                }
                '&' => { self.bump(); match self.peek() { Some('&') => { self.bump(); Tok::AmpAmp } Some('=') => { self.bump(); Tok::AmpEq } _ => Tok::Amp } }
                '|' => { self.bump(); match self.peek() { Some('|') => { self.bump(); Tok::PipePipe } Some('=') => { self.bump(); Tok::PipeEq } _ => Tok::Pipe } }
                '!' => { self.bump(); match self.peek() { Some('=') => { self.bump(); Tok::BangEq } _ => Tok::Bang } }
                '=' => { self.bump(); match self.peek() { Some('=') => { self.bump(); Tok::EqEq } _ => Tok::Eq } }
                '<' => { self.bump(); match self.peek() { Some('<') => { self.bump(); match self.peek() { Some('=') => { self.bump(); Tok::ShiftLEq } _ => Tok::ShiftL } } Some('=') => { self.bump(); Tok::LtEq } _ => Tok::Lt } }
                '>' => { self.bump(); match self.peek() { Some('>') => { self.bump(); match self.peek() { Some('=') => { self.bump(); Tok::ShiftREq } _ => Tok::ShiftR } } Some('=') => { self.bump(); Tok::GtEq } _ => Tok::Gt } }
                '#' => { self.bump(); if self.peek() == Some('!') { self.bump(); Tok::HashBang } else { Tok::Hash } }
                other => {
                    // any other char (incl. non-ASCII outside literals): diagnostic + skip
                    let p = self.pos();
                    self.err(p, "L040", format!("unexpected character '{}' (U+{:04X})", other, other as u32));
                    self.bump();
                    continue; // do not emit a token
                }
            };
            let end_off = self.byte_off.get(self.i).copied().unwrap_or(0);
            if self.i <= start_idx {
                // Progress guard (never panic — Part 7.5): force forward motion + diagnostic.
                self.err(start, "L099", "internal lexer stall — skipped a character");
                self.bump();
            }
            out.push(Token { tok, pos: start, end_off });
        }
    }
}

/// Lex a source string. Returns (tokens ending in Eof, diagnostics). Never panics.
pub fn tokenize(file: &str, src: &str) -> (Vec<Token>, DiagList) {
    let mut lx = Lexer::new(file, src);
    let toks = lx.tokenize_all();
    (toks, lx.diags)
}

#[cfg(test)]
mod unit {
    use super::*;

    fn names(src: &str) -> Vec<String> {
        let (toks, d) = tokenize("unit", src);
        assert!(!d.has_errors(), "unexpected diags: {:?}", d.items.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        toks.iter().map(|t| t.tok.name()).collect()
    }

    #[test]
    fn operators_longest_match() {
        assert_eq!(names("<<= >>= ... && || == ->"), vec![
            // `->` is not an operator: lexes as Minus Gt (parser will reject) — documented
            "<<=", ">>=", "...", "&&", "||", "==", "-", ">", "EOF",
        ]);
    }

    #[test]
    fn range_vs_float() {
        assert_eq!(names("case 4...8:"), vec!["Kw(case)", "IntLit(4)", "...", "IntLit(8)", ":", "EOF"]);
        // `4.` is NOT a float (`.` joins only when followed by a digit) — resolves
        // the `case 4...8` vs float ambiguity deterministically.
        assert_eq!(names("4.5 4. 1e3 1.5e-3 .5"), vec![
            "FloatLit(4.5)", "IntLit(4)", ".", "FloatLit(1000.0)", "FloatLit(0.0015)", ".", "IntLit(5)", "EOF",
        ]);
    }

    #[test]
    fn dotdot_suggests_range() {
        let (_, d) = tokenize("t", "1..2");
        assert!(d.items.iter().any(|x| x.code == "L030" && x.suggestion.is_some()));
    }

    #[test]
    fn bad_char_has_line_col() {
        let (_, d) = tokenize("t", "x\n  @");
        let e = d.items.iter().find(|x| x.code == "L040").expect("diag");
        assert_eq!((e.pos.line, e.pos.col), (2, 3));
    }

    #[test]
    fn never_panics_on_garbage() {
        for s in ["\"", "'\\", "/*", "0x", "0b", "1e", "\u{0}", "\u{1F600}", "18446744073709551616", "\\q"] {
            let _ = tokenize("t", s);
        }
    }
}
