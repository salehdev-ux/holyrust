//! Component D — `hr_codegen` (SPEC §2.2D, backend per D003): compiles a
//! TypedUnit into an x86-64 machine-code blob, loaded via mmap by hr_runtime.
//!
//! Internal conventions (D006/D028):
//!  * expression results live in RAX; F64 travels as IEEE-754 bits; class-typed
//!    expressions evaluate to the ADDRESS of the value;
//!  * R15 = *Rt for the whole execution; RBP/RSP stable; RBX/RCX/RDX/RSI/RDI/
//!    R8/R9/R14/XMM0-1 are scratch — emit_* helpers may clobber them freely,
//!    only RAX crosses helper boundaries (documented exceptions below);
//!  * runtime calls: args in SysV regs, then a 16-byte alignment dance around
//!    `mov rax,[r15+off]; call rax`; check_read/check_write/borrow_* PRESERVE
//!    RAX; move_value/print/trap/throw clobber it;
//!  * calls between generated functions go through per-function indirection
//!    cells (`movabs r14, cell; call [r14]`) — the hot-patch point (D015);
//!  * args: pre-reserved block, arg_i at [rsp+8i] (callee: [rbp+16+8i]);
//!    variadic callees get hidden argc at [rsp+0] (callee: [rbp+16], args +24).

pub mod emitter;

use emitter::*;

use alloc::{format, string::{String, ToString}, vec::Vec};

use crate::diag::{DiagList, Diagnostic, Pos};
use crate::hr_check::*;
use crate::hr_parser::ast::{AssignOp, BinOp, CaseVal, OwnershipLevel, Span, UnOp};
use crate::hr_runtime::*;

pub struct CompiledUnit {
    pub entry: u64,
    pub code_len: usize,
}

struct LoopCtx {
    brk: u32,
    cont: u32,
    is_switch: bool,
}

/// A registered borrow awaiting its lexical release (LANG_SPEC §7).
struct BorrowRec {
    place: TExpr,   // the borrowed place (re-materialized at release time)
    excl: bool,
    is_str: bool,   // Str borrows key on the BLOCK ptr, others on the place addr
    block_scoped: bool, // true: lives until end of enclosing block; false: end of statement
}

// check_read kinds (contract with hr_runtime Phase-4 impls)
const KIND_STR_SLOT: u64 = 0; // addr = slot holding a Str ptr: trap if ptr == 0
const KIND_INLINE: u64 = 1; // addr = value with in-band header: trap if state == Moved

pub struct Compiler<'s> {
    sess: &'s mut Session,
    file: String,
    em: Emitter,
    diags: DiagList,
    frame: i32,
    variadic: bool,
    strict: OwnershipLevel,
    unsafe_depth: u32,
    loops: Vec<LoopCtx>,
    fn_offsets: Vec<(u32, usize)>,
    borrows: Vec<BorrowRec>,
    marks: Vec<(usize, bool)>, // (borrows index, is_loop) scope markers
    borrow_block: bool,        // true while emitting a Ptr-typed decl-init/assign-rhs
}

fn level_ge(l: OwnershipLevel, min: OwnershipLevel) -> bool {
    let v = |x: OwnershipLevel| match x {
        OwnershipLevel::Off => 0,
        OwnershipLevel::Loose => 1,
        OwnershipLevel::Strict => 2,
    };
    v(l) >= v(min)
}

fn is_place(te: &TExpr) -> bool {
    matches!(
        te,
        TExpr::Var(VarRes::Local(_), _, _)
            | TExpr::Var(VarRes::Param(_), _, _)
            | TExpr::Global(..)
            | TExpr::Member(..)
            | TExpr::Unary(UnOp::Deref, _, _, _)
    )
}

fn deref_is_mut(te: &TExpr) -> bool {
    match te {
        TExpr::Unary(UnOp::Deref, inner, _, _) => matches!(inner.ty(), Type::Ptr(_, true)),
        _ => false,
    }
}

impl<'s> Compiler<'s> {
    pub fn new(sess: &'s mut Session, file: &str) -> Self {
        Compiler {
            sess,
            file: file.to_string(),
            em: Emitter::new(),
            diags: DiagList::default(),
            frame: 0,
            variadic: false,
            strict: OwnershipLevel::Strict,
            unsafe_depth: 0,
            loops: Vec::new(),
            fn_offsets: Vec::new(),
            borrows: Vec::new(),
            marks: Vec::new(),
            borrow_block: false,
        }
    }

    // ---------------- borrow release machinery (Phase 4) ----------------

    /// Emit `borrow_release` for one record: recompute the key WITHOUT checks
    /// (cleanup must never trap), then call the runtime hook.
    fn emit_release(&mut self, rec: &BorrowRec) {
        if rec.is_str {
            self.emit_place_addr(&rec.place); // slot address
            self.em.mov_r_mem(RAX, RAX, 0); // block ptr (raw; may be 0 → no-op)
        } else {
            self.emit_place_addr(&rec.place);
        }
        self.em.mov_rr(RDI, R15);
        self.em.mov_rr(RSI, RAX);
        self.em.mov_r_i64(RDX, if rec.excl { 1 } else { 0 });
        self.call_rt(OFF_BORROW_RELEASE);
    }

    /// Release (and drop) all records at or above `idx`.
    fn release_from(&mut self, idx: usize) {
        let idx = idx.min(self.borrows.len());
        let recs: Vec<BorrowRec> = self.borrows.drain(idx..).collect();
        for rec in recs.iter().rev() {
            self.emit_release(rec);
        }
    }

    fn release_all(&mut self) {
        let n = self.borrows.len();
        self.release_from(0);
        let _ = n;
    }

    /// End-of-statement cleanup: release statement-scoped borrows recorded
    /// since `idx` (block-scoped ones stay on the list).
    fn release_stmt_from(&mut self, idx: usize) {
        let mut i = self.borrows.len();
        while i > idx.min(self.borrows.len()) {
            i -= 1;
            if !self.borrows[i].block_scoped {
                let rec = self.borrows.remove(i);
                self.emit_release(&rec);
            }
        }
    }

    /// Release everything above the innermost LOOP marker (break/continue).
    fn release_to_loop(&mut self) {
        let midx = self
            .marks
            .iter()
            .rev()
            .find(|(_, is_loop)| *is_loop)
            .map(|(i, _)| *i);
        if let Some(i) = midx {
            self.release_from(i);
        }
    }

    fn push_mark(&mut self, is_loop: bool) {
        self.marks.push((self.borrows.len(), is_loop));
    }
    fn pop_mark_release(&mut self) {
        if let Some((idx, _)) = self.marks.pop() {
            self.release_from(idx);
        }
    }

    fn internal_error(&mut self, sp: Span, msg: impl Into<String>) {
        self.diags.push(
            Diagnostic::error(&self.file, sp.pos, "G001", msg)
                .with_suggestion("compiler-internal invariant violated — please report this input"),
        );
    }

    fn loc(&mut self, sp: Span) -> i64 {
        self.sess.loc_id(&self.file, sp.pos.line, sp.pos.col) as i64
    }

    // ---------------- runtime call helpers ----------------

    /// `mov rax,[r15+off]; call rax` with a 16-byte alignment dance.
    /// Clobbers RAX/R14; other arg regs pass through untouched.
    fn call_rt(&mut self, off: usize) {
        self.em.mov_rr(R14, RSP);
        self.em.and_r_i32(RSP, -16);
        self.em.mov_r_mem(RAX, R15, off as i32);
        self.em.call_r(RAX);
        self.em.mov_rr(RSP, R14);
    }

    fn emit_trap(&mut self, kind: u64, sp: Span) {
        let loc = self.loc(sp);
        self.em.mov_rr(RDI, R15);
        self.em.mov_r_i64(RSI, loc);
        self.em.mov_r_i64(RDX, kind as i64);
        self.call_rt(OFF_TRAP);
        self.em.ud2(); // unreachable: trap never returns
    }

    fn emit_stack_check(&mut self, sp: Span) {
        let ok = self.em.label();
        self.em.mov_r_mem(RAX, R15, OFF_STACK_LIMIT as i32);
        self.em.cmp_rr(RSP, RAX);
        self.em.jcc(CC::AE, ok);
        self.emit_trap(K_STACK_OVERFLOW, sp);
        self.em.place(ok);
    }

    fn checks_on(&self, min: OwnershipLevel) -> bool {
        self.unsafe_depth == 0 && level_ge(self.strict, min)
    }

    /// RAX-in = address to check; RAX preserved.
    fn emit_check_read(&mut self, kind: u64, sp: Span) {
        if !self.checks_on(OwnershipLevel::Loose) {
            return;
        }
        let loc = self.loc(sp);
        self.em.push(RAX);
        self.em.mov_rr(RDI, R15);
        self.em.mov_rr(RSI, RAX);
        self.em.mov_r_i64(RDX, kind as i64);
        self.em.mov_r_i64(RCX, loc);
        self.call_rt(OFF_CHECK_READ);
        self.em.pop(RAX);
    }

