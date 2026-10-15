//! PS/2 mouse (8042 auxiliary port, polled) and its cursor — TempleOS-RS
//! Phase 3 (T3.x).
//!
//! Polling keeps the interrupt-free v1 contract (D040). The controller is
//! initialised once, then `poll()` drains whole 3-byte packets and updates the
//! position. The cursor saves the pixels it covers and restores them before
//! moving, so drawing it does not destroy what is underneath — a cursor that
//! erases the screen would be worse than no cursor.

use crate::{gfx, inb, outb};

const W: i64 = 640;
const H: i64 = 480;
/// Cursor is 5x5 with the hotspot in the middle (odd size, so it is exact).
const CUR: i64 = 5;

static mut X: i64 = W / 2;
static mut Y: i64 = H / 2;
static mut ENABLED: bool = false;
static mut SHOWN: bool = false;
static mut SAVED: [u8; (CUR * CUR) as usize] = [0; (CUR * CUR) as usize];
static mut SAVED_AT: (i64, i64) = (0, 0);
/// Redraw only when something actually changed. `poll()` is called from the
/// shell's idle loop, so repainting unconditionally would burn ~250 port
/// operations per iteration and starve the guest (measured: it made command
/// processing slow enough to break the Phase 2 pixel assertions).
static mut DIRTY: bool = true;
static mut PKT: [u8; 3] = [0; 3];
static mut IDX: usize = 0;

/// Bounded spins: a controller that never answers must not hang the boot. 10k
/// iterations is a worst case of tens of milliseconds under emulation, where
/// 100k was observed pushing the boot gate (PROGRESS TOS-P4.2).
fn wait_write() {
    for _ in 0..10_000 {
        if unsafe { inb(0x64) } & 0x02 == 0 {
            return;
        }
    }
}
fn wait_read() {
    for _ in 0..10_000 {
        if unsafe { inb(0x64) } & 0x01 != 0 {
            return;
        }
    }
}
fn ctrl(b: u8) {
    wait_write();
    unsafe { outb(0x64, b) };
}
fn data(b: u8) {
    wait_write();
    unsafe { outb(0x60, b) };
}
/// Send a byte *to the mouse* (0xD4 prefixes an auxiliary-port write).
fn to_mouse(b: u8) {
    ctrl(0xD4);
    data(b);
}

fn read_aux() -> Option<u8> {
    let st = unsafe { inb(0x64) };
    if st & 0x01 == 0 || st & 0x20 == 0 {
        return None;
    }
    Some(unsafe { inb(0x60) })
}

fn drain() {
    for _ in 0..8 {
        let _ = read_aux();
    }
}

/// Enable the auxiliary port and put the mouse into streaming mode.
pub fn init() -> i64 {
    ctrl(0xA8); // enable the auxiliary (mouse) device
    ctrl(0x20);
    wait_read();
    let cfg = unsafe { inb(0x60) };
    // Clear bit 5 (aux clock disable) and set bit 1 (aux IRQ). We poll rather
    // than service the IRQ, but the clock must be enabled for data to arrive.
    let cfg = (cfg & !0x20) | 0x02;
    ctrl(0x60);
    data(cfg);

    to_mouse(0xF6); // set defaults (100 Hz, 1:1, streaming off)
    drain();
    to_mouse(0xF4); // enable data reporting
    drain();

    unsafe { core::ptr::addr_of_mut!(ENABLED).write(true) };
    1
}

/// Current cursor column. **Samples the device first**: the shell's idle loop is
/// not running while guest code executes, so without this a guest loop would see
/// a position frozen at whatever it was when the shell last idled — which is
/// exactly the input a flight simulator needs while it flies.
pub fn x() -> i64 {
    poll();
    unsafe { core::ptr::addr_of!(X).read() }
}
/// Current cursor row; see `x()`.
pub fn y() -> i64 {
    poll();
    unsafe { core::ptr::addr_of!(Y).read() }
}

