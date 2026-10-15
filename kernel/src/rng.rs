//! Entropy for the God-speaker oracle — TempleOS-RS Phase 4 (T4.x).
//!
//! With interrupts frozen out of v1 there is no jitter to harvest from timers,
//! so the CPU timestamp counter is the one honest source of variation
//! available. It seeds and stirs a xorshift64\* generator; mixing the TSC into
//! the state on every call keeps the sequence advancing even if the generator
//! is asked for numbers in a tight loop.

static mut STATE: u64 = 0;

fn next_u64() -> u64 {
    let tsc = unsafe { core::arch::x86_64::_rdtsc() };
    unsafe {
        let s = core::ptr::addr_of_mut!(STATE);
        if *s == 0 {
            *s = tsc | 1; // xorshift must never run with a zero state
        } else {
            *s ^= tsc.rotate_left(17);
        }
        let mut x = *s;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        *s = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// A value in `0..n`, or 0 when `n <= 0`.
pub fn below(n: i64) -> i64 {
    if n <= 0 {
        return 0;
    }
    (next_u64() % n as u64) as i64
}