    /// RAX-in = key address; RAX preserved (still the key on return).
    fn emit_check_write(&mut self, mut_ok: bool, sp: Span) {
        if !self.checks_on(OwnershipLevel::Strict) {
            return;
        }
        let loc = self.loc(sp);
        self.em.push(RAX);
        self.em.mov_rr(RDI, R15);
        self.em.mov_rr(RSI, RAX);
        self.em.mov_r_i64(RDX, 0);
        self.em.mov_r_i64(RCX, if mut_ok { 1 } else { 0 });
        self.em.mov_r_i64(R8, loc);
        self.call_rt(OFF_CHECK_WRITE);
        self.em.pop(RAX);
    }

    /// Ownership level dispatch: loose/strict use the runtime move hook
    /// (invalidates the source); `off`/unsafe use a plain inline copy (the
    /// measurable overhead difference required by the Phase 4 exit criteria).
    fn emit_move_or_copy(&mut self, is_str: bool, size: usize, sp: Span) {
        if self.checks_on(OwnershipLevel::Loose) {
            self.emit_move_value(is_str, size, sp);
        } else {
            self.emit_copy_raw(is_str, size);
        }
    }

    /// rsi=dst, rdx=src: bitwise copy without any bookkeeping.
    fn emit_copy_raw(&mut self, is_str: bool, size: usize) {
        if is_str {
            self.em.mov_r_mem(RAX, RDX, 0);
            self.em.mov_mem_r(RSI, 0, RAX);
        } else {
            for i in 0..(size / 8) {
                self.em.mov_r_mem(RAX, RDX, (i * 8) as i32);
                self.em.mov_mem_r(RSI, (i * 8) as i32, RAX);
            }
        }
    }

    /// rsi=dst addr, rdx=src addr (caller sets; both may be clobbered after).
    fn emit_move_value(&mut self, is_str: bool, size: usize, sp: Span) {
        let loc = self.loc(sp);
        self.em.mov_rr(RDI, R15);
        self.em.mov_r_i64(RCX, size as i64);
        self.em.mov_r_i64(R8, if is_str { 1 } else { 0 });
        self.em.mov_r_i64(R9, loc);
        self.call_rt(OFF_MOVE);
    }

    /// RAX-in = address to borrow; RAX-out = the pointer value to use.
    fn emit_borrow(&mut self, excl: bool, sp: Span) {
        if !self.checks_on(OwnershipLevel::Strict) {
            return;
        }
        let loc = self.loc(sp);
        self.em.mov_rr(RDI, R15);
        self.em.mov_rr(RSI, RAX);
        self.em.mov_r_i64(RDX, loc);
        self.call_rt(if excl { OFF_BORROW_EXCL } else { OFF_BORROW_SHARED });
        // runtime returns the address in RAX
    }

    // ---------------- memory helpers ----------------

    fn emit_load(&mut self, ty: &Type, base: Reg, disp: i32) {
        match ty {
            Type::U0 => self.em.xor_rr(RAX, RAX),
            Type::U8 => self.em.load_sz(RAX, base, disp, 1, false),
            Type::I8 => self.em.load_sz(RAX, base, disp, 1, true),
            Type::U16 => self.em.load_sz(RAX, base, disp, 2, false),
            Type::I16 => self.em.load_sz(RAX, base, disp, 2, true),
            Type::U32 => self.em.load_sz(RAX, base, disp, 4, false),
            Type::I32 => self.em.load_sz(RAX, base, disp, 4, true),
            // 8-byte: scalars, F64 bits, Str ptr, Ptr, class-address slots
            _ => self.em.mov_r_mem(RAX, base, disp),
        }
    }

    fn emit_store(&mut self, ty: &Type, base: Reg, disp: i32) {
        match ty {
            Type::U0 => {}
            Type::U8 | Type::I8 => self.em.store_sz(base, disp, RAX, 1),
            Type::U16 | Type::I16 => self.em.store_sz(base, disp, RAX, 2),
            Type::U32 | Type::I32 => self.em.store_sz(base, disp, RAX, 4),
            _ => self.em.mov_mem_r(base, disp, RAX),
        }
    }

    fn local_disp(&self, slot: u32, k: u32) -> i32 {
        -((slot + k) as i32 * 8)
    }
    fn param_disp(&self, i: u32) -> i32 {
        if self.variadic {
            24 + 8 * i as i32
        } else {
            16 + 8 * i as i32
        }
    }
    fn slots_of(&self, ty: &Type) -> u32 {
        if *ty == Type::U0 {
            0
        } else {
            ty.slots(&self.sess.classes).max(1)
        }
    }

    // ---------------- unit driver ----------------

    pub fn compile_unit(&mut self, unit: &TypedUnit) -> Result<CompiledUnit, DiagList> {
        let mut top: Vec<&TStmt> = Vec::new();
        for it in &unit.items {
            match it {
                TItem::Fn(tf) => self.compile_fn(tf),
                TItem::Stmt(ts) => top.push(ts),
            }
        }
        if self.diags.has_errors() {
            return Err(core::mem::take(&mut self.diags));
        }
        let entry_off = self.emit_entry(&top);
        if self.diags.has_errors() {
            // entry emission may add internal errors — never load broken code
            return Err(core::mem::take(&mut self.diags));
        }
        let (code, abs) = match core::mem::replace(&mut self.em, Emitter::new()).finalize() {
            Ok(v) => v,
            Err(e) => {
                self.internal_error(Span::at(Pos::default()), e);
                return Err(core::mem::take(&mut self.diags));
            }
        };
        let base = match self.sess.load_code(&code, &abs) {
            Ok(b) => b,
            Err(e) => {
                self.internal_error(Span::at(Pos::default()), e);
                return Err(core::mem::take(&mut self.diags));
            }
        };
        for (idx, off) in &self.fn_offsets {
            self.sess.set_fn_code(*idx, base + *off as u64);
        }
        Ok(CompiledUnit {
            entry: base + entry_off as u64,
            code_len: code.len(),
        })
    }

    fn emit_entry(&mut self, top: &[&TStmt]) -> usize {
        let off = self.em.here();
        self.em.push(RBP);
        self.em.mov_rr(RBP, RSP);
        // Host ABI boundary: preserve ALL callee-saved registers the guest may
        // clobber (rbx = scratch, r14 = alignment dance, r15 = ctx; r12/r13 are
        // never touched but saved defensively — cheap insurance, one unit entry).
        self.em.push(R15);
        self.em.push(R14);
        self.em.push(R13);
        self.em.push(R12);
        self.em.push(RBX);
        self.em.mov_rr(R15, RDI);
        self.em.sub_rsp(16);
        // reset REPL echo state
        self.em.mov_r_i64(RAX, -1);
        self.em.mov_mem_r(R15, OFF_LAST_KIND as i32, RAX);
        let landing = self.em.label();
        self.em.mov_rr(RDI, R15);
        self.em.mov_rr(RSI, RSP);
        self.em.mov_rr(RDX, RBP);
        self.em.movabs_label(RCX, landing);
        self.call_rt(OFF_UNIT_ENTER);

        self.frame = 0;
        self.variadic = false;
        self.strict = OwnershipLevel::Strict;
        self.unsafe_depth = 0;
        self.loops.clear();
        self.borrows.clear();
        self.marks.clear();
        for ts in top {
            self.emit_stmt(ts);
        }
        self.release_all(); // unit-scope borrow release (globals etc.)
        self.em.mov_rr(RDI, R15);
        self.call_rt(OFF_UNIT_EXIT);
        self.em.xor_rr(RAX, RAX);
        let done = self.em.label();
        self.em.jmp_label(done);
        self.em.place(landing); // trap/throw-to-top lands here (rsp/rbp restored)
        self.em.mov_rr(RDI, R15);
        self.call_rt(OFF_UNIT_EXIT);
        self.em.xor_rr(RAX, RAX);
        self.em.place(done);
        self.em.add_rsp(16);
        self.em.pop(RBX);
        self.em.pop(R12);
        self.em.pop(R13);
        self.em.pop(R14);
        self.em.pop(R15);
        self.em.mov_rr(RSP, RBP);
        self.em.pop(RBP);
        self.em.ret();
        off
    }

    fn compile_fn(&mut self, tf: &TFn) {
        let off = self.em.here();
        self.fn_offsets.push((tf.idx, off));
        self.frame = tf.frame;
        self.variadic = tf.variadic;
        self.strict = tf.strict;
        self.unsafe_depth = 0;
        self.loops.clear();

        self.em.push(RBP);
        self.em.mov_rr(RBP, RSP);
        self.em.sub_rsp(tf.frame.max(16));
        self.emit_stack_check(tf.span);
        for ts in &tf.body {
            self.emit_stmt(ts);
        }
        self.em.xor_rr(RAX, RAX); // implicit `return 0;`
        self.em.leave();
        self.em.ret();
    }

    // ---------------- statements ----------------

    fn emit_stmt(&mut self, ts: &TStmt) {
        let n = self.borrows.len();
        self.emit_stmt_inner(ts);
        self.release_stmt_from(n);
    }

