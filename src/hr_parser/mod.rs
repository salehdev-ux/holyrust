//! Component B — `hr_parser` (SPEC §2.2B).
//! Recursive-descent (LL-ish) parser with:
//!  * error recovery: report location → skip to next `;`/`}` → continue (REPL-critical);
//!  * progress guards everywhere (a failed construct always consumes ≥1 token at
//!    statement level — no infinite loops, Part 7.5);
//!  * explicit nesting-depth limit (adversarial `((((…` cannot overflow the stack).

pub mod ast;

use alloc::{boxed::Box, format, string::String, vec::Vec};
use core::mem::discriminant;

use crate::diag::{DiagList, Diagnostic, Pos};
use crate::hr_lexer::{tokenize, Kw, Tok, Token};
use ast::*;

const MAX_DEPTH: u32 = 96;

pub struct Parser<'a> {
    file: &'a str,
    toks: Vec<Token>,
    i: usize,
    pub diags: DiagList,
    depth: u32,
}

impl<'a> Parser<'a> {
    pub fn new(file: &'a str, toks: Vec<Token>) -> Self {
        Parser {
            file,
            toks,
            i: 0,
            diags: DiagList::default(),
            depth: 0,
        }
    }

    // ----- token cursor -----

    fn peek(&self) -> &Tok {
        &self.toks[self.i.min(self.toks.len() - 1)].tok
    }
    fn peek_at(&self, k: usize) -> &Tok {
        &self.toks[(self.i + k).min(self.toks.len() - 1)].tok
    }
    fn cur_pos(&self) -> Pos {
        self.toks[self.i.min(self.toks.len() - 1)].pos
    }
    fn cur_end(&self) -> u32 {
        self.toks[self.i.min(self.toks.len() - 1)].end_off
    }
    fn at_eof(&self) -> bool {
        matches!(self.peek(), Tok::Eof)
    }
    fn at(&self, t: &Tok) -> bool {
        // NOTE: payload-carrying variants (Kw) must compare values — a bare
        // discriminant check made `eat(Kw(Else))` match EVERY keyword (P3 bug:
        // `if … } I64 a = …` parsed the decl as an else-branch).
        match (self.peek(), t) {
            (Tok::Kw(a), Tok::Kw(b)) => a == b,
            (a, b) => discriminant(a) == discriminant(b),
        }
    }
    fn bump(&mut self) -> Token {
        let t = self.toks[self.i.min(self.toks.len() - 1)].clone();
        if self.i < self.toks.len() - 1 {
            self.i += 1;
        }
        t
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.at(t) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, t: &Tok, ctx: &str) -> Option<Pos> {
        if self.at(t) {
            Some(self.bump().pos)
        } else {
            self.err_here(
                "P001",
                format!("expected `{}` {}, found `{}`", t.name(), ctx, self.peek().name()),
            );
            None
        }
    }
    fn err_here(&mut self, code: &str, msg: impl Into<String>) {
        let pos = self.cur_pos();
        self.diags
            .push(Diagnostic::error(self.file, pos, code, msg));
    }
    fn err_here_sugg(&mut self, code: &str, msg: impl Into<String>, sugg: impl Into<String>) {
        let pos = self.cur_pos();
        self.diags.push(
            Diagnostic::error(self.file, pos, code, msg).with_suggestion(sugg),
        );
    }
    fn err_at(&mut self, pos: Pos, code: &str, msg: impl Into<String>) {
        self.diags
            .push(Diagnostic::error(self.file, pos, code, msg));
    }