/// Make the cursor visible (only meaningful once graphics mode is active).
pub fn show() {
    unsafe {
        core::ptr::addr_of_mut!(SHOWN).write(true);
        core::ptr::addr_of_mut!(DIRTY).write(true);
    }
    redraw();
}

/// Forget what was saved underneath the cursor. Anything that repaints the
/// screen wholesale (a fill, a mode change) must call this, or the next redraw
/// would restore pixels from the *old* screen over the new one.
pub fn invalidate() {
    unsafe {
        core::ptr::addr_of_mut!(SAVED_AT).write((-1000, -1000));
        core::ptr::addr_of_mut!(SAVED).write([0; (CUR * CUR) as usize]);
        core::ptr::addr_of_mut!(DIRTY).write(true);
    }
}

/// Drain pending packets and move the cursor. Called from the shell's idle loop.
pub fn poll() {
    if !unsafe { core::ptr::addr_of!(ENABLED).read() } {
        return;
    }
    while let Some(b) = read_aux() {
        unsafe {
            let idx = core::ptr::addr_of!(IDX).read();
            // Byte 0 always has bit 3 set; use it to resynchronise after a
            // dropped byte rather than shifting the packet forever.
            if idx == 0 && b & 0x08 == 0 {
                continue;
            }
            let pkt = core::ptr::addr_of_mut!(PKT);
            (*pkt)[idx] = b;
            if idx < 2 {
                core::ptr::addr_of_mut!(IDX).write(idx + 1);
                continue;
            }
            core::ptr::addr_of_mut!(IDX).write(0);
            if (*pkt)[0] & 0xC0 != 0 {
                continue; // overflow packet: drop it
            }
            let dx = (*pkt)[1] as i8 as i64;
            let dy = (*pkt)[2] as i8 as i64;
            // PS/2 reports dy positive for "up", screen y grows downward.
            let nx = (core::ptr::addr_of!(X).read() + dx).clamp(0, W - 1);
            let ny = (core::ptr::addr_of!(Y).read() - dy).clamp(0, H - 1);
            if nx != core::ptr::addr_of!(X).read() || ny != core::ptr::addr_of!(Y).read() {
                core::ptr::addr_of_mut!(DIRTY).write(true);
            }
            core::ptr::addr_of_mut!(X).write(nx);
            core::ptr::addr_of_mut!(Y).write(ny);
        }
    }
    redraw();
}

fn restore_old() {
    unsafe {
        let (px, py) = core::ptr::addr_of!(SAVED_AT).read();
        let saved = core::ptr::addr_of!(SAVED) as *const u8;
        let mut j = 0i64;
        while j < CUR {
            let mut i = 0i64;
            while i < CUR {
                let c = *saved.add((j * CUR + i) as usize);
                gfx::plot(px + i, py + j, c as i64);
                i += 1;
            }
            j += 1;
        }
    }
}

fn redraw() {
    if !unsafe { core::ptr::addr_of!(SHOWN).read() }
        || !unsafe { core::ptr::addr_of!(DIRTY).read() }
        || !gfx::is_graphics()
    {
        return;
    }
    unsafe { core::ptr::addr_of_mut!(DIRTY).write(false) };
    restore_old();
    unsafe {
        let ox = core::ptr::addr_of!(X).read() - CUR / 2;
        let oy = core::ptr::addr_of!(Y).read() - CUR / 2;
        let saved = core::ptr::addr_of_mut!(SAVED) as *mut u8;
        let mut j = 0i64;
        while j < CUR {
            let mut i = 0i64;
            while i < CUR {
                let c = gfx::read_pixel(ox + i, oy + j);
                *saved.add((j * CUR + i) as usize) = c;
                // A crosshair: the centre row and column only, so most of what
                // is underneath stays visible.
                if i == CUR / 2 || j == CUR / 2 {
                    gfx::plot(ox + i, oy + j, 15);
                }
                i += 1;
            }
            j += 1;
        }
        core::ptr::addr_of_mut!(SAVED_AT).write((ox, oy));
    }
}
