//! Phase 4 Task 5 — the ownership test matrix (≥30 cases): legal moves,
//! illegal use-after-move (must be caught 100%), borrow conflicts, unsafe
//! escapes, strictness levels, catchability. Exit criteria:
//!  * ZERO segfaults (any crash kills this test process → suite fails);
//!  * every illegal use-after-move case produces the diagnostic;
//!  * heap integrity (canary walk) verified after EVERY case.

use holyrust::hr_runtime::Session;
use holyrust::pipeline;

enum Want {
    /// exact guest stdout, no abort, no compile errors
    Out(&'static str),
    /// unit aborted with a runtime diagnostic containing this text
    Trap(&'static str),
}

struct Case {
    n: u32,
    name: &'static str,
    src: &'static str,
    want: Want,
}

fn cases() -> Vec<Case> {
    use Want::*;
    vec![
        // ---- legal moves (1–8) ----
        Case { n: 1, name: "literals_are_immortal_copies", src:
            "Str a = \"x\"; Str b = \"y\"; Print(a, b, \"\\n\");", want: Out("xy\n") },
        Case { n: 2, name: "str_move_use_target", src:
            "Str a = \"m\\n\"; Str b = a; Print(b);", want: Out("m\n") },
        Case { n: 3, name: "str_reinit_after_move", src:
            "Str a = \"1\"; Str b = a; a = \"2\"; Print(a, b, \"\\n\");", want: Out("21\n") },
        Case { n: 4, name: "class_move_use_target", src:
            "class C { I64 x; }; C a; a.x = 7; C b = a; Print(b.x, \"\\n\");", want: Out("7\n") },
        Case { n: 5, name: "class_full_reassign_after_move", src:
            "class C { I64 x; }; C a; a.x = 1; C b = a; C src; src.x = 9; a = src; Print(a.x, \"\\n\");",
            want: Out("9\n") },
        Case { n: 6, name: "move_into_fn_param", src:
            "I64 First(Str s) { return s[0]; } Str a = \"A\"; Print(First(a), \"\\n\");",
            want: Out("65\n") },
        Case { n: 7, name: "move_chain", src:
            "Str a = \"c\\n\"; Str b = a; Str c = b; Print(c);", want: Out("c\n") },
        Case { n: 8, name: "scalars_are_copy", src:
            "I64 a = 5; I64 b = a; Print(a, b, \"\\n\"); U8 u = 3; U8 v = u; Print(u + v, \"\\n\");",
            want: Out("55\n6\n") },
        // ---- use-after-move: ALWAYS caught (9–16) ----
        Case { n: 9, name: "read_moved_str", src:
            "Str a = \"x\"; Str b = a; Print(a);", want: Trap("use of a moved value") },
        Case { n: 10, name: "index_moved_str", src:
            "Str a = \"x\"; Str b = a; Print(a[0]);", want: Trap("use of a moved value") },
        Case { n: 11, name: "pass_moved_str", src:
            "I64 F(Str s) { return 0; } Str a = \"x\"; Str b = a; Print(F(a));",
            want: Trap("use of a moved value") },
        Case { n: 12, name: "move_the_moved_str", src:
            "Str a = \"x\"; Str b = a; Str c = a; Print(c);", want: Trap("use of a moved value") },
        Case { n: 13, name: "read_moved_class_field", src:
            "class C { I64 x; }; C a; a.x = 1; C b = a; Print(a.x);",
            want: Trap("use of a moved value") },
        Case { n: 14, name: "move_the_moved_class", src:
            "class C { I64 x; }; C a; C b = a; C c = a;", want: Trap("use of a moved value") },
        Case { n: 15, name: "borrow_moved_str", src:
            "Str a = \"x\"; Str b = a; I64 *p = &a; Print(b);", want: Trap("use of a moved value") },
        Case { n: 16, name: "fn_returns_moved_param", src:
            "I64 F(Str s) { Str t = s; return s[0]; } Str a = \"A\"; Print(F(a));",
            want: Trap("use of a moved value") },
        // ---- borrow XOR rule (17–26) ----
        Case { n: 17, name: "two_shared_borrows_ok", src:
            "I64 n = 1; I64 *p = &n; I64 *q = &n; Print(*p + *q, \"\\n\");", want: Out("2\n") },
        Case { n: 18, name: "excl_while_shared_conflicts", src:
            "I64 n = 1; I64 *p = &n; I64 *q = &mut n;", want: Trap("borrow conflict") },
        Case { n: 19, name: "shared_while_excl_conflicts", src:
            "I64 n = 1; I64 *q = &mut n; I64 *p = &n;", want: Trap("borrow conflict") },
        Case { n: 20, name: "two_excl_conflict", src:
            "I64 n = 1; I64 *a = &mut n; I64 *b = &mut n;", want: Trap("borrow conflict") },
        Case { n: 21, name: "write_via_shared_traps", src:
            "I64 n = 1; I64 *p = &n; *p = 2;", want: Trap("write through a shared borrow") },
        Case { n: 22, name: "write_via_excl_ok", src:
            "I64 n = 1; I64 *q = &mut n; *q = 2; Print(n, \"\\n\");", want: Out("2\n") },
        Case { n: 23, name: "block_scoped_release", src:
            "I64 n = 1; { I64 *q = &mut n; *q = 2; } I64 *r = &mut n; *r = 3; Print(n, \"\\n\");",
            want: Out("3\n") },
        Case { n: 24, name: "stmt_scoped_arg_release", src:
            "I64 Id(I64 *p) { return *p; } I64 n = 7; Print(Id(&n), \"\\n\"); I64 *q = &mut n; *q = 8; Print(n, \"\\n\");",
            want: Out("7\n8\n") },
        Case { n: 25, name: "read_origin_while_shared", src:
            "I64 n = 5; I64 *p = &n; Print(n, *p, \"\\n\");", want: Out("55\n") },
        Case { n: 26, name: "excl_scope_then_origin_read", src:
            "I64 n = 5; { I64 *q = &mut n; *q = 6; } Print(n, \"\\n\");", want: Out("6\n") },
        // ---- unsafe escapes (27–29) ----
        Case { n: 27, name: "unsafe_shared_write", src:
            "I64 n = 1; I64 *p = &n; unsafe { *p = 2; } Print(n, \"\\n\");", want: Out("2\n") },
        Case { n: 28, name: "unsafe_reads_moved_class", src:
            "class C { I64 x; }; C a; a.x = 42; C b = a; I64 v = 0; unsafe { v = a.x; } Print(v, \"\\n\");",
            want: Out("42\n") },
        Case { n: 29, name: "unsafe_borrow_and_write", src:
            "I64 n = 1; I64 *p = &n; I64 *q = &n; unsafe { q = &mut n; *q = 2; } Print(n, \"\\n\");",
            want: Out("2\n") },
        // ---- catchability + strictness levels (30–36) ----
        Case { n: 30, name: "use_after_move_is_catchable", src:
            "Str a = \"x\"; Str b = a; try { Print(a); } catch { Print(catch_code, \":\", catch_msg, \"\\n\"); }",
            want: Out("-10:use of a moved value (moved or never initialized)\n") },
        Case { n: 31, name: "borrow_conflict_is_catchable", src:
            "I64 n = 1; I64 *p = &n; try { I64 *q = &mut n; } catch { Print(catch_code, \"\\n\"); }",
            want: Out("-11\n") },
        Case { n: 32, name: "off_disables_everything", src:
            "#![ownership(off)]\nclass C { I64 x; }; C a; a.x = 5; C b = a; Print(a.x, b.x, \"\\n\"); I64 n = 1; I64 *p = &n; I64 *q = &mut n; Print(n, \"\\n\");",
            want: Out("55\n1\n") },
        Case { n: 33, name: "loose_still_catches_moves", src:
            "#![ownership(loose)]\nStr a = \"x\"; Str b = a; Print(a);",
            want: Trap("use of a moved value") },
        Case { n: 34, name: "loose_skips_borrow_rules", src:
            "#![ownership(loose)]\nI64 n = 1; I64 *p = &n; I64 *q = &mut n; *q = 2; Print(n, \"\\n\");",
            want: Out("2\n") },
        Case { n: 35, name: "strict_is_the_default", src:
            "Str a = \"x\"; Str b = a; Print(a);", want: Trap("use of a moved value") },
        Case { n: 36, name: "block_level_attribute", src:
            "class C { I64 x; };\nC a; a.x = 3;\n{\n#![ownership(off)]\nC b = a;\nPrint(a.x, b.x, \"\\n\");\n}\nC c = a;\n",
            want: Out("33\n") },
    ]
}

#[test]
fn ownership_matrix() {
    let cs = cases();
    let mut passed = 0;
    let mut moves_checked = 0;
    for c in &cs {
        run_case(c);
        if matches!(c.want, Want::Trap(t) if t == "use of a moved value") {
            moves_checked += 1;
        }
        passed += 1;
    }
    // exit criterion: illegal use-after-move caught in 100% of its cases
    // (cases 9–16 strict + 33 loose + 35 default = 10 trap cases)
    assert_eq!(moves_checked, 10, "matrix must contain all 10 move-trap cases");
    assert!(passed >= 30, "matrix must have ≥30 cases, ran {}", passed);
}

fn run_case(c: &Case) {
    let mut sess = Session::new();
    sess.rt_mut().capture = Some(Vec::new());
    sess.rt_mut().diag_log.clear();
    let oc = pipeline::compile_and_run(&mut sess, &format!("case{}", c.n), c.src);
    let cap = sess.rt_mut().capture.take().unwrap_or_default();
    let stdout = String::from_utf8_lossy(&cap).into_owned();
    let rdiags = sess.rt().diag_log.clone();

    // ZERO SEGFAULTS: reaching here at all proves it; also heap integrity:
    match sess.heap_check() {
        Ok(_) => {}
        Err(e) => panic!("case {} ({}): heap corrupt: {}", c.n, c.name, e),
    }
    assert!(!oc.diags.has_errors(), "case {} ({}): compile errors:\n{}", c.n, c.name, oc.diags.render(c.src));

    match &c.want {
        Want::Out(want) => {
            assert!(!oc.aborted, "case {} ({}): aborted; rdiags={:?}", c.n, c.name, rdiags);
            assert_eq!(stdout, *want, "case {} ({}) stdout", c.n, c.name);
        }
        Want::Trap(needle) => {
            assert!(oc.aborted, "case {} ({}): expected trap, got stdout {:?}", c.n, c.name, stdout);
            assert!(
                rdiags.iter().any(|d| d.contains(needle)),
                "case {} ({}): diagnostic {:?} lacks {:?}",
                c.n, c.name, rdiags, needle
            );
        }
    }
}
