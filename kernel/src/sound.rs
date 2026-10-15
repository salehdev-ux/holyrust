//! PC speaker — TempleOS-RS Phase 4 (T4.x).
//!
//! Channel 2 of the 8254 generates the tone; port 0x61 bits 0 and 1 gate the
//! timer output into the speaker. Every tone is also written through
//! `hr_out` **before** the wait, because SPEC_TOS G4 explicitly accepts "at
//! least a serial log of frequency changes" — a headless CI cannot listen, so
//! the ordered transcript is the evidence a tone was really programmed.

use alloc::format;

use crate::{inb, outb};

/// The PIT's input clock, in Hz (channel 2 shares it with channel 0).
const PIT_HZ: u64 = 1_193_182;

/// Play `freq` Hz for `ms` milliseconds, then silence the speaker.
pub fn beep(freq: i64, ms: i64) {
    if freq <= 0 || ms <= 0 {
        return;
    }
    let div = (PIT_HZ / freq as u64).clamp(1, 0xFFFF) as u16;
    unsafe {
        outb(0x43, 0xB6); // channel 2, lo/hi byte, mode 3 (square wave), binary
        outb(0x42, (div & 0xFF) as u8);
        outb(0x42, (div >> 8) as u8);
        let p = inb(0x61);
        outb(0x61, p | 0x03); // timer-2 gate on, speaker data on
    }
    let line = format!("[tone] {} Hz for {} ms\n", freq, ms);
    holyrust::hr_out::out_bytes(line.as_bytes());
    crate::timer::sleep_ms(ms);
    off();
}

/// Silence the speaker.
pub fn off() {
    unsafe {
        let p = inb(0x61);
        outb(0x61, p & !0x03);
    }
}
