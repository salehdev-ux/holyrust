//! Phase 3 end-to-end execution tests. Every test compiles HolyRust source
//! through the real pipeline and asserts on exact guest stdout, abort status,
//! and runtime diagnostics. Performance gates are tests, not aspirations
//! (SPEC Part 4 Stage 3).

use holyrust::hr_runtime::Session;
use holyrust::pipeline;
use std::time::Instant;

struct Out {
    stdout: String,
    aborted: bool,
    /// runtime diagnostic lines (mirrored from stderr)
    rdiags: Vec<String>,
    /// rendered compile-time diagnostics
    cdiags: String,
    has_cerrs: bool,
}

fn run_in(sess: &mut Session, src: &str) -> Out {
    sess.rt_mut().capture = Some(Vec::new());
    sess.rt_mut().diag_log.clear();
    let oc = pipeline::compile_and_run(sess, "test", src);
    let cap = sess.rt_mut().capture.take().unwrap_or_default();
    Out {
        stdout: String::from_utf8_lossy(&cap).into_owned(),
        aborted: oc.aborted,
        rdiags: sess.rt().diag_log.clone(),
        cdiags: oc.diags.render(src),
        has_cerrs: oc.diags.has_errors(),
    }
}

fn run(src: &str) -> Out {
    let mut s = Session::new();
    run_in(&mut s, src)
}

macro_rules! assert_out {
    ($src:expr, $want:expr) => {
        let o = run($src);
        assert!(!o.has_cerrs, "compile errors:\n{}", o.cdiags);
        assert!(!o.aborted, "unit aborted; rdiags={:?}", o.rdiags);
        assert_eq!(o.stdout, $want, "\n--- source ---\n{}", $src);
    };
}

// ---------------- L4/L5 + spec exit criterion ----------------

#[test]
fn hello_world_50ms_gate() {
    let src = "\"Hello, world\\n\";";
    let mut s = Session::new();
    // warmup
    let _ = run_in(&mut s, src);
    let mut times = Vec::new();
    for _ in 0..100 {
        let t0 = Instant::now();
        let o = run_in(&mut s, src);
        times.push(t0.elapsed());
        assert_eq!(o.stdout, "Hello, world\n");
    }
    times.sort();
    let med = times[times.len() / 2];
    let p95 = times[(times.len() as f64 * 0.95) as usize];
    assert!(
        med.as_millis() < 50,
        "SPEC gate: median compile+exec {}ms ≥ 50ms",
        med.as_millis()
    );
    assert!(p95.as_millis() < 50, "SPEC gate: p95 {}ms ≥ 50ms", p95.as_millis());
}

#[test]
fn str_assignments_are_silent() {
    // L5: auto-print applies to bare string expressions, not assignments
    assert_out!(
        "Str a = \"1\"; a = \"2\"; Print(a, \"\\n\"); a;",
        "2\n2"
    );
}

#[test]
fn bare_strings_auto_print() {
    assert_out!("\"one\\n\"; \"two\\n\"; Str s = \"three\\n\"; s;", "one\ntwo\nthree\n");
}

// ---------------- integer arithmetic ----------------

#[test]
fn int_arith_and_precedence() {
    assert_out!(
        "Print(1 + 2 * 3, \" \", (1 + 2) * 3, \" \", 7 / 2, \" \", -7 / 2, \" \", 7 % 3, \" \", -7 % 3, \"\\n\");",
        "7 9 3 -3 1 -1\n"
    );
}

#[test]
fn widths_and_truncation() {
    assert_out!(
        "U8 a = 200; U8 b = 100; Print(a + b, \"\\n\"); I64 x = 300; U8 y = x; Print(y, \"\\n\");",
        // U8+U8 promotes to I64 (LANG_SPEC §3): 300. 300 → U8 truncates to 44.
        "300\n44\n"
    );
}

#[test]
fn unsigned_ops() {
    assert_out!(
        "U64 big = 18446744073709551615; Print(big / 3, \"\\n\"); Print(big > 3, \"\\n\"); I64 s = -8; Print(s >> 1, \"\\n\");",
        "6148914691236517205\n1\n-4\n"
    );
}

#[test]
fn bitwise_and_shifts() {
    assert_out!(
        "Print(12 & 10, \" \", 12 | 10, \" \", 12 ^ 10, \" \", ~5, \" \", 1 << 10, \" \", 1024 >> 3, \"\\n\");",
        "8 14 6 -6 1024 128\n"
    );
}

// ---------------- floats ----------------

