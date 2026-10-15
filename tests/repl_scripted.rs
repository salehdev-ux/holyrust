//! Phase 5 exit criteria:
//!  * a scripted REPL session (piped input) defines a function, calls it,
//!    mutates a value, and triggers one intentional ownership error — all in
//!    one session, non-interactive, and the session SURVIVES the error;
//!  * an `#exe`-generated constant is spliced and used at runtime;
//!  * REPL latency per statement ≤ 100 ms p95 (read from session.log, which
//!    the shell writes for exactly this purpose).

use std::io::Write;
use std::process::{Command, Stdio};

const SCRIPT: &str = r#"I64 Add(I64 a, I64 b = 10) { return a + b; }
Print(Add(5), "\n");
I64 counter = 0;
counter = counter + 41;
counter += 1;
Print(counter, "\n");
Str s = "owned\n";
Str t = s;
Print(s);
Print("session alive\n");
#exe { Print("I64 sq7 = 49;\n"); }
Print(sq7, "\n");
5 + 5;
#quit
"#;

// HolyC-style top-level echo semantics (LANG_SPEC §13): `counter = counter+41`
// echoes 41, `counter += 1` echoes 42, `5 + 5;` echoes 10; the moved-Str
// Print aborts its unit silently on stdout (diagnostic goes to stderr).
const EXPECT_STDOUT: &str = "15\n41\n42\n42\nsession alive\n49\n10\n";

#[test]
fn scripted_repl_session() {
    let bin = env!("CARGO_BIN_EXE_hr");
    let mut child = Command::new(bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hr");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(SCRIPT.as_bytes())
        .expect("write script");
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("wait");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert_eq!(stdout, EXPECT_STDOUT, "--- stderr was ---\n{}", stderr);
    assert!(
        stderr.contains("use of a moved value"),
        "ownership diagnostic missing:\n{}",
        stderr
    );
    assert!(
        stderr.contains("session continues"),
        "shell must survive the trap:\n{}",
        stderr
    );
    assert!(out.status.success(), "exit status {:?}", out.status);

    // ---- latency gate from session.log (last session block) ----
    let log = std::fs::read_to_string("session.log").expect("session.log must exist");
    let start = log
        .rfind("=== HolyRust session start")
        .expect("session header");
    let block = &log[start..];
    let mut lat: Vec<f64> = Vec::new();
    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("[stmt ") {
            if let Some(ms) = rest.split("] ").nth(1) {
                if let Some(v) = ms.strip_suffix("ms") {
                    if let Ok(f) = v.parse::<f64>() {
                        lat.push(f);
                    }
                }
            }
        }
    }
    assert!(
        lat.len() >= 9,
        "expected ≥9 logged statements, got {}: {:?}",
        lat.len(),
        lat
    );
    lat.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p95 = lat[((lat.len() as f64) * 0.95) as usize];
    assert!(p95 <= 100.0, "SPEC gate: REPL p95 latency {}ms > 100ms", p95);

    // input lines are logged too (debugging duty, §2.2F)
    assert!(block.contains("hr> Print(Add(5)"), "input not logged");
}