    fn emit_stmt_inner(&mut self, ts: &TStmt) {
        match ts {
            TStmt::Empty(_) => {}
            TStmt::SetStrict(l, _) => self.strict = *l,
            TStmt::Decl(target, ty, init, sp) => {
                let sp = *sp;
                let init = init.clone();
                let ty = ty.clone();
                let target = target.clone();
                self.emit_decl(&target, &ty, init.as_deref(), sp);
            }
            TStmt::ExprStmt(te, eff, _) => {
                let eff = *eff;
                let ty = te.ty();
                self.emit_expr(te);
                match eff {
                    Effect::Discard => {}
                    Effect::AutoPrint => self.emit_print_rax_str(),
                    Effect::SaveLast => {
                        self.em.mov_mem_r(R15, OFF_LAST_VAL as i32, RAX);
                        self.em.mov_r_i64(RAX, ty.tag() as i64);
                        self.em.mov_mem_r(R15, OFF_LAST_KIND as i32, RAX);
                    }
                }
            }
            TStmt::Block(stmts, _) => {
                let saved = self.strict;
                let stmts = stmts.clone();
                self.push_mark(false);
                for s in &stmts {
                    self.emit_stmt(s);
                }
                self.pop_mark_release(); // lexical borrow release (LANG_SPEC §7)
                self.strict = saved;
            }
            TStmt::If(c, t, e, _) => {
                let (c, t, e) = ((**c).clone(), (**t).clone(), e.as_ref().map(|x| (**x).clone()));
                let l_else = self.em.label();
                let l_end = self.em.label();
                let cty = c.ty();
                self.emit_expr(&c);
                self.emit_jump_if_false(&cty, l_else);
                self.emit_stmt(&t);
                if let Some(el) = e {
                    self.em.jmp_label(l_end);
                    self.em.place(l_else);
                    self.emit_stmt(&el);
                    self.em.place(l_end);
                } else {
                    self.em.place(l_else);
                    self.em.place(l_end);
                }
            }
            TStmt::While(c, b, _) => {
                let (c, b) = ((**c).clone(), (**b).clone());
                let cty = c.ty();
                let l_top = self.em.label();
                let l_end = self.em.label();
                self.em.place(l_top);
                let n = self.borrows.len();
                self.emit_expr(&c);
                self.release_stmt_from(n);
                self.emit_jump_if_false(&cty, l_end);
                self.loops.push(LoopCtx { brk: l_end, cont: l_top, is_switch: false });
                self.push_mark(true);
                self.emit_stmt(&b);
                self.pop_mark_release();
                self.loops.pop();
                self.em.jmp_label(l_top);
                self.em.place(l_end);
            }
            TStmt::DoWhile(b, c, _) => {
                let (b, c) = ((**b).clone(), (**c).clone());
                let cty = c.ty();
                let l_top = self.em.label();
                let l_end = self.em.label();
                self.em.place(l_top);
                self.loops.push(LoopCtx { brk: l_end, cont: l_top, is_switch: false });
                self.push_mark(true);
                self.emit_stmt(&b);
                self.pop_mark_release();
                self.loops.pop();
                let n = self.borrows.len();
                self.emit_expr(&c);
                self.release_stmt_from(n);
                self.emit_jump_if_true(&cty, l_top);
                self.em.place(l_end);
            }
            TStmt::For(i, c, s, b, _) => {
                let (i, c, s, b) = (
                    i.as_ref().map(|x| (**x).clone()),
                    c.as_ref().map(|x| (**x).clone()),
                    s.as_ref().map(|x| (**x).clone()),
                    (**b).clone(),
                );
                if let Some(init) = &i {
                    self.emit_stmt(init);
                }
                let l_top = self.em.label();
                let l_cont = self.em.label();
                let l_end = self.em.label();
                self.em.place(l_top);
                if let Some(cond) = &c {
                    let cty = cond.ty();
                    let n = self.borrows.len();
                    self.emit_expr(cond);
                    self.release_stmt_from(n);
                    self.emit_jump_if_false(&cty, l_end);
                }
                self.loops.push(LoopCtx { brk: l_end, cont: l_cont, is_switch: false });
                self.push_mark(true);
                self.emit_stmt(&b);
                self.pop_mark_release();
                self.loops.pop();
                self.em.place(l_cont);
                if let Some(step) = &s {
                    let n = self.borrows.len();
                    self.emit_expr(step);
                    self.release_stmt_from(n);
                }
                self.em.jmp_label(l_top);
                self.em.place(l_end);
            }
            TStmt::Switch(scrut, arms, _) => {
                let (scrut, arms) = ((**scrut).clone(), arms.clone());
                let l_end = self.em.label();
                self.loops.push(LoopCtx { brk: l_end, cont: l_end, is_switch: true });
                self.emit_expr(&scrut);
                self.em.mov_rr(RBX, RAX); // keep scrutinee across arm tests
                let mut default_body: Option<Vec<TStmt>> = None;
                for arm in &arms {
                    match &arm.vals {
                        Some(vals) => {
                            let l_hit = self.em.label();
                            let l_next = self.em.label();
                            for v in vals {
                                match v {
                                    CaseVal::One(x) => {
                                        self.emit_case_cmp(*x);
                                        self.em.jcc(CC::E, l_hit);
                                    }
                                    CaseVal::Range(a, b) => {
                                        let l_skip = self.em.label();
                                        self.emit_case_cmp(*a);
                                        self.em.jcc(CC::L, l_skip);
                                        self.emit_case_cmp(*b);
                                        self.em.jcc(CC::LE, l_hit);
                                        self.em.place(l_skip);
                                    }
                                }
                            }
                            self.em.jmp_label(l_next);
                            self.em.place(l_hit);
                            self.push_mark(false);
                            for s in &arm.body {
                                self.emit_stmt(s);
                            }
                            self.pop_mark_release();
                            self.em.jmp_label(l_end); // no fallthrough (D019)
                            self.em.place(l_next);
                        }
                        None => default_body = Some(arm.body.clone()),
                    }
                }
                if let Some(body) = &default_body {
                    self.push_mark(false);
                    for s in body {
                        self.emit_stmt(s);
                    }
                    self.pop_mark_release();
                }
                self.loops.pop();
                self.em.place(l_end);
            }
            TStmt::Break(pops, sp) => {
                let pops = *pops;
                let sp = *sp;
                self.release_to_loop();
                self.emit_try_pops(pops);
                match self.loops.last().map(|x| x.brk) {
                    Some(l) => self.em.jmp_label(l),
                    None => self.internal_error(sp, "`break` outside loop reached codegen"),
                }
            }
            TStmt::Continue(pops, sp) => {
                let pops = *pops;
                let sp = *sp;
                self.release_to_loop();
                self.emit_try_pops(pops);
                let l = self.loops.iter().rev().find(|x| !x.is_switch).map(|x| x.cont);
                match l {
                    Some(l) => self.em.jmp_label(l),
                    None => self.internal_error(sp, "`continue` outside loop reached codegen"),
                }
            }
            TStmt::Return(v, pops, _) => {
                let (v, pops) = (v.clone(), *pops);
                match &v {
                    Some(te) => {
                        self.emit_expr(te);
                        self.em.push(RAX); // protect across release/trypop calls
                    }
                    None => self.em.xor_rr(RAX, RAX),
                }
                self.release_all();
                self.emit_try_pops(pops);
                if v.is_some() {
                    self.em.pop(RAX);
                }
                self.em.leave();
                self.em.ret();
            }
            TStmt::Try(body, handler, _) => {
                let (body, handler) = ((**body).clone(), (**handler).clone());
                let l_catch = self.em.label();
                let l_end = self.em.label();
                self.em.mov_rr(RDI, R15);
                self.em.mov_rr(RSI, RSP);
                self.em.mov_rr(RDX, RBP);
                self.em.movabs_label(RCX, l_catch);
                self.call_rt(OFF_TRY_ENTER);
                let b0 = self.borrows.len();
                self.emit_stmt(&body);
                self.em.mov_rr(RDI, R15);
                self.call_rt(OFF_TRY_POP);
                self.em.jmp_label(l_end);
                self.em.place(l_catch); // trampoline restored rsp/rbp of try entry
                // D033: borrows registered inside the try body may reference dead
                // frames — drop the records WITHOUT releasing (tracker entries leak
                // conservatively until unit_enter clears them).
                if self.borrows.len() > b0 {
                    self.borrows.truncate(b0);
                }
                self.emit_stmt(&handler);
                self.em.place(l_end);
            }
            TStmt::Throw(code, msg, sp) => {
                let (code, msg, sp) = (code.clone(), msg.clone(), *sp);
                match (&code, &msg) {
                    (Some(c), None) => {
                        self.emit_expr(c);
                        self.em.mov_rr(RSI, RAX);
                        self.em.xor_rr(RDX, RDX);
                    }
                    (None, Some(m)) => {
                        self.emit_expr(m);
                        self.em.mov_rr(RDX, RAX);
                        self.em.xor_rr(RSI, RSI);
                    }
                    (Some(c), Some(m)) => {
                        self.emit_expr(c);
                        self.em.push(RAX);
                        self.emit_expr(m);
                        self.em.mov_rr(RDX, RAX);
                        self.em.pop(RSI);
                    }
                    (None, None) => {
                        self.internal_error(sp, "throw without payload reached codegen");
                        self.em.xor_rr(RSI, RSI);
                        self.em.xor_rr(RDX, RDX);
                    }
                }
                self.em.mov_rr(RDI, R15);
                self.call_rt(OFF_THROW);
                self.em.ud2(); // throw never returns
            }
            TStmt::Unsafe(inner, _) => {
                let inner = (**inner).clone();
                self.unsafe_depth += 1;
                self.emit_stmt(&inner);
                self.unsafe_depth -= 1;
            }
        }
    }

