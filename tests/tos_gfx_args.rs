//! TempleOS-RS Phase 2: **does builtin argument passing survive consecutive
//! builtin calls?**
//!
//! The kernel CI showed `Plot` drawing the wrong pixels when several builtin
//! calls shared one compilation unit, while a lone `Plot` on its own line was
//! correct. That is invisible from the host through the real device (there is
//! none), so this test installs recording graphics hooks and asserts the exact
//! `(x, y, colour)` triples the codegen passed — turning a hard-to-see
//! codegen bug into an ordinary assertion.
//!
//! Deliberately alone in its own test binary: the hooks are process-global, and
//! `cargo test` runs tests in parallel threads.

use std::sync::Mutex;

use holyrust::hr_runtime::GfxHooks;

static CALLS: Mutex<Vec<(i64, i64, i64)>> = Mutex::new(Vec::new());

fn rec_init() {}
fn rec_clear(color: i64) {
    CALLS.lock().unwrap().push((-1, -1, color));
}
fn rec_plot(x: i64, y: i64, color: i64) {
    CALLS.lock().unwrap().push((x, y, color));
}
fn rec_glyph(x: i64, y: i64, color: i64, _code: i64) {
    CALLS.lock().unwrap().push((x, y, color));
}

mod tos_common;
use tos_common::{boot, eval};

fn install_hooks() {
    let hooks = GfxHooks {
        init: rec_init,
        clear: rec_clear,
        plot: rec_plot,
        glyph: rec_glyph,
    };
    unsafe {
        core::ptr::addr_of_mut!(holyrust::hr_runtime::GFX).write(Some(hooks));
    }
}

#[test]
fn builtin_arguments_survive_consecutive_builtin_calls() {
    let mut sess = boot();
    install_hooks();

    // One unit, mixing arities: 0-arg, 1-arg, then three 3-arg calls.
    eval(
        &mut sess,
        "GfxInit(); GfxClear(0); Plot(100, 100, 15); Plot(101, 100, 4); Plot(100, 101, 2);",
    );
    let one_unit = CALLS.lock().unwrap().clone();
    assert_eq!(
        one_unit,
        vec![(-1, -1, 0), (100, 100, 15), (101, 100, 4), (100, 101, 2)],
        "arguments were corrupted when several builtin calls shared a unit"
    );

    // The same calls split across units must agree exactly.
    CALLS.lock().unwrap().clear();
    eval(&mut sess, "GfxClear(0);");
    eval(&mut sess, "Plot(100, 100, 15);");
    eval(&mut sess, "Plot(101, 100, 4);");
    eval(&mut sess, "Plot(100, 101, 2);");
    let split = CALLS.lock().unwrap().clone();
    assert_eq!(split, one_unit, "same-unit and split-unit calls disagree");

    // And the 4-argument case, which uses the fourth SysV register (r8).
    CALLS.lock().unwrap().clear();
    eval(&mut sess, "GfxChar(8, 16, 7, 65); GfxChar(16, 16, 7, 66);");
    let glyphs = CALLS.lock().unwrap().clone();
    assert_eq!(
        glyphs,
        vec![(8, 16, 7), (16, 16, 7)],
        "GfxChar arguments were corrupted"
    );
}
