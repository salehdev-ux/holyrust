//! x86-64 machine-code emitter (SPEC §2.2D via D003: self-contained backend).
//! Emits position-fixed code with two fixup kinds:
//!  * `Rel32`  — jmp/jcc to labels inside the same blob (resolved at finalize);
//!  * `Abs64`  — movabs immediates that reference blob-internal labels
//!               (resolved by the loader once the mmap base is known).
//!
//! Discipline: this module only builds byte vectors; it never executes anything
//! and never panics on caller misuse that we can detect (unplaced labels are an
//! internal-error Result, not a panic).

use alloc::{string::String, vec::Vec};

pub type Reg = u8;
pub const RAX: Reg = 0;
pub const RCX: Reg = 1;
pub const RDX: Reg = 2;
pub const RBX: Reg = 3;
pub const RSP: Reg = 4;
pub const RBP: Reg = 5;
pub const RSI: Reg = 6;
pub const RDI: Reg = 7;
pub const R8: Reg = 8;
pub const R9: Reg = 9;
pub const R12: Reg = 12;
pub const R13: Reg = 13;
pub const R14: Reg = 14;
pub const R15: Reg = 15;

pub const XMM0: u8 = 0;
pub const XMM1: u8 = 1;

/// Condition codes (x86 encoding).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CC {
    E = 4,
    NE = 5,
    L = 12,  // signed <
    GE = 13, // signed >=
    LE = 14, // signed <=
    G = 15,  // signed >
    B = 2,   // unsigned <
    AE = 3,  // unsigned >=
    BE = 6,  // unsigned <=
    A = 7,   // unsigned >
    P = 10,  // parity (used for ucomisd unordered/NaN)
    NP = 11,
}

enum Fixup {
    Rel32 { at: usize, label: u32 },
    Abs64 { at: usize, label: u32 },
}

pub struct Emitter {
    pub code: Vec<u8>,
    fixups: Vec<Fixup>,
    labels: Vec<Option<usize>>,
}

impl Default for Emitter {
    fn default() -> Self {
        Self::new()
    }
}

impl Emitter {
    pub fn new() -> Self {
        Emitter {
            code: Vec::with_capacity(4096),
            fixups: Vec::new(),
            labels: Vec::new(),
        }
    }

    // ---------- labels ----------
    pub fn label(&mut self) -> u32 {
        self.labels.push(None);
        (self.labels.len() - 1) as u32
    }
    pub fn place(&mut self, l: u32) {
        if let Some(slot) = self.labels.get_mut(l as usize) {
            *slot = Some(self.code.len());
        }
    }
    pub fn here(&self) -> usize {
        self.code.len()
    }

    // ---------- raw helpers ----------
    fn b(&mut self, x: u8) {
        self.code.push(x);
    }
    fn b32(&mut self, x: i32) {
        self.code.extend_from_slice(&x.to_le_bytes());
    }
    fn b64(&mut self, x: u64) {
        self.code.extend_from_slice(&x.to_le_bytes());
    }
    fn rex(&mut self, w: bool, reg: u8, rm: u8) {
        let v = 0x40 | ((w as u8) << 3) | (((reg >> 3) & 1) << 2) | ((rm >> 3) & 1);
        if v != 0x40 {
            self.b(v);
        } else {
            self.b(0x40); // always emit REX: harmless for our ops and keeps spl/bpl/sil/dil sane
        }
    }
    fn rex_opt(&mut self, w: bool, reg: u8, rm: u8) {
        // emit REX only when needed (W, or high regs)
        let v = 0x40 | ((w as u8) << 3) | (((reg >> 3) & 1) << 2) | ((rm >> 3) & 1);
        if v != 0x40 {
            self.b(v);
        }
    }
    fn modrm_reg(&mut self, reg: u8, rm: u8) {
        self.b(0xC0 | ((reg & 7) << 3) | (rm & 7));
    }
    /// ModRM (+SIB +disp) for [base + disp]; `reg` is the register field.
    /// Byte order per Intel SDM: ModRM, then SIB (if rm=100), then displacement.
    fn modrm_mem(&mut self, reg: u8, base: Reg, disp: i32) {
        let need_sib = (base & 7) == 4; // rsp/r12
        let rm_field = if need_sib { 4u8 } else { base & 7 };
        if disp == 0 && (base & 7) != 5 {
            self.b(((reg & 7) << 3) | rm_field);
        } else if disp >= -128 && disp <= 127 {
            self.b(0x40 | ((reg & 7) << 3) | rm_field);
        } else {
            self.b(0x80 | ((reg & 7) << 3) | rm_field);
        }
        if need_sib {
            // scale=0, index=none(100), base=rsp/r12 (&7)
            self.b((0u8 << 6) | (4u8 << 3) | (base & 7));
        }
        if !(disp == 0 && (base & 7) != 5) {
            if disp >= -128 && disp <= 127 {
                self.b(disp as u8);
            } else {
                self.b32(disp);
            }
        }
    }

