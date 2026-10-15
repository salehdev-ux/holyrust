//! Phase 2 Task 3 — AST snapshot regression tests.
//! Dumps are compared byte-for-byte against tests/snapshots/*.ast.
//! Regenerate deliberately with: UPDATE_SNAPSHOTS=1 cargo test --test parser_snapshots

use holyrust::hr_parser::{ast::dump_ast, parse_program};

const DEMOS: &[(&str, &str)] = &[
    ("hello", include_str!("../demos/hello.hr")),
    ("fizzbuzz", include_str!("../demos/fizzbuzz.hr")),
    ("ownership_demo", include_str!("../demos/ownership_demo.hr")),
    ("exe_block", include_str!("../demos/exe_block.hr")),
];

/// One inline snippet per frozen-table feature (L1–L11) so snapshots pin the
/// grammar even if demos change.
const FEATURES: &[(&str, &str)] = &[
    ("l1_types", "U0 v; U8 a = 255; U16 b = 0xFF; U32 c = 1; U64 d = 2; I8 e = -1; I16 f = -2; I32 g = -3; I64 h = -4; F64 i = 1.5e3;"),
    ("l2_defaults", "I64 Foo(I64 a = 1, I64 b = 2) { return a + b; } Foo(,6); Foo(); Foo(1, 2);"),
    ("l3_variadic", "I64 Sum(...) { I64 t = 0; for (I64 i = 0; i < argc; i++) { t += argv[i]; } return t; } Sum(1,2,3);"),
    ("l4_toplevel", "I64 x = 1; x = x + 1; Print(x);"),
    ("l5_bare_string", "\"hi\\n\";"),
    ("l6_switch", "switch (x % 15) { case 0: \"fb\\n\"; case 3, 6, 9, 12: \"f\\n\"; case 4...8: y = 1; default: Print(x, \"\\n\"); }"),
    ("l7_exe", "#exe { Print(\"I64 z = 7;\\n\"); }"),
    ("l8_ownership", "Str a = \"x\"; Str b = a; I64 n = 5; I64 *p = &n; I64 *q = &mut n; *p = *q + 1;"),
    ("l9_try", "try { throw 42; } catch { Print(catch_code, catch_msg); } try { throw 7, \"boom\"; } catch { }"),
    ("l10_class", "class Point { I64 x; I64 y; U0 tag; }; Point p; p.x = 3;"),
    ("l11_unsafe", "unsafe { *p = 9; }"),
    ("attr_ownership", "#![ownership(off)] I64 f() { return 1; } #![ownership(strict)]"),
    ("control_flow", "while (a < b) { a++; } do { b--; } while (b > 0); if (a == b) { c = 1; } else { c = 2; } for (;;) { break; } for (i = 0; i < 3; i++) { continue; }"),
    ("casts_ops", "I64 a = (I64)1.5 + -2 * ~3 % 4 / 5; a <<= 1; a >>= 2; F64 f = (F64)a; U8 u = 'A';"),
];

fn snapshot_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

fn check(name: &str, src: &str) {
    let (prog, diags) = parse_program(&format!("{}.hr", name), src);
    assert!(
        !diags.has_errors(),
        "snapshot source {} must parse clean:\n{}",
        name,
        diags.render(src)
    );
    let dump = dump_ast(&prog);
    let path = snapshot_dir().join(format!("{}.ast", name));
    if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
        std::fs::create_dir_all(snapshot_dir()).expect("create snapshot dir");
        std::fs::write(&path, &dump).expect("write snapshot");
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing snapshot {} — run UPDATE_SNAPSHOTS=1 cargo test --test parser_snapshots",
            path.display()
        )
    });
    if expected != dump {
        // compact first-difference hint
        let e: Vec<&str> = expected.lines().collect();
        let g: Vec<&str> = dump.lines().collect();
        let mut at = 0;
        while at < e.len() && at < g.len() && e[at] == g[at] {
            at += 1;
        }
        panic!(
            "SNAPSHOT MISMATCH {} at line {}\n--- want: {}\n--- got:  {}",
            name,
            at + 1,
            e.get(at).unwrap_or(&"<eof>"),
            g.get(at).unwrap_or(&"<eof>")
        );
    }
}

#[test]
fn demo_snapshots() {
    for (name, src) in DEMOS {
        check(name, src);
    }
}

#[test]
fn feature_snapshots() {
    for (name, src) in FEATURES {
        check(name, src);
    }
}

/// Determinism: parsing twice yields identical dumps (no addresses/hash order).
#[test]
fn dumps_are_deterministic() {
    for (name, src) in DEMOS.iter().chain(FEATURES.iter()) {
        let (p1, _) = parse_program(name, src);
        let (p2, _) = parse_program(name, src);
        assert_eq!(dump_ast(&p1), dump_ast(&p2), "non-deterministic dump for {}", name);
    }
}
