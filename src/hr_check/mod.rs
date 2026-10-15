//! Component C — `hr_check` (SPEC §2.2C): semantic validation + the
//! configurable-strictness layer. Output: a *typed* AST (this module's `T*`
//! types) plus diagnostics. Every rejection is a positioned Diagnostic —
//! the checker never panics on user input.

use alloc::{boxed::Box, format, string::{String, ToString}, vec, vec::Vec};
use core::mem::replace as mem_replace;

use crate::compat::Map;

use crate::diag::{DiagList, Diagnostic, Pos};
use crate::hr_parser::ast::*;
use crate::hr_runtime::{Session, T_F64, T_I64, T_OBJ, T_STR, T_U0, T_U64};

// ------------------------------------------------------------------ types

#[derive(Clone, Debug, PartialEq)]
pub enum Type {
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
    Class(u32),
    /// pointer/borrow; bool = exclusive (`&mut`) — D009
    Ptr(Box<Type>, bool),
}

impl Type {
    pub fn size(&self, classes: &[ClassInfo]) -> usize {
        match self {
            Type::U0 => 0,
            Type::U8 | Type::I8 => 1,
            Type::U16 | Type::I16 => 2,
            Type::U32 | Type::I32 => 4,
            Type::U64 | Type::I64 | Type::F64 | Type::Str => 8,
            Type::Ptr(..) => 8,
            Type::Class(id) => classes.get(*id as usize).map(|c| c.size).unwrap_or(8),
        }
    }
    /// stack-slot count (8-byte granularity); U0 → 0
    pub fn slots(&self, classes: &[ClassInfo]) -> u32 {
        ((self.size(classes) + 7) / 8) as u32
    }
    pub fn is_int(&self) -> bool {
        matches!(
            self,
            Type::U8 | Type::U16 | Type::U32 | Type::U64 | Type::I8 | Type::I16 | Type::I32 | Type::I64
        )
    }
    pub fn is_unsigned(&self) -> bool {
        matches!(self, Type::U8 | Type::U16 | Type::U32 | Type::U64)
    }
    pub fn is_signed_int(&self) -> bool {
        matches!(self, Type::I8 | Type::I16 | Type::I32 | Type::I64)
    }
    pub fn is_float(&self) -> bool {
        matches!(self, Type::F64)
    }
    pub fn is_num(&self) -> bool {
        self.is_int() || self.is_float()
    }
    pub fn is_scalar(&self) -> bool {
        self.is_num() || matches!(self, Type::U0)
    }
    /// Move type under L8 (D008): assignment moves & invalidates the source.
    pub fn is_movable(&self) -> bool {
        matches!(self, Type::Str | Type::Class(_))
    }
    /// C-style usual arithmetic conversions, simplified (LANG_SPEC §3):
    /// float wins; sub-64-bit ints promote to I64; 64-bit mixed → U64.
    pub fn promote_pair(a: &Type, b: &Type) -> Type {
        if a.is_float() || b.is_float() {
            return Type::F64;
        }
        if a.width_bits() < 64 && b.width_bits() < 64 {
            return Type::I64;
        }
        if a.is_unsigned() || b.is_unsigned() {
            Type::U64
        } else {
            Type::I64
        }
    }
    pub fn width_bits(&self) -> u32 {
        match self {
            Type::U8 | Type::I8 => 8,
            Type::U16 | Type::I16 => 16,
            Type::U32 | Type::I32 => 32,
            _ => 64,
        }
    }
    pub fn tag(&self) -> u8 {
        match self {
            Type::U0 => T_U0,
            Type::F64 => T_F64,
            Type::Str => T_STR,
            Type::Class(_) => T_OBJ,
            t if t.is_unsigned() || matches!(t, Type::Ptr(..)) => T_U64,
            _ => T_I64,
        }
    }
    pub fn name(&self) -> String {
        match self {
            Type::U0 => "U0".into(),
            Type::U8 => "U8".into(),
            Type::U16 => "U16".into(),
            Type::U32 => "U32".into(),
            Type::U64 => "U64".into(),
            Type::I8 => "I8".into(),
            Type::I16 => "I16".into(),
            Type::I32 => "I32".into(),
            Type::I64 => "I64".into(),
            Type::F64 => "F64".into(),
            Type::Str => "Str".into(),
            Type::Class(id) => format!("<class#{}>", id),
            Type::Ptr(t, m) => format!("{}{}*", if *m { "&mut " } else { "" }, t.name()),
        }
    }
}

#[derive(Clone, Debug)]
pub enum ConstDefault {
    Int(i64),
    Float(f64),
}

#[derive(Clone, Debug)]
pub struct FnSig {
    pub name: String,
    pub ret: Type,
    pub params: Vec<(Type, String, Option<ConstDefault>)>,
    pub variadic: bool,
    pub cell: u64,
    pub defined: bool,
    pub strict: OwnershipLevel,
}

#[derive(Clone, Debug)]
pub struct ClassInfo {
    pub name: String,
    pub fields: Vec<(Type, String)>,
    pub offsets: Vec<usize>, // field byte offsets (past the 32-byte header)
    pub size: usize,         // total incl. header, multiple of 8
}

#[derive(Clone, Debug)]
pub struct GlobalInfo {
    pub ty: Type,
    pub addr: u64,
    pub size: usize,
}

pub const HEADER_SIZE: usize = 32; // {state, type_id, borrow, len/pad} — D007/D022

// ------------------------------------------------------------------ typed AST

#[derive(Clone, Debug, PartialEq)]
pub enum DeclTarget {
    None,          // U0 declaration: no storage
    Local(u32),    // slot base
    Global(String),// name (address in session globals)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VarRes {
    Local(u32), // slot base (frame offset = rbp - 8*(slot+slots_needed))
    Param(u32), // arg index (frame offset depends on variadic; D028)
    Argc,
    Argv,
    CatchCode,
    CatchMsg,
}

#[derive(Clone, Debug)]
pub enum TExpr {
    Lit(i64, Type, Span),
    LitF(f64, Span),
    LitS(u64, Span), // interned block address
    Var(VarRes, Type, Span),
    Global(String, Type, Span),
    Unary(UnOp, Box<TExpr>, Type, Span),
    Binary(BinOp, Box<TExpr>, Box<TExpr>, Type, Span),
    Assign(Box<TExpr>, AssignOp, Box<TExpr>, Type, Span),
    Index(Box<TExpr>, Box<TExpr>, Type, Span), // base: Var(Argv) or Str
    Call(Box<CallInfo>, Span),
    /// base, byte offset, base_is_ptr, field type
    Member(Box<TExpr>, u32, bool, Type, Span),
    Cast(Type, Box<TExpr>, Span),
}

