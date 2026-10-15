//! Phase 1 Task 2 — golden test suite: for EVERY construct in SPEC table 2.4
//! (L1–L11), a source snippet → expected token stream. Plus position tracking
//! and error-format exit criteria.

use holyrust::hr_lexer::{tokenize, Tok};

/// Render a token stream as one stable line (EOF included) for golden compare.
fn golden(src: &str) -> String {
    let (toks, diags) = tokenize("golden", src);
    assert!(
        !diags.has_errors(),
        "unexpected lexer errors for {:?}:\n{}",
        src,
        diags.render(src)
    );
    toks.iter()
        .map(|t| t.tok.name())
        .collect::<Vec<_>>()
        .join(" ")
}

fn assert_golden(src: &str, expect: &str) {
    let got = golden(src);
    let expect = format!("{} EOF", expect.trim());
    if got != expect {
        panic!("GOLDEN MISMATCH\n--- src: {:?}\n--- want: {}\n--- got:  {}", src, expect, got);
    }
}

// ---------- L1: types ----------
#[test]
fn l1_all_type_names() {
    assert_golden(
        "U0 U8 U16 U32 U64 I8 I16 I32 I64 F64 Str",
        "Kw(U0) Kw(U8) Kw(U16) Kw(U32) Kw(U64) Kw(I8) Kw(I16) Kw(I32) Kw(I64) Kw(F64) Kw(Str)",
    );
}

#[test]
fn l1_declarations_and_literals() {
    assert_golden(
        "I64 x = 42; U8 c = 'a'; F64 f = 1.5e-3; I64 h = 0xFF; I64 b = 0b1010;",
        "Kw(I64) Ident(x) = IntLit(42) ; Kw(U8) Ident(c) = CharLit(97) ; \
         Kw(F64) Ident(f) = FloatLit(0.0015) ; Kw(I64) Ident(h) = IntLit(255) ; \
         Kw(I64) Ident(b) = IntLit(10) ;",
    );
}

// ---------- L2: default args any position, omission call ----------
#[test]
fn l2_default_args_any_position() {
    assert_golden(
        "I64 Foo(I64 a = 1, I64 b = 2) { return a + b; } Foo(,6); Foo();",
        "Kw(I64) Ident(Foo) ( Kw(I64) Ident(a) = IntLit(1) , Kw(I64) Ident(b) = IntLit(2) ) \
         { Kw(return) Ident(a) + Ident(b) ; } Ident(Foo) ( , IntLit(6) ) ; Ident(Foo) ( ) ;",
    );
}

// ---------- L3: variadics ----------
#[test]
fn l3_variadic_argc_argv() {
    assert_golden(
        "I64 Sum(...) { I64 t = 0; for (I64 i = 0; i < argc; i++) { t += argv[i]; } return t; }",
        "Kw(I64) Ident(Sum) ( ... ) { Kw(I64) Ident(t) = IntLit(0) ; \
         Kw(for) ( Kw(I64) Ident(i) = IntLit(0) ; Ident(i) < Ident(argc) ; Ident(i) ++ ) \
         { Ident(t) += Ident(argv) [ Ident(i) ] ; } Kw(return) Ident(t) ; }",
    );
}

// ---------- L4: top-level statements ----------
#[test]
fn l4_toplevel_statements() {
    assert_golden(
        "I64 x = 1; x = x + 1; Print(x);",
        "Kw(I64) Ident(x) = IntLit(1) ; Ident(x) = Ident(x) + IntLit(1) ; Ident(Print) ( Ident(x) ) ;",
    );
}

// ---------- L5: bare string auto-print ----------
#[test]
fn l5_bare_string() {
    assert_golden(
        "\"hi\\n\"; \"tab\\there\\x41\";",
        "StrLit(\"hi\\n\") ; StrLit(\"tab\\thereA\") ;",
    );
}

// ---------- L6: switch with ranges ----------
#[test]
fn l6_switch_ranges() {
    assert_golden(
        "switch (x) { case 0: break; case 4...8: y = 1; break; default: z = 2; }",
        "Kw(switch) ( Ident(x) ) { Kw(case) IntLit(0) : Kw(break) ; \
         Kw(case) IntLit(4) ... IntLit(8) : Ident(y) = IntLit(1) ; Kw(break) ; \
         Kw(default) : Ident(z) = IntLit(2) ; }",
    );
}

// ---------- L7: #exe blocks ----------
#[test]
fn l7_exe_block_and_attribute() {
    assert_golden(
        "#![ownership(off)]\n#exe { Print(\"I64 z = 7;\"); }",
        "#! [ Ident(ownership) ( Ident(off) ) ] # Ident(exe) { Ident(Print) ( StrLit(\"I64 z = 7;\") ) ; }",
    );
}

// ---------- L8: ownership surface syntax ----------
#[test]
fn l8_borrow_move_syntax() {
    assert_golden(
        "Str a = \"x\"; Str b = a; I64 n = 5; I64 *p = &n; I64 *q = &mut n; *p = *q + 1;",
        "Kw(Str) Ident(a) = StrLit(\"x\") ; Kw(Str) Ident(b) = Ident(a) ; \
         Kw(I64) Ident(n) = IntLit(5) ; Kw(I64) * Ident(p) = & Ident(n) ; \
         Kw(I64) * Ident(q) = & Kw(mut) Ident(n) ; * Ident(p) = * Ident(q) + IntLit(1) ;",
    );
}