    // ---------- stack ----------
    pub fn push(&mut self, r: Reg) {
        self.rex_opt(false, 0, r);
        self.b(0x50 | (r & 7));
    }
    pub fn pop(&mut self, r: Reg) {
        self.rex_opt(false, 0, r);
        self.b(0x58 | (r & 7));
    }
    /// add/sub rsp, imm (aligned frame adjust)
    pub fn add_rsp(&mut self, imm: i32) {
        self.alu_rsp(0, imm);
    }
    pub fn sub_rsp(&mut self, imm: i32) {
        self.alu_rsp(5, imm);
    }
    fn alu_rsp(&mut self, op: u8, imm: i32) {
        self.b(0x48);
        if imm >= -128 && imm <= 127 {
            self.b(0x83);
            self.modrm_reg(op, RSP);
            self.b(imm as u8);
        } else {
            self.b(0x81);
            self.modrm_reg(op, RSP);
            self.b32(imm);
        }
    }

    // ---------- mov ----------
    pub fn mov_rr(&mut self, dst: Reg, src: Reg) {
        self.rex(true, src, dst);
        self.b(0x89);
        self.modrm_reg(src, dst);
    }
    pub fn mov_r_i64(&mut self, dst: Reg, v: i64) {
        if v >= i32::MIN as i64 && v <= i32::MAX as i64 {
            // mov r64, imm32 (sign-extended)
            self.rex(true, 0, dst);
            self.b(0xC7);
            self.modrm_reg(0, dst);
            self.b32(v as i32);
        } else {
            self.movabs(dst, v as u64);
        }
    }
    pub fn movabs(&mut self, dst: Reg, v: u64) {
        self.rex(true, 0, dst);
        self.b(0xB8 | (dst & 7));
        self.b64(v);
    }
    /// movabs dst, <label address> — patched by the loader (base + label).
    pub fn movabs_label(&mut self, dst: Reg, label: u32) {
        self.rex(true, 0, dst);
        self.b(0xB8 | (dst & 7));
        let at = self.code.len();
        self.b64(0);
        self.fixups.push(Fixup::Abs64 { at, label });
    }
    pub fn mov_r_mem(&mut self, dst: Reg, base: Reg, disp: i32) {
        self.rex(true, dst, base);
        self.b(0x8B);
        self.modrm_mem(dst, base, disp);
    }
    pub fn mov_mem_r(&mut self, base: Reg, disp: i32, src: Reg) {
        self.rex(true, src, base);
        self.b(0x89);
        self.modrm_mem(src, base, disp);
    }
    pub fn mov_mem_i32(&mut self, base: Reg, disp: i32, v: i32) {
        self.rex(true, 0, base);
        self.b(0xC7);
        self.modrm_mem(0, base, disp);
        self.b32(v);
    }

