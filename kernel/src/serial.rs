//! COM1 (0x3F8) — the CI transcript channel and a secondary key source
//! (with `-serial stdio`, host stdin arrives here).

const PORT: u16 = 0x3F8;

pub fn init() {
    unsafe {
        crate::outb(PORT + 1, 0x00); // disable interrupts
        crate::outb(PORT + 3, 0x80); // DLAB on
        crate::outb(PORT + 0, 0x01); // divisor lo (115200-ish; QEMU ignores)
        crate::outb(PORT + 1, 0x00); // divisor hi
        crate::outb(PORT + 3, 0x03); // 8N1, DLAB off
        crate::outb(PORT + 2, 0xC7); // enable + clear FIFOs
        crate::outb(PORT + 4, 0x0B); // RTS/DTR/out2
    }
}

pub fn write(bytes: &[u8]) {
    for &b in bytes {
        unsafe {
            // wait for THR empty
            while crate::inb(PORT + 5) & 0x20 == 0 {
                core::hint::spin_loop();
            }
            crate::outb(PORT, b);
        }
    }
}

/// Non-blocking receive (used as an alternate keyboard for scripted CI).
pub fn rx_poll() -> Option<u8> {
    unsafe {
        if crate::inb(PORT + 5) & 1 != 0 {
            Some(crate::inb(PORT))
        } else {
            None
        }
    }
}
