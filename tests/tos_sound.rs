//! TempleOS-RS Phase 4 host-side tests (SPEC_TOS T4.x / G4).
//!
//! There is no speaker on the host, so audibility is asserted in QEMU through the
//! kernel's serial tone log. What belongs here is the *contract*: `Beep`/`SoundOff`
//! route through the installed hooks, `Random(n)` always returns a value in
//! `0..n` even when the entropy source misbehaves, and with no hooks installed
//! these builtins report zero instead of fabricating sound or randomness.
//!
//! Alone in its own test binary: the hooks are process-global.

use std::sync::atomic::{AtomicI64, Ordering};

use holyrust::hr_runtime::{RngHooks, SoundHooks};

static BEEPS: AtomicI64 = AtomicI64::new(0);
static LAST_FREQ: AtomicI64 = AtomicI64::new(0);
static LAST_MS: AtomicI64 = AtomicI64::new(0);
static OFFS: AtomicI64 = AtomicI64::new(0);
/// What the "entropy source" returns next — set to nonsense on purpose below.
static RNG_VALUE: AtomicI64 = AtomicI64::new(0);

fn h_beep(freq: i64, ms: i64) {
    BEEPS.fetch_add(1, Ordering::SeqCst);
    LAST_FREQ.store(freq, Ordering::SeqCst);
    LAST_MS.store(ms, Ordering::SeqCst);
}
fn h_off() {
    OFFS.fetch_add(1, Ordering::SeqCst);
}
fn h_below(_n: i64) -> i64 {
    RNG_VALUE.load(Ordering::SeqCst)
}

mod tos_common;
use tos_common::{boot, eval};

fn install_hooks() {
    unsafe {
        core::ptr::addr_of_mut!(holyrust::hr_runtime::SOUND).write(Some(SoundHooks {
            beep: h_beep,
            off: h_off,
        }));
        core::ptr::addr_of_mut!(holyrust::hr_runtime::RNG).write(Some(RngHooks { below: h_below }));
    }
}

#[test]
fn sound_and_random_follow_the_hooks_and_random_cannot_escape_its_range() {
    let mut sess = boot();

    // 1. No device: honest zeros, not invented sound or randomness.
    unsafe {
        core::ptr::addr_of_mut!(holyrust::hr_runtime::SOUND).write(None);
        core::ptr::addr_of_mut!(holyrust::hr_runtime::RNG).write(None);
    }
    assert_eq!(
        eval(&mut sess, "Print(Beep(440, 10), \"|\", SoundOff(), \"|\", Random(10), \"\\n\");"),
        "0|0|0\n",
        "with no speaker or entropy source the builtins must report zero"
    );

    // 2. With hooks installed the builtins forward faithfully.
    install_hooks();
    BEEPS.store(0, Ordering::SeqCst);
    OFFS.store(0, Ordering::SeqCst);
    assert_eq!(eval(&mut sess, "Print(Beep(440, 120), \"\\n\");"), "1\n");
    assert_eq!(BEEPS.load(Ordering::SeqCst), 1);
    assert_eq!(LAST_FREQ.load(Ordering::SeqCst), 440);
    assert_eq!(LAST_MS.load(Ordering::SeqCst), 120);

    // Non-positive frequency/duration is a no-op, not a call to the device.
    assert_eq!(eval(&mut sess, "Print(Beep(0, 50), Beep(440, 0), \"\\n\");"), "00\n");
    assert_eq!(BEEPS.load(Ordering::SeqCst), 1, "Beep(<=0) must not reach the device");

    assert_eq!(eval(&mut sess, "Print(SoundOff(), \"\\n\");"), "1\n");
    assert_eq!(OFFS.load(Ordering::SeqCst), 1);

    // 3. `Random(n)` is clamped to its contract even if the entropy source
    //    misbehaves — the oracle indexes a word list with this value, so a bad
    //    source must not become an out-of-range index in guest code.
    RNG_VALUE.store(5, Ordering::SeqCst);
    assert_eq!(eval(&mut sess, "Print(Random(10), \"\\n\");"), "5\n");
    RNG_VALUE.store(99, Ordering::SeqCst); // beyond n
    assert_eq!(eval(&mut sess, "Print(Random(10), \"\\n\");"), "9\n", "must clamp to n-1");
    RNG_VALUE.store(-7, Ordering::SeqCst); // negative
    assert_eq!(eval(&mut sess, "Print(Random(10), \"\\n\");"), "0\n", "must clamp to 0");
    RNG_VALUE.store(12345, Ordering::SeqCst);
    assert_eq!(eval(&mut sess, "Print(Random(0), Random(-3), \"\\n\");"), "00\n", "n<=0 is 0");

    // 4. The HolyRust oracle layer compiles against this contract and always
    //    prints exactly one verse.
    let verse = eval(&mut sess, "Oracle();");
    assert!(!verse.trim().is_empty(), "Oracle() must say something");
    assert!(verse.ends_with('\n'), "a verse is one line: {verse:?}");
}