    /// Sized load: dst = load([base+disp]) zero/sign-extended to 64 bits.
    pub fn load_sz(&mut self, dst: Reg, base: Reg, disp: i32, size: u8, signed: bool) {
        match size {
            1 => {
                self.rex(true, dst, base);
                self.b(0x0F);
                self.b(if signed { 0xBE } else { 0xB6 });
                self.modrm_mem(dst, base, disp);
            }
            2 => {
                self.rex(true, dst, base);
                self.b(0x0F);
                self.b(if signed { 0xBF } else { 0xB7 });
                self.modrm_mem(dst, base, disp);
            }
            4 => {
                if signed {
                    self.rex(true, dst, base);
                    self.b(0x63); // movsxd
                    self.modrm_mem(dst, base, disp);
                } else {
                    self.rex_opt(false, dst, base);
                    self.b(0x8B); // mov r32, m32 (zero-extends)
                    self.modrm_mem(dst, base, disp);
                }
            }
            _ => self.mov_r_mem(dst, base, disp),
        }
    }
    /// Sized store: [base+disp] = low bytes of src.
    pub fn store_sz(&mut self, base: Reg, disp: i32, src: Reg, size: u8) {
        match size {
            1 => {
                // REX selects SPL/BPL/SIL/DIL (instead of legacy AH/CH/DH/BH) and
                // covers r8-r15; force it whenever a low reg 4..7 is involved.
                let rexv = 0x40u8 | (((src >> 3) & 1) << 2) | ((base >> 3) & 1);
                let force = ((src & 7) >= 4 && src < 8) || ((base & 7) >= 4 && base < 8);
                if rexv != 0x40 || force {
                    self.b(rexv);
                }
                self.b(0x88);
                self.modrm_mem(src, base, disp);
            }
            2 => {
                self.b(0x66);
                self.rex_opt(false, src, base);
                self.b(0x89);
                self.modrm_mem(src, base, disp);
            }
            4 => {
                self.rex_opt(false, src, base);
                self.b(0x89);
                self.modrm_mem(src, base, disp);
            }
            _ => self.mov_mem_r(base, disp, src),
        }
    }

    pub fn lea(&mut self, dst: Reg, base: Reg, disp: i32) {
        self.rex(true, dst, base);
        self.b(0x8D);
        self.modrm_mem(dst, base, disp);
    }

    /// ModRM+SIB for [base + index*scale + disp]; `reg` = register field.
    fn modrm_mem_index(&mut self, reg: u8, base: Reg, index: Reg, scale: u8, disp: i32) {
        let sc = match scale {
            1 => 0u8,
            2 => 1,
            4 => 2,
            8 => 3,
            _ => 0,
        };
        let sib = (sc << 6) | ((index & 7) << 3) | (base & 7);
        let mod_bits = if disp == 0 && (base & 7) != 5 {
            0x00
        } else if disp >= -128 && disp <= 127 {
            0x40
        } else {
            0x80
        };
        self.b(mod_bits | ((reg & 7) << 3) | 4); // rm=100 → SIB
        self.b(sib);
        if mod_bits == 0x40 {
            self.b(disp as u8);
        } else if mod_bits == 0x80 {
            self.b32(disp);
        }
    }
    fn rex_indexed(&mut self, w: bool, reg: u8, index: u8, base: u8) {
        let v = 0x40 | ((w as u8) << 3) | (((reg >> 3) & 1) << 2) | (((index >> 3) & 1) << 1) | ((base >> 3) & 1);
        if v != 0x40 {
            self.b(v);
        }
    }
    /// mov dst, [base + index*8 + disp] (64-bit)
    pub fn mov_r_mem_index(&mut self, dst: Reg, base: Reg, index: Reg, disp: i32) {
        self.rex_indexed(true, dst, index, base);
        self.b(0x8B);
        self.modrm_mem_index(dst, base, index, 8, disp);
    }
    /// lea dst, [base + index*1 + disp]
    pub fn lea_r_mem_index(&mut self, dst: Reg, base: Reg, index: Reg, disp: i32) {
        self.rex_indexed(true, dst, index, base);
        self.b(0x8D);
        self.modrm_mem_index(dst, base, index, 1, disp);
    }
    pub fn pxor(&mut self, d: u8, s: u8) {
        self.b(0x66);
        self.rex_opt(false, d, s);
        self.b(0x0F);
        self.b(0xEF);
        self.modrm_reg(d, s);
    }

