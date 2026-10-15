//! PIT millisecond clock + CMOS real-time clock — TempleOS-RS Phase 3 (T3.x).
//!
//! The spec puts interrupts out of v1 scope, so time is kept by **polling** the
//! 8254 down-counter and accumulating wraps: with a 1193 divisor the counter
//! ticks at ~1 kHz, so one counter unit is one millisecond. `Time()` and
//! `Sleep()` both call `poll()`, which means the clock is exact while the guest
//! is waiting and merely coarse otherwise — the honest behaviour for a
//! cooperative, interrupt-free v1.

use crate::{inb, outb};

/// The PIT's input clock, in Hz.
const PIT_HZ: u64 = 1_193_182;
/// Divisor for ~1 kHz: one counter unit == one millisecond.
const DIVISOR: u16 = (PIT_HZ / 1000) as u16;

static mut LAST: u16 = 0;
static mut MILLIS: u64 = 0;
static mut READY: bool = false;

fn read_counter() -> u16 {
    unsafe {
        outb(0x43, 0x00); // latch channel 0
        let lo = inb(0x40) as u16;
        let hi = inb(0x40) as u16;
        (hi << 8) | lo
    }
}

/// Program channel 0 as a rate generator and start the millisecond clock.
pub fn init() {
    unsafe {
        outb(0x43, 0x34); // channel 0, lo/hi byte, mode 2 (rate generator), binary
        outb(0x40, (DIVISOR & 0xFF) as u8);
        outb(0x40, (DIVISOR >> 8) as u8);
        core::ptr::addr_of_mut!(LAST).write(read_counter());
        core::ptr::addr_of_mut!(READY).write(true);
    }
}

/// Accumulate elapsed milliseconds. Cheap, idempotent, and safe to call often:
/// the counter counts DOWN, so "now > last" means it wrapped.
pub fn poll() {
    unsafe {
        if !core::ptr::addr_of!(READY).read() {
            return;
        }
        let now = read_counter();
        let last = core::ptr::addr_of!(LAST).read();
        let delta = if now <= last {
            last - now
        } else {
            last + DIVISOR - now
        };
        let m = core::ptr::addr_of_mut!(MILLIS);
        *m += delta as u64;
        core::ptr::addr_of_mut!(LAST).write(now);
    }
}

/// Milliseconds since `init`.
pub fn millis() -> i64 {
    poll();
    unsafe { core::ptr::addr_of!(MILLIS).read() as i64 }
}

/// Busy-wait for at least `ms` milliseconds (no scheduler exists to yield to).
pub fn sleep_ms(ms: i64) {
    if ms <= 0 {
        return;
    }
    let target = millis() as u64 + ms as u64;
    while (millis() as u64) < target {
        core::hint::spin_loop();
    }
}

fn cmos(reg: u8) -> u8 {
    unsafe {
        outb(0x70, reg);
        inb(0x71)
    }
}

fn bcd(v: u8) -> i64 {
    ((v >> 4) as i64) * 10 + (v & 0x0F) as i64
}

/// Calendar date as `YYYYMMDD` from the CMOS RTC, or 0 when it reads as unset.
/// Register 0x0B bit 2 says whether the fields are binary or BCD; both are
/// handled rather than assumed.
pub fn date() -> i64 {
    // Wait out an update-in-progress so day/month/year come from one tick. The
    // bound is deliberately small: each iteration is two port operations, and a
    // million of them costs seconds under emulation — which showed up as the
    // boot gate failing (PROGRESS TOS-P4.2). ~10k iterations is 20 ms even in
    // the worst case, and the fields are validated below anyway.
    let mut guard = 0u32;
    while cmos(0x0A) & 0x80 != 0 && guard < 10_000 {
        guard += 1;
    }
    let raw_d = cmos(0x07);
    let raw_m = cmos(0x08);
    let raw_y = cmos(0x09);
    let status = cmos(0x0B);
    let (d, m, y) = if status & 0x04 == 0 {
        (bcd(raw_d), bcd(raw_m), bcd(raw_y))
    } else {
        (raw_d as i64, raw_m as i64, raw_y as i64)
    };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return 0; // implausible: report "no calendar" rather than a wrong date
    }
    let year = 2000 + y;
    year * 10000 + m * 100 + d
}