    fn emit_case_cmp(&mut self, v: i64) {
        self.em.mov_rr(RAX, RBX); // reload scrutinee (bodies may have clobbered rax)
        if v >= i32::MIN as i64 && v <= i32::MAX as i64 {
            self.em.cmp_r_i32(RAX, v as i32);
        } else {
            self.em.movabs(RCX, v as u64);
            self.em.cmp_rr(RAX, RCX);
        }
    }

    fn emit_try_pops(&mut self, pops: u32) {
        for _ in 0..pops {
            self.em.mov_rr(RDI, R15);
            self.call_rt(OFF_TRY_POP);
        }
    }

    fn emit_print_rax_str(&mut self) {
        let tags = self.sess.make_tags(&[T_STR]);
        self.em.sub_rsp(16);
        self.em.mov_mem_r(RSP, 0, RAX);
        self.em.mov_rr(RDI, R15);
        self.em.mov_r_i64(RSI, 1);
        self.em.mov_rr(RDX, RSP);
        self.em.movabs(RCX, tags);
        self.call_rt(OFF_PRINT);
        self.em.add_rsp(16);
    }

    fn emit_jump_if_false(&mut self, ty: &Type, l: u32) {
        match ty {
            Type::F64 => {
                // jump iff ORDERED and == 0.0 (NaN counts truthy, like C)
                self.em.movq_rx(XMM0, RAX);
                self.em.pxor(XMM1, XMM1);
                self.em.ucomisd(XMM0, XMM1);
                let l_end = self.em.label();
                self.em.jcc(CC::P, l_end); // NaN → truthy → no jump
                self.em.jcc(CC::E, l); // ordered zero → jump
                self.em.place(l_end);
            }
            _ => {
                self.em.test_rr(RAX, RAX);
                self.em.jcc(CC::E, l);
            }
        }
    }

    fn emit_jump_if_true(&mut self, ty: &Type, l: u32) {
        match ty {
            Type::F64 => {
                self.em.movq_rx(XMM0, RAX);
                self.em.pxor(XMM1, XMM1);
                self.em.ucomisd(XMM0, XMM1);
                self.em.jcc(CC::P, l); // NaN → truthy → jump
                self.em.jcc(CC::NE, l); // ordered non-zero → jump
            }
            _ => {
                self.em.test_rr(RAX, RAX);
                self.em.jcc(CC::NE, l);
            }
        }
    }

    // ---------------- declarations ----------------

    fn emit_decl(&mut self, target: &DeclTarget, ty: &Type, init: Option<&TExpr>, sp: Span) {
        if *ty == Type::U0 {
            return; // zero-sized: no storage (D011)
        }
        match target {
            DeclTarget::Local(slot) => {
                let k = self.slots_of(ty);
                let disp = self.local_disp(*slot, k);
                match init {
                    Some(te) => {
                        let saved = self.borrow_block;
                        if matches!(ty, Type::Ptr(..)) {
                            self.borrow_block = true; // `T *p = &x;` → block-scoped borrow
                        }
                        self.emit_init_local(te, ty, disp, sp);
                        self.borrow_block = saved;
                    }
                    None => self.emit_zero(ty, RBP, disp),
                }
            }
            DeclTarget::Global(name) => {
                let addr = match self.sess.globals.get(name) {
                    Some(g) => g.addr,
                    None => {
                        self.internal_error(sp, format!("global `{}` missing at codegen", name));
                        return;
                    }
                };
                match init {
                    None => {
                        self.em.movabs(RBX, addr);
                        self.emit_zero(ty, RBX, 0);
                    }
                    Some(te) => {
                        let saved = self.borrow_block;
                        if matches!(ty, Type::Ptr(..)) {
                            self.borrow_block = true;
                        }
                        self.emit_init_global(te, ty, addr, sp);
                        self.borrow_block = saved;
                    }
                }
            }
            DeclTarget::None => {}
        }
    }

    fn emit_zero(&mut self, ty: &Type, base: Reg, disp: i32) {
        if let Type::Class(id) = ty {
            let size = self
                .sess
                .classes
                .get(*id as usize)
                .map(|c| c.size)
                .unwrap_or(HEADER_SIZE);
            self.em.xor_rr(RAX, RAX);
            for i in 0..(size / 8) {
                self.em.mov_mem_r(base, disp + (i as i32) * 8, RAX);
            }
            self.em.mov_r_i64(RAX, ST_OWNED as i64);
            self.em.mov_mem_r(base, disp, RAX);
            self.em.mov_r_i64(RAX, (CLASS_TYPE_BASE + *id as u64) as i64);
            self.em.mov_mem_r(base, disp + 8, RAX);
        } else {
            self.em.xor_rr(RAX, RAX);
            self.emit_store(ty, base, disp);
        }
    }

    /// Initialize a LOCAL place at [rbp+disp]. RBP is never clobbered by
    /// sub-emissions, so the destination stays valid throughout.
    fn emit_init_local(&mut self, te: &TExpr, ty: &Type, disp: i32, sp: Span) {
        match ty {
            Type::Str => {
                if let TExpr::LitS(addr, _) = te {
                    self.em.movabs(RAX, *addr);
                    self.em.mov_mem_r(RBP, disp, RAX);
                } else if is_place(te) {
                    self.emit_place_addr(te); // src slot addr → rax
                    self.em.mov_rr(RDX, RAX);
                    self.em.lea(RSI, RBP, disp); // dst slot addr
                    self.emit_move_or_copy(true, 8, sp);
                } else {
                    self.emit_expr(te); // rvalue pointer (call result): ownership passes
                    self.em.mov_mem_r(RBP, disp, RAX);
                }
            }
            Type::Class(_) => {
                let size = ty.size(&self.sess.classes);
                if is_place(te) {
                    self.emit_place_addr(te);
                    self.em.mov_rr(RDX, RAX);
                    self.em.lea(RSI, RBP, disp);
                    self.emit_move_or_copy(false, size, sp);
                } else {
                    self.internal_error(sp, "internal: non-place class initializer");
                }
            }
            _ => {
                self.emit_expr(te);
                self.emit_store(ty, RBP, disp);
            }
        }
    }

    /// Initialize a GLOBAL place at absolute `addr`. The address is kept on the
    /// machine stack across sub-emissions (RBX is scratch and must not hold it).
    fn emit_init_global(&mut self, te: &TExpr, ty: &Type, addr: u64, sp: Span) {
        match ty {
            Type::Str => {
                if let TExpr::LitS(lit, _) = te {
                    self.em.movabs(RAX, *lit);
                    self.em.movabs(RCX, addr);
                    self.em.mov_mem_r(RCX, 0, RAX);
                } else if is_place(te) {
                    self.em.movabs(RDX, addr);
                    self.em.push(RDX); // dst, survives sub-emission
                    self.emit_place_addr(te);
                    self.em.mov_rr(RDX, RAX); // src
                    self.em.mov_r_mem(RSI, RSP, 0); // dst
                    self.emit_move_or_copy(true, 8, sp);
                    self.em.add_rsp(8);
                } else {
                    self.em.movabs(RCX, addr);
                    self.em.push(RCX);
                    self.emit_expr(te);
                    self.em.pop(RCX);
                    self.em.mov_mem_r(RCX, 0, RAX);
                }
            }
            Type::Class(_) => {
                let size = ty.size(&self.sess.classes);
                if is_place(te) {
                    self.em.movabs(RDX, addr);
                    self.em.push(RDX);
                    self.emit_place_addr(te);
                    self.em.mov_rr(RDX, RAX);
                    self.em.mov_r_mem(RSI, RSP, 0);
                    self.emit_move_or_copy(false, size, sp);
                    self.em.add_rsp(8);
                } else {
                    self.internal_error(sp, "internal: non-place class initializer");
                }
            }
            _ => {
                self.em.movabs(RCX, addr);
                self.em.push(RCX);
                self.emit_expr(te);
                self.em.pop(RCX);
                self.emit_store(ty, RCX, 0);
            }
        }
    }

    // ---------------- expressions ----------------