// ---------- L9: try/catch/throw ----------
#[test]
fn l9_try_catch_throw() {
    assert_golden(
        "try { throw 42; throw \"boom\"; } catch { Print(catch_code); Print(catch_msg); }",
        "Kw(try) { Kw(throw) IntLit(42) ; Kw(throw) StrLit(\"boom\") ; } \
         Kw(catch) { Ident(Print) ( Ident(catch_code) ) ; Ident(Print) ( Ident(catch_msg) ) ; }",
    );
}

// ---------- L10: class ----------
#[test]
fn l10_class() {
    assert_golden(
        "class Point { I64 x; I64 y; U0 tag; }; Point p; p.x = 3;",
        "Kw(class) Ident(Point) { Kw(I64) Ident(x) ; Kw(I64) Ident(y) ; Kw(U0) Ident(tag) ; } ; \
         Ident(Point) Ident(p) ; Ident(p) . Ident(x) = IntLit(3) ;",
    );
}

// ---------- L11: unsafe ----------
#[test]
fn l11_unsafe_block() {
    assert_golden(
        "unsafe { *p = 9; }",
        "Kw(unsafe) { * Ident(p) = IntLit(9) ; }",
    );
}

// ---------- operators, full set ----------
#[test]
fn operators_full_set() {
    assert_golden(
        "+ - * / % & | ^ ~ ! << >> && || == != < <= > >= = += -= *= /= %= &= |= ^= <<= >>= ++ --",
        "+ - * / % & | ^ ~ ! << >> && || == != < <= > >= = += -= *= /= %= &= |= ^= <<= >>= ++ --",
    );
}

// ---------- comments ----------
#[test]
fn comments_are_skipped() {
    assert_golden(
        "// line\nI64 /* block */ x; // trailing",
        "Kw(I64) Ident(x) ;",
    );
}

// ---------- positions: line:col exact ----------
#[test]
fn positions_track_line_and_column() {
    let src = "I64 x = 1;\n  \"hello\";\n\tFoo(,6);";
    let (toks, diags) = tokenize("pos", src);
    assert!(!diags.has_errors(), "{}", diags.render(src));
    let find = |n: usize| -> Tok { toks[n].tok.clone() };
    // line1: Kw(I64)@1:1 Ident(x)@1:5 ...
    assert_eq!((toks[0].pos.line, toks[0].pos.col), (1, 1));
    assert!(matches!(find(0), Tok::Kw(_)));
    assert_eq!((toks[1].pos.line, toks[1].pos.col), (1, 5));
    // line2: StrLit at 2:3
    let s = toks.iter().find(|t| matches!(t.tok, Tok::StrLit(_)));
    let s = s.expect("string token");
    assert_eq!((s.pos.line, s.pos.col), (2, 3));
    // line3: Foo at 3:2 (tab counts as one column — documented)
    let f = toks.iter().find(|t| matches!(&t.tok, Tok::Ident(i) if i == "Foo"));
    let f = f.expect("Foo token");
    assert_eq!((f.pos.line, f.pos.col), (3, 2));
    // byte offsets are exact
    assert_eq!(&src[s.pos.offset as usize..s.end_off as usize], "\"hello\"");
}

// ---------- error messages include line:column (exit criterion) ----------
#[test]
fn errors_include_line_column() {
    let src = "I64 x = 1;\n  @bad\n\"unterminated\n";
    let (_, diags) = tokenize("errs", src);
    let rendered = diags.render(src);
    // @ at line 2 col 3
    assert!(rendered.contains("errs 2:3"), "missing @ position:\n{}", rendered);
    // unterminated string starts line 3 col 1
    assert!(rendered.contains("errs 3:1"), "missing string position:\n{}", rendered);
    assert!(rendered.contains("unterminated"), "{}", rendered);
    assert!(rendered.contains('│') || rendered.contains('|'), "no source snippet rendered");
}

// ---------- malformed input → diagnostics, never a crash, stream still ends EOF ----------
#[test]
fn malformed_input_recovers() {
    for src in [
        "0x", "0b", "'ab'", "''", "'\\", "1..2", "123abc", "18446744073709551616",
        "/* nope", "\"x\ny\"", "\\q", "é",
    ] {
        let (toks, diags) = tokenize("bad", src);
        assert!(diags.has_errors(), "expected a diagnostic for {:?}", src);
        assert_eq!(toks.last().map(|t| t.tok.name()), Some("EOF".to_string()));
        // rendered message must carry a position
        let r = diags.render(src);
        assert!(r.contains("bad "), "diagnostic lacks file/pos: {}", r);
    }
}

#[test]
fn dotdot_suggestion_present() {
    let (_, d) = tokenize("t", "case 1..2:");
    assert!(d.items.iter().any(|x| x.suggestion.as_deref() == Some("did you mean the range operator `...`?")));
}
