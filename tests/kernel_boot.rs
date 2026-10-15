//! SPEC Phase 6 exit criterion 3 (real-kernel track): the scripted QEMU
//! session runs as part of `cargo test`. Skips honestly (with a printed
//! notice) when qemu-system-x86_64 is unavailable — the host-side simulation
//! (`tests/kernel_sim.rs`) remains the portable CI deliverable (Q5/D016).

/// D043 (Windows port): `python3` is a Linux name. On Windows it resolves to a
/// Microsoft Store *stub* that spawns fine but exits 9009, which turned an
/// honest skip into a hard failure. Probe for an interpreter that actually
/// reports a version instead of trusting the name.
fn find_python() -> Option<String> {
    for cand in ["python3", "python", "py"] {
        if let Ok(o) = std::process::Command::new(cand).arg("--version").output() {
            if o.status.success() {
                let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
                s.push_str(&String::from_utf8_lossy(&o.stderr));
                if s.to_ascii_lowercase().contains("python") {
                    return Some(cand.to_string());
                }
            }
        }
    }
    None
}

#[test]
fn kernel_boot_and_live_patch_scripted_session() {
    if std::env::var("HR_SKIP_KERNEL_CI").is_ok() {
        eprintln!("kernel_boot: skipped via HR_SKIP_KERNEL_CI");
        return;
    }
    let py = match find_python() {
        Some(p) => p,
        None => {
            eprintln!("kernel_boot: skipped — no working python interpreter on PATH");
            return;
        }
    };
    let out = match std::process::Command::new(&py)
        .arg("tools/kernel_ci.py")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("kernel_boot: skipped — cannot spawn {}: {}", py, e);
            return;
        }
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    println!("{}", stdout);
    if stdout.contains("KERNEL-CI: SKIP") {
        return; // documented honest skip
    }
    assert!(
        stdout.contains("KERNEL-CI: PASS"),
        "kernel CI failed (rc={:?}):\n{}\n{}",
        out.status.code(),
        stdout,
        stderr
    );
}