    fn emit_expr(&mut self, te: &TExpr) {
        match te {
            TExpr::Lit(v, ty, _) => {
                if *ty == Type::U0 {
                    self.em.xor_rr(RAX, RAX);
                } else {
                    self.em.mov_r_i64(RAX, *v);
                }
            }
            TExpr::LitF(v, _) => self.em.mov_r_i64(RAX, v.to_bits() as i64),
            TExpr::LitS(addr, _) => self.em.movabs(RAX, *addr),
            TExpr::Var(res, ty, sp) => {
                let (res, ty, sp) = (*res, ty.clone(), *sp);
                self.emit_var(res, &ty, sp);
            }
            TExpr::Global(name, ty, sp) => {
                let (name, ty, sp) = (name.clone(), ty.clone(), *sp);
                let g = self.sess.globals.get(&name).cloned();
                match g {
                    Some(g) => {
                        if matches!(ty, Type::Class(_)) {
                            self.em.movabs(RAX, g.addr);
                            self.emit_check_read(KIND_INLINE, sp);
                        } else if ty == Type::Str {
                            self.em.movabs(RAX, g.addr);
                            self.emit_check_read(KIND_STR_SLOT, sp);
                            self.em.mov_r_mem(RAX, RAX, 0);
                        } else if ty == Type::U0 {
                            self.em.xor_rr(RAX, RAX);
                        } else {
                            self.em.movabs(RBX, g.addr);
                            self.emit_load(&ty, RBX, 0);
                        }
                    }
                    None => {
                        self.internal_error(sp, format!("global `{}` missing at codegen", name));
                        self.em.xor_rr(RAX, RAX);
                    }
                }
            }
            TExpr::Unary(op, inner, _, sp) => {
                let (op, inner, sp) = (*op, (**inner).clone(), *sp);
                self.emit_unary(op, &inner, sp);
            }
            TExpr::Binary(op, a, b, _, sp) => {
                let (op, a, b, sp) = (*op, (**a).clone(), (**b).clone(), *sp);
                self.emit_binary(op, &a, &b, sp);
            }
            TExpr::Assign(l, aop, r, _, sp) => {
                let (l, aop, r, sp) = ((**l).clone(), *aop, (**r).clone(), *sp);
                self.emit_assign(&l, aop, &r, sp);
            }
            TExpr::Index(base, idx, _, sp) => {
                let (base, idx, sp) = ((**base).clone(), (**idx).clone(), *sp);
                self.emit_index(&base, &idx, sp);
            }
            TExpr::Call(ci, sp) => {
                let (ci, sp) = ((**ci).clone(), *sp);
                self.emit_call(&ci, sp);
            }
            TExpr::Member(base, off, is_ptr, ty, sp) => {
                let (base, off, is_ptr, ty, sp) =
                    ((**base).clone(), *off, *is_ptr, ty.clone(), *sp);
                self.emit_class_addr(&base, is_ptr, sp);
                self.em.mov_rr(RBX, RAX); // class value address
                if matches!(ty, Type::Class(_)) {
                    self.em.lea(RAX, RBX, off as i32);
                    self.emit_check_read(KIND_INLINE, sp);
                } else if ty == Type::Str {
                    self.em.lea(RAX, RBX, off as i32); // slot address
                    self.emit_check_read(KIND_STR_SLOT, sp);
                    self.em.mov_r_mem(RAX, RAX, 0);
                } else {
                    self.emit_load(&ty, RBX, off as i32);
                }
            }
            TExpr::Cast(to, inner, _) => {
                let (to, inner) = (to.clone(), (**inner).clone());
                let from = inner.ty();
                self.emit_expr(&inner);
                self.emit_convert(&from, &to);
            }
        }
    }

    /// rax ← address of the class value denoted by te (Ptr or value form).
    fn emit_class_addr(&mut self, te: &TExpr, is_ptr: bool, sp: Span) {
        self.emit_expr(te);
        if is_ptr {
            // rax = pointer; the pointee carries the header
            self.emit_check_read(KIND_INLINE, sp);
        }
    }

    fn emit_var(&mut self, res: VarRes, ty: &Type, sp: Span) {
        match res {
            VarRes::Local(slot) => {
                let k = self.slots_of(ty);
                let disp = self.local_disp(slot, k);
                match ty {
                    Type::U0 => self.em.xor_rr(RAX, RAX),
                    Type::Class(_) => {
                        self.em.lea(RAX, RBP, disp);
                        self.emit_check_read(KIND_INLINE, sp);
                    }
                    Type::Str => {
                        self.em.lea(RAX, RBP, disp);
                        self.emit_check_read(KIND_STR_SLOT, sp);
                        self.em.mov_r_mem(RAX, RAX, 0);
                    }
                    _ => self.emit_load(ty, RBP, disp),
                }
            }
            VarRes::Param(i) => {
                let disp = self.param_disp(i);
                match ty {
                    Type::U0 => self.em.xor_rr(RAX, RAX),
                    Type::Class(_) => {
                        self.em.lea(RAX, RBP, disp);
                        self.emit_check_read(KIND_INLINE, sp);
                    }
                    Type::Str => {
                        self.em.lea(RAX, RBP, disp);
                        self.emit_check_read(KIND_STR_SLOT, sp);
                        self.em.mov_r_mem(RAX, RAX, 0);
                    }
                    _ => self.emit_load(ty, RBP, disp),
                }
            }
            VarRes::Argc => self.em.mov_r_mem(RAX, RBP, 16),
            VarRes::Argv => self.em.lea(RAX, RBP, 24),
            VarRes::CatchCode => self.em.mov_r_mem(RAX, R15, OFF_THROW_CODE as i32),
            VarRes::CatchMsg => {
                self.em.mov_r_mem(RAX, R15, OFF_THROW_MSG as i32);
                let l_ok = self.em.label();
                self.em.test_rr(RAX, RAX);
                self.em.jcc(CC::NE, l_ok);
                let empty = self.sess.intern_str("");
                self.em.movabs(RAX, empty);
                self.em.place(l_ok);
            }
        }
    }

    fn emit_unary(&mut self, op: UnOp, inner: &TExpr, sp: Span) {
        let ity = inner.ty();
        match op {
            UnOp::Neg => {
                self.emit_expr(inner);
                if ity.is_float() {
                    self.em.movq_rx(XMM0, RAX);
                    self.em.pxor(XMM1, XMM1);
                    self.em.subsd(XMM1, XMM0);
                    self.em.movq_xr(RAX, XMM1);
                } else {
                    self.em.neg_r(RAX);
                }
            }
            UnOp::Not => {
                self.emit_expr(inner);
                // rax = !(v): compute truthy flag then invert
                let l_true = self.em.label();
                let l_end = self.em.label();
                match &ity {
                    Type::F64 => {
                        self.em.movq_rx(XMM0, RAX);
                        self.em.pxor(XMM1, XMM1);
                        self.em.ucomisd(XMM0, XMM1);
                        self.em.jcc(CC::P, l_end); // NaN → truthy → !v = 0
                        self.em.jcc(CC::E, l_true); // zero → !v = 1
                        self.em.xor_rr(RAX, RAX);
                        self.em.jmp_label(l_end);
                    }
                    _ => {
                        self.em.test_rr(RAX, RAX);
                        self.em.jcc(CC::E, l_true);
                        self.em.xor_rr(RAX, RAX);
                        self.em.jmp_label(l_end);
                    }
                }
                self.em.place(l_true);
                self.em.mov_r_i64(RAX, 1);
                self.em.place(l_end);
            }
            UnOp::BitNot => {
                self.emit_expr(inner);
                self.em.not_r(RAX);
            }
            UnOp::Deref => {
                self.emit_expr(inner); // ptr value in rax
                match inner.ty() {
                    Type::Ptr(t, _) => match &*t {
                        Type::Str => {
                            self.emit_check_read(KIND_STR_SLOT, sp);
                            self.em.mov_r_mem(RAX, RAX, 0);
                        }
                        Type::Class(_) => {
                            self.emit_check_read(KIND_INLINE, sp);
                            // address already in rax
                        }
                        other => {
                            let o = other.clone();
                            self.emit_load(&o, RAX, 0);
                        }
                    },
                    _ => self.internal_error(sp, "deref of non-pointer reached codegen"),
                }
            }
            UnOp::AddrOf | UnOp::AddrOfMut => {
                let excl = op == UnOp::AddrOfMut;
                if ity == Type::Str {
                    // borrow the underlying block: slot value (checked) is the key
                    self.emit_expr(inner); // loads ptr with slot check
                    self.emit_borrow(excl, sp);
                } else {
                    self.emit_place_addr(inner);
                    self.emit_borrow(excl, sp);
                }
                if self.checks_on(OwnershipLevel::Strict) {
                    // borrow was registered — schedule its lexical release
                    self.borrows.push(BorrowRec {
                        block_scoped: self.borrow_block,
                        is_str: ity == Type::Str,
                        excl,
                        place: inner.clone(),
                    });
                }
            }
            UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec => {
                let pre = matches!(op, UnOp::PreInc | UnOp::PreDec);
                let inc = matches!(op, UnOp::PreInc | UnOp::PostInc);
                self.emit_place_addr(inner);
                self.em.mov_rr(RBX, RAX); // place addr
                self.emit_load_place(inner, RBX, 0); // old → rax
                if !pre {
                    self.em.push(RAX); // save old as the result
                }
                if ity.is_float() {
                    self.em.movq_rx(XMM0, RAX);
                    self.em.mov_r_i64(RAX, f64::to_bits(if inc { 1.0 } else { -1.0 }) as i64);
                    self.em.movq_rx(XMM1, RAX);
                    if inc {
                        self.em.addsd(XMM0, XMM1);
                    } else {
                        self.em.subsd(XMM0, XMM1);
                    }
                    self.em.movq_xr(RAX, XMM0);
                } else if inc {
                    self.em.add_r_i32(RAX, 1);
                } else {
                    self.em.sub_r_i32(RAX, 1);
                }
                self.emit_store_place(inner, RBX, 0); // value in rax
                if pre {
                    self.emit_load_place(inner, RBX, 0); // result = new
                } else {
                    self.em.pop(RAX); // result = old
                }
            }
        }
    }