#[test]
fn float_arith_and_mixed() {
    assert_out!(
        "F64 f = 1.5; Print(f * f, \"\\n\"); Print(1 / 4, \" \", 1.0 / 4, \"\\n\"); Print(3 + 0.5, \"\\n\");",
        "2.25\n0 0.25\n3.5\n"
    );
}

#[test]
fn float_conversions() {
    assert_out!(
        "F64 f = (F64)7 / 2; Print(f, \"\\n\"); I64 t = (I64)2.99; Print(t, \"\\n\"); Print((I64)-2.99, \"\\n\");",
        "3.5\n2\n-2\n"
    );
}

#[test]
fn float_compare_nan() {
    assert_out!(
        "F64 nan = 0.0 / 0.0; Print(nan == nan, \" \", nan != nan, \" \", nan < 1.0, \" \", nan > 1.0, \"\\n\"); if (nan) { \"nan is truthy\\n\"; }",
        "0 1 0 0\nnan is truthy\n"
    );
}

// ---------------- control flow ----------------

#[test]
fn if_else_chains() {
    assert_out!(
        "for (I64 i = 0; i < 4; i++) { if (i == 0) { Print(\"z\"); } else if (i < 3) { Print(\"m\"); } else { Print(\"e\"); } } \"\\n\";",
        "zmme\n"
    );
}

#[test]
fn while_do_break_continue() {
    assert_out!(
        "I64 i = 0; while (1) { i++; if (i == 3) { continue; } if (i > 5) { break; } Print(i); } \"|\";
         I64 j = 0; do { j += 2; Print(j); } while (j < 6); \"\\n\";",
        "1245|246\n"
    );
}

#[test]
fn for_loop_forms() {
    assert_out!(
        "for (I64 i = 0; i < 3; i++) { Print(i); } \"|\"; I64 k = 0; for (;;) { k++; if (k >= 3) { break; } } Print(k, \"\\n\");",
        "012|3\n"
    );
}

