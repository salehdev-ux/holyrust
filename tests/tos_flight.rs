//! TempleOS-RS Phase 7 host-side tests: the AfterEgypt-style flight simulator.
//!
//! The host has no display, so the test installs **recording graphics hooks** and
//! a **scripted mouse**, then asserts on the pixels the simulator actually asked
//! for. That turns "the horizon tilts when you bank" into an ordinary assertion,
//! and it runs in milliseconds instead of a QEMU boot.
//!
//! It also covers D057 at the contract level: `SimKey()` must fall back to the
//! kernel's key source when the simulated queue is empty, because that is the
//! only way guest code can read the keyboard while it is running.
//!
//! Alone in its own test binary: the hooks are process-global.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use holyrust::hr_runtime::{GfxHooks, KeyHooks, MouseHooks};

/// Every plot the guest asked for this frame: (x, y, colour).
static PLOTS: Mutex<Vec<(i64, i64, i64)>> = Mutex::new(Vec::new());
static MOUSE_X: AtomicI64 = AtomicI64::new(320);
static MOUSE_Y: AtomicI64 = AtomicI64::new(240);
/// The key the "keyboard" will report next; -1 means nothing pending.
static NEXT_KEY: AtomicI64 = AtomicI64::new(-1);

fn h_init() {}
fn h_clear(_c: i64) {
    PLOTS.lock().unwrap().clear(); // a clear starts a frame
}
fn h_plot(x: i64, y: i64, c: i64) {
    PLOTS.lock().unwrap().push((x, y, c));
}
fn h_glyph(x: i64, y: i64, c: i64, _code: i64) {
    PLOTS.lock().unwrap().push((x, y, c));
}
fn h_mx() -> i64 {
    MOUSE_X.load(Ordering::SeqCst)
}
fn h_my() -> i64 {
    MOUSE_Y.load(Ordering::SeqCst)
}
fn h_key() -> i64 {
    NEXT_KEY.swap(-1, Ordering::SeqCst)
}

mod tos_common;
use tos_common::{boot, eval};

fn install_hooks() {
    unsafe {
        core::ptr::addr_of_mut!(holyrust::hr_runtime::GFX).write(Some(GfxHooks {
            init: h_init,
            clear: h_clear,
            plot: h_plot,
            glyph: h_glyph,
        }));
        core::ptr::addr_of_mut!(holyrust::hr_runtime::MOUSE).write(Some(MouseHooks {
            init: || 1,
            x: h_mx,
            y: h_my,
        }));
        core::ptr::addr_of_mut!(holyrust::hr_runtime::KEYS).write(Some(KeyHooks { poll: h_key }));
    }
}

/// Mean y of the horizon-coloured pixels in column `x`, or `None`.
/// The horizon is the only thing drawn in colour 9 (light blue).
fn horizon_at(x: i64) -> Option<f64> {
    let p = PLOTS.lock().unwrap();
    let ys: Vec<i64> = p.iter().filter(|q| q.2 == 9 && q.0 == x).map(|q| q.1).collect();
    if ys.is_empty() {
        None
    } else {
        Some(ys.iter().sum::<i64>() as f64 / ys.len() as f64)
    }
}

fn colours_present() -> Vec<i64> {
    let mut v: Vec<i64> = PLOTS.lock().unwrap().iter().map(|q| q.2).collect();
    v.sort_unstable();
    v.dedup();
    v
}