    fn emit_binary(&mut self, op: BinOp, a: &TExpr, b: &TExpr, sp: Span) {
        use BinOp::*;
        if op == LogAnd {
            let l_false = self.em.label();
            let l_end = self.em.label();
            let aty = a.ty();
            let bty = b.ty();
            self.emit_expr(a);
            self.emit_jump_if_false(&aty, l_false);
            self.emit_expr(b);
            self.emit_jump_if_false(&bty, l_false);
            self.em.mov_r_i64(RAX, 1);
            self.em.jmp_label(l_end);
            self.em.place(l_false);
            self.em.xor_rr(RAX, RAX);
            self.em.place(l_end);
            return;
        }
        if op == LogOr {
            let l_true = self.em.label();
            let l_end = self.em.label();
            let aty = a.ty();
            let bty = b.ty();
            self.emit_expr(a);
            self.emit_jump_if_true(&aty, l_true);
            self.emit_expr(b);
            self.emit_jump_if_true(&bty, l_true);
            self.em.xor_rr(RAX, RAX);
            self.em.jmp_label(l_end);
            self.em.place(l_true);
            self.em.mov_r_i64(RAX, 1);
            self.em.place(l_end);
            return;
        }
        let aty = a.ty();
        let bty = b.ty();
        let float = aty.is_float() || bty.is_float();
        self.emit_expr(a);
        self.em.push(RAX);
        self.emit_expr(b);
        self.em.mov_rr(RCX, RAX); // rhs
        self.em.pop(RAX); // lhs
        if float {
            self.em.movq_rx(XMM0, RAX);
            self.em.movq_rx(XMM1, RCX);
            match op {
                Add => self.em.addsd(XMM0, XMM1),
                Sub => self.em.subsd(XMM0, XMM1),
                Mul => self.em.mulsd(XMM0, XMM1),
                Div => self.em.divsd(XMM0, XMM1),
                Eq | Ne | Lt | Le | Gt | Ge => {
                    self.em.ucomisd(XMM0, XMM1);
                    self.emit_float_cc(op);
                    return;
                }
                _ => {
                    self.internal_error(sp, "float operand for non-arithmetic op");
                    return;
                }
            }
            self.em.movq_xr(RAX, XMM0);
            return;
        }
        let unsigned = aty.is_unsigned();
        match op {
            Add => self.em.add_rr(RAX, RCX),
            Sub => self.em.sub_rr(RAX, RCX),
            Mul => self.em.imul_rr(RAX, RCX),
            BitAnd => self.em.and_rr(RAX, RCX),
            BitOr => self.em.or_rr(RAX, RCX),
            Xor => self.em.xor_rr(RAX, RCX),
            Div | Mod => {
                if self.checks_on(OwnershipLevel::Strict) {
                    let l_ok = self.em.label();
                    self.em.test_rr(RCX, RCX);
                    self.em.jcc(CC::NE, l_ok);
                    self.emit_trap(K_DIV_ZERO, sp);
                    self.em.place(l_ok);
                }
                if unsigned {
                    self.em.xor_rr(RDX, RDX);
                    self.em.div_r(RCX);
                } else {
                    self.em.cqo();
                    self.em.idiv_r(RCX);
                }
                if op == Mod {
                    self.em.mov_rr(RAX, RDX);
                }
            }
            Shl => self.em.shl_cl(RAX),
            Shr => {
                if unsigned {
                    self.em.shr_cl(RAX);
                } else {
                    self.em.sar_cl(RAX);
                }
            }
            Eq | Ne | Lt | Le | Gt | Ge => {
                self.em.cmp_rr(RAX, RCX);
                let cc = match (op, unsigned) {
                    (Eq, _) => CC::E,
                    (Ne, _) => CC::NE,
                    (Lt, false) => CC::L,
                    (Le, false) => CC::LE,
                    (Gt, false) => CC::G,
                    (Ge, false) => CC::GE,
                    (Lt, true) => CC::B,
                    (Le, true) => CC::BE,
                    (Gt, true) => CC::A,
                    (Ge, true) => CC::AE,
                    _ => CC::E,
                };
                self.em.setcc_al(cc);
                self.em.movzx_rax_al();
            }
            LogAnd | LogOr => {
                // early-returned above; reaching here is a compiler invariant bug
                self.internal_error(sp, "internal: logical op in arithmetic path");
            }
        }
    }

    /// flags already set by ucomisd (xmm0 vs xmm1); rax ← 0/1, NaN-correct.
    fn emit_float_cc(&mut self, op: BinOp) {
        let l_unord = self.em.label();
        let l_end = self.em.label();
        self.em.jcc(CC::P, l_unord);
        let cc = match op {
            BinOp::Eq => CC::E,
            BinOp::Ne => CC::NE,
            BinOp::Lt => CC::B,
            BinOp::Le => CC::BE,
            BinOp::Gt => CC::A,
            BinOp::Ge => CC::AE,
            _ => CC::E,
        };
        self.em.setcc_al(cc);
        self.em.movzx_rax_al();
        self.em.jmp_label(l_end);
        self.em.place(l_unord);
        self.em.mov_r_i64(RAX, if op == BinOp::Ne { 1 } else { 0 });
        self.em.place(l_end);
    }

    fn emit_convert(&mut self, from: &Type, to: &Type) {
        if from == to {
            return;
        }
        match (from.is_float(), to.is_float()) {
            (false, false) => {
                let w = to.width_bits();
                if w < 64 {
                    let sh = (64 - w) as u8;
                    // shl then sar/shr by immediate (cl-free)
                    self.em.shl_r_i8(RAX, sh);
                    if to.is_unsigned() {
                        self.em.shr_r_i8(RAX, sh);
                    } else {
                        self.em.sar_r_i8(RAX, sh);
                    }
                }
                // 64-bit signedness flips are bit reinterpretations: no-op
            }
            (false, true) => {
                if from.is_unsigned() && from.width_bits() == 64 {
                    // U64 → F64: ≥2^63 via halve-convert-double (1-ulp rounding, documented)
                    let l_neg = self.em.label();
                    let l_end = self.em.label();
                    self.em.test_rr(RAX, RAX);
                    self.em.jcc(CC::L, l_neg);
                    self.em.cvtsi2sd(XMM0, RAX);
                    self.em.movq_xr(RAX, XMM0);
                    self.em.jmp_label(l_end);
                    self.em.place(l_neg);
                    self.em.shr_r_i8(RAX, 1);
                    self.em.cvtsi2sd(XMM0, RAX);
                    self.em.addsd(XMM0, XMM0);
                    self.em.movq_xr(RAX, XMM0);
                    self.em.place(l_end);
                } else {
                    self.em.cvtsi2sd(XMM0, RAX);
                    self.em.movq_xr(RAX, XMM0);
                }
            }
            (true, false) => {
                if to.is_unsigned() && to.width_bits() == 64 {
                    let l_big = self.em.label();
                    let l_end = self.em.label();
                    self.em.movq_rx(XMM0, RAX);
                    self.em.cvttsd2si(RAX, XMM0);
                    self.em.test_rr(RAX, RAX);
                    self.em.jcc(CC::L, l_big);
                    self.em.jmp_label(l_end);
                    self.em.place(l_big);
                    self.em.movabs(RDX, 0x43E0_0000_0000_0000u64); // 2^63
                    self.em.movq_rx(XMM1, RDX);
                    self.em.subsd(XMM0, XMM1);
                    self.em.cvttsd2si(RAX, XMM0);
                    self.em.movabs(RDX, 0x8000_0000_0000_0000u64);
                    self.em.or_rr(RAX, RDX);
                    self.em.place(l_end);
                } else {
                    self.em.movq_rx(XMM0, RAX);
                    self.em.cvttsd2si(RAX, XMM0);
                    let w = to.width_bits();
                    if w < 64 {
                        let sh = (64 - w) as u8;
                        self.em.shl_r_i8(RAX, sh);
                        if to.is_unsigned() {
                            self.em.shr_r_i8(RAX, sh);
                        } else {
                            self.em.sar_r_i8(RAX, sh);
                        }
                    }
                }
            }
            (true, true) => {}
        }
    }