    // ---------- alu ----------
    fn alu_rr(&mut self, op: u8, dst: Reg, src: Reg) {
        self.rex(true, src, dst);
        self.b(op);
        self.modrm_reg(src, dst);
    }
    pub fn add_rr(&mut self, d: Reg, s: Reg) {
        self.alu_rr(0x01, d, s);
    }
    pub fn sub_rr(&mut self, d: Reg, s: Reg) {
        self.alu_rr(0x29, d, s);
    }
    pub fn and_rr(&mut self, d: Reg, s: Reg) {
        self.alu_rr(0x21, d, s);
    }
    pub fn or_rr(&mut self, d: Reg, s: Reg) {
        self.alu_rr(0x09, d, s);
    }
    pub fn xor_rr(&mut self, d: Reg, s: Reg) {
        self.alu_rr(0x31, d, s);
    }
    pub fn cmp_rr(&mut self, d: Reg, s: Reg) {
        self.alu_rr(0x39, d, s);
    }
    pub fn test_rr(&mut self, d: Reg, s: Reg) {
        self.alu_rr(0x85, d, s);
    }
    fn alu_r_i32(&mut self, op: u8, r: Reg, imm: i32) {
        self.rex(true, 0, r);
        if imm >= -128 && imm <= 127 {
            self.b(0x83);
            self.modrm_reg(op, r);
            self.b(imm as u8);
        } else {
            self.b(0x81);
            self.modrm_reg(op, r);
            self.b32(imm);
        }
    }
    pub fn add_r_i32(&mut self, r: Reg, imm: i32) {
        self.alu_r_i32(0, r, imm);
    }
    pub fn sub_r_i32(&mut self, r: Reg, imm: i32) {
        self.alu_r_i32(5, r, imm);
    }
    pub fn cmp_r_i32(&mut self, r: Reg, imm: i32) {
        self.alu_r_i32(7, r, imm);
    }
    pub fn and_r_i32(&mut self, r: Reg, imm: i32) {
        self.alu_r_i32(4, r, imm);
    }
    pub fn imul_rr(&mut self, d: Reg, s: Reg) {
        self.rex(true, d, s);
        self.b(0x0F);
        self.b(0xAF);
        self.modrm_reg(d, s);
    }
    fn group_f7(&mut self, ext: u8, r: Reg) {
        self.rex(true, 0, r);
        self.b(0xF7);
        self.modrm_reg(ext, r);
    }
    pub fn neg_r(&mut self, r: Reg) {
        self.group_f7(3, r);
    }
    pub fn not_r(&mut self, r: Reg) {
        self.group_f7(2, r);
    }
    pub fn idiv_r(&mut self, r: Reg) {
        self.group_f7(7, r);
    }
    pub fn div_r(&mut self, r: Reg) {
        self.group_f7(6, r);
    }
    pub fn inc_r(&mut self, r: Reg) {
        self.rex(true, 0, r);
        self.b(0xFF);
        self.modrm_reg(0, r);
    }
    pub fn dec_r(&mut self, r: Reg) {
        self.rex(true, 0, r);
        self.b(0xFF);
        self.modrm_reg(1, r);
    }
    pub fn cqo(&mut self) {
        self.b(0x48);
        self.b(0x99);
    }
    fn shift_cl(&mut self, ext: u8, r: Reg) {
        self.rex(true, 0, r);
        self.b(0xD3);
        self.modrm_reg(ext, r);
    }
    pub fn shl_cl(&mut self, r: Reg) {
        self.shift_cl(4, r);
    }
    pub fn shr_cl(&mut self, r: Reg) {
        self.shift_cl(5, r);
    }
    pub fn sar_cl(&mut self, r: Reg) {
        self.shift_cl(7, r);
    }
    /// shl r, imm8
    pub fn shl_r_i8(&mut self, r: Reg, imm: u8) {
        self.rex(true, 0, r);
        self.b(0xC1);
        self.modrm_reg(4, r);
        self.b(imm);
    }
    /// sar r, imm8
    pub fn sar_r_i8(&mut self, r: Reg, imm: u8) {
        self.rex(true, 0, r);
        self.b(0xC1);
        self.modrm_reg(7, r);
        self.b(imm);
    }
    /// shr r, imm8
    pub fn shr_r_i8(&mut self, r: Reg, imm: u8) {
        self.rex(true, 0, r);
        self.b(0xC1);
        self.modrm_reg(5, r);
        self.b(imm);
    }

    pub fn setcc_al(&mut self, cc: CC) {
        self.b(0x0F);
        self.b(0x90 | (cc as u8));
        self.modrm_reg(0, RAX);
    }
    pub fn movzx_rax_al(&mut self) {
        self.b(0x48);
        self.b(0x0F);
        self.b(0xB6);
        self.modrm_reg(RAX, RAX);
    }