#[test]
fn flight_renders_a_wireframe_world_and_the_horizon_follows_the_mouse() {
    let mut sess = boot();
    install_hooks();

    // --- boot into flight mode -------------------------------------------------
    let ready = eval(&mut sess, "FlightInit();");
    assert!(ready.contains("flight: ready"), "FlightInit said: {ready:?}");

    // --- frame 1: the whole scene must be drawn --------------------------------
    eval(&mut sess, "FlightStep();");
    let placed = PLOTS.lock().unwrap().len();
    assert!(placed > 2000, "a flight frame should draw a real scene, got {placed} plots");
    let cols = colours_present();
    for (c, what) in [(6, "ground"), (8, "sky"), (9, "horizon"), (14, "pyramid"), (15, "HUD")] {
        assert!(cols.contains(&c), "nothing drawn in the {what} colour ({c}); saw {cols:?}");
    }

    // Level flight: the horizon should be level, i.e. the same height at both ends.
    let left = horizon_at(250).expect("horizon missing at x=250");
    let right = horizon_at(390).expect("horizon missing at x=390");
    assert!(
        (right - left).abs() < 3.0,
        "a level aircraft must draw a level horizon (left {left}, right {right})"
    );
    let centre_before = horizon_at(320).expect("horizon missing at x=320");

    // --- bank: mouse X drives roll --------------------------------------------
    MOUSE_X.fetch_add(60, Ordering::SeqCst);
    eval(&mut sess, "FlightStep();");
    let left = horizon_at(250).expect("horizon missing after banking");
    let right = horizon_at(390).expect("horizon missing after banking");
    assert!(
        (right - left).abs() >= 5.0,
        "banking must tilt the horizon (left {left}, right {right})"
    );

    // --- pitch: mouse Y drives the horizon's height ----------------------------
    MOUSE_Y.fetch_add(60, Ordering::SeqCst);
    eval(&mut sess, "FlightStep();");
    let centre_after = horizon_at(320).expect("horizon missing after pitching");
    assert!(
        (centre_after - centre_before).abs() >= 5.0,
        "pitching must move the horizon (before {centre_before}, after {centre_after})"
    );

    // --- level out with SPACE --------------------------------------------------
    NEXT_KEY.store(32, Ordering::SeqCst);
    eval(&mut sess, "FlightStep();");
    let left = horizon_at(250).expect("horizon missing after levelling");
    let right = horizon_at(390).expect("horizon missing after levelling");
    assert!(
        (right - left).abs() < 3.0,
        "SPACE must level the wings (left {left}, right {right})"
    );

    // --- keys: `w`/`s` work the throttle through the key fallback (D057) -------
    assert_eq!(eval(&mut sess, "Print(fThr, \"\\n\");"), "50\n");
    NEXT_KEY.store(119, Ordering::SeqCst); // 'w'
    eval(&mut sess, "FlightStep();");
    assert_eq!(eval(&mut sess, "Print(fThr, \"\\n\");"), "55\n", "w must raise the throttle");
    NEXT_KEY.store(115, Ordering::SeqCst); // 's'
    eval(&mut sess, "FlightStep();");
    assert_eq!(eval(&mut sess, "Print(fThr, \"\\n\");"), "50\n", "s must lower the throttle");

    // Throttle is bounded: holding `w` must not run away.
    unsafe {
        core::ptr::addr_of_mut!(holyrust::hr_runtime::KEYS).write(Some(KeyHooks { poll: h_key }));
    }
    for _ in 0..15 {
        NEXT_KEY.store(119, Ordering::SeqCst);
        eval(&mut sess, "FlightStep();");
    }
    assert_eq!(eval(&mut sess, "Print(fThr, \"\\n\");"), "100\n", "throttle must clamp at 100");

    // --- the aircraft stays above ground and in the world ----------------------
    assert_eq!(eval(&mut sess, "Print(fY >= 8, \"\\n\");"), "1\n");

    // --- exit cleanly ----------------------------------------------------------
    let landed = eval(&mut sess, "FlightExit();");
    assert!(landed.contains("flight: landed"), "FlightExit said: {landed:?}");
    assert_eq!(eval(&mut sess, "Print(fReady, \"\\n\");"), "0\n");
    // ...and the OS is still healthy afterwards.
    assert_eq!(
        eval(&mut sess, "Print(\"alive \", HeapCheck(), \" \", Count(), \"\\n\");"),
        "alive 1 8\n"
    );
}