    /// Error recovery: skip to just past the next `;` (consumed), or stop before
    /// `}`/EOF (not consumed). Always safe to call.
    fn synchronize_stmt(&mut self) {
        loop {
            match self.peek() {
                Tok::Eof => break,
                Tok::RBrace => break,
                Tok::Semi => {
                    self.bump();
                    break;
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn enter(&mut self) -> bool {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.depth -= 1;
            self.err_here_sugg(
                "P002",
                format!("nesting too deep (limit {})", MAX_DEPTH),
                "split the expression or statement into smaller pieces",
            );
            return false;
        }
        true
    }
    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    // ----- lookahead helpers -----

    fn at_type_kw(&self) -> bool {
        matches!(self.peek(), Tok::Kw(k) if TypeSpec::is_type_kw(*k))
    }
    fn at_type_kw_at(&self, k: usize) -> bool {
        matches!(self.peek_at(k), Tok::Kw(kw) if TypeSpec::is_type_kw(*kw))
    }
    /// Does the cursor start a declaration/function (`Type name` / `Type *name` /
    /// `ClassName name` / `ClassName *name`)?
    fn starts_decl(&self) -> bool {
        if self.at_type_kw() {
            return true;
        }
        if let Tok::Ident(_) = self.peek() {
            if let Tok::Ident(_) = self.peek_at(1) {
                return true;
            }
            if matches!(self.peek_at(1), Tok::Star) && matches!(self.peek_at(2), Tok::Ident(_)) {
                return true;
            }
        }
        false
    }

    // ----- program / items -----

    pub fn parse_program(&mut self) -> Program {
        let mut items = Vec::new();
        loop {
            if self.at_eof() {
                break;
            }
            let before = self.i;
            items.push(self.parse_item());
            if self.i == before {
                // progress guard (no infinite loop even on weird token soup)
                self.err_here("P003", format!("cannot parse `{}` here — skipped", self.peek().name()));
                self.bump();
            }
        }
        Program { items }
    }

    fn parse_item(&mut self) -> Item {
        match self.peek().clone() {
            Tok::HashBang => {
                let s = self.parse_ownership_attr();
                Item::Stmt(s)
            }
            Tok::Hash => {
                let s = self.parse_directive_stmt();
                Item::Stmt(s)
            }
            Tok::Kw(Kw::Class) => Item::Class(self.parse_class()),
            _ if self.starts_decl() => {
                let start = self.cur_pos();
                let (ty, name) = self.parse_decl_head();
                if self.at(&Tok::LParen) {
                    self.parse_fn_rest(ty, name, start)
                } else {
                    let init = if self.eat(&Tok::Eq) {
                        Some(self.parse_expr())
                    } else {
                        None
                    };
                    self.expect(&Tok::Semi, "after declaration");
                    Item::Stmt(Stmt::Decl(ty, name, init, Span::new(start, self.cur_end())))
                }
            }
            _ => Item::Stmt(self.parse_stmt()),
        }
    }

    fn parse_ownership_attr(&mut self) -> Stmt {
        let start = self.cur_pos();
        self.bump(); // #!
        self.expect(&Tok::LBracket, "in attribute");
        let name = match self.bump().tok {
            Tok::Ident(s) => s,
            other => {
                self.err_here("P010", format!("expected attribute name, found `{}`", other.name()));
                String::new()
            }
        };
        self.expect(&Tok::LParen, "in attribute");
        let level = match self.bump().tok {
            Tok::Ident(s) => match s.as_str() {
                "off" => OwnershipLevel::Off,
                "loose" => OwnershipLevel::Loose,
                "strict" => OwnershipLevel::Strict,
                _ => {
                    self.err_here_sugg(
                        "P011",
                        format!("unknown ownership level `{}`", s),
                        "levels are: off | loose | strict",
                    );
                    OwnershipLevel::Strict
                }
            },
            other => {
                self.err_here("P011", format!("expected ownership level, found `{}`", other.name()));
                OwnershipLevel::Strict
            }
        };
        self.expect(&Tok::RParen, "in attribute");
        self.expect(&Tok::RBracket, "in attribute");
        if name != "ownership" && !name.is_empty() {
            self.err_at(start, "P012", format!("unknown attribute `{}!`", name));
        }
        Stmt::Ownership(level, Span::new(start, self.cur_end()))
    }

    fn parse_directive_stmt(&mut self) -> Stmt {
        let start = self.cur_pos();
        self.bump(); // #
        match self.peek().clone() {
            Tok::Ident(s) if s == "exe" => {
                self.bump();
                self.expect(&Tok::LBrace, "after #exe");
                let body = self.parse_block_body(start);
                Stmt::Exe(Box::new(Stmt::Block(body, Span::new(start, self.cur_end()))), Span::new(start, self.cur_end()))
            }
            other => {
                self.err_here_sugg(
                    "P013",
                    format!("unknown directive `#{}`", other.name()),
                    "the only v1 directive is `#exe { … }`",
                );
                self.synchronize_stmt();
                Stmt::Empty(Span::new(start, self.cur_end()))
            }
        }
    }

    fn parse_class(&mut self) -> ClassDef {
        let start = self.cur_pos();
        self.bump(); // class
        let name = match self.bump().tok {
            Tok::Ident(s) => s,
            other => {
                self.err_here("P020", format!("expected class name, found `{}`", other.name()));
                String::new()
            }
        };
        self.expect(&Tok::LBrace, "in class definition");
        let mut fields = Vec::new();
        loop {
            if self.at(&Tok::RBrace) || self.at_eof() {
                break;
            }
            let before = self.i;
            if self.starts_decl() && !self.at(&Tok::LParen) {
                let (ty, fname) = self.parse_decl_head();
                if self.eat(&Tok::Eq) {
                    self.err_here("P021", "field initializers are not supported in v1 (assign after declaration)");
                    let _ = self.parse_expr();
                }
                self.expect(&Tok::Semi, "after class field");
                fields.push((ty, fname));
            } else {
                self.err_here("P022", format!("expected field declaration in class body, found `{}`", self.peek().name()));
            }
            if self.i == before {
                self.synchronize_stmt();
                if self.i == before {
                    self.bump();
                }
            }
        }
        self.expect(&Tok::RBrace, "at end of class body");
        let _ = self.eat(&Tok::Semi); // optional trailing `;` after class
        ClassDef {
            name,
            fields,
            span: Span::new(start, self.cur_end()),
        }
    }

    /// Parses `Type [*] name` — shared by declarations, params, fn definitions.
    fn parse_decl_head(&mut self) -> (TypeSpec, String) {
        let mut ty = self.parse_typespec();
        while self.at(&Tok::Star) {
            let p = self.cur_pos();
            self.bump();
            ty = TypeSpec::Ptr(Box::new(ty));
            if matches!(ty, TypeSpec::Ptr(ref inner) if matches!(**inner, TypeSpec::Ptr(_))) {
                self.err_at(p, "P030", "only single-level pointers (`T *p`) exist in v1");
            }
        }
        let name = match self.bump().tok {
            Tok::Ident(s) => s,
            other => {
                self.err_here("P031", format!("expected a name, found `{}`", other.name()));
                String::new()
            }
        };
        (ty, name)
    }

    fn parse_typespec(&mut self) -> TypeSpec {
        match self.peek().clone() {
            Tok::Kw(k) => {
                if let Some(ts) = TypeSpec::from_kw(k) {
                    self.bump();
                    return ts;
                }
            }
            Tok::Ident(s) => {
                self.bump();
                return TypeSpec::Named(s);
            }
            _ => {}
        }
        self.err_here("P032", format!("expected a type, found `{}`", self.peek().name()));
        TypeSpec::I64 // recovery default; the diagnostic already fired
    }

    fn parse_fn_rest(&mut self, ret: TypeSpec, name: String, start: Pos) -> Item {
        self.bump(); // (
        let mut params = Vec::new();
        let mut variadic = false;
        loop {
            if self.at(&Tok::RParen) || self.at_eof() {
                break;
            }
            if self.at(&Tok::DotDotDot) {
                let p = self.cur_pos();
                self.bump();
                variadic = true;
                if !self.at(&Tok::RParen) {
                    self.err_at(p, "P040", "`...` must be the last parameter");
                }
            } else if self.starts_decl() || self.at_type_kw() {
                let (ty, pname) = self.parse_decl_head();
                let default = if self.eat(&Tok::Eq) {
                    Some(self.parse_expr())
                } else {
                    None
                };
                params.push(Param { ty, name: pname, default });
            } else {
                self.err_here("P041", format!("expected parameter, found `{}`", self.peek().name()));
                self.bump();
            }
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RParen, "at end of parameter list");
        if self.at(&Tok::Semi) {
            self.err_here("P042", "function prototypes are not supported in v1 — provide a body");
            self.bump();
            return Item::Empty(Span::new(start, self.cur_end()));
        }
        self.expect(&Tok::LBrace, "before function body");
        let body = self.parse_block_body(start);
        Item::Fn(FnDef {
            ret,
            name,
            params,
            variadic,
            body,
            span: Span::new(start, self.cur_end()),
        })
    }

    // ----- statements -----

    /// Parses statements until matching `}` (consumed) or EOF.
    fn parse_block_body(&mut self, open: Pos) -> Vec<Stmt> {
        let mut out = Vec::new();
        loop {
            if self.at(&Tok::RBrace) {
                self.bump();
                break;
            }
            if self.at_eof() {
                self.err_at(open, "P050", "unclosed `{` — expected `}` before end of input");
                break;
            }
            let before = self.i;
            let s = self.parse_stmt();
            out.push(s);
            if self.i == before {
                self.err_here("P003", format!("cannot parse `{}` here — skipped", self.peek().name()));
                self.bump();
            }
        }
        out
    }

    fn parse_stmt(&mut self) -> Stmt {
        if !self.enter() {
            self.synchronize_stmt();
            return Stmt::Empty(Span::at(self.cur_pos()));
        }
        let s = self.parse_stmt_inner();
        self.leave();
        s
    }

    fn parse_stmt_inner(&mut self) -> Stmt {
        let start = self.cur_pos();
        match self.peek().clone() {
            Tok::Semi => {
                self.bump();
                Stmt::Empty(Span::new(start, self.cur_end()))
            }
            Tok::LBrace => {
                self.bump();
                let body = self.parse_block_body(start);
                Stmt::Block(body, Span::new(start, self.cur_end()))
            }
            Tok::HashBang => self.parse_ownership_attr(),
            Tok::Hash => self.parse_directive_stmt(),
            Tok::Kw(Kw::If) => self.parse_if(start),
            Tok::Kw(Kw::While) => self.parse_while(start),
            Tok::Kw(Kw::Do) => self.parse_do_while(start),
            Tok::Kw(Kw::For) => self.parse_for(start),
            Tok::Kw(Kw::Switch) => self.parse_switch(start),
            Tok::Kw(Kw::Try) => self.parse_try(start),
            Tok::Kw(Kw::Throw) => self.parse_throw(start),
            Tok::Kw(Kw::Unsafe) => {
                self.bump();
                let inner = self.parse_stmt();
                Stmt::Unsafe(Box::new(inner), Span::new(start, self.cur_end()))
            }
            Tok::Kw(Kw::Return) => {
                self.bump();
                let v = if self.at(&Tok::Semi) {
                    None
                } else {
                    Some(self.parse_expr())
                };
                self.expect(&Tok::Semi, "after return");
                Stmt::Return(v, Span::new(start, self.cur_end()))
            }
            Tok::Kw(Kw::Break) => {
                self.bump();
                self.expect(&Tok::Semi, "after break");
                Stmt::Break(Span::new(start, self.cur_end()))
            }
            Tok::Kw(Kw::Continue) => {
                self.bump();
                self.expect(&Tok::Semi, "after continue");
                Stmt::Continue(Span::new(start, self.cur_end()))
            }
            Tok::Kw(Kw::Class) => {
                self.err_here("P060", "class definitions are only allowed at top level in v1");
                let c = self.parse_class();
                let _ = c;
                Stmt::Empty(Span::new(start, self.cur_end()))
            }
            _ if self.starts_decl() && !self.decl_looks_like_call() => {
                let (ty, name) = self.parse_decl_head();
                if self.at(&Tok::LParen) {
                    // function definition in statement position (e.g. inside a block)?
                    // v1: functions are top-level only — diagnose and skip.
                    self.err_here("P061", "function definitions are only allowed at top level in v1");
                    let it = self.parse_fn_rest(ty, name, start);
                    let _ = it;
                    return Stmt::Empty(Span::new(start, self.cur_end()));
                }
                let init = if self.eat(&Tok::Eq) {
                    Some(self.parse_expr())
                } else {
                    None
                };
                if !self.eat(&Tok::Semi) {
                    self.err_here_sugg(
                        "P062",
                        format!("expected `;` after declaration, found `{}`", self.peek().name()),
                        "end the declaration with `;`",
                    );
                    self.synchronize_stmt();
                }
                Stmt::Decl(ty, name, init, Span::new(start, self.cur_end()))
            }
            _ => {
                let e = self.parse_expr();
                if !self.eat(&Tok::Semi) {
                    if !self.at(&Tok::RBrace) && !self.at_eof() {
                        self.err_here_sugg(
                            "P063",
                            format!("expected `;` after statement, found `{}`", self.peek().name()),
                            "end the statement with `;`",
                        );
                        self.synchronize_stmt();
                    } else {
                        // be forgiving right before `}` / EOF (REPL ergonomics)
                        self.eat(&Tok::Semi);
                    }
                }
                Stmt::ExprStmt(e, Span::new(start, self.cur_end()))
            }
        }
    }

    /// `Ident (` could be a call-statement, not a declaration of a named-type.
    fn decl_looks_like_call(&self) -> bool {
        matches!(self.peek(), Tok::Ident(_)) && matches!(self.peek_at(1), Tok::LParen)
    }

    fn parse_paren_expr(&mut self, ctx: &str) -> Expr {
        self.expect(&Tok::LParen, ctx);
        let e = self.parse_expr();
        self.expect(&Tok::RParen, ctx);
        e
    }

    fn parse_if(&mut self, start: Pos) -> Stmt {
        self.bump(); // if
        let cond = self.parse_paren_expr("after `if`");
        let then = self.parse_stmt();
        let els = if self.eat(&Tok::Kw(Kw::Else)) {
            Some(Box::new(self.parse_stmt()))
        } else {
            None
        };
        Stmt::If(cond, Box::new(then), els, Span::new(start, self.cur_end()))
    }

    fn parse_while(&mut self, start: Pos) -> Stmt {
        self.bump();
        let cond = self.parse_paren_expr("after `while`");
        let body = self.parse_stmt();
        Stmt::While(cond, Box::new(body), Span::new(start, self.cur_end()))
    }

    fn parse_do_while(&mut self, start: Pos) -> Stmt {
        self.bump();
        let body = self.parse_stmt();
        self.expect(&Tok::Kw(Kw::While), "after do-body");
        let cond = self.parse_paren_expr("after `do … while`");
        let _ = self.eat(&Tok::Semi);
        Stmt::DoWhile(Box::new(body), cond, Span::new(start, self.cur_end()))
    }

    fn parse_for(&mut self, start: Pos) -> Stmt {
        self.bump();
        self.expect(&Tok::LParen, "after `for`");
        // init
        let init = if self.at(&Tok::Semi) {
            None
        } else if self.starts_decl() && !self.decl_looks_like_call() {
            let (ty, name) = self.parse_decl_head();
            let i2 = if self.eat(&Tok::Eq) {
                Some(self.parse_expr())
            } else {
                None
            };
            Some(Box::new(Stmt::Decl(ty, name, i2, Span::new(start, self.cur_end()))))
        } else {
            Some(Box::new(Stmt::ExprStmt(self.parse_expr(), Span::new(start, self.cur_end()))))
        };
        self.expect(&Tok::Semi, "after for-init");
        let cond = if self.at(&Tok::Semi) {
            None
        } else {
            Some(self.parse_expr())
        };
        self.expect(&Tok::Semi, "after for-condition");
        let step = if self.at(&Tok::RParen) {
            None
        } else {
            Some(self.parse_expr())
        };
        self.expect(&Tok::RParen, "at end of for-header");
        let body = self.parse_stmt();
        Stmt::For(init, cond, step, Box::new(body), Span::new(start, self.cur_end()))
    }

    fn parse_switch(&mut self, start: Pos) -> Stmt {
        self.bump();
        let scrut = self.parse_paren_expr("after `switch`");
        self.expect(&Tok::LBrace, "in switch");
        let mut arms = Vec::new();
        loop {
            if self.at(&Tok::RBrace) {
                self.bump();
                break;
            }
            if self.at_eof() {
                self.err_at(start, "P070", "unclosed switch — expected `}`");
                break;
            }
            let before = self.i;
            let astart = self.cur_pos();
            let vals = match self.peek().clone() {
                Tok::Kw(Kw::Case) => {
                    self.bump();
                    Some(self.parse_case_vals())
                }
                Tok::Kw(Kw::Default) => {
                    self.bump();
                    None
                }
                other => {
                    self.err_here_sugg(
                        "P071",
                        format!("expected `case` or `default` in switch body, found `{}`", other.name()),
                        "switch bodies contain only case/default arms in v1",
                    );
                    self.synchronize_stmt();
                    if self.i == before {
                        self.bump();
                    }
                    continue;
                }
            };
            self.expect(&Tok::Colon, "after case label");
            let mut body = Vec::new();
            loop {
                match self.peek() {
                    Tok::Kw(Kw::Case) | Tok::Kw(Kw::Default) | Tok::RBrace | Tok::Eof => break,
                    _ => {
                        let b4 = self.i;
                        body.push(self.parse_stmt());
                        if self.i == b4 {
                            self.bump();
                        }
                    }
                }
            }
            arms.push(CaseArm {
                vals,
                body,
                span: Span::new(astart, self.cur_end()),
            });
            if self.i == before {
                self.bump();
            }
        }
        Stmt::Switch(scrut, arms, Span::new(start, self.cur_end()))
    }

    fn parse_case_vals(&mut self) -> Vec<CaseVal> {
        let mut out = Vec::new();
        loop {
            let neg = self.eat(&Tok::Minus);
            let v = match self.bump().tok {
                Tok::IntLit(v) => v,
                Tok::CharLit(v) => v,
                other => {
                    self.err_here_sugg(
                        "P072",
                        format!("case label must be an integer/char literal, found `{}`", other.name()),
                        "v1 case labels are literals: `case 4:` or `case 4...8:`",
                    );
                    0
                }
            };
            let v = if neg { v.wrapping_neg() } else { v };
            if self.at(&Tok::DotDotDot) {
                self.bump();
                let neg2 = self.eat(&Tok::Minus);
                let hi = match self.bump().tok {
                    Tok::IntLit(v) => v,
                    Tok::CharLit(v) => v,
                    other => {
                        self.err_here("P073", format!("range end must be an integer/char literal, found `{}`", other.name()));
                        0
                    }
                };
                let hi = if neg2 { hi.wrapping_neg() } else { hi };
                out.push(CaseVal::Range(v, hi));
            } else {
                out.push(CaseVal::One(v));
            }
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        out
    }

    fn parse_try(&mut self, start: Pos) -> Stmt {
        self.bump();
        let body = self.parse_stmt();
        self.expect(&Tok::Kw(Kw::Catch), "after try-body");
        let handler = self.parse_stmt();
        Stmt::Try(Box::new(body), Box::new(handler), Span::new(start, self.cur_end()))
    }

    fn parse_throw(&mut self, start: Pos) -> Stmt {
        self.bump();
        let mut vals = vec![self.parse_expr()];
        if self.eat(&Tok::Comma) {
            vals.push(self.parse_expr());
            if self.at(&Tok::Comma) {
                self.err_here("P080", "throw takes at most two values: `throw code, msg;`");
            }
        }
        self.expect(&Tok::Semi, "after throw");
        Stmt::Throw(vals, Span::new(start, self.cur_end()))
    }

    // ----- expressions (precedence climbing) -----

    pub fn parse_expr(&mut self) -> Expr {
        if !self.enter() {
            return Expr::Error(Span::at(self.cur_pos()));
        }
        let e = self.parse_binary(1);
        self.leave();
        e
    }

    fn parse_binary(&mut self, level: u8) -> Expr {
        if level > 10 {
            return self.parse_unary();
        }
        let mut lhs = self.parse_binary(level + 1);
        loop {
            let op = match (level, self.peek()) {
                (1, Tok::PipePipe) => BinOp::LogOr,
                (2, Tok::AmpAmp) => BinOp::LogAnd,
                (3, Tok::Pipe) => BinOp::BitOr,
                (4, Tok::Caret) => BinOp::Xor,
                (5, Tok::Amp) => BinOp::BitAnd,
                (6, Tok::EqEq) => BinOp::Eq,
                (6, Tok::BangEq) => BinOp::Ne,
                (7, Tok::Lt) => BinOp::Lt,
                (7, Tok::LtEq) => BinOp::Le,
                (7, Tok::Gt) => BinOp::Gt,
                (7, Tok::GtEq) => BinOp::Ge,
                (8, Tok::ShiftL) => BinOp::Shl,
                (8, Tok::ShiftR) => BinOp::Shr,
                (9, Tok::Plus) => BinOp::Add,
                (9, Tok::Minus) => BinOp::Sub,
                (10, Tok::Star) => BinOp::Mul,
                (10, Tok::Slash) => BinOp::Div,
                (10, Tok::Percent) => BinOp::Mod,
                _ => break,
            };
            let start = lhs.span().pos;
            self.bump();
            let rhs = self.parse_binary(level + 1);
            let end = self.cur_end();
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs), Span::new(start, end));
        }
        // assignment (right-assoc), lowest precedence — handled at level-1 return path
        if level == 1 {
            lhs = self.parse_assign_tail(lhs);
        }
        return lhs;
    }

    fn parse_assign_tail(&mut self, lhs: Expr) -> Expr {
        let aop = match self.peek() {
            Tok::Eq => AssignOp::Set,
            Tok::PlusEq => AssignOp::Add,
            Tok::MinusEq => AssignOp::Sub,
            Tok::StarEq => AssignOp::Mul,
            Tok::SlashEq => AssignOp::Div,
            Tok::PercentEq => AssignOp::Mod,
            Tok::AmpEq => AssignOp::And,
            Tok::PipeEq => AssignOp::Or,
            Tok::CaretEq => AssignOp::Xor,
            Tok::ShiftLEq => AssignOp::Shl,
            Tok::ShiftREq => AssignOp::Shr,
            _ => return lhs,
        };
        let start = lhs.span().pos;
        self.bump();
        let rhs = self.parse_expr(); // right-assoc, full expr (allows a = b = c)
        Expr::Assign(Box::new(lhs), aop, Box::new(rhs), Span::new(start, self.cur_end()))
    }

    fn parse_unary(&mut self) -> Expr {
        let start = self.cur_pos();
        let u = match self.peek() {
            Tok::Minus => Some(UnOp::Neg),
            Tok::Bang => Some(UnOp::Not),
            Tok::Tilde => Some(UnOp::BitNot),
            Tok::Star => Some(UnOp::Deref),
            Tok::PlusPlus => Some(UnOp::PreInc),
            Tok::MinusMinus => Some(UnOp::PreDec),
            Tok::Plus => None, // unary plus: no-op, drop it
            Tok::Amp => {
                self.bump();
                if self.eat(&Tok::Kw(Kw::Mut)) {
                    let inner = self.parse_unary();
                    return Expr::Unary(UnOp::AddrOfMut, Box::new(inner), Span::new(start, self.cur_end()));
                }
                let inner = self.parse_unary();
                return Expr::Unary(UnOp::AddrOf, Box::new(inner), Span::new(start, self.cur_end()));
            }
            Tok::LParen if self.at_type_kw_at(1) => {
                // cast: (Type) unary
                self.bump();
                let ty = self.parse_typespec();
                self.expect(&Tok::RParen, "in cast");
                let inner = self.parse_unary();
                return Expr::Cast(ty, Box::new(inner), Span::new(start, self.cur_end()));
            }
            _ => None,
        };
        if matches!(self.peek(), Tok::Minus | Tok::Bang | Tok::Tilde | Tok::Star | Tok::PlusPlus | Tok::MinusMinus | Tok::Plus) {
            self.bump();
        }
        let prim = self.parse_primary();
        let inner = self.parse_postfix(prim);
        match u {
            Some(op) => Expr::Unary(op, Box::new(inner), Span::new(start, self.cur_end())),
            None => inner,
        }
    }

    fn parse_postfix(&mut self, mut e: Expr) -> Expr {
        loop {
            let start = e.span().pos;
            match self.peek() {
                Tok::LBracket => {
                    self.bump();
                    let idx = self.parse_expr();
                    self.expect(&Tok::RBracket, "in index expression");
                    e = Expr::Index(Box::new(e), Box::new(idx), Span::new(start, self.cur_end()));
                }
                Tok::Dot => {
                    self.bump();
                    let field = match self.bump().tok {
                        Tok::Ident(s) => s,
                        other => {
                            self.err_here("P090", format!("expected field name after `.`, found `{}`", other.name()));
                            String::new()
                        }
                    };
                    e = Expr::Member(Box::new(e), field, Span::new(start, self.cur_end()));
                }
                Tok::LParen => {
                    let name = match &e {
                        Expr::Ident(n, _) => n.clone(),
                        _ => {
                            self.err_here_sugg(
                                "P091",
                                "call target must be a function name in v1",
                                "first-class function values are out of scope (FUTURE_IDEAS F11)",
                            );
                            String::new()
                        }
                    };
                    self.bump(); // (
                    let args = self.parse_call_args();
                    e = Expr::Call(name, args, Span::new(start, self.cur_end()));
                }
                Tok::PlusPlus => {
                    self.bump();
                    e = Expr::Unary(UnOp::PostInc, Box::new(e), Span::new(start, self.cur_end()));
                }
                Tok::MinusMinus => {
                    self.bump();
                    e = Expr::Unary(UnOp::PostDec, Box::new(e), Span::new(start, self.cur_end()));
                }
                _ => break,
            }
        }
        e
    }

    fn parse_call_args(&mut self) -> Vec<Option<Expr>> {
        let mut args = Vec::new();
        if self.eat(&Tok::RParen) {
            return args;
        }
        loop {
            if self.at(&Tok::Comma) {
                // omitted default slot (L2): `Foo(,6)`
                self.bump();
                args.push(None);
                if self.at(&Tok::RParen) {
                    break;
                }
                continue;
            }
            if self.at(&Tok::RParen) {
                break;
            }
            args.push(Some(self.parse_expr()));
            if self.eat(&Tok::Comma) {
                continue;
            }
            break;
        }
        self.expect(&Tok::RParen, "at end of argument list");
        args
    }

    fn parse_primary(&mut self) -> Expr {
        let start = self.cur_pos();
        let t = self.bump();
        match t.tok {
            Tok::IntLit(v) => Expr::LitInt(v, Span::new(start, t.end_off)),
            Tok::FloatLit(v) => Expr::LitFloat(v, Span::new(start, t.end_off)),
            Tok::StrLit(s) => Expr::LitStr(s, Span::new(start, t.end_off)),
            Tok::CharLit(c) => Expr::LitChar(c, Span::new(start, t.end_off)),
            Tok::Ident(s) => Expr::Ident(s, Span::new(start, t.end_off)),
            Tok::LParen => {
                let inner = self.parse_expr();
                self.expect(&Tok::RParen, "in parenthesized expression");
                inner
            }
            other => {
                self.err_at(
                    start,
                    "P092",
                    format!("expected an expression, found `{}`", other.name()),
                );
                Expr::Error(Span::new(start, t.end_off))
            }
        }
    }
}

/// Full front-end entry: source text → (Program, diagnostics). Never panics on
/// user input; diagnostics carry positions.
pub fn parse_program(file: &str, src: &str) -> (Program, DiagList) {
    let (toks, mut diags) = tokenize(file, src);
    let mut p = Parser::new(file, toks);
    let prog = p.parse_program();
    diags.items.extend(p.diags.items);
    // keep diagnostics in source order for stable rendering
    diags.items.sort_by_key(|d| d.pos.offset);
    (prog, diags)
}

#[cfg(test)]
mod unit {
    use super::*;

    fn parse_ok(src: &str) -> Program {
        let (p, d) = parse_program("unit", src);
        let rendered = d.render(src);
        assert!(!d.has_errors(), "unexpected parse errors:\n{}", rendered);
        p
    }

    #[test]
    fn default_holes_parse() {
        let p = parse_ok("Foo(,6); Foo(); Bar(1,,3);");
        match &p.items[0] {
            Item::Stmt(Stmt::ExprStmt(Expr::Call(name, args, _), _)) => {
                assert_eq!(name, "Foo");
                assert_eq!(args.len(), 2);
                assert!(args[0].is_none());
                assert!(args[1].is_some());
            }
            other => panic!("expected call, got {:?}", other),
        }
        match &p.items[2] {
            Item::Stmt(Stmt::ExprStmt(Expr::Call(_, args, _), _)) => {
                assert!(args[0].is_some() && args[1].is_none() && args[2].is_some());
            }
            other => panic!("expected call, got {:?}", other),
        }
    }

    #[test]
    fn case_range_parses() {
        let p = parse_ok("switch (x) { case 4...8: y = 1; case -3, 'a': y = 2; default: y = 3; }");
        match &p.items[0] {
            Item::Stmt(Stmt::Switch(_, arms, _)) => {
                assert_eq!(arms.len(), 3);
                assert_eq!(arms[0].vals, Some(vec![CaseVal::Range(4, 8)]));
                assert_eq!(arms[1].vals, Some(vec![CaseVal::One(-3), CaseVal::One(97)]));
                assert_eq!(arms[2].vals, None);
            }
            _ => panic!("expected switch"),
        }
    }

    #[test]
    fn unary_vs_binary_ambiguity() {
        let p = parse_ok("a = &b; c = d & e; f = *g * h;");
        match &p.items[0] {
            Item::Stmt(Stmt::ExprStmt(Expr::Assign(_, _, rhs, _), _)) => {
                assert!(matches!(**rhs, Expr::Unary(UnOp::AddrOf, _, _)));
            }
            _ => panic!(),
        }
        match &p.items[1] {
            Item::Stmt(Stmt::ExprStmt(Expr::Assign(_, _, rhs, _), _)) => {
                assert!(matches!(**rhs, Expr::Binary(BinOp::BitAnd, _, _, _)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn cast_vs_paren() {
        let p = parse_ok("(I64)x + (y + 1);");
        match &p.items[0] {
            Item::Stmt(Stmt::ExprStmt(Expr::Binary(BinOp::Add, lhs, _, _), _)) => {
                assert!(matches!(**lhs, Expr::Cast(TypeSpec::I64, _, _)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn recovery_continues_after_error() {
        // two errors, but the good declaration after them still parses
        let (p, d) = parse_program("unit", "I64 x = ; @ I64 y = 2;");
        assert!(d.has_errors());
        let decls: Vec<_> = p
            .items
            .iter()
            .filter_map(|it| match it {
                Item::Stmt(Stmt::Decl(_, n, _, _)) => Some(n.as_str()),
                _ => None,
            })
            .collect();
        assert!(decls.contains(&"y"), "recovery lost the good decl: {:?}", decls);
        assert!(d.items.iter().all(|x| x.pos.line >= 1));
    }

    #[test]
    fn deep_nesting_is_diagnostic_not_crash() {
        let src = format!("I64 x = {}1{};", "(".repeat(500), ")".repeat(500));
        let (_, d) = parse_program("unit", &src);
        assert!(d.items.iter().any(|x| x.code == "P002"), "expected depth diagnostic");
    }

    #[test]
    fn fn_with_defaults_and_variadic() {
        let p = parse_ok("I64 Foo(I64 a = 1, I64 b = 2) { return a + b; }\nI64 Sum(I64 n, ...) { return argc + argv[0]; }");
        match &p.items[0] {
            Item::Fn(f) => {
                assert_eq!(f.name, "Foo");
                assert_eq!(f.params.len(), 2);
                assert!(f.params[0].default.is_some());
                assert!(!f.variadic);
            }
            _ => panic!(),
        }
        match &p.items[1] {
            Item::Fn(f) => {
                assert!(f.variadic);
                assert_eq!(f.params.len(), 1);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn exe_block_and_ownership_attr() {
        let p = parse_ok("#![ownership(loose)]\n#exe { Print(\"I64 q = 1;\"); }\n");
        assert!(matches!(&p.items[0], Item::Stmt(Stmt::Ownership(OwnershipLevel::Loose, _))));
        assert!(matches!(&p.items[1], Item::Stmt(Stmt::Exe(_, _))));
    }

    #[test]
    fn class_def() {
        let p = parse_ok("class Point { I64 x; U0 tag; Str s; };");
        match &p.items[0] {
            Item::Class(c) => {
                assert_eq!(c.name, "Point");
                assert_eq!(c.fields.len(), 3);
                assert_eq!(c.fields[1].0, TypeSpec::U0);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn borrow_syntax() {
        let p = parse_ok("I64 *q = &mut n; I64 *p = &n; *p = *q + 1;");
        match &p.items[0] {
            Item::Stmt(Stmt::Decl(TypeSpec::Ptr(inner), _, Some(init), _)) => {
                assert_eq!(**inner, TypeSpec::I64);
                assert!(matches!(init, Expr::Unary(UnOp::AddrOfMut, _, _)));
            }
            _ => panic!(),
        }
    }
}
