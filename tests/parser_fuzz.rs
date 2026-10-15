//! Parser fuzz (SPEC Part 4 Stage 3: "Fuzz/property tests for parser and lexer
//! must run at every phase touching them"). Same deterministic generator as
//! lexer_fuzz; property: lex+parse never panics, always terminates, and every
//! diagnostic carries a well-formed position.

use holyrust::hr_parser::parse_program;
use std::panic::{catch_unwind, AssertUnwindSafe};

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

const TOKENS: &[&str] = &[
    "I64", "U8", "F64", "Str", "U0", "x", "Foo", "if", "else", "while", "do", "for", "switch",
    "case", "default", "break", "continue", "return", "try", "catch", "throw", "unsafe", "class",
    "mut", "argc", "argv", "Print", "{", "}", "(", ")", "[", "]", ";", ",", ":", ".", "...", "..",
    "+", "-", "*", "/", "%", "&", "&&", "|", "||", "^", "~", "!", "<<", ">>", "=", "==", "!=",
    "<", "<=", ">", ">=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>=", "++", "--",
    "#", "#!", "#exe", "#![ownership(off)]", "\"str\\n\"", "'a'", "0x1F", "0b101", "42", "1.5e-3",
    "// comment\n", "/* block */", "\n", " ", "@", "é", "\\", "\"unterminated", "=",
];

fn gen(rng: &mut Rng) -> String {
    let n = rng.below(120) as usize;
    let mut s = String::new();
    for _ in 0..n {
        let pick = rng.below(TOKENS.len() as u64) as usize;
        s.push_str(TOKENS[pick]);
        if rng.below(4) == 0 {
            s.push(' ');
        }
        if rng.below(16) == 0 {
            s.push('\n');
        }
    }
    s
}

#[test]
fn parser_fuzz_10k_no_panics() {
    let mut rng = Rng(0xDEADBEEFCAFEBABE);
    for iter in 0..10_000u32 {
        let src = gen(&mut rng);
        let res = catch_unwind(AssertUnwindSafe(|| parse_program("fuzz", &src)));
        match res {
            Err(_) => panic!("parser PANICKED on iter {} input:\n{}", iter, src),
            Ok((prog, diags)) => {
                for d in &diags.items {
                    assert!(d.pos.line >= 1 && d.pos.col >= 1, "bad diag pos iter {}: {}", iter, d);
                }
                // sanity: dump must not panic either
                let _ = holyrust::hr_parser::ast::dump_ast(&prog);
            }
        }
    }
}