    fn emit_index(&mut self, base: &TExpr, idx: &TExpr, sp: Span) {
        if matches!(base, TExpr::Var(VarRes::Argv, _, _)) {
            self.emit_expr(idx); // rax = index
            if self.checks_on(OwnershipLevel::Strict) {
                let l_ok = self.em.label();
                self.em.mov_rr(RCX, RAX); // rcx = idx (survives the check)
                self.em.mov_r_mem(RAX, RBP, 16); // argc
                self.em.cmp_rr(RCX, RAX);
                self.em.jcc(CC::B, l_ok); // unsigned: negative idx also caught
                self.emit_trap(K_INDEX_OOB, sp);
                self.em.place(l_ok);
                // rcx still = idx here
            } else {
                self.em.mov_rr(RCX, RAX);
            }
            self.em.lea(RAX, RBP, 24);
            self.em.mov_r_mem_index(RAX, RAX, RCX, 0);
        } else {
            // Str indexing: base evaluates to the block pointer
            self.emit_expr(base);
            self.emit_check_read(KIND_INLINE, sp); // block header (moved/invalid)
            self.em.push(RAX); // block ptr (idx eval may clobber)
            self.emit_expr(idx);
            self.em.pop(RBX); // block ptr
            if self.checks_on(OwnershipLevel::Strict) {
                let l_ok = self.em.label();
                self.em.mov_rr(RCX, RAX); // idx
                self.em.mov_r_mem(RAX, RBX, H_LEN as i32);
                self.em.cmp_rr(RCX, RAX);
                self.em.jcc(CC::B, l_ok);
                self.emit_trap(K_INDEX_OOB, sp);
                self.em.place(l_ok);
                self.em.mov_rr(RAX, RCX);
            }
            self.em.lea_r_mem_index(RAX, RBX, RAX, H_DATA as i32);
            self.em.load_sz(RAX, RAX, 0, 1, false);
        }
    }

    fn emit_call(&mut self, ci: &CallInfo, sp: Span) {
        let n = ci.args.len();
        match &ci.kind {
            CallKind::Builtin(bid) => {
                // SysV argument registers: rdi=ctx, rsi=arg0, rdx=arg1.
                // Arguments are evaluated into a reserved stack block first
                // (exactly as Print does) because evaluating a later argument
                // can clobber a register already holding an earlier one.
                // v1 capped builtins at one argument; TempleOS-RS Phase 1
                // needs two (`FsWrite(path, content)`).
                if n > 0 {
                    let block = (((n * 8) + 15) & !15) as i32;
                    self.em.sub_rsp(block);
                    for (i, a) in ci.args.iter().enumerate() {
                        self.emit_expr(a);
                        self.em.mov_mem_r(RSP, (i * 8) as i32, RAX);
                    }
                    // SysV argument registers after ctx (rdi): rsi, rdx, rcx, r8.
                    // Loaded last-to-first so nothing is clobbered while loading.
                    if n > 3 {
                        self.em.load_sz(R8, RSP, 24, 8, false);
                    }
                    if n > 2 {
                        self.em.load_sz(RCX, RSP, 16, 8, false);
                    }
                    if n > 1 {
                        self.em.load_sz(RDX, RSP, 8, 8, false);
                    }
                    self.em.load_sz(RSI, RSP, 0, 8, false);
                    self.em.add_rsp(block);
                }
                self.em.mov_rr(RDI, R15);
                let off = match bid {
                    BuiltinId::SimPush => OFF_SIM_PUSH,
                    BuiltinId::SimKey => OFF_SIM_KEY,
                    BuiltinId::HotPatch => OFF_HOT_PATCH,
                    BuiltinId::HeapCheck => OFF_HEAP_CHECK,
                    BuiltinId::FsWrite => OFF_FS_WRITE,
                    BuiltinId::FsRead => OFF_FS_READ,
                    BuiltinId::FsList => OFF_FS_LIST,
                    BuiltinId::FsDelete => OFF_FS_DELETE,
                    BuiltinId::StrLen => OFF_STR_LEN,
                    BuiltinId::StrEq => OFF_STR_EQ,
                    BuiltinId::GfxInit => OFF_GFX_INIT,
                    BuiltinId::GfxClear => OFF_GFX_CLEAR,
                    BuiltinId::Plot => OFF_GFX_PLOT,
                    BuiltinId::GfxChar => OFF_GFX_CHAR,
                    BuiltinId::Sleep => OFF_SLEEP_MS,
                    BuiltinId::Time => OFF_TIME_MS,
                    BuiltinId::Date => OFF_DATE,
                    BuiltinId::MouseInit => OFF_MOUSE_INIT,
                    BuiltinId::MouseX => OFF_MOUSE_X,
                    BuiltinId::MouseY => OFF_MOUSE_Y,
                    BuiltinId::Beep => OFF_BEEP,
                    BuiltinId::SoundOff => OFF_SOUND_OFF,
                    BuiltinId::Random => OFF_RANDOM,
                    BuiltinId::EdOpen => OFF_ED_OPEN,
                    BuiltinId::EdLines => OFF_ED_LINES,
                    BuiltinId::EdLine => OFF_ED_LINE,
                    BuiltinId::EdSet => OFF_ED_SET,
                    BuiltinId::EdFind => OFF_ED_FIND,
                    BuiltinId::EdSave => OFF_ED_SAVE,
                    BuiltinId::EdClose => OFF_ED_CLOSE,
                };
                self.call_rt(off);
                let _ = sp;
            }
            CallKind::Print => {
                let block = (((n * 8) + 15) & !15).max(16) as i32;
                self.em.sub_rsp(block);
                for (i, a) in ci.args.iter().enumerate() {
                    let off = (i * 8) as i32;
                    let ty = a.ty();
                    if matches!(ty, Type::Class(_)) {
                        // class arg: materialize address then keep as-is (T_OBJ)
                        self.emit_expr(a);
                    } else {
                        self.emit_expr(a);
                    }
                    self.em.mov_mem_r(RSP, off, RAX);
                }
                self.em.mov_rr(RDI, R15);
                self.em.mov_r_i64(RSI, n as i64);
                self.em.mov_rr(RDX, RSP);
                self.em.movabs(RCX, ci.tags_addr);
                self.call_rt(OFF_PRINT);
                self.em.add_rsp(block);
                self.em.xor_rr(RAX, RAX); // Print returns U0
            }
            CallKind::User(idx, variadic) => {
                let hidden = if *variadic { 1 } else { 0 };
                let slots = n + hidden;
                let block = (((slots * 8) + 15) & !15).max(16) as i32;
                self.em.sub_rsp(block);
                for (i, a) in ci.args.iter().enumerate() {
                    let off = ((hidden + i) * 8) as i32;
                    self.materialize_arg(a, off, sp);
                }
                if *variadic {
                    self.em.mov_mem_i32(RSP, 0, n as i32);
                }
                let cell = match self.sess.fns.get(*idx as usize) {
                    Some(f) => f.cell,
                    None => {
                        self.internal_error(sp, "call to unregistered function");
                        return;
                    }
                };
                self.em.movabs(R14, cell);
                self.em.call_mem_base(R14);
                self.em.add_rsp(block);
            }
        }
    }

    /// Store argument value into [rsp+off]. Value semantics: scalars copy;
    /// Str/class places MOVE into the arg slot (D008: by-value = move).
    fn materialize_arg(&mut self, a: &TExpr, off: i32, sp: Span) {
        let ty = a.ty();
        match &ty {
            Type::Class(_) => {
                let size = ty.size(&self.sess.classes);
                self.emit_expr(a); // source value address
                self.em.mov_rr(RDX, RAX);
                self.em.lea(RSI, RSP, off);
                self.emit_move_or_copy(false, size, sp);
            }
            Type::Str => {
                if let TExpr::LitS(addr, _) = a {
                    self.em.movabs(RAX, *addr);
                    self.em.mov_mem_r(RSP, off, RAX);
                } else if is_place(a) {
                    self.emit_place_addr(a);
                    self.em.mov_rr(RDX, RAX);
                    self.em.lea(RSI, RSP, off);
                    self.emit_move_or_copy(true, 8, sp);
                } else {
                    self.emit_expr(a);
                    self.em.mov_mem_r(RSP, off, RAX);
                }
            }
            _ => {
                self.emit_expr(a);
                self.em.mov_mem_r(RSP, off, RAX);
            }
        }
    }

    // ---------------- places ----------------

    /// rax ← ADDRESS of the place denoted by te (checker guaranteed place-ness).
    fn emit_place_addr(&mut self, te: &TExpr) {
        match te {
            TExpr::Var(res, ty, _) => {
                let (res, ty) = (*res, ty.clone());
                match res {
                    VarRes::Local(slot) => {
                        let k = self.slots_of(&ty);
                        let disp = self.local_disp(slot, k);
                        self.em.lea(RAX, RBP, disp);
                    }
                    VarRes::Param(i) => {
                        let disp = self.param_disp(i);
                        self.em.lea(RAX, RBP, disp);
                    }
                    _ => self.em.xor_rr(RAX, RAX), // unreachable (checker)
                }
            }
            TExpr::Global(name, _, _) => {
                let name = name.clone();
                let addr = self.sess.globals.get(&name).map(|g| g.addr).unwrap_or(0);
                self.em.movabs(RAX, addr);
            }
            TExpr::Member(base, off, is_ptr, _, sp) => {
                let (base, off, is_ptr, sp) =
                    ((**base).clone(), *off as i32, *is_ptr, *sp);
                self.emit_class_addr(&base, is_ptr, sp);
                self.em.mov_rr(RBX, RAX);
                self.em.lea(RAX, RBX, off);
            }
            TExpr::Unary(UnOp::Deref, inner, _, _) => {
                let inner = (**inner).clone();
                self.emit_expr(&inner); // the pointer value IS the address
            }
            _ => self.em.xor_rr(RAX, RAX), // unreachable (checker)
        }
    }