    // ---------- control ----------
    pub fn jmp_label(&mut self, l: u32) {
        self.b(0xE9);
        let at = self.code.len();
        self.b32(0);
        self.fixups.push(Fixup::Rel32 { at, label: l });
    }
    pub fn jcc(&mut self, cc: CC, l: u32) {
        self.b(0x0F);
        self.b(0x80 | (cc as u8));
        let at = self.code.len();
        self.b32(0);
        self.fixups.push(Fixup::Rel32 { at, label: l });
    }
    pub fn call_r(&mut self, r: Reg) {
        self.rex_opt(false, 0, r);
        self.b(0xFF);
        self.modrm_reg(2, r);
    }
    /// call qword [r] — indirect call through a memory cell (hot-patchable, D015).
    pub fn call_mem_base(&mut self, r: Reg) {
        self.rex_opt(false, 0, r);
        self.b(0xFF);
        self.modrm_mem(2, r, 0);
    }
    pub fn ret(&mut self) {
        self.b(0xC3);
    }
    pub fn leave(&mut self) {
        self.b(0xC9);
    }
    pub fn int3(&mut self) {
        self.b(0xCC);
    }
    pub fn ud2(&mut self) {
        self.b(0x0F);
        self.b(0x0B);
    }

    // ---------- sse ----------
    pub fn movq_rx(&mut self, x: u8, r: Reg) {
        // movq xmm, r64 : 66 REX.W 0F 6E /r
        self.b(0x66);
        self.rex(true, x, r);
        self.b(0x0F);
        self.b(0x6E);
        self.modrm_reg(x, r);
    }
    pub fn movq_xr(&mut self, r: Reg, x: u8) {
        // movq r64, xmm : 66 REX.W 0F 7E /r
        self.b(0x66);
        self.rex(true, x, r);
        self.b(0x0F);
        self.b(0x7E);
        self.modrm_reg(x, r);
    }
    pub fn movsd_load(&mut self, x: u8, base: Reg, disp: i32) {
        // canonical prefix order: legacy F2, then REX (must immediately precede opcode)
        self.b(0xF2);
        self.rex_opt(false, x, base);
        self.b(0x0F);
        self.b(0x10);
        self.modrm_mem(x, base, disp);
    }
    pub fn movsd_store(&mut self, base: Reg, disp: i32, x: u8) {
        self.b(0xF2);
        self.rex_opt(false, x, base);
        self.b(0x0F);
        self.b(0x11);
        self.modrm_mem(x, base, disp);
    }
    fn sse_rr(&mut self, opcode: u8, dst: u8, src: u8) {
        self.b(0xF2);
        self.rex_opt(false, dst, src);
        self.b(0x0F);
        self.b(opcode);
        self.modrm_reg(dst, src);
    }
    pub fn addsd(&mut self, d: u8, s: u8) {
        self.sse_rr(0x58, d, s);
    }
    pub fn subsd(&mut self, d: u8, s: u8) {
        self.sse_rr(0x5C, d, s);
    }
    pub fn mulsd(&mut self, d: u8, s: u8) {
        self.sse_rr(0x59, d, s);
    }
    pub fn divsd(&mut self, d: u8, s: u8) {
        self.sse_rr(0x5E, d, s);
    }
    /// ucomisd a, b  (compare a vs b; use with 66 0F 2E — canonical: F2? No:
    /// ucomisd is NP 66? It is `66 0F 2E` per Intel: SSE2 compare uses 0F 2E with
    /// 66 mandatory operand-size prefix.)
    pub fn ucomisd(&mut self, a: u8, b: u8) {
        self.b(0x66);
        self.rex_opt(false, a, b);
        self.b(0x0F);
        self.b(0x2E);
        self.modrm_reg(a, b);
    }
    pub fn cvtsi2sd(&mut self, x: u8, r: Reg) {
        self.b(0xF2);
        self.rex(true, x, r);
        self.b(0x0F);
        self.b(0x2A);
        self.modrm_reg(x, r);
    }
    pub fn cvttsd2si(&mut self, r: Reg, x: u8) {
        self.b(0xF2);
        self.rex(true, r, x);
        self.b(0x0F);
        self.b(0x2C);
        self.modrm_reg(r, x);
    }

