//! Phase 1 Task 3 — property test: the lexer never panics on arbitrary byte
//! input. 10k random inputs from a deterministic seeded PRNG (no external
//! fuzzing crate — DECISIONS D005). Also asserts stream invariants:
//! exactly one trailing EOF, monotonic byte offsets, positions well-formed.

use holyrust::hr_lexer::tokenize;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// xorshift64* — small, deterministic, good enough for coverage-style fuzzing.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const INTERESTING: &[u8] = br#"{}()[];,.:+-*/%&|^~!<>=#"'0123456789abexX
if else while do for switch case default break continue return try catch throw
unsafe class mut U0 U8 U16 U33 U64 I8 I16 I32 I64 F64 Str Print argc argv
... .. . => == != <= >= && || << >> += -= *= /= %= &= |= ^= <<= >>= ++ --
#![ownership(off)] #exe { } \n \t \x41 \q /* */ //
"#;
// multi-byte UTF-8 fragments (raw byte strings must stay ASCII)
const HIGH_BYTES: &[u8] = b"\xc3\xa9\xf0\x9f\x98\x80\xff\xfe\x80";

fn gen_input(rng: &mut Rng) -> String {
    let len = rng.below(600) as usize;
    let mut bytes = Vec::with_capacity(len);
    let mode = rng.below(10);
    for _ in 0..len {
        let b = match mode {
            // 40%: fully arbitrary bytes
            0..=3 => rng.below(256) as u8,
            // 60%: biased toward source-like characters
            _ => {
                if rng.below(8) == 0 {
                    let idx = rng.below(HIGH_BYTES.len() as u64) as usize;
                    HIGH_BYTES[idx]
                } else {
                    let idx = rng.below(INTERESTING.len() as u64) as usize;
                    INTERESTING[idx]
                }
            }
        };
        bytes.push(b);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn lexer_fuzz_10k_no_panics() {
    let mut rng = Rng(0x9E3779B97F4A7C15);
    for iter in 0..10_000u32 {
        let src = gen_input(&mut rng);
        let res = catch_unwind(AssertUnwindSafe(|| tokenize("fuzz", &src)));
        match res {
            Err(_) => panic!("lexer PANICKED on iter {} input {:?}", iter, &src[..src.len().min(200)]),
            Ok((toks, diags)) => {
                // invariant: exactly one EOF, at the end
                assert_eq!(
                    toks.last().map(|t| t.tok.name()),
                    Some("EOF".to_string()),
                    "iter {} input {:?}",
                    iter,
                    src
                );
                assert!(
                    toks.iter().filter(|t| t.tok.name() == "EOF").count() == 1,
                    "multiple EOFs, iter {}",
                    iter
                );
                // invariant: offsets monotonic, positions well-formed
                let mut last = 0u32;
                for t in &toks {
                    assert!(t.pos.offset >= last, "offset regression iter {}", iter);
                    last = t.end_off.max(t.pos.offset);
                    assert!(t.pos.line >= 1 && t.pos.col >= 1, "bad pos iter {}", iter);
                }
                // invariant: every diagnostic has a plausible position
                for d in &diags.items {
                    assert!(
                        (d.pos.line as usize) <= src.lines().count() + 1,
                        "diag line out of range iter {}",
                        iter
                    );
                }
            }
        }
    }
}