    /// Load the value of a place whose ADDRESS is in addr_reg → rax.
    fn emit_load_place(&mut self, te: &TExpr, addr_reg: Reg, disp: i32) {
        let ty = te.ty();
        let sp = te.span();
        match &ty {
            Type::Str => {
                if disp == 0 {
                    self.em.mov_rr(RAX, addr_reg);
                } else {
                    self.em.lea(RAX, addr_reg, disp);
                }
                self.emit_check_read(KIND_STR_SLOT, sp);
                self.em.mov_r_mem(RAX, addr_reg, disp);
            }
            Type::Class(_) => {
                if disp == 0 {
                    self.em.mov_rr(RAX, addr_reg);
                } else {
                    self.em.lea(RAX, addr_reg, disp);
                }
                self.emit_check_read(KIND_INLINE, sp);
            }
            t => self.emit_load(t, addr_reg, disp),
        }
    }

    /// Store rax into the place at addr_reg+disp (deref places get the write check).
    fn emit_store_place(&mut self, te: &TExpr, addr_reg: Reg, disp: i32) {
        let ty = te.ty();
        if matches!(te, TExpr::Unary(UnOp::Deref, _, _, _)) {
            // check_write key = pointee address (addr_reg); preserve value in rax
            self.em.push(RAX);
            if disp == 0 {
                self.em.mov_rr(RAX, addr_reg);
            } else {
                self.em.lea(RAX, addr_reg, disp);
            }
            self.emit_check_write(deref_is_mut(te), te.span());
            self.em.pop(RAX);
        }
        self.emit_store(&ty, addr_reg, disp);
    }

    fn emit_assign(&mut self, l: &TExpr, aop: AssignOp, r: &TExpr, sp: Span) {
        let lty = l.ty();
        let saved_bb = self.borrow_block;
        if matches!(lty, Type::Ptr(..)) {
            self.borrow_block = true;
        }
        self.emit_assign_inner(l, aop, r, sp);
        self.borrow_block = saved_bb;
    }

    fn emit_assign_inner(&mut self, l: &TExpr, aop: AssignOp, r: &TExpr, sp: Span) {
        let lty = l.ty();
        if aop == AssignOp::Set {
            match &lty {
                Type::Class(_) => {
                    let size = lty.size(&self.sess.classes);
                    self.emit_place_addr(l); // dst addr
                    self.em.push(RAX); // keep dst across sub-emissions
                    if matches!(l, TExpr::Unary(UnOp::Deref, _, _, _)) {
                        self.emit_check_write(deref_is_mut(l), sp); // rax = dst key
                    }
                    self.emit_expr(r); // src value address
                    self.em.mov_rr(RDX, RAX);
                    self.em.mov_r_mem(RSI, RSP, 0); // dst
                    self.emit_move_or_copy(false, size, sp);
                    self.em.pop(RAX); // result = dst address (class value)
                }
                Type::Str => {
                    self.emit_place_addr(l);
                    self.em.push(RAX); // dst slot addr
                    if matches!(l, TExpr::Unary(UnOp::Deref, _, _, _)) {
                        self.emit_check_write(deref_is_mut(l), sp);
                    }
                    if let TExpr::LitS(addr, _) = r {
                        self.em.movabs(RAX, *addr);
                        self.em.mov_r_mem(RCX, RSP, 0);
                        self.em.mov_mem_r(RCX, 0, RAX);
                        self.em.add_rsp(8);
                        // result = the pointer value
                        self.em.movabs(RAX, *addr);
                    } else if is_place(r) {
                        self.emit_place_addr(r);
                        self.em.mov_rr(RDX, RAX); // src slot addr
                        self.em.mov_r_mem(RSI, RSP, 0); // dst slot addr
                        self.emit_move_or_copy(true, 8, sp);
                        self.em.mov_r_mem(RCX, RSP, 0);
                        self.em.add_rsp(8);
                        self.em.mov_r_mem(RAX, RCX, 0); // result = new ptr
                    } else {
                        self.emit_expr(r); // rvalue pointer
                        self.em.mov_r_mem(RCX, RSP, 0);
                        self.em.add_rsp(8);
                        self.em.mov_mem_r(RCX, 0, RAX);
                    }
                }
                _ => {
                    self.emit_place_addr(l);
                    self.em.push(RAX); // place addr
                    self.emit_expr(r); // value
                    self.em.pop(RBX); // place addr in RBX: check_write clobbers
                                      // all SysV arg regs (rcx included); rbx is
                                      // callee-saved and survives the extern call.
                    if matches!(l, TExpr::Unary(UnOp::Deref, _, _, _)) {
                        self.em.push(RAX); // value
                        self.em.mov_rr(RAX, RBX); // key
                        self.emit_check_write(deref_is_mut(l), sp);
                        self.em.pop(RAX);
                    }
                    self.emit_store(&lty, RBX, 0);
                }
            }
            return;
        }
        // compound assignment: old ⊕ rhs stored back; value = result
        self.emit_place_addr(l);
        self.em.mov_rr(RBX, RAX);
        self.emit_load_place(l, RBX, 0); // old → rax
        self.em.push(RAX); // old
        self.em.push(RBX); // place addr
        self.emit_expr(r); // rhs
        self.em.mov_rr(RCX, RAX);
        self.em.pop(RBX); // place addr
        self.em.pop(RAX); // old
        let common = if lty.is_float() || r.ty().is_float() {
            Type::F64
        } else {
            Type::promote_pair(&lty, &r.ty())
        };
        let binop = match aop {
            AssignOp::Add => BinOp::Add,
            AssignOp::Sub => BinOp::Sub,
            AssignOp::Mul => BinOp::Mul,
            AssignOp::Div => BinOp::Div,
            AssignOp::Mod => BinOp::Mod,
            AssignOp::And => BinOp::BitAnd,
            AssignOp::Or => BinOp::BitOr,
            AssignOp::Xor => BinOp::Xor,
            AssignOp::Shl => BinOp::Shl,
            AssignOp::Shr => BinOp::Shr,
            AssignOp::Set => BinOp::Add, // unreachable
        };
        self.emit_arith_regs(binop, &common, sp);
        self.emit_store_place(l, RBX, 0);
        // result: reload for places whose store may differ (narrow truncation)
        self.emit_load_place(l, RBX, 0);
    }

    /// rax=lhs, rcx=rhs; result → rax.
    fn emit_arith_regs(&mut self, op: BinOp, ty: &Type, sp: Span) {
        if ty.is_float() {
            self.em.movq_rx(XMM0, RAX);
            self.em.movq_rx(XMM1, RCX);
            match op {
                BinOp::Add => self.em.addsd(XMM0, XMM1),
                BinOp::Sub => self.em.subsd(XMM0, XMM1),
                BinOp::Mul => self.em.mulsd(XMM0, XMM1),
                BinOp::Div => self.em.divsd(XMM0, XMM1),
                _ => self.internal_error(sp, "float compound-op mismatch"),
            }
            self.em.movq_xr(RAX, XMM0);
            return;
        }
        let unsigned = ty.is_unsigned();
        match op {
            BinOp::Add => self.em.add_rr(RAX, RCX),
            BinOp::Sub => self.em.sub_rr(RAX, RCX),
            BinOp::Mul => self.em.imul_rr(RAX, RCX),
            BinOp::BitAnd => self.em.and_rr(RAX, RCX),
            BinOp::BitOr => self.em.or_rr(RAX, RCX),
            BinOp::Xor => self.em.xor_rr(RAX, RCX),
            BinOp::Div | BinOp::Mod => {
                if self.checks_on(OwnershipLevel::Strict) {
                    let l_ok = self.em.label();
                    self.em.test_rr(RCX, RCX);
                    self.em.jcc(CC::NE, l_ok);
                    self.emit_trap(K_DIV_ZERO, sp);
                    self.em.place(l_ok);
                }
                if unsigned {
                    self.em.xor_rr(RDX, RDX);
                    self.em.div_r(RCX);
                } else {
                    self.em.cqo();
                    self.em.idiv_r(RCX);
                }
                if op == BinOp::Mod {
                    self.em.mov_rr(RAX, RDX);
                }
            }
            BinOp::Shl => self.em.shl_cl(RAX),
            BinOp::Shr => {
                if unsigned {
                    self.em.shr_cl(RAX);
                } else {
                    self.em.sar_cl(RAX);
                }
            }
            _ => self.internal_error(sp, "compound-op mismatch"),
        }
    }
}

// ---------------- public entry ----------------

pub fn compile_unit(sess: &mut Session, unit: &TypedUnit) -> Result<CompiledUnit, DiagList> {
    let mut c = Compiler::new(sess, &unit.file);
    c.compile_unit(unit)
}
