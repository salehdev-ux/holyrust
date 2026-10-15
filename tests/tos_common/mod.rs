//! Shared harness for the TempleOS-RS host-side tests (SPEC_TOS §5.1 "unit"
//! level). The kernel boots by writing `/TOS/*` into its in-memory filesystem
//! and evaluating every `.HR` file; `boot()` does exactly the same thing on the
//! host, so a regression in the OS's own source is caught in milliseconds
//! instead of a QEMU boot cycle. `tools/kernel_ci.py` then re-asserts the same
//! behaviour bare-metal, which is what makes the fast test trustworthy rather
//! than merely convenient.

#![allow(dead_code)]

use holyrust::hr_runtime::Session;
use holyrust::pipeline;

pub const FILES_HR: &str = include_str!("../../tos/Files.HR");
pub const GRAPHICS_HR: &str = include_str!("../../tos/Graphics.HR");
pub const SOUND_HR: &str = include_str!("../../tos/Sound.HR");
pub const ORACLE_HR: &str = include_str!("../../tos/Oracle.HR");
pub const EDITOR_HR: &str = include_str!("../../tos/Editor.HR");
pub const FLIGHT_HR: &str = include_str!("../../tos/Flight.HR");
pub const SHELL_HR: &str = include_str!("../../tos/Shell.HR");
pub const DOC_TXT: &str = include_str!("../../tos/Doc/TempleOS-RS.txt");

/// The system files the kernel seeds at boot, in kernel order.
/// Keep in step with `SYSTEM_FILES` in `kernel/src/shell.rs`.
pub fn system_files() -> [(&'static str, &'static str); 8] {
    [
        ("/TOS/Files.HR", FILES_HR),
        ("/TOS/Graphics.HR", GRAPHICS_HR),
        ("/TOS/Sound.HR", SOUND_HR),
        ("/TOS/Oracle.HR", ORACLE_HR),
        ("/TOS/Editor.HR", EDITOR_HR),
        ("/TOS/Flight.HR", FLIGHT_HR),
        ("/TOS/Shell.HR", SHELL_HR),
        ("/TOS/Doc/TempleOS-RS.txt", DOC_TXT),
    ]
}

/// Seed `/TOS` exactly as the kernel does, then evaluate every HolyRust source
/// before any user code runs (T1.5).
pub fn boot() -> Session {
    let mut sess = Session::new();
    for (path, body) in system_files() {
        assert!(
            sess.rt_mut().fs.write(path, body.as_bytes()),
            "seeding {path} failed"
        );
    }
    for (path, body) in system_files() {
        if path.ends_with(".HR") {
            eval(&mut sess, body);
        }
    }
    sess
}

/// Compile + execute one unit with guest output captured, asserting clean
/// diagnostics. Returns whatever the guest printed.
pub fn eval(sess: &mut Session, src: &str) -> String {
    sess.rt_mut().capture = Some(Vec::new());
    let out = pipeline::compile_and_run(sess, "tos", src);
    assert!(
        !out.diags.has_errors(),
        "TempleOS-RS source failed to compile:\n{}\n--- source ---\n{}",
        out.diags.render(src),
        src
    );
    assert!(!out.aborted, "unit aborted by a runtime diagnostic:\n{src}");
    let bytes = sess.rt_mut().capture.take().unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Compile + execute one unit, asserting clean diagnostics, but returning the
/// diagnostics too (for the negative tests that expect a runtime trap).
pub fn eval_raw(sess: &mut Session, src: &str) -> (String, bool) {
    sess.rt_mut().capture = Some(Vec::new());
    let out = pipeline::compile_and_run(sess, "tos", src);
    assert!(!out.diags.has_errors(), "compile error:\n{}", out.diags.render(src));
    let bytes = sess.rt_mut().capture.take().unwrap_or_default();
    (String::from_utf8_lossy(&bytes).into_owned(), out.aborted)
}
