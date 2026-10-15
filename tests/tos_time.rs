//! TempleOS-RS Phase 3 host-side tests (SPEC_TOS T3.x / G3).
//!
//! The host has no PIT and no PS/2 mouse, so — as with graphics — the real
//! behaviour is asserted in QEMU. What is worth proving here is the *contract*:
//! `Sleep`/`Time`/`Date`/`Mouse*` route through the installed hooks, they
//! return the hook's values unchanged, and with no hooks installed they report
//! zero instead of inventing a time or a position.
//!
//! One test function only: the hooks are process-global and `cargo test` runs
//! tests in parallel threads.

use std::sync::atomic::{AtomicI64, Ordering};

use holyrust::hr_runtime::{MouseHooks, TimeHooks};

static NOW: AtomicI64 = AtomicI64::new(0);
static SLEPT: AtomicI64 = AtomicI64::new(0);
static MX: AtomicI64 = AtomicI64::new(11);
static MY: AtomicI64 = AtomicI64::new(22);
static MOUSE_READY: AtomicI64 = AtomicI64::new(0);

fn h_millis() -> i64 {
    NOW.load(Ordering::SeqCst)
}
fn h_sleep(ms: i64) {
    SLEPT.fetch_add(ms, Ordering::SeqCst);
    NOW.fetch_add(ms, Ordering::SeqCst);
}
fn h_date() -> i64 {
    20261015
}
fn h_mouse_init() -> i64 {
    MOUSE_READY.fetch_add(1, Ordering::SeqCst);
    1
}
fn h_mx() -> i64 {
    MX.load(Ordering::SeqCst)
}
fn h_my() -> i64 {
    MY.load(Ordering::SeqCst)
}

mod tos_common;
use tos_common::{boot, eval};

fn install_hooks() {
    unsafe {
        core::ptr::addr_of_mut!(holyrust::hr_runtime::TIME).write(Some(TimeHooks {
            millis: h_millis,
            sleep_ms: h_sleep,
            date: h_date,
        }));
        core::ptr::addr_of_mut!(holyrust::hr_runtime::MOUSE).write(Some(MouseHooks {
            init: h_mouse_init,
            x: h_mx,
            y: h_my,
        }));
    }
}

#[test]
fn timer_and_mouse_builtins_follow_the_installed_hooks() {
    let mut sess = boot();

    // 1. With no device installed the builtins must be honest, not creative.
    unsafe {
        core::ptr::addr_of_mut!(holyrust::hr_runtime::TIME).write(None);
        core::ptr::addr_of_mut!(holyrust::hr_runtime::MOUSE).write(None);
    }
    assert_eq!(
        eval(&mut sess, "Print(Sleep(10), \"|\", Time(), \"|\", Date(), \"\\n\");"),
        "0|0|0\n",
        "with no timer, Sleep/Time/Date must report zero"
    );
    assert_eq!(
        eval(&mut sess, "Print(MouseX(), \",\", MouseY(), \"\\n\");"),
        "0,0\n",
        "with no pointing device, the position must be zero"
    );

    // 2. With hooks installed the builtins forward faithfully: `Sleep` advances
    //    the hook's clock by exactly the requested amount and returns it, so a
    //    caller can verify how long it waited (G3's `Sleep(100); Time();`).
    install_hooks();
    NOW.store(1000, Ordering::SeqCst);
    SLEPT.store(0, Ordering::SeqCst);
    assert_eq!(
        eval(
            &mut sess,
            "Print(Time(), \"|\", Sleep(250), \"|\", Time(), \"|\", Date(), \"\\n\");"
        ),
        "1000|250|1250|20261015\n"
    );
    assert_eq!(SLEPT.load(Ordering::SeqCst), 250, "Sleep must pass the duration through");

    // 3. Mouse position is read through the hooks each time, so a change is
    //    visible to the guest without restarting anything.
    assert_eq!(eval(&mut sess, "Print(MouseX(), \",\", MouseY(), \"\\n\");"), "11,22\n");
    MX.store(40, Ordering::SeqCst);
    MY.store(41, Ordering::SeqCst);
    assert_eq!(eval(&mut sess, "Print(\"moved:\", MouseX(), \",\", MouseY(), \"\\n\");"), "moved:40,41\n");
    assert_eq!(eval(&mut sess, "Print(MouseInit(), \"\\n\");"), "1\n");
    assert_eq!(MOUSE_READY.load(Ordering::SeqCst), 1, "MouseInit must reach the device once");

    // 4. Negative and zero sleeps must not touch the clock at all.
    let before = SLEPT.load(Ordering::SeqCst);
    assert_eq!(eval(&mut sess, "Print(Sleep(0), \"|\", Sleep(-5), \"\\n\");"), "0|0\n");
    assert_eq!(SLEPT.load(Ordering::SeqCst), before, "Sleep(<=0) must be a no-op");
}