    // ---------- finalize ----------
    /// Resolve Rel32 fixups. Abs64 fixups are returned for the loader to patch
    /// with `base + label_offset`.
    pub fn finalize(mut self) -> Result<(Vec<u8>, Vec<(usize, usize)>), String> {
        // Abs64: resolve label -> code offset; loader adds the mmap base.
        let mut abs: Vec<(usize, usize)> = Vec::new();
        for f in &self.fixups {
            if let Fixup::Abs64 { at, label } = f {
                let Some(target) = self.labels.get(*label as usize).copied().flatten() else {
                    return Err(format!("internal codegen error: abs label {} never placed", label));
                };
                abs.push((*at, target));
            }
        }
        for f in core::mem::take(&mut self.fixups) {
            if let Fixup::Rel32 { at, label } = f {
                let Some(target) = self.labels.get(label as usize).copied().flatten() else {
                    return Err(format!("internal codegen error: label {} never placed", label));
                };
                let rel = target as i64 - (at as i64 + 4);
                if rel < i32::MIN as i64 || rel > i32::MAX as i64 {
                    return Err("internal codegen error: jump distance overflows i32".into());
                }
                let bytes = (rel as i32).to_le_bytes();
                if at + 4 > self.code.len() {
                    return Err("internal codegen error: fixup out of range".into());
                }
                self.code[at..at + 4].copy_from_slice(&bytes);
            }
        }
        Ok((self.code, abs))
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    fn bytes(f: impl FnOnce(&mut Emitter)) -> Vec<u8> {
        let mut e = Emitter::new();
        f(&mut e);
        e.finalize().expect("finalize").0
    }
    fn hex(v: &[u8]) -> String {
        v.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ")
    }
    fn assert_enc(f: impl FnOnce(&mut Emitter), expect: &str) {
        let got = hex(&bytes(f));
        assert_eq!(got, expect, "encoding mismatch");
    }

    #[test]
    fn prologue_epilogue() {
        assert_enc(|e| {
            e.push(RBP);
            e.mov_rr(RBP, RSP);
            e.sub_rsp(32);
            e.leave();
            e.ret();
        }, "55 48 89 E5 48 83 EC 20 C9 C3");
    }

    #[test]
    fn mov_forms() {
        assert_enc(|e| e.mov_rr(RAX, RCX), "48 89 C8");
        assert_enc(|e| e.mov_r_i64(RAX, 7), "48 C7 C0 07 00 00 00");
        assert_enc(|e| e.movabs(RAX, 0x1122334455667788), "48 B8 88 77 66 55 44 33 22 11");
        assert_enc(|e| e.mov_r_mem(RAX, RBP, -8), "48 8B 45 F8");
        assert_enc(|e| e.mov_mem_r(RBP, -8, RAX), "48 89 45 F8");
        assert_enc(|e| e.mov_r_mem(RAX, RBP, 16), "48 8B 45 10");
        assert_enc(|e| e.mov_r_mem(RAX, RSP, 0), "48 8B 04 24");
        assert_enc(|e| e.movabs(R8, 1), "49 B8 01 00 00 00 00 00 00 00");
    }

    #[test]
    fn sized_access() {
        // movzx rax, byte [rcx]
        assert_enc(|e| e.load_sz(RAX, RCX, 0, 1, false), "48 0F B6 01");
        // movsx rax, byte [rcx]
        assert_enc(|e| e.load_sz(RAX, RCX, 0, 1, true), "48 0F BE 01");
        // movzx rax, word [rcx+24]
        assert_enc(|e| e.load_sz(RAX, RCX, 24, 2, false), "48 0F B7 41 18");
        // movsxd rax, [rcx]
        assert_enc(|e| e.load_sz(RAX, RCX, 0, 4, true), "48 63 01");
        // mov eax, [rcx]
        assert_enc(|e| e.load_sz(RAX, RCX, 0, 4, false), "8B 01");
        // mov byte [rsi], al  (REX forced: SIL not AH)
        assert_enc(|e| e.store_sz(RSI, 0, RAX, 1), "40 88 06");
        // mov word [rcx], ax
        assert_enc(|e| e.store_sz(RCX, 0, RAX, 2), "66 89 01");
        // mov [rcx], eax
        assert_enc(|e| e.store_sz(RCX, 0, RAX, 4), "89 01");
        // mov [rcx], rax
        assert_enc(|e| e.store_sz(RCX, 0, RAX, 8), "48 89 01");
    }

    #[test]
    fn alu_and_ctrl() {
        assert_enc(|e| e.add_rr(RAX, RCX), "48 01 C8");
        assert_enc(|e| e.sub_rr(RAX, RCX), "48 29 C8");
        assert_enc(|e| e.imul_rr(RAX, RCX), "48 0F AF C1");
        assert_enc(|e| e.cmp_rr(RCX, RAX), "48 39 C1");
        assert_enc(|e| e.cmp_r_i32(RAX, 5), "48 83 F8 05");
        assert_enc(|e| e.idiv_r(RCX), "48 F7 F9");
        assert_enc(|e| e.cqo(), "48 99");
        assert_enc(|e| e.shl_cl(RAX), "48 D3 E0");
        assert_enc(|e| e.neg_r(RAX), "48 F7 D8");
        assert_enc(|e| e.setcc_al(CC::L), "0F 9C C0");
        assert_enc(|e| e.movzx_rax_al(), "48 0F B6 C0");
        assert_enc(|e| e.call_r(RAX), "FF D0");
        assert_enc(|e| e.call_mem_base(R14), "41 FF 16");
        assert_enc(|e| e.and_r_i32(RSP, -16), "48 83 E4 F0");
        assert_enc(|e| e.mov_rr(RSP, R14), "4C 89 F4");
        assert_enc(|e| e.lea(RAX, RBP, -16), "48 8D 45 F0");
    }

    #[test]
    fn sse_encodings() {
        assert_enc(|e| e.movq_rx(XMM0, RAX), "66 48 0F 6E C0");
        assert_enc(|e| e.movq_xr(RAX, XMM0), "66 48 0F 7E C0");
        assert_enc(|e| e.addsd(XMM0, XMM1), "F2 0F 58 C1");
        assert_enc(|e| e.divsd(XMM0, XMM1), "F2 0F 5E C1");
        assert_enc(|e| e.ucomisd(XMM0, XMM1), "66 0F 2E C1");
        assert_enc(|e| e.cvtsi2sd(XMM0, RAX), "F2 48 0F 2A C0");
        assert_enc(|e| e.cvttsd2si(RAX, XMM0), "F2 48 0F 2C C0");
        assert_enc(|e| e.movsd_load(XMM0, RAX, 0), "F2 0F 10 00");
        assert_enc(|e| e.movsd_store(RBP, -8, XMM0), "F2 0F 11 45 F8");
    }

    #[test]
    fn sib_disp_ordering_regression() {
        // BUG FIXTURE (P3): SIB must precede the displacement byte.
        // mov [rsp], rax
        assert_enc(|e| e.mov_mem_r(RSP, 0, RAX), "48 89 04 24");
        // mov [rsp+8], rax
        assert_enc(|e| e.mov_mem_r(RSP, 8, RAX), "48 89 44 24 08");
        // mov [rsp+16], rax
        assert_enc(|e| e.mov_mem_r(RSP, 16, RAX), "48 89 44 24 10");
        // mov rax, [rsp+24]
        assert_enc(|e| e.mov_r_mem(RAX, RSP, 24), "48 8B 44 24 18");
        // mov rax, [rsp+1000] (disp32)
        assert_enc(|e| e.mov_r_mem(RAX, RSP, 1000), "48 8B 84 24 E8 03 00 00");
        // mov rax, [rbp+16]
        assert_enc(|e| e.mov_r_mem(RAX, RBP, 16), "48 8B 45 10");
        // mov qword [rsp+0], imm32
        assert_enc(|e| e.mov_mem_i32(RSP, 0, 3), "48 C7 04 24 03 00 00 00");
        // mov byte [rsp+8], al
        assert_enc(|e| e.store_sz(RSP, 8, RAX, 1), "40 88 44 24 08");
    }

    #[test]
    fn indexed_and_sse_extra() {
        // mov rax, [rcx + rdx*8]
        assert_enc(|e| e.mov_r_mem_index(RAX, RCX, RDX, 0), "48 8B 04 D1");
        // lea rax, [rcx + rdx*1 + 32]
        assert_enc(|e| e.lea_r_mem_index(RAX, RCX, RDX, 32), "48 8D 44 11 20");
        // pxor xmm0, xmm0
        assert_enc(|e| e.pxor(XMM0, XMM0), "66 0F EF C0");
    }

    #[test]
    fn jumps_resolve() {
        let mut e = Emitter::new();
        let l = e.label();
        e.jcc(CC::E, l);
        e.mov_r_i64(RAX, 1);
        e.place(l);
        e.ret();
        let (code, _) = e.finalize().expect("ok");
        assert_eq!(hex(&code), "0F 84 07 00 00 00 48 C7 C0 01 00 00 00 C3");
    }

    #[test]
    fn unplaced_label_is_error_not_panic() {
        let mut e = Emitter::new();
        let l = e.label();
        e.jmp_label(l);
        assert!(e.finalize().is_err());
    }
}
