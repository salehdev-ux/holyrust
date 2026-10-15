//! PS/2 keyboard (SPEC Component G) — polled in v1.
//! DEGRADE note (PROGRESS P6b): the spec lists "interrupt handlers"; polling
//! was chosen for determinism and simplicity of the demo (human typing is far
//! slower than the poll loop). IRQ1/IDT wiring is quarantined as FUTURE_IDEAS
//! F16 — recorded honestly rather than half-built.

const DATA: u16 = 0x60;
const STATUS: u16 = 0x64;

static mut SHIFT: bool = false;

/// US QWERTY scancode set 1 → ASCII (printable subset + enter/backspace).
fn translate(sc: u8, shift: bool) -> Option<char> {
    let c = match sc {
        0x1C => '\n',
        0x0E => '\u{8}', // backspace
        0x39 => ' ',
        0x02..=0x0D => {
            let row = b"1234567890-=".get((sc - 0x02) as usize).copied().unwrap_or(b'?');
            let row_s = b"!@#$%^&*()_+".get((sc - 0x02) as usize).copied().unwrap_or(b'?');
            (if shift { row_s } else { row }) as char
        }
        0x10..=0x1B => {
            let row = b"qwertyuiop[]".get((sc - 0x10) as usize).copied().unwrap_or(b'?');
            let row_s = b"QWERTYUIOP{}".get((sc - 0x10) as usize).copied().unwrap_or(b'?');
            (if shift { row_s } else { row }) as char
        }
        0x1E..=0x28 => {
            let row = b"asdfghjkl;'".get((sc - 0x1E) as usize).copied().unwrap_or(b'?');
            let row_s = b"ASDFGHJKL:\"".get((sc - 0x1E) as usize).copied().unwrap_or(b'?');
            (if shift { row_s } else { row }) as char
        }
        0x2C..=0x35 => {
            let row = b"zxcvbnm,./".get((sc - 0x2C) as usize).copied().unwrap_or(b'?');
            let row_s = b"ZXCVBNM<>?".get((sc - 0x2C) as usize).copied().unwrap_or(b'?');
            (if shift { row_s } else { row }) as char
        }
        _ => return None,
    };
    Some(c)
}

/// Non-blocking: returns a character if a keypress is pending.
///
/// Status bit 5 marks a byte from the *auxiliary* device. Since Phase 3 enabled
/// the PS/2 mouse, that bit must be checked before reading 0x60 — otherwise this
/// poller eats mouse packets and the mouse poller eats key scancodes. The byte
/// is deliberately left in the buffer for the mouse driver.
pub fn poll() -> Option<char> {
    unsafe {
        let st = crate::inb(STATUS);
        if st & 1 == 0 {
            return None;
        }
        if st & 0x20 != 0 {
            return None; // auxiliary (mouse) data — not ours
        }
        let sc = crate::inb(DATA);
        // shift press/release tracking
        if sc == 0x2A || sc == 0x36 {
            *core::ptr::addr_of_mut!(SHIFT) = true;
            return None;
        }
        if sc == 0xAA || sc == 0xB6 {
            *core::ptr::addr_of_mut!(SHIFT) = false;
            return None;
        }
        if sc & 0x80 != 0 {
            return None; // other release codes
        }
        translate(sc, *core::ptr::addr_of!(SHIFT))
    }
}
