//! AST types for `hr_parser` (SPEC §2.2B).
//! Stability rule for snapshots: all types derive Debug and contain NO pointers,
//! addresses, or hash-iteration order — `format!("{:#?}")` is the dump format.

use alloc::{boxed::Box, format, string::String, vec::Vec};

use crate::diag::Pos;
use crate::hr_lexer::Kw;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub pos: Pos,
    pub end: u32,
}

impl Span {
    pub fn new(pos: Pos, end: u32) -> Self {
        Span { pos, end }
    }
    pub fn at(pos: Pos) -> Self {
        Span { pos, end: pos.offset }
    }
}

/// Parsed type specifier. `Named` = class name (resolved by hr_check).
#[derive(Clone, Debug, PartialEq)]
pub enum TypeSpec {
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
    Named(String),
    Ptr(Box<TypeSpec>),
}

impl TypeSpec {
    pub fn from_kw(k: Kw) -> Option<TypeSpec> {
        Some(match k {
            Kw::U0 => TypeSpec::U0,
            Kw::U8 => TypeSpec::U8,
            Kw::U16 => TypeSpec::U16,
            Kw::U32 => TypeSpec::U32,
            Kw::U64 => TypeSpec::U64,
            Kw::I8 => TypeSpec::I8,
            Kw::I16 => TypeSpec::I16,
            Kw::I32 => TypeSpec::I32,
            Kw::I64 => TypeSpec::I64,
            Kw::F64 => TypeSpec::F64,
            Kw::Str => TypeSpec::Str,
            _ => return None,
        })
    }
    pub fn is_type_kw(k: Kw) -> bool {
        TypeSpec::from_kw(k).is_some()
    }
    pub fn name(&self) -> String {
        match self {
            TypeSpec::U0 => "U0".into(),
            TypeSpec::U8 => "U8".into(),
            TypeSpec::U16 => "U16".into(),
            TypeSpec::U32 => "U32".into(),
            TypeSpec::U64 => "U64".into(),
            TypeSpec::I8 => "I8".into(),
            TypeSpec::I16 => "I16".into(),
            TypeSpec::I32 => "I32".into(),
            TypeSpec::I64 => "I64".into(),
            TypeSpec::F64 => "F64".into(),
            TypeSpec::Str => "Str".into(),
            TypeSpec::Named(s) => s.clone(),
            TypeSpec::Ptr(t) => format!("{}*", t.name()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Shl,
    Shr,
    BitAnd,
    BitOr,
    Xor,
    LogAnd,
    LogOr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AssignOp {
    Set,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
    Deref,
    AddrOf,
    AddrOfMut,
    PreInc,
    PreDec,
    PostInc,
    PostDec,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    LitInt(i64, Span),
    LitFloat(f64, Span),
    LitStr(String, Span),
    LitChar(i64, Span),
    Ident(String, Span),
    Unary(UnOp, Box<Expr>, Span),
    Binary(BinOp, Box<Expr>, Box<Expr>, Span),
    Assign(Box<Expr>, AssignOp, Box<Expr>, Span),
    Index(Box<Expr>, Box<Expr>, Span),
    /// Call by name; `None` arg = omitted default slot (`Foo(,6)`, L2).
    Call(String, Vec<Option<Expr>>, Span),
    Member(Box<Expr>, String, Span),
    Cast(TypeSpec, Box<Expr>, Span),
    /// Parse-error placeholder — lets the tree stay well-formed for tooling.
    Error(Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::LitInt(_, s) => *s,
            Expr::LitFloat(_, s) => *s,
            Expr::LitStr(_, s) => *s,
            Expr::LitChar(_, s) => *s,
            Expr::Ident(_, s) => *s,
            Expr::Unary(_, _, s) => *s,
            Expr::Binary(_, _, _, s) => *s,
            Expr::Assign(_, _, _, s) => *s,
            Expr::Index(_, _, s) => *s,
            Expr::Call(_, _, s) => *s,
            Expr::Member(_, _, s) => *s,
            Expr::Cast(_, _, s) => *s,
            Expr::Error(s) => *s,
        }
    }
    pub fn is_error(&self) -> bool {
        matches!(self, Expr::Error(_))
    }
}

/// switch case label value (v1: integer/char literals only, optional unary minus).
#[derive(Clone, Debug, PartialEq)]
pub enum CaseVal {
    One(i64),
    Range(i64, i64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CaseArm {
    /// None = `default:`
    pub vals: Option<Vec<CaseVal>>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OwnershipLevel {
    Off,
    Loose,
    Strict,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Decl(TypeSpec, String, Option<Expr>, Span),
    ExprStmt(Expr, Span),
    Block(Vec<Stmt>, Span),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>, Span),
    While(Expr, Box<Stmt>, Span),
    DoWhile(Box<Stmt>, Expr, Span),
    For(Option<Box<Stmt>>, Option<Expr>, Option<Expr>, Box<Stmt>, Span),
    Switch(Expr, Vec<CaseArm>, Span),
    Break(Span),
    Continue(Span),
    Return(Option<Expr>, Span),
    Try(Box<Stmt>, Box<Stmt>, Span),
    /// `throw e;` or `throw code, msg;` (LANG_SPEC §9)
    Throw(Vec<Expr>, Span),
    Unsafe(Box<Stmt>, Span),
    /// `#![ownership(level)]` as a statement: applies to remainder of enclosing block (D010).
    Ownership(OwnershipLevel, Span),
    /// `#exe { … }` — compile-time execution block (L7); body spliced as source later.
    Exe(Box<Stmt>, Span),
    /// Error-recovery placeholder.
    Empty(Span),
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Decl(_, _, _, s) => *s,
            Stmt::ExprStmt(_, s) => *s,
            Stmt::Block(_, s) => *s,
            Stmt::If(_, _, _, s) => *s,
            Stmt::While(_, _, s) => *s,
            Stmt::DoWhile(_, _, s) => *s,
            Stmt::For(_, _, _, _, s) => *s,
            Stmt::Switch(_, _, s) => *s,
            Stmt::Break(s) => *s,
            Stmt::Continue(s) => *s,
            Stmt::Return(_, s) => *s,
            Stmt::Try(_, _, s) => *s,
            Stmt::Throw(_, s) => *s,
            Stmt::Unsafe(_, s) => *s,
            Stmt::Ownership(_, s) => *s,
            Stmt::Exe(_, s) => *s,
            Stmt::Empty(s) => *s,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub ty: TypeSpec,
    pub name: String,
    pub default: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FnDef {
    pub ret: TypeSpec,
    pub name: String,
    pub params: Vec<Param>,
    /// `...` present in parameter list (L3)
    pub variadic: bool,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClassDef {
    pub name: String,
    pub fields: Vec<(TypeSpec, String)>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Fn(FnDef),
    Class(ClassDef),
    Stmt(Stmt),
    Empty(Span),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Program {
    pub items: Vec<Item>,
}

/// Stable AST dump used by `--dump-ast` and snapshot tests.
pub fn dump_ast(p: &Program) -> String {
    format!("{:#?}\n", p)
}