impl TExpr {
    pub fn ty(&self) -> Type {
        match self {
            TExpr::Lit(_, t, _) => t.clone(),
            TExpr::LitF(..) => Type::F64,
            TExpr::LitS(..) => Type::Str,
            TExpr::Var(_, t, _) => t.clone(),
            TExpr::Global(_, t, _) => t.clone(),
            TExpr::Unary(_, _, t, _) => t.clone(),
            TExpr::Binary(_, _, _, t, _) => t.clone(),
            TExpr::Assign(_, _, _, t, _) => t.clone(),
            TExpr::Index(_, _, t, _) => t.clone(),
            TExpr::Call(c, _) => c.ret.clone(),
            TExpr::Member(_, _, _, t, _) => t.clone(),
            TExpr::Cast(t, _, _) => t.clone(),
        }
    }
    pub fn span(&self) -> Span {
        match self {
            TExpr::Lit(_, _, s) => *s,
            TExpr::LitF(_, s) => *s,
            TExpr::LitS(_, s) => *s,
            TExpr::Var(_, _, s) => *s,
            TExpr::Global(_, _, s) => *s,
            TExpr::Unary(_, _, _, s) => *s,
            TExpr::Binary(_, _, _, _, s) => *s,
            TExpr::Assign(_, _, _, _, s) => *s,
            TExpr::Index(_, _, _, s) => *s,
            TExpr::Call(_, s) => *s,
            TExpr::Member(_, _, _, _, s) => *s,
            TExpr::Cast(_, _, s) => *s,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CallInfo {
    pub kind: CallKind,
    pub args: Vec<TExpr>,
    pub ret: Type,
    pub tags_addr: u64, // 0 when not Print
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BuiltinId {
    SimPush,  // (I64) -> I64
    SimKey,   // () -> I64
    HotPatch, // (Str) -> I64
    HeapCheck,// () -> I64
    // TempleOS-RS Phase 1 (T1.2): the filesystem primitives. The user-facing
    // layer (`Dir`, `FWrite`, `FRead`, `FDel`, `Cat`) is plain HolyRust in
    // /TOS/Files.HR, so NO filesystem policy lives here — only raw storage.
    FsWrite,  // (Str path, Str content) -> I64 (1 ok, 0 refused)
    FsRead,   // (Str path) -> Str ("" when absent)
    FsList,   // (I64 index) -> Str ("" past the end; index-stable order)
    FsDelete, // (Str path) -> I64 (1 removed, 0 absent)
    // Phase 1 gap discovered while writing /TOS/Files.HR: v1 exposed `Str[i]`
    // but no way to ask a Str for its length, and the bounds check correctly
    // rejects index == len, so guest code could not locate the end of a string
    // at all. `StrLen` is that missing primitive (TempleOS has one too).
    StrLen,   // (Str) -> I64
    // Second Phase 1 gap: a `Str` *variable* is consumed when passed to a
    // user-defined function (D008 move semantics), so HolyRust alone cannot
    // compare one Str against several literals — the second comparison reads a
    // moved value. A non-consuming equality primitive is therefore required,
    // exactly as `StrLen` was for measuring. Readings and builtins never move.
    StrEq,    // (Str, Str) -> I64 (1 when byte-identical)
    // TempleOS-RS Phase 2 (T2.2): display primitives. The *shapes* (Line, Rect,
    // Circle, TextOut) are HolyRust in /TOS/Graphics.HR, not builtins.
    GfxInit,  // () -> I64 (1 when a device was initialised)
    GfxClear, // (I64 color) -> I64
    Plot,     // (I64 x, I64 y, I64 color) -> I64
    GfxChar,  // (I64 x, I64 y, I64 color, I64 code) -> I64
    // TempleOS-RS Phase 3 (T3.x): timer and pointing device.
    Sleep,    // (I64 ms) -> I64
    Time,     // () -> I64 (ms since boot)
    Date,     // () -> I64 (YYYYMMDD, 0 when no calendar clock)
    MouseInit,// () -> I64
    MouseX,   // () -> I64
    MouseY,   // () -> I64
    // TempleOS-RS Phase 4 (T4.x): PC speaker and the oracle's entropy source.
    Beep,     // (I64 freq, I64 ms) -> I64
    SoundOff, // () -> I64
    Random,   // (I64 n) -> I64 in 0..n
    // TempleOS-RS Phase 5 (T5.x): the line editor over the filesystem. The
    // commands built on these (`Ed`, `EdShow`, `EdReplaceLine`) are HolyRust.
    EdOpen,   // (Str path) -> I64 line count, -1 when absent
    EdLines,  // () -> I64
    EdLine,   // (I64 i) -> Str
    EdSet,    // (I64 i, Str text) -> I64
    EdFind,   // (Str needle) -> I64 line index or -1
    EdSave,   // () -> I64
    EdClose,  // () -> I64
}

impl BuiltinId {
    /// (arg types, return type) — LANG_SPEC §14
    pub fn sig(self) -> (&'static [Type], Type) {
        use crate::hr_check::Type as T;
        // small static tables via lazily-constructed vecs would allocate;
        // match arms below are the single source of truth for check_call.
        match self {
            BuiltinId::SimPush | BuiltinId::SimKey | BuiltinId::HotPatch | BuiltinId::HeapCheck => {
                (&[], T::I64)
            }
            BuiltinId::FsWrite | BuiltinId::FsRead | BuiltinId::FsList | BuiltinId::FsDelete => {
                (&[], self.ret_ty())
            }
            BuiltinId::StrLen => (&[], T::I64),
            BuiltinId::StrEq => (&[], T::I64),
            BuiltinId::GfxInit
            | BuiltinId::GfxClear
            | BuiltinId::Plot
            | BuiltinId::GfxChar => (&[], T::I64),
            BuiltinId::Sleep
            | BuiltinId::Time
            | BuiltinId::Date
            | BuiltinId::MouseInit
            | BuiltinId::MouseX
            | BuiltinId::MouseY => (&[], T::I64),
            BuiltinId::Beep | BuiltinId::SoundOff | BuiltinId::Random => (&[], T::I64),
            BuiltinId::EdOpen
            | BuiltinId::EdLines
            | BuiltinId::EdLine
            | BuiltinId::EdSet
            | BuiltinId::EdFind
            | BuiltinId::EdSave
            | BuiltinId::EdClose => (&[], self.ret_ty()),
        }
    }
    pub fn arg_tys(self) -> Vec<Type> {
        match self {
            BuiltinId::SimPush => vec![Type::I64],
            BuiltinId::HotPatch => vec![Type::Str],
            BuiltinId::SimKey | BuiltinId::HeapCheck => vec![],
            BuiltinId::FsWrite => vec![Type::Str, Type::Str],
            BuiltinId::FsRead | BuiltinId::FsDelete => vec![Type::Str],
            BuiltinId::FsList => vec![Type::I64],
            BuiltinId::StrLen => vec![Type::Str],
            BuiltinId::StrEq => vec![Type::Str, Type::Str],
            BuiltinId::GfxInit => vec![],
            BuiltinId::GfxClear => vec![Type::I64],
            BuiltinId::Plot => vec![Type::I64, Type::I64, Type::I64],
            BuiltinId::GfxChar => vec![Type::I64, Type::I64, Type::I64, Type::I64],
            BuiltinId::Sleep => vec![Type::I64],
            BuiltinId::Time
            | BuiltinId::Date
            | BuiltinId::MouseInit
            | BuiltinId::MouseX
            | BuiltinId::MouseY => vec![],
            BuiltinId::Beep => vec![Type::I64, Type::I64],
            BuiltinId::SoundOff => vec![],
            BuiltinId::Random => vec![Type::I64],
            BuiltinId::EdOpen | BuiltinId::EdFind => vec![Type::Str],
            BuiltinId::EdLine => vec![Type::I64],
            BuiltinId::EdSet => vec![Type::I64, Type::Str],
            BuiltinId::EdLines | BuiltinId::EdSave | BuiltinId::EdClose => vec![],
        }
    }
    /// Declared result type. Scalars are already 0/1/I64; the FS read/list
    /// primitives hand back a fresh `Str` block, so the type must say so or
    /// the checker would reject `Str s = FsRead(p);`.
    pub fn ret_ty(self) -> Type {
        match self {
            BuiltinId::FsRead | BuiltinId::FsList => Type::Str,
            BuiltinId::EdLine => Type::Str,
            _ => Type::I64,
        }
    }
    pub fn from_name(name: &str) -> Option<BuiltinId> {
        Some(match name {
            "SimPush" => BuiltinId::SimPush,
            "SimKey" => BuiltinId::SimKey,
            "HotPatch" => BuiltinId::HotPatch,
            "HeapCheck" => BuiltinId::HeapCheck,
            "FsWrite" => BuiltinId::FsWrite,
            "FsRead" => BuiltinId::FsRead,
            "FsList" => BuiltinId::FsList,
            "FsDelete" => BuiltinId::FsDelete,
            "StrLen" => BuiltinId::StrLen,
            "StrEq" => BuiltinId::StrEq,
            "GfxInit" => BuiltinId::GfxInit,
            "GfxClear" => BuiltinId::GfxClear,
            "Plot" => BuiltinId::Plot,
            "GfxChar" => BuiltinId::GfxChar,
            "Sleep" => BuiltinId::Sleep,
            "Time" => BuiltinId::Time,
            "Date" => BuiltinId::Date,
            "MouseInit" => BuiltinId::MouseInit,
            "MouseX" => BuiltinId::MouseX,
            "MouseY" => BuiltinId::MouseY,
            "Beep" => BuiltinId::Beep,
            "SoundOff" => BuiltinId::SoundOff,
            "Random" => BuiltinId::Random,
            "EdOpen" => BuiltinId::EdOpen,
            "EdLines" => BuiltinId::EdLines,
            "EdLine" => BuiltinId::EdLine,
            "EdSet" => BuiltinId::EdSet,
            "EdFind" => BuiltinId::EdFind,
            "EdSave" => BuiltinId::EdSave,
            "EdClose" => BuiltinId::EdClose,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub enum CallKind {
    User(u32, bool), // fn idx, variadic
    Print,
    Builtin(BuiltinId),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    Discard,
    AutoPrint, // L5: bare Str statement
    SaveLast,  // top-level value → ctx.last_val for REPL echo
}

#[derive(Clone, Debug)]
pub struct TArm {
    pub vals: Option<Vec<CaseVal>>,
    pub body: Vec<TStmt>,
}

#[derive(Clone, Debug)]
pub enum TStmt {
    Decl(DeclTarget, Type, Option<Box<TExpr>>, Span),
    ExprStmt(TExpr, Effect, Span),
    Block(Vec<TStmt>, Span),
    If(Box<TExpr>, Box<TStmt>, Option<Box<TStmt>>, Span),
    While(Box<TExpr>, Box<TStmt>, Span),
    DoWhile(Box<TStmt>, Box<TExpr>, Span),
    For(Option<Box<TStmt>>, Option<Box<TExpr>>, Option<Box<TExpr>>, Box<TStmt>, Span),
    Switch(Box<TExpr>, Vec<TArm>, Span),
    Break(u32, Span),    // u32 = active try-buffers to pop first
    Continue(u32, Span),
    Return(Option<Box<TExpr>>, u32, Span),
    Try(Box<TStmt>, Box<TStmt>, Span),
    Throw(Option<Box<TExpr>>, Option<Box<TExpr>>, Span), // code, msg
    Unsafe(Box<TStmt>, Span),
    SetStrict(OwnershipLevel, Span),
    Empty(Span),
}

#[derive(Clone, Debug)]
pub struct TFn {
    pub idx: u32,
    pub name: String,
    pub ret: Type,
    pub frame: i32, // bytes, multiple of 16
    pub nslots: u32,
    pub variadic: bool,
    pub params: Vec<(Type, String)>,
    pub body: Vec<TStmt>,
    pub strict: OwnershipLevel,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TItem {
    Fn(TFn),
    Stmt(TStmt),
}

#[derive(Clone, Debug, Default)]
pub struct TypedUnit {
    pub file: String,
    pub items: Vec<TItem>,
}

// ------------------------------------------------------------------ checker

struct Binding {
    res: VarRes,
    ty: Type,
}

struct FnEnv {
    is_fn: bool,
    name: String,
    ret: Type,
    variadic: bool,
    scopes: Vec<Map<String, Binding>>,
    slots: Vec<Type>,
    try_depth: u32,
    unsafe_depth: u32,
    handler_depth: u32,
    loop_depth: u32,
    switch_depth: u32,
}

pub struct Checker<'s> {
    pub sess: &'s mut Session,
    pub diags: DiagList,
    file: String,
    strict: OwnershipLevel,
    env: FnEnv,
}

fn top_env() -> FnEnv {
    FnEnv {
        is_fn: false,
        name: "<top>".into(),
        ret: Type::U0,
        variadic: false,
        scopes: vec![Map::new()],
        slots: Vec::new(),
        try_depth: 0,
        unsafe_depth: 0,
        handler_depth: 0,
        loop_depth: 0,
        switch_depth: 0,
    }
}

/// Check one parsed unit against the session state. On error, `TypedUnit`
/// items may be partial — callers must check `diags.has_errors()` first.
pub fn check_unit<'s>(sess: &'s mut Session, file: &str, prog: &Program) -> (TypedUnit, DiagList) {
    let mut c = Checker {
        sess,
        diags: DiagList::default(),
        file: file.to_string(),
        strict: OwnershipLevel::Strict,
        env: top_env(),
    };
    let unit = c.run(prog);
    (unit, c.diags)
}

impl<'s> Checker<'s> {
    fn err(&mut self, pos: Pos, code: &str, msg: impl Into<String>) {
        self.diags
            .push(Diagnostic::error(&self.file, pos, code, msg));
    }
    fn err_sugg(&mut self, pos: Pos, code: &str, msg: impl Into<String>, sugg: impl Into<String>) {
        self.diags.push(
            Diagnostic::error(&self.file, pos, code, msg).with_suggestion(sugg),
        );
    }

    fn run(&mut self, prog: &Program) -> TypedUnit {
        // ---- pass 1: classes, then function signatures ----
        for it in &prog.items {
            if let Item::Class(c) = it {
                self.register_class(c);
            }
        }
        // signatures + file-level strictness attributes in source order (the
        // attribute in effect at a definition becomes that function's level)
        for it in &prog.items {
            match it {
                Item::Fn(f) => self.register_sig(f),
                Item::Stmt(Stmt::Ownership(l, _)) => self.strict = *l,
                _ => {}
            }
        }
        // ---- pass 2: bodies in source order ----
        let mut unit = TypedUnit {
            file: self.file.clone(),
            items: Vec::new(),
        };
        for it in &prog.items {
            match it {
                Item::Class(_) => {}
                Item::Fn(f) => {
                    if let Some(tf) = self.check_fn(f) {
                        unit.items.push(TItem::Fn(tf));
                    }
                }
                Item::Stmt(s) => {
                    let ts = self.check_top_stmt(s);
                    unit.items.push(TItem::Stmt(ts));
                }
                Item::Empty(_) => {}
            }
        }
        unit
    }

    fn register_class(&mut self, c: &ClassDef) {
        // resolve field types
        let mut fields = Vec::new();
        for (ts, name) in &c.fields {
            let ty = self.resolve_type(ts, c.span.pos);
            fields.push((ty, name.clone()));
        }
        // redefinition rule (D029)
        if let Some(id) = self.sess.class_by_name.get(&c.name).copied() {
            let old = &self.sess.classes[id as usize];
            if old.fields == fields {
                return; // identical redefinition: silently reuse (REPL re-runs)
            }
            self.err(
                c.span.pos,
                "C029",
                format!("class `{}` is redefined with a different layout", c.name),
            );
            return;
        }
        let mut offsets = Vec::new();
        let mut size = HEADER_SIZE;
        for (ty, _) in &fields {
            offsets.push(size);
            size += ty.size(&self.sess.classes);
            size = (size + 7) & !7;
        }
        let id = self.sess.classes.len() as u32;
        self.sess.class_by_name.insert(c.name.clone(), id);
        self.sess.classes.push(ClassInfo {
            name: c.name.clone(),
            fields,
            offsets,
            size,
        });
    }

    fn resolve_type(&mut self, ts: &TypeSpec, pos: Pos) -> Type {
        match ts {
            TypeSpec::U0 => Type::U0,
            TypeSpec::U8 => Type::U8,
            TypeSpec::U16 => Type::U16,
            TypeSpec::U32 => Type::U32,
            TypeSpec::U64 => Type::U64,
            TypeSpec::I8 => Type::I8,
            TypeSpec::I16 => Type::I16,
            TypeSpec::I32 => Type::I32,
            TypeSpec::I64 => Type::I64,
            TypeSpec::F64 => Type::F64,
            TypeSpec::Str => Type::Str,
            TypeSpec::Named(n) => match self.sess.class_by_name.get(n) {
                Some(id) => Type::Class(*id),
                None => {
                    self.err_sugg(
                        pos,
                        "C001",
                        format!("unknown type name `{}`", n),
                        "declare it with `class`, or use a builtin type (U0, U8..U64, I8..I64, F64, Str)",
                    );
                    Type::I64
                }
            },
            TypeSpec::Ptr(inner) => Type::Ptr(Box::new(self.resolve_type(inner, pos)), false),
        }
    }

    fn register_sig(&mut self, f: &FnDef) {
        let ret = self.resolve_type(&f.ret, f.span.pos);
        if matches!(ret, Type::Class(_)) {
            self.err(
                f.span.pos,
                "C016",
                format!("function `{}` returns a class value — not supported in v1 (D025)", f.name),
            );
        }
        let mut params = Vec::new();
        for p in &f.params {
            let ty = self.resolve_type(&p.ty, f.span.pos);
            let def = match &p.default {
                Some(Expr::LitInt(v, _)) => Some(ConstDefault::Int(*v)),
                Some(Expr::LitFloat(v, _)) => Some(ConstDefault::Float(*v)),
                Some(Expr::LitChar(v, _)) => Some(ConstDefault::Int(*v)),
                Some(other) => {
                    self.err(
                        other.span().pos,
                        "C011",
                        "default argument must be a literal constant in v1",
                    );
                    None
                }
                None => None,
            };
            params.push((ty, p.name.clone(), def));
        }
        let strict = self.strict;
        if let Some(idx) = self.sess.fn_by_name.get(&f.name).copied() {
            // REPL-style redefinition: same cell → callers see the new code (D015)
            let sig = &mut self.sess.fns[idx as usize];
            sig.ret = ret;
            sig.params = params;
            sig.variadic = f.variadic;
            sig.defined = true;
            sig.strict = strict;
        } else {
            let sig = FnSig {
                name: f.name.clone(),
                ret,
                params,
                variadic: f.variadic,
                cell: 0,
                defined: true,
                strict,
            };
            self.sess.register_fn(sig);
        }
    }

    // ---------------- function checking ----------------

    fn check_fn(&mut self, f: &FnDef) -> Option<TFn> {
        let idx = match self.sess.fn_by_name.get(&f.name).copied() {
            Some(i) => i,
            None => return None, // register_sig diagnosed already
        };
        let sig = self.sess.fns[idx as usize].clone();
        let old_env = mem_replace(&mut self.env, top_env());
        let old_strict = self.strict;
        self.strict = sig.strict;
        self.env.is_fn = true;
        self.env.name = f.name.clone();
        self.env.ret = sig.ret.clone();
        self.env.variadic = f.variadic;
        // bind params
        for (i, p) in sig.params.iter().enumerate() {
            self.env.scopes[0].insert(
                p.1.clone(),
                Binding {
                    res: VarRes::Param(i as u32),
                    ty: p.0.clone(),
                },
            );
        }
        let body = self.check_block_stmts(&f.body);
        if self.env.try_depth != 0 {
            // internal invariant, not user-triggered
            self.err(f.span.pos, "C099", "internal: unbalanced try depth");
        }
        let nslots = self.env.slots.len() as u32;
        let frame = (((nslots * 8) + 15) & !15) as i32;
        let tf = TFn {
            idx,
            name: f.name.clone(),
            ret: sig.ret.clone(),
            frame,
            nslots,
            variadic: f.variadic,
            params: sig.params.iter().map(|p| (p.0.clone(), p.1.clone())).collect(),
            body,
            strict: sig.strict,
            span: f.span,
        };
        self.env = old_env;
        self.strict = old_strict;
        Some(tf)
    }

    fn push_scope(&mut self) {
        self.env.scopes.push(Map::new());
    }
    fn pop_scope(&mut self) {
        if self.env.scopes.len() > 1 {
            self.env.scopes.pop();
        }
    }
    fn lookup(&self, name: &str) -> Option<&Binding> {
        for sc in self.env.scopes.iter().rev() {
            if let Some(b) = sc.get(name) {
                return Some(b);
            }
        }
        None
    }
    fn bind(&mut self, name: String, b: Binding) {
        if let Some(sc) = self.env.scopes.last_mut() {
            sc.insert(name, b);
        }
    }
    fn alloc_slot(&mut self, ty: Type) -> u32 {
        let n = ty.slots(&self.sess.classes).max(1);
        let base = self.env.slots.len() as u32;
        for _ in 0..n {
            self.env.slots.push(Type::I64);
        }
        base
    }

    fn check_top_stmt(&mut self, s: &Stmt) -> TStmt {
        match s {
            Stmt::Decl(ty, name, init, sp) => self.check_global_decl(ty, name, init.as_ref(), *sp),
            Stmt::Ownership(l, sp) => {
                self.strict = *l;
                TStmt::SetStrict(*l, *sp)
            }
            Stmt::Exe(_, sp) => {
                self.err(
                    sp.pos,
                    "C090",
                    "#exe blocks are only supported at top level in v1",
                );
                TStmt::Empty(*sp)
            }
            _ => {
                // top-level control statement: check in the top env
                let ts = self.check_stmt(s, true);
                ts
            }
        }
    }

    fn check_global_decl(&mut self, ts: &TypeSpec, name: &str, init: Option<&Expr>, sp: Span) -> TStmt {
        let ty = self.resolve_type(ts, sp.pos);
        if ty == Type::U0 {
            if init.is_some() {
                self.err(sp.pos, "C030", "U0 globals cannot have an initial value");
            }
            return TStmt::Decl(DeclTarget::None, Type::U0, None, sp);
        }
        if self.sess.globals.contains_key(name) {
            let g = match self.sess.globals.get(name) {
                Some(g) => g.clone(),
                None => return TStmt::Decl(DeclTarget::None, ty, None, sp),
            };
            if !storage_eq(&g.ty, &ty) {
                self.err(
                    sp.pos,
                    "C020",
                    format!(
                        "global `{}` already exists with type {} — cannot redeclare as {}",
                        name,
                        g.ty.name(),
                        ty.name()
                    ),
                );
                return TStmt::Empty(sp);
            }
            // same type: reuse slot, re-init (REPL re-run semantics, LANG_SPEC §13)
        } else {
            let size = ty.size(&self.sess.classes).max(8);
            let addr = self.sess.alloc_zeroed(size);
            self.sess.globals.insert(
                name.to_string(),
                GlobalInfo {
                    ty: ty.clone(),
                    addr,
                    size,
                },
            );
        }
        let mut ty = ty;
        let te = match init {
            Some(e) => {
                match self.check_expr(e, None) {
                    Some(mut ce) => {
                        if matches!(ty, Type::Ptr(_, false)) {
                            if let Type::Ptr(inner, true) = ce.ty() {
                                ty = Type::Ptr(inner, true);
                            }
                        }
                        ce = self.coerce(ce, &ty);
                        Some(ce)
                    }
                    None => None,
                }
            }
            None => None,
        };
        // record the inherited (post-init) type so later reads of the global
        // see `&mut`-ness (storage layout is identical either way)
        if let Some(g) = self.sess.globals.get_mut(name) {
            g.ty = ty.clone();
        }
        TStmt::Decl(
            DeclTarget::Global(name.to_string()),
            ty.clone(),
            te.map(Box::new),
            sp,
        )
    }

    fn check_block_stmts(&mut self, stmts: &[Stmt]) -> Vec<TStmt> {
        let mut out = Vec::new();
        for s in stmts {
            out.push(self.check_stmt(s, false));
        }
        out
    }

    fn check_stmt(&mut self, s: &Stmt, top_level: bool) -> TStmt {
        match s {
            Stmt::Decl(ty, name, init, sp) => {
                if !self.env.is_fn {
                    return self.check_global_decl(ty, name, init.as_ref(), *sp);
                }
                let t = self.resolve_type(ty, sp.pos);
                if self.lookup(name).is_some() {
                    self.err(
                        sp.pos,
                        "C019",
                        format!("`{}` is already declared in this scope (shadowing is not allowed in v1)", name),
                    );
                }
                if t == Type::U0 {
                    if init.is_some() {
                        self.err(sp.pos, "C030", "U0 values cannot be initialized");
                    }
                    return TStmt::Decl(DeclTarget::None, Type::U0, None, *sp);
                }
                let mut t = t;
                let te = match init {
                    Some(e) => match self.check_expr(e, None) {
                        Some(mut ce) => {
                            // `T *p = &mut x;` — declared pointer types carry no mut
                            // marker; inherit exclusivity from the initializer (§7)
                            if matches!(t, Type::Ptr(_, false)) {
                                if let Type::Ptr(inner, true) = ce.ty() {
                                    t = Type::Ptr(inner, true);
                                }
                            }
                            ce = self.coerce(ce, &t);
                            Some(ce)
                        }
                        None => None,
                    },
                    None => None,
                };
                let slot = self.alloc_slot(t.clone());
                self.bind(
                    name.clone(),
                    Binding {
                        res: VarRes::Local(slot),
                        ty: t.clone(),
                    },
                );
                TStmt::Decl(DeclTarget::Local(slot), t, te.map(Box::new), *sp)
            }
            Stmt::ExprStmt(e, sp) => {
                match self.check_expr(e, None) {
                    Some(te) => {
                        let ty = te.ty();
                        // L5 auto-print applies to bare string EXPRESSIONS, not to
                        // assignments that happen to yield a Str (those are silent;
                        // they still feed the REPL echo via SaveLast).
                        let is_assign = matches!(te, TExpr::Assign(..));
                        let eff = if ty == Type::Str && !is_assign {
                            Effect::AutoPrint
                        } else if top_level && ty != Type::U0 {
                            Effect::SaveLast
                        } else {
                            Effect::Discard
                        };
                        TStmt::ExprStmt(te, eff, *sp)
                    }
                    None => TStmt::Empty(*sp),
                }
            }
            Stmt::Block(stmts, sp) => {
                self.push_scope();
                let body = self.check_block_stmts(stmts);
                self.pop_scope();
                TStmt::Block(body, *sp)
            }
            Stmt::If(c, t, e, sp) => {
                let cond = self.check_cond(c);
                let then = self.check_stmt(t, false);
                let els = e.as_ref().map(|x| self.check_stmt(x, false));
                TStmt::If(Box::new(cond), Box::new(then), els.map(Box::new), *sp)
            }
            Stmt::While(c, b, sp) => {
                let cond = self.check_cond(c);
                self.env.loop_depth += 1;
                let body = self.check_stmt(b, false);
                self.env.loop_depth -= 1;
                TStmt::While(Box::new(cond), Box::new(body), *sp)
            }
            Stmt::DoWhile(b, c, sp) => {
                self.env.loop_depth += 1;
                let body = self.check_stmt(b, false);
                self.env.loop_depth -= 1;
                let cond = self.check_cond(c);
                TStmt::DoWhile(Box::new(body), Box::new(cond), *sp)
            }
            Stmt::For(i, c, st, b, sp) => {
                self.push_scope();
                let init = i.as_ref().map(|x| self.check_stmt(x, false));
                let cond = c.as_ref().map(|x| self.check_cond(x));
                let step = st.as_ref().and_then(|x| self.check_expr(x, None));
                self.env.loop_depth += 1;
                let body = self.check_stmt(b, false);
                self.env.loop_depth -= 1;
                self.pop_scope();
                TStmt::For(
                    init.map(Box::new),
                    cond.map(Box::new),
                    step.map(Box::new),
                    Box::new(body),
                    *sp,
                )
            }
            Stmt::Switch(scrut, arms, sp) => {
                let mut sc = match self.check_expr(scrut, None) {
                    Some(t) => t,
                    None => return TStmt::Empty(*sp),
                };
                if !sc.ty().is_int() {
                    self.err(scrut.span().pos, "C021", "switch scrutinee must be an integer");
                    sc = TExpr::Cast(Type::I64, Box::new(sc), *sp);
                } else if sc.ty().width_bits() < 64 || sc.ty().is_unsigned() {
                    sc = self.coerce(sc, &Type::I64);
                }
                self.env.switch_depth += 1;
                self.env.loop_depth += 1; // `break` targets the switch
                let mut tarms = Vec::new();
                let mut seen_default = false;
                for a in arms {
                    if a.vals.is_none() {
                        if seen_default {
                            self.err(a.span.pos, "C031", "duplicate `default` in switch");
                        }
                        seen_default = true;
                    }
                    self.push_scope();
                    let body = self.check_block_stmts(&a.body);
                    self.pop_scope();
                    tarms.push(TArm {
                        vals: a.vals.clone(),
                        body,
                    });
                }
                self.env.loop_depth -= 1;
                self.env.switch_depth -= 1;
                TStmt::Switch(Box::new(sc), tarms, *sp)
            }
            Stmt::Break(sp) => {
                if self.env.loop_depth == 0 {
                    self.err(sp.pos, "C022", "`break` outside of a loop or switch");
                }
                TStmt::Break(self.env.try_depth, *sp)
            }
            Stmt::Continue(sp) => {
                if self.env.loop_depth == 0 || self.env.loop_depth == self.env.switch_depth {
                    self.err(sp.pos, "C023", "`continue` outside of a loop");
                }
                TStmt::Continue(self.env.try_depth, *sp)
            }
            Stmt::Return(v, sp) => {
                let ret_ty = self.env.ret.clone();
                let te = match (v, &ret_ty) {
                    (None, Type::U0) => None,
                    (None, t) => {
                        self.err(sp.pos, "C032", format!("function must return a value of type {}", t.name()));
                        None
                    }
                    (Some(_), Type::U0) => {
                        self.err(sp.pos, "C033", "U0 function must not return a value");
                        None
                    }
                    (Some(e), t) => self.check_expr(e, Some(t)).map(|x| self.coerce(x, t)),
                };
                if let Some(e) = v {
                    // D025: returning a borrow of a local is rejected statically
                    if matches!(e, Expr::Unary(UnOp::AddrOf | UnOp::AddrOfMut, inner, _)
                        if matches!(**inner, Expr::Ident(_, _)))
                    {
                        self.err(sp.pos, "C006", "cannot return a borrow of a local — it dies with the frame (D025)");
                    }
                }
                TStmt::Return(te.map(Box::new), self.env.try_depth, *sp)
            }
            Stmt::Try(b, h, sp) => {
                self.env.try_depth += 1;
                let body = self.check_stmt(b, false);
                self.env.handler_depth += 1;
                self.push_scope();
                let handler = self.check_stmt(h, false);
                self.pop_scope();
                self.env.handler_depth -= 1;
                self.env.try_depth -= 1;
                TStmt::Try(Box::new(body), Box::new(handler), *sp)
            }
            Stmt::Throw(vals, sp) => {
                if vals.len() == 1 {
                    let te = match self.check_expr(&vals[0], None) {
                        Some(t) => t,
                        None => return TStmt::Empty(*sp),
                    };
                    match te.ty() {
                        Type::Str => TStmt::Throw(None, Some(Box::new(te)), *sp),
                        t if t.is_int() => {
                            TStmt::Throw(Some(Box::new(self.coerce(te, &Type::I64))), None, *sp)
                        }
                        _ => {
                            self.err(sp.pos, "C024", "throw value must be an integer code or a Str message");
                            TStmt::Empty(*sp)
                        }
                    }
                } else {
                    let a = self.check_expr(&vals[0], Some(&Type::I64));
                    let b = self.check_expr(&vals[1], Some(&Type::Str));
                    match (a, b) {
                        (Some(a), Some(b)) => TStmt::Throw(
                            Some(Box::new(self.coerce(a, &Type::I64))),
                            Some(Box::new(self.coerce(b, &Type::Str))),
                            *sp,
                        ),
                        _ => TStmt::Empty(*sp),
                    }
                }
            }
            Stmt::Unsafe(inner, sp) => {
                self.env.unsafe_depth += 1;
                let t = self.check_stmt(inner, false);
                self.env.unsafe_depth -= 1;
                TStmt::Unsafe(Box::new(t), *sp)
            }
            Stmt::Ownership(l, sp) => {
                self.strict = *l;
                TStmt::SetStrict(*l, *sp)
            }
            Stmt::Exe(_, sp) => {
                self.err(sp.pos, "C090", "#exe blocks are only supported at top level in v1");
                TStmt::Empty(*sp)
            }
            Stmt::Empty(sp) => TStmt::Empty(*sp),
        }
    }

    fn check_cond(&mut self, c: &Expr) -> TExpr {
        match self.check_expr(c, None) {
            Some(te) => {
                let t = te.ty();
                if t.is_scalar() && t != Type::U0 || matches!(t, Type::Ptr(..)) {
                    te
                } else {
                    self.err(
                        c.span().pos,
                        "C025",
                        format!("condition must be a scalar or pointer, found {}", t.name()),
                    );
                    te
                }
            }
            None => TExpr::Lit(0, Type::I64, c.span()),
        }
    }

    // ---------------- expressions ----------------

    fn promote(a: &Type, b: &Type) -> Type {
        Type::promote_pair(a, b)
    }
    fn coerce(&mut self, te: TExpr, target: &Type) -> TExpr {
        let src = te.ty();
        let tsp = te.span();
        if &src == target {
            return te;
        }
        // literal range check for constants
        if let TExpr::Lit(v, _, sp) = &te {
            if target.is_int() {
                let w = target.width_bits();
                let fits = if target.is_unsigned() {
                    *v >= 0 && (w >= 64 || (*v as u64) < (1u64 << w))
                } else {
                    w >= 64 || (*v >= -(1i64 << (w - 1)) && *v < (1i64 << (w - 1)))
                };
                if !fits {
                    self.err(
                        sp.pos,
                        "C002",
                        format!("literal {} does not fit in {}", v, target.name()),
                    );
                }
            }
        }
        if src.is_num() && target.is_num() {
            return TExpr::Cast(target.clone(), Box::new(te), tsp);
        }
        if src == Type::Str || target == &Type::Str || src.is_movable() || target.is_movable() {
            // allow identical movables (handled above); everything else is an error
            if src != *target {
                self.err(
                    te.span().pos,
                    "C026",
                    format!("cannot convert {} to {}", src.name(), target.name()),
                );
            }
            return te;
        }
        TExpr::Cast(target.clone(), Box::new(te), tsp)
    }

    fn check_expr(&mut self, e: &Expr, expect: Option<&Type>) -> Option<TExpr> {
        let te = match e {
            Expr::LitInt(v, s) => {
                let mut t = Type::I64;
                if *v < 0 || *v > i64::MAX {
                    // bit patterns above i64::MAX arrived as u64-as-i64
                    if (*v as u64) > i64::MAX as u64 {
                        t = Type::U64;
                    }
                }
                TExpr::Lit(*v, t, *s)
            }
            Expr::LitFloat(v, s) => TExpr::LitF(*v, *s),
            Expr::LitStr(str, s) => {
                let addr = self.sess.intern_str(str);
                TExpr::LitS(addr, *s)
            }
            Expr::LitChar(v, s) => TExpr::Lit(*v, Type::I64, *s),
            Expr::Ident(name, s) => self.check_ident(name, *s)?,
            Expr::Unary(op, inner, s) => self.check_unary(*op, inner, *s)?,
            Expr::Binary(op, a, b, s) => self.check_binary(*op, a, b, *s)?,
            Expr::Assign(l, aop, r, s) => self.check_assign(l, *aop, r, *s)?,
            Expr::Index(base, idx, s) => {
                let tb = self.check_expr(base, None)?;
                let ti = self.check_expr(idx, Some(&Type::I64))?;
                let ti = self.coerce(ti, &Type::I64);
                match &tb {
                    TExpr::Var(VarRes::Argv, _, _) => TExpr::Index(Box::new(tb), Box::new(ti), Type::I64, *s),
                    _ if tb.ty() == Type::Str => {
                        TExpr::Index(Box::new(tb), Box::new(ti), Type::U8, *s)
                    }
                    _ => {
                        self.err_sugg(
                            s.pos,
                            "C012",
                            format!("cannot index a value of type {}", tb.ty().name()),
                            "v1 indexing exists for `argv[i]` and `Str[i]` only (arrays: FUTURE_IDEAS F1)",
                        );
                        return None;
                    }
                }
            }
            Expr::Call(name, args, s) => self.check_call(name, args, *s)?,
            Expr::Member(base, field, s) => {
                let tb = self.check_expr(base, None)?;
                let bt = tb.ty();
                let (cid, is_ptr) = match &bt {
                    Type::Class(id) => (*id, false),
                    Type::Ptr(inner, _) => match &**inner {
                        Type::Class(id) => (*id, true),
                        other => {
                            self.err(
                                s.pos,
                                "C018",
                                format!("`{}` has no members (type {}*)", field, other.name()),
                            );
                            return None;
                        }
                    },
                    other => {
                        self.err(
                            s.pos,
                            "C018",
                            format!("type {} has no member `{}`", other.name(), field),
                        );
                        return None;
                    }
                };
                let cls = match self.sess.classes.get(cid as usize) {
                    Some(c) => c.clone(),
                    None => return None,
                };
                let fi = cls.fields.iter().position(|(_, n)| n == field);
                match fi {
                    Some(i) => {
                        let off = cls.offsets[i] as u32;
                        let fty = cls.fields[i].0.clone();
                        TExpr::Member(Box::new(tb), off, is_ptr, fty, *s)
                    }
                    None => {
                        let known: Vec<&str> = cls.fields.iter().map(|(_, n)| n.as_str()).collect();
                        self.err_sugg(
                            s.pos,
                            "C027",
                            format!("class `{}` has no field `{}`", cls.name, field),
                            format!("fields are: {:?}", known),
                        );
                        return None;
                    }
                }
            }
            Expr::Cast(ts, inner, s) => {
                let target = self.resolve_type(ts, s.pos);
                if target == Type::U0 || target.is_movable() || matches!(target, Type::Ptr(..)) {
                    self.err(s.pos, "C017", format!("cannot cast to {}", target.name()));
                    return None;
                }
                let ti = self.check_expr(inner, None)?;
                if !ti.ty().is_num() {
                    self.err(
                        s.pos,
                        "C017",
                        format!("cast operand must be numeric, found {}", ti.ty().name()),
                    );
                    return None;
                }
                TExpr::Cast(target, Box::new(ti), *s)
            }
            Expr::Error(s) => {
                let _ = s;
                return None;
            }
        };
        Some(match expect {
            Some(t) => self.coerce(te, t),
            None => te,
        })
    }

    fn check_ident(&mut self, name: &str, sp: Span) -> Option<TExpr> {
        if let Some(b) = self.lookup(name) {
            let (res, ty) = (b.res, b.ty.clone());
            return Some(TExpr::Var(res, ty, sp));
        }
        if name == "argc" || name == "argv" {
            if self.env.is_fn && self.env.variadic {
                return Some(if name == "argc" {
                    TExpr::Var(VarRes::Argc, Type::I64, sp)
                } else {
                    TExpr::Var(VarRes::Argv, Type::I64, sp)
                });
            }
            self.err_sugg(
                sp.pos,
                "C028",
                "`argc`/`argv` exist only inside variadic functions",
                "declare the function with `(...)` to use argc/argv",
            );
            return None;
        }
        if name == "catch_code" || name == "catch_msg" {
            if self.env.handler_depth > 0 {
                return Some(if name == "catch_code" {
                    TExpr::Var(VarRes::CatchCode, Type::I64, sp)
                } else {
                    TExpr::Var(VarRes::CatchMsg, Type::Str, sp)
                });
            }
            self.err(sp.pos, "C028", "`catch_code`/`catch_msg` exist only inside a catch block");
            return None;
        }
        if let Some(g) = self.sess.globals.get(name) {
            return Some(TExpr::Global(name.to_string(), g.ty.clone(), sp));
        }
        if self.sess.fn_by_name.contains_key(name) {
            self.err(
                sp.pos,
                "C003",
                format!("function `{}` used as a value — functions are not values in v1", name),
            );
            return None;
        }
        // suggestion: case-insensitive / prefix match against known names
        let mut sugg = None;
        for known in self
            .sess
            .globals
            .keys()
            .chain(self.sess.fn_by_name.keys())
            .chain(self.sess.class_by_name.keys())
        {
            if known.eq_ignore_ascii_case(name) {
                sugg = Some(format!("did you mean `{}`?", known));
                break;
            }
        }
        match sugg {
            Some(s) => self.err_sugg(sp.pos, "C004", format!("unknown identifier `{}`", name), s),
            None => self.err(sp.pos, "C004", format!("unknown identifier `{}`", name)),
        }
        None
    }

    fn check_place(&mut self, e: &Expr, for_write: bool) -> Option<TExpr> {
        let te = self.check_expr(e, None)?;
        match &te {
            TExpr::Var(VarRes::Local(_), _, _) => Some(te),
            TExpr::Var(VarRes::Param(_), _, _) => Some(te),
            TExpr::Global(..) => Some(te),
            TExpr::Member(..) => Some(te),
            TExpr::Unary(UnOp::Deref, _, _, _) => Some(te),
            _ => {
                self.err(
                    e.span().pos,
                    "C034",
                    if for_write {
                        "expression is not assignable"
                    } else {
                        "expression cannot be borrowed (not a place)"
                    },
                );
                None
            }
        }
    }

    fn check_unary(&mut self, op: UnOp, inner: &Expr, sp: Span) -> Option<TExpr> {
        match op {
            UnOp::Neg => {
                let t = self.check_expr(inner, None)?;
                if !t.ty().is_num() {
                    self.err(sp.pos, "C035", format!("cannot negate {}", t.ty().name()));
                    return None;
                }
                let ty = if t.ty().is_int() && t.ty().width_bits() < 64 {
                    Type::I64
                } else {
                    t.ty()
                };
                let t = self.coerce(t, &ty);
                Some(TExpr::Unary(UnOp::Neg, Box::new(t), ty, sp))
            }
            UnOp::Not => {
                let t = self.check_expr(inner, None)?;
                if !t.ty().is_num() && !matches!(t.ty(), Type::Ptr(..)) {
                    self.err(sp.pos, "C035", format!("cannot apply `!` to {}", t.ty().name()));
                    return None;
                }
                Some(TExpr::Unary(UnOp::Not, Box::new(t), Type::I64, sp))
            }
            UnOp::BitNot => {
                let t = self.check_expr(inner, None)?;
                if !t.ty().is_int() {
                    self.err(sp.pos, "C035", "`~` requires an integer operand");
                    return None;
                }
                let ty = if t.ty().width_bits() < 64 {
                    Type::I64
                } else {
                    t.ty()
                };
                let t = self.coerce(t, &ty);
                Some(TExpr::Unary(UnOp::BitNot, Box::new(t), ty, sp))
            }
            UnOp::Deref => {
                let t = self.check_expr(inner, None)?;
                match t.ty() {
                    Type::Ptr(inner_ty, _) => {
                        Some(TExpr::Unary(UnOp::Deref, Box::new(t), *inner_ty, sp))
                    }
                    other => {
                        self.err(
                            sp.pos,
                            "C036",
                            format!("cannot dereference {} (not a pointer)", other.name()),
                        );
                        None
                    }
                }
            }
            UnOp::AddrOf | UnOp::AddrOfMut => {
                let place = self.check_place(inner, false)?;
                let ty = place.ty();
                Some(TExpr::Unary(
                    op,
                    Box::new(place),
                    Type::Ptr(Box::new(ty), op == UnOp::AddrOfMut),
                    sp,
                ))
            }
            UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec => {
                let place = self.check_place(inner, true)?;
                let ty = place.ty();
                if !ty.is_int() && !ty.is_float() {
                    self.err(sp.pos, "C037", format!("cannot ++/-- a value of type {}", ty.name()));
                    return None;
                }
                Some(TExpr::Unary(op, Box::new(place), ty, sp))
            }
        }
    }

    fn check_binary(&mut self, op: BinOp, a: &Expr, b: &Expr, sp: Span) -> Option<TExpr> {
        use BinOp::*;
        let ta = self.check_expr(a, None)?;
        let tb = self.check_expr(b, None)?;
        let aty = ta.ty();
        let bty = tb.ty();
        match op {
            LogAnd | LogOr => {
                if !aty.is_num() && !matches!(aty, Type::Ptr(..)) || !bty.is_num() && !matches!(bty, Type::Ptr(..)) {
                    self.err(sp.pos, "C038", "`&&`/`||` require scalar or pointer operands");
                    return None;
                }
                Some(TExpr::Binary(op, Box::new(ta), Box::new(tb), Type::I64, sp))
            }
            Eq | Ne => {
                if aty.is_num() && bty.is_num() {
                    let c = Self::promote(&aty, &bty);
                    Some(TExpr::Binary(
                        op,
                        Box::new(self.coerce(ta, &c)),
                        Box::new(self.coerce(tb, &c)),
                        Type::I64,
                        sp,
                    ))
                } else if matches!(aty, Type::Ptr(..)) && aty == bty {
                    Some(TExpr::Binary(op, Box::new(ta), Box::new(tb), Type::I64, sp))
                } else {
                    self.err_sugg(
                        sp.pos,
                        "C008",
                        format!("cannot compare {} with {}", aty.name(), bty.name()),
                        if aty == Type::Str || bty == Type::Str {
                            "Str equality is not supported in v1"
                        } else {
                            "comparison requires matching numeric or pointer types"
                        },
                    );
                    None
                }
            }
            Lt | Le | Gt | Ge => {
                if !aty.is_num() || !bty.is_num() {
                    self.err(sp.pos, "C038", format!("`{}` requires numeric operands", op_name(op)));
                    return None;
                }
                let c = Self::promote(&aty, &bty);
                Some(TExpr::Binary(
                    op,
                    Box::new(self.coerce(ta, &c)),
                    Box::new(self.coerce(tb, &c)),
                    Type::I64,
                    sp,
                ))
            }
            Add | Sub | Mul | Div | Mod => {
                if !aty.is_num() || !bty.is_num() {
                    self.err(
                        sp.pos,
                        "C009",
                        format!(
                            "arithmetic requires numeric operands ({} {} {})",
                            aty.name(),
                            op_name(op),
                            bty.name()
                        ),
                    );
                    return None;
                }
                let c = Self::promote(&aty, &bty);
                if op == Mod && c.is_float() {
                    self.err(sp.pos, "C007", "`%` is integer-only in v1");
                    return None;
                }
                Some(TExpr::Binary(
                    op,
                    Box::new(self.coerce(ta, &c)),
                    Box::new(self.coerce(tb, &c)),
                    c,
                    sp,
                ))
            }
            Shl | Shr => {
                if !aty.is_int() || !bty.is_int() {
                    self.err(sp.pos, "C039", "shifts require integer operands");
                    return None;
                }
                let lt = if aty.width_bits() < 64 { Type::I64 } else { aty };
                Some(TExpr::Binary(
                    op,
                    Box::new(self.coerce(ta, &lt)),
                    Box::new(self.coerce(tb, &Type::I64)),
                    lt,
                    sp,
                ))
            }
            BitAnd | BitOr | Xor => {
                if !aty.is_int() || !bty.is_int() {
                    self.err(sp.pos, "C039", "bit operators require integer operands");
                    return None;
                }
                let c = Self::promote(&aty, &bty);
                Some(TExpr::Binary(
                    op,
                    Box::new(self.coerce(ta, &c)),
                    Box::new(self.coerce(tb, &c)),
                    c,
                    sp,
                ))
            }
        }
    }

    fn check_assign(&mut self, l: &Expr, aop: AssignOp, r: &Expr, sp: Span) -> Option<TExpr> {
        let tl = self.check_place(l, true)?;
        let lty = tl.ty();
        if lty == Type::U0 {
            self.err(sp.pos, "C040", "cannot assign to a U0 value");
            return None;
        }
        let tr = match aop {
            AssignOp::Set => self.check_expr(r, Some(&lty))?,
            _ => {
                let t = self.check_expr(r, None)?;
                if !lty.is_num() || !t.ty().is_num() {
                    self.err(sp.pos, "C041", "compound assignment requires numeric operands");
                    return None;
                }
                if matches!(aop, AssignOp::Div | AssignOp::Mod) && (lty.is_float() || t.ty().is_float()) {
                    self.err(sp.pos, "C007", "`/=` and `%=` on floats are integer-only in v1");
                    return None;
                }
                let c = Self::promote(&lty, &t.ty());
                self.coerce(t, &c)
            }
        };
        Some(TExpr::Assign(Box::new(tl), aop, Box::new(tr), lty, sp))
    }

    fn check_call(&mut self, name: &str, args: &[Option<Expr>], sp: Span) -> Option<TExpr> {
        if let Some(bid) = BuiltinId::from_name(name) {
            let want = bid.arg_tys();
            if args.len() != want.len() {
                self.err(
                    sp.pos,
                    "C050",
                    format!("builtin `{}` takes {} argument(s), got {}", name, want.len(), args.len()),
                );
                return None;
            }
            let mut targs = Vec::new();
            for (a, wty) in args.iter().zip(want.iter()) {
                match a {
                    Some(e) => {
                        let te = self.check_expr(e, Some(wty))?;
                        targs.push(self.coerce(te, wty));
                    }
                    None => {
                        self.err(sp.pos, "C051", format!("builtin `{}` has no default arguments", name));
                        return None;
                    }
                }
            }
            return Some(TExpr::Call(
                Box::new(CallInfo {
                    kind: CallKind::Builtin(bid),
                    args: targs,
                    ret: bid.ret_ty(),
                    tags_addr: 0,
                }),
                sp,
            ));
        }
        if name == "Print" {
            let mut targs = Vec::new();
            let mut tags = Vec::new();
            for a in args {
                match a {
                    Some(e) => match self.check_expr(e, None) {
                        Some(te) => {
                            tags.push(te.ty().tag());
                            targs.push(te);
                        }
                        None => return None,
                    },
                    None => {
                        self.err(sp.pos, "C042", "Print has no default arguments — `,,` not allowed");
                        return None;
                    }
                }
            }
            let tags_addr = self.sess.make_tags(&tags);
            return Some(TExpr::Call(
                Box::new(CallInfo {
                    kind: CallKind::Print,
                    args: targs,
                    ret: Type::U0,
                    tags_addr,
                }),
                sp,
            ));
        }
        let idx = match self.sess.fn_by_name.get(name).copied() {
            Some(i) => i,
            None => {
                self.err(sp.pos, "C043", format!("call to unknown function `{}`", name));
                return None;
            }
        };
        let sig = self.sess.fns[idx as usize].clone();
        let nparams = sig.params.len();
        // resolve holes & arity
        if !sig.variadic && args.len() > nparams {
            self.err(
                sp.pos,
                "C014",
                format!(
                    "`{}` takes {} argument(s) but {} were given",
                    name,
                    nparams,
                    args.len()
                ),
            );
            return None;
        }
        if sig.variadic && args.len() < nparams {
            // missing named params: fill from defaults if possible below; else error later
        }
        let mut targs = Vec::new();
        let mut tags = Vec::new();
        for (i, a) in args.iter().enumerate() {
            if i < nparams {
                let (pty, _, def) = &sig.params[i];
                match a {
                    Some(e) => {
                        let te = self.check_expr(e, Some(pty))?;
                        targs.push(self.coerce(te, pty));
                    }
                    None => match def {
                        Some(ConstDefault::Int(v)) => targs.push(TExpr::Lit(*v, pty.clone(), sp)),
                        Some(ConstDefault::Float(v)) => targs.push(TExpr::LitF(*v, sp)),
                        None => {
                            self.err(
                                sp.pos,
                                "C013",
                                format!(
                                    "argument {} of `{}` is omitted but parameter `{}` has no default",
                                    i + 1,
                                    name,
                                    sig.params[i].1
                                ),
                            );
                            return None;
                        }
                    },
                }
            } else {
                // variadic tail
                match a {
                    Some(e) => {
                        let te = self.check_expr(e, None)?;
                        let t = te.ty();
                        if t == Type::U0 {
                            self.err(sp.pos, "C044", "cannot pass U0 to a variadic tail");
                            return None;
                        }
                        tags.push(t.tag());
                        targs.push(te);
                    }
                    None => {
                        self.err(sp.pos, "C042", "cannot omit variadic tail arguments");
                        return None;
                    }
                }
            }
        }
        // missing trailing params without given args
        for i in args.len()..nparams {
            let (pty, pname, def) = &sig.params[i];
            match def {
                Some(ConstDefault::Int(v)) => targs.push(TExpr::Lit(*v, pty.clone(), sp)),
                Some(ConstDefault::Float(v)) => targs.push(TExpr::LitF(*v, sp)),
                None => {
                    self.err(
                        sp.pos,
                        "C015",
                        format!("missing argument `{}` for `{}` (no default)", pname, name),
                    );
                    return None;
                }
            }
        }
        // tags for the Print-like uniform argv view: user fns don't need tags,
        // but variadic callees may pass argv to Print; tags only used at Print sites.
        let tags_addr = self.sess.make_tags(&tags);
        Some(TExpr::Call(
            Box::new(CallInfo {
                kind: CallKind::User(idx, sig.variadic),
                args: targs,
                ret: sig.ret.clone(),
                tags_addr,
            }),
            sp,
        ))
    }
}

/// Type equality for STORAGE purposes: pointer mutability flags do not
/// change layout, so redeclarations may differ only in that flag.
fn storage_eq(a: &Type, b: &Type) -> bool {
    match (a, b) {
        (Type::Ptr(x, _), Type::Ptr(y, _)) => storage_eq(x, y),
        _ => a == b,
    }
}

fn op_name(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::Xor => "^",
        BinOp::LogAnd => "&&",
        BinOp::LogOr => "||",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
    }
}