#[test]
fn switch_full_semantics() {
    // implicit cases (no fallthrough), lists, ranges, default, break, nested-in-loop continue
    assert_out!(
        "for (I64 i = 0; i < 10; i++) {
           switch (i) {
             case 0: Print(\"a\");
             case 2...4: Print(\"r\");
             case 6, 8: Print(\"l\");
             case 9: break;
             default: Print(\".\");
           }
         } \"\\n\";
         I64 x = 5; switch (x) { case 5: Print(\"hit\\n\"); }",
        "a.rrr.l.l\nhit\n"
    );
}

// ---------------- functions ----------------

#[test]
fn functions_recursion_forward_ref() {
    assert_out!(
        "Print(Fib(20), \"\\n\");            // forward reference works
         I64 Fib(I64 n) { if (n < 2) { return n; } return Fib(n - 1) + Fib(n - 2); }
         U0 Noop() { } Noop();
         F64 Half(F64 v) { return v / 2; } Print(Half(7.0), \"\\n\");",
        "6765\n3.5\n"
    );
}

#[test]
fn default_args_any_position() {
    assert_out!(
        "I64 Foo(I64 a = 1, I64 b = 2, I64 c = 3) { return a * 100 + b * 10 + c; }
         Print(Foo(), \" \", Foo(,6), \" \", Foo(5,,7), \" \", Foo(5, 6), \"\\n\");",
        "123 163 527 563\n"
    );
}

#[test]
fn variadic_argc_argv() {
    assert_out!(
        "I64 Sum(...) { I64 t = 0; for (I64 i = 0; i < argc; i++) { t += argv[i]; } return t; }
         Print(Sum(1, 2, 3), \" \", Sum(), \"\\n\");
         I64 Mixed(I64 a, ...) { return a * 100 + argc; }
         Print(Mixed(7, 8, 9), \"\\n\");        // a=7, argc=3 (named + variadic, LANG_SPEC §5)
         Print(Mixed(7), \"\\n\");",
        "6 0\n703\n701\n"
    );
}

#[test]
fn fn_redefinition_swaps_cell() {
    let mut s = Session::new();
    let o1 = run_in(&mut s, "I64 F() { return 1; } Print(F());");
    assert_eq!(o1.stdout, "1");
    let o2 = run_in(&mut s, "I64 F() { return 2; } Print(F());");
    assert_eq!(o2.stdout, "2"); // hot-swapped via indirection cell (D015)
}

// ---------------- strings ----------------

#[test]
fn string_escapes_and_indexing() {
    assert_out!(
        "Str s = \"A\\t\\x42\\\\\\n\"; Print(s); Print(s[0], \" \", s[1], \" \", s[2], \" \", s[3], \"\\n\");",
        "A\tB\\\n65 9 66 92\n"
    );
}

#[test]
fn string_index_oob_traps() {
    let o = run("Str s = \"abc\"; Print(s[3]);");
    assert!(o.aborted, "expected abort on OOB");
    assert!(o.rdiags.iter().any(|d| d.contains("index out of bounds")), "{:?}", o.rdiags);
}

// ---------------- classes ----------------

#[test]
fn class_fields_and_pointers() {
    assert_out!(
        "class Point { I64 x; I64 y; U0 tag; };
         Point p;                       // zero-initialized
         Print(p.x, p.y, \"\\n\");
         p.x = 3; p.y = 4;
         Point *q = &p;
         Print(q.x, \" \", (*q).y, \"\\n\");   // auto-deref member access
         Print(p.x * p.x + p.y * p.y, \"\\n\");",
        "00\n3 4\n25\n"
    );
}

#[test]
fn deref_read_write_and_unsafe() {
    // valid under BOTH Phase 3 (no-op hooks) and Phase 4 (real borrow rules):
    // shared read, unsafe write (L11), exclusive borrow on a fresh variable
    assert_out!(
        "I64 n = 5;
         I64 *p = &n;
         Print(*p, \"\\n\");
         unsafe { *p = 7; }
         Print(n, \"\\n\");
         I64 m = 1;
         I64 *q = &mut m;
         *q = 2;
         Print(m, *q, \"\\n\");",
        "5\n7\n22\n"
    );
}

#[test]
fn u0_is_zero_sized() {
    // SPEC Component E: "verify zero size in tests"
    let mut s = Session::new();
    let o = run_in(
        &mut s,
        "class A { I64 x; }; class B { U0 u; I64 x; U0 v; };",
    );
    assert!(!o.has_cerrs, "{}", o.cdiags);
    let a = &s.classes[*s.class_by_name.get("A").unwrap() as usize];
    let b = &s.classes[*s.class_by_name.get("B").unwrap() as usize];
    assert_eq!(a.size, b.size, "U0 fields must add zero bytes");
    assert_eq!(a.size, 40, "32-byte header + one 8-byte field");
}

#[test]
fn class_by_value_param_copies() {
    assert_out!(
        "class P { I64 x; };
         I64 GetX(P p) { return p.x; }
         P a; a.x = 9;
         Print(GetX(a), \"\\n\");",
        "9\n"
    );
}

// ---------------- try / catch / throw ----------------

#[test]
fn throw_catch_forms() {
    assert_out!(
        "try { throw 42; } catch { Print(catch_code, \"\\n\"); }
         try { throw \"boom\"; } catch { Print(catch_msg, \"\\n\"); }
         try { throw 7, \"seven\"; } catch { Print(catch_code, \":\", catch_msg, \"\\n\"); }",
        "42\nboom\n7:seven\n"
    );
}

#[test]
fn nested_try_and_loop_balance() {
    // jmp-buffer stack must stay balanced across break/continue inside try
    assert_out!(
        "for (I64 i = 0; i < 3; i++) {
           try { if (i == 1) { throw 1; } Print(\"t\", i); } catch { Print(\"c\", i); }
           if (i == 2) { break; }
         }
         for (I64 j = 0; j < 2; j++) { try { continue; } catch { } Print(\"never\"); }
         try { throw 9; } catch { Print(\"|ok\", catch_code, \"\\n\"); }
         try { throw 5; } catch { Print(\"balanced\\n\"); }",
        "t0c1t2|ok9\nbalanced\n"
    );
}

#[test]
fn uncaught_throw_aborts_with_diagnostic() {
    let o = run("\"before\\n\"; throw 5, \"fatal\"; \"never\\n\";");
    assert!(o.aborted);
    assert!(o.stdout.contains("before"));
    assert!(!o.stdout.contains("never"));
    assert!(o.rdiags.iter().any(|d| d.contains("uncaught throw") && d.contains("fatal")), "{:?}", o.rdiags);
}

// ---------------- traps are diagnostics, never crashes ----------------

#[test]
fn div_by_zero_traps() {
    let o = run("I64 z = 0; Print(1 / z);");
    assert!(o.aborted);
    assert!(o.rdiags.iter().any(|d| d.contains("division by zero")), "{:?}", o.rdiags);
}

#[test]
fn stack_overflow_traps() {
    let o = run("I64 Boom() { return Boom(); } Print(Boom());");
    assert!(o.aborted);
    assert!(o.rdiags.iter().any(|d| d.contains("stack overflow")), "{:?}", o.rdiags);
}

#[test]
fn argv_index_oob_traps() {
    let o = run("I64 F(...) { return argv[argc]; } Print(F(1));");
    assert!(o.aborted);
    assert!(o.rdiags.iter().any(|d| d.contains("index out of bounds")), "{:?}", o.rdiags);
}

// ---------------- session persistence (REPL prerequisite) ----------------

#[test]
fn globals_persist_across_units() {
    let mut s = Session::new();
    let o1 = run_in(&mut s, "I64 g = 40;");
    assert!(!o1.has_cerrs, "{}", o1.cdiags);
    let o2 = run_in(&mut s, "g = g + 2; Print(g, \"\\n\");");
    assert_eq!(o2.stdout, "42\n");
    // same-type redeclaration re-initializes (LANG_SPEC §13)
    let o3 = run_in(&mut s, "I64 g = 1; Print(g);");
    assert_eq!(o3.stdout, "1");
    assert!(!o3.has_cerrs, "{}", o3.cdiags);
    // different type is a diagnostic, not a crash
    let o4 = run_in(&mut s, "Str g = \"nope\";");
    assert!(o4.has_cerrs && o4.cdiags.contains("C020"), "{}", o4.cdiags);
}

// ---------------- #exe compile-time execution (lands early; exit in Phase 5) ----------------

#[test]
fn exe_block_splices_generated_source() {
    assert_out!(
        "#exe { for (I64 i = 0; i < 4; i++) { Print(\"I64 v\", i, \" = \", i * i, \";\\n\"); } }
         Print(v0, v1, v2, v3, \"\\n\");",
        "0149\n"
    );
}

#[test]
fn exe_block_trap_is_compile_diagnostic() {
    let o = run("#exe { throw 1; } \"after\\n\";");
    assert!(o.has_cerrs && o.cdiags.contains("C092"), "{}", o.cdiags);
}

// ---------------- demos (SPEC Part 5 item 1, incremental per PROGRESS) ----------------

#[test]
fn demo_hello_runs() {
    let src = include_str!("../demos/hello.hr");
    let o = run(src);
    assert!(!o.has_cerrs, "{}", o.cdiags);
    assert!(!o.aborted, "{:?}", o.rdiags);
    assert_eq!(o.stdout, "Hello, world\nThe answer is 42\n");
}

#[test]
fn demo_fizzbuzz_runs() {
    let src = include_str!("../demos/fizzbuzz.hr");
    let o = run(src);
    assert!(!o.has_cerrs, "{}", o.cdiags);
    assert!(!o.aborted, "{:?}", o.rdiags);
    let want: String = (1..=30)
        .map(|i| match (i % 3, i % 5) {
            (0, 0) => "FizzBuzz\n".to_string(),
            (0, _) => "Fizz\n".to_string(),
            (_, 0) => "Buzz\n".to_string(),
            _ => format!("{}\n", i),
        })
        .collect();
    assert_eq!(o.stdout, want);
}

#[test]
fn demo_ownership_runs() {
    // Phase 4 exit: the full L8/L9/L10/L11 showcase, traps CAUGHT live by try
    let src = include_str!("../demos/ownership_demo.hr");
    let o = run(src);
    assert!(!o.has_cerrs, "{}", o.cdiags);
    assert!(!o.aborted, "aborted: {:?}", o.rdiags);
    assert_eq!(
        o.stdout,
        "point: 3,4\ncaught use-after-move: use of a moved value (moved or never initialized)\nb is still fine: who owns me?\nshared read: 5\ncaught borrow conflict: borrow conflict (rule: one mutable XOR many immutable)\nafter unsafe write, n = 9\n"
    );
}

#[test]
fn demo_exe_block_runs() {
    let src = include_str!("../demos/exe_block.hr");
    let o = run(src);
    assert!(!o.has_cerrs, "{}", o.cdiags);
    assert!(!o.aborted, "{:?}", o.rdiags);
    assert_eq!(o.stdout, "compile-time-generated squares: 0 1 4 9 16 25 36 49 \n");
}

// ---------------- compile diagnostics (never panics) ----------------

#[test]
fn compile_errors_have_positions_and_suggestions() {
    let o = run("I64 x = unknown_thing + 1;");
    assert!(o.has_cerrs);
    assert!(o.cdiags.contains("C004") && o.cdiags.contains("1:9"), "{}", o.cdiags);
    let o2 = run("Foo(1, 2); I64 Foo(I64 a) { return a; }");
    assert!(o2.has_cerrs && o2.cdiags.contains("C014"), "{}", o2.cdiags);
    let o3 = run("I64 f(I64 a) { return a; } f();");
    assert!(o3.has_cerrs && o3.cdiags.contains("C015"), "{}", o3.cdiags);
    let o4 = run("I64 f(I64 a = x) { return a; }");
    assert!(o4.has_cerrs && o4.cdiags.contains("C011"), "{}", o4.cdiags);
}
