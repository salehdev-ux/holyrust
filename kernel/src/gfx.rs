//! VGA 640x480x16 (mode 12h) — TempleOS-RS Phase 2 (SPEC_TOS T2.1/T2.2).
//!
//! The kernel is entered in **32-bit protected mode** by the multiboot loader,
//! so the BIOS `INT 10h` path is not available; the mode is programmed directly
//! through the VGA register set (the standard mode-12h sequence). That keeps the
//! whole feature in port I/O with no real-mode excursion.
//!
//! Mode 12h is *planar*: the 4 bits of a pixel live in the 4 planes at the same
//! address, so a single pixel is written by setting the Sequencer map mask to
//! all planes, masking one bit with the Graphics Controller's Bit Mask register,
//! and storing the colour byte. The framebuffer window is 64 KiB at 0xA0000 and
//! each plane is 38,400 bytes (640x480/8).
//!
//! The console keeps working after the mode switch (BC2): `put` renders bytes
//! with the compiled-in 8x8 font (GfxChar/TextOut path), so boot output, prompts
//! and diagnostics remain visible to a human while serial carries the CI.

use crate::font8x8::FONT8X8;
use crate::{inb, outb};

pub const W: i64 = 640;
pub const H: i64 = 480;
/// Bytes per plane: 640 * 480 / 8.
const PLANE_BYTES: usize = 38_400;
/// One scan line holds 640 pixels = 80 bytes.
const ROW_BYTES: usize = 80;
const FB: usize = 0xA0000;

/// Console geometry in 8x8 cells.
const COLS: usize = (W as usize) / 8;
const ROWS: usize = (H as usize) / 8;

static mut GRAPHICS: bool = false;
static mut CROW: usize = 0;
static mut CCOL: usize = 0;

fn wseq(index: u8, value: u8) {
    unsafe {
        outb(0x3C4, index);
        outb(0x3C5, value);
    }
}
fn wcrtc(index: u8, value: u8) {
    unsafe {
        outb(0x3D4, index);
        outb(0x3D5, value);
    }
}
fn wgc(index: u8, value: u8) {
    unsafe {
        outb(0x3CE, index);
        outb(0x3CF, value);
    }
}
fn wattr(index: u8, value: u8) {
    unsafe {
        let _ = inb(0x3DA); // reset the address/data flip-flop
        outb(0x3C0, index);
        outb(0x3C0, value);
    }
}

/// Mode 12h CRTC register file (index 0x00-0x17). 0x11 lands on 0x8C, which
/// re-asserts the CR0-CR7 write protection we released to program the timings.
const CRTC_12H: [u8; 24] = [
    0x5F, 0x4F, 0x50, 0x82, 0x54, 0x80, 0x0B, 0x3E, 0x00, 0x40, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0xEA, 0x8C, 0xDF, 0x28, 0x00, 0xE7, 0x04, 0xE3,
];
/// Graphics controller: write mode 0, read mode 0, bit mask open.
const GC_12H: [u8; 9] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x0F, 0xFF];
/// Attribute controller: the 16 standard EGA/VGA colours, graphics mode on.
const ATTR_12H: [u8; 21] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
    0x01, // 0x10 mode control: graphics, line graphics off
    0x00, // 0x11 overscan
    0x0F, // 0x12 memory plane enable: all four planes
    0x00, // 0x13 horizontal panning
    0x00, // 0x14 colour select
];

/// The 16 standard EGA/VGA colours as 6-bit DAC values, programmed explicitly
/// so the palette is exactly what the source says rather than whatever the
/// firmware left behind (mode 12h's default palette is close to this, but "close
/// to" is not something a pixel assertion should depend on).
const DAC_16: [u8; 48] = [
    0x00, 0x00, 0x00, // 0 black
    0x00, 0x00, 0x2A, // 1 blue
    0x00, 0x2A, 0x00, // 2 green
    0x00, 0x2A, 0x2A, // 3 cyan
    0x2A, 0x00, 0x00, // 4 red
    0x2A, 0x00, 0x2A, // 5 magenta
    0x2A, 0x15, 0x00, // 6 brown
    0x2A, 0x2A, 0x2A, // 7 light grey
    0x15, 0x15, 0x15, // 8 dark grey
    0x15, 0x15, 0x3F, // 9 light blue
    0x15, 0x3F, 0x15, // 10 light green
    0x15, 0x3F, 0x3F, // 11 light cyan
    0x3F, 0x15, 0x15, // 12 light red
    0x3F, 0x15, 0x3F, // 13 light magenta
    0x3F, 0x3F, 0x15, // 14 yellow
    0x3F, 0x3F, 0x3F, // 15 white
];

pub fn is_graphics() -> bool {
    unsafe { core::ptr::addr_of!(GRAPHICS).read() }
}

/// Program mode 12h and clear to black.
pub fn init() {
    unsafe {
        outb(0x3C2, 0xE3); // miscellaneous output: 25 MHz dot clock (mode 12h timing)
        wseq(0x00, 0x01); // synchronous reset
        wseq(0x01, 0x01); // clocking mode: 8 dots per character
        wseq(0x02, 0x0F); // map mask: all planes
        wseq(0x03, 0x00); // character map select
        wseq(0x04, 0x06); // memory mode: chain-4 off, odd/even off, ext memory
        wseq(0x00, 0x03); // end reset

        wcrtc(0x11, 0x00); // release CR0-CR7 write protection
        for (i, v) in CRTC_12H.iter().enumerate() {
            wcrtc(i as u8, *v);
        }
        for (i, v) in GC_12H.iter().enumerate() {
            wgc(i as u8, *v);
        }
        for (i, v) in ATTR_12H.iter().enumerate() {
            wattr(i as u8, *v);
        }
        let _ = inb(0x3DA);
        outb(0x3C0, 0x20); // re-enable video output

        // DAC: entry i takes colour i of the EGA table.
        outb(0x3C8, 0x00);
        for v in DAC_16.iter() {
            outb(0x3C9, *v);
        }

        core::ptr::addr_of_mut!(GRAPHICS).write(true);
        core::ptr::addr_of_mut!(CROW).write(0);
        core::ptr::addr_of_mut!(CCOL).write(0);
    }
    clear(0);
}

/// Put the Graphics Controller into **write mode 2**.
///
/// This is the mechanism that makes planar colour work at all: in write mode 2
/// the CPU data byte is *expanded* across the planes — bit 0 goes to plane 0,
/// bit 1 to plane 1, and so on — so writing `0b0101` paints colour 5. Write mode
/// 0 (the "obvious" choice) sends bit j of the data byte to every selected plane
/// at pixel position j, which can only ever produce colours 0 and 15; that was
/// the bug that made `GfxClear(15)` fill the screen while `GfxClear(0)` and
/// `GfxClear(4)` did nothing visible (PROGRESS TOS-P2.2).
fn write_mode_2() {
    wgc(0x01, 0x00); // set/reset off — the data byte carries the colour
    wgc(0x00, 0x00);
    wgc(0x03, 0x00); // no data rotate, no logical function
    wgc(0x05, 0x02); // write mode 2
    wseq(0x02, 0x0F); // all four planes writable
}

/// Fill the whole screen with `color`.
pub fn clear(color: i64) {
    let c = (color & 0x0F) as u8;
    unsafe {
        write_mode_2();
        wgc(0x08, 0xFF); // bit mask: whole byte
        let p = FB as *mut u8;
        for i in 0..PLANE_BYTES {
            *p.add(i) = c; // colour expands to the four planes
        }
        core::ptr::addr_of_mut!(CROW).write(0);
        core::ptr::addr_of_mut!(CCOL).write(0);
    }
}

/// Draw one pixel. Out-of-range coordinates are ignored (clipping, not a trap):
/// a drawing primitive that aborted the shell on a stray coordinate would be a
/// worse shell.
pub fn plot(x: i64, y: i64, color: i64) {
    if x < 0 || y < 0 || x >= W || y >= H {
        return;
    }
    let off = (y as usize) * ROW_BYTES + ((x as usize) >> 3);
    let bit = 0x80u8 >> ((x as usize) & 7);
    unsafe {
        write_mode_2();
        // Load the four plane latches with the CURRENT contents of this byte
        // before the masked write. With a bit mask the Graphics Controller
        // merges the write into its latches, so stale latches make the write
        // also revert bits an earlier plot set in the SAME byte — two pixels in
        // one byte and the first silently disappeared (measured; PROGRESS
        // TOS-P2.4). Any read of the aperture loads all four latches at once;
        // Read Map Select only chooses which one is handed to the CPU.
        let _ = core::ptr::read_volatile((FB + off) as *const u8);
        wgc(0x08, bit); // bit mask: this pixel only
        core::ptr::write_volatile((FB + off) as *mut u8, (color & 0x0F) as u8);
        wgc(0x08, 0xFF);
    }
}

/// Read one pixel's 4-bit colour. Out-of-range coordinates read as black.
/// Planar memory stores one bit per plane, so the colour is assembled by
/// selecting each plane with the Graphics Controller's Read Map Select.
pub fn read_pixel(x: i64, y: i64) -> u8 {
    if x < 0 || y < 0 || x >= W || y >= H {
        return 0;
    }
    let off = (y as usize) * ROW_BYTES + ((x as usize) >> 3);
    let bit = 0x80u8 >> ((x as usize) & 7);
    let mut c = 0u8;
    unsafe {
        let mut plane = 0u8;
        while plane < 4 {
            wgc(0x04, plane);
            let b = core::ptr::read_volatile((FB + off) as *const u8);
            if b & bit != 0 {
                c |= 1 << plane;
            }
            plane += 1;
        }
        wgc(0x04, 0x00);
    }
    c
}

/// Draw one 8x8 glyph of `code` with its top-left corner at (x, y).
///
/// `FONT8X8` comes from Daniel Hepper's `font8x8_basic`, which stores **bit 0 as
/// the leftmost pixel** of each row. Reading bit 7 first — the more common
/// convention, and what the table's own header comment claimed — mirrors every
/// glyph horizontally. That went unnoticed from Phase 2 to Phase 7 because every
/// graphics assertion counted lit pixels instead of reading the text; the flight
/// HUD is what finally made it visible.
pub fn glyph(x: i64, y: i64, color: i64, code: i64) {
    let idx = (code as usize) & 0x7F;
    let rows = &FONT8X8[idx];
    let mut ry = 0usize;
    while ry < 8 {
        let bits = rows[ry];
        let mut rx = 0usize;
        while rx < 8 {
            if bits & (0x01 << rx) != 0 {
                plot(x + rx as i64, y + ry as i64, color);
            }
            rx += 1;
        }
        ry += 1;
    }
}

/// Render console bytes into the framebuffer (the BC2 text overlay). Mirrors the
/// text-mode console's behaviour: `\n`, `\r`, backspace, printable ASCII, and a
/// substitute glyph for anything else.
pub fn put(bytes: &[u8], color: u8) {
    for &b in bytes {
        match b {
            b'\n' => newline(),
            b'\r' => unsafe { core::ptr::addr_of_mut!(CCOL).write(0) },
            0x08 => unsafe {
                let c = core::ptr::addr_of_mut!(CCOL);
                if *c > 0 {
                    *c -= 1;
                }
                let (c, r) = (*c, core::ptr::addr_of!(CROW).read());
                cell(c, r, b' ', color);
            },
            0x20..=0x7E => advance(b, color),
            _ => advance(b'.', color),
        }
    }
}

fn advance(ch: u8, color: u8) {
    unsafe {
        let c = core::ptr::addr_of_mut!(CCOL);
        let r = core::ptr::addr_of!(CROW).read();
        cell(*c, r, ch, color);
        *c += 1;
        if *c >= COLS {
            newline();
        }
    }
}

fn cell(col: usize, row: usize, ch: u8, color: u8) {
    // Blank the whole cell first so glyphs do not overdraw each other.
    let x = (col * 8) as i64;
    let y = (row * 8) as i64;
    let mut ry = 0i64;
    while ry < 8 {
        let mut rx = 0i64;
        while rx < 8 {
            plot(x + rx, y + ry, 0);
            rx += 1;
        }
        ry += 1;
    }
    glyph(x, y, color as i64, ch as i64);
}

fn newline() {
    unsafe {
        core::ptr::addr_of_mut!(CCOL).write(0);
        let r = core::ptr::addr_of_mut!(CROW);
        *r += 1;
        if *r >= ROWS {
            scroll();
            *r = ROWS - 1;
        }
    }
}

/// Scroll the framebuffer up by one text row. Planar memory cannot be memmoved
/// in one pass: each plane is read and written with matching map selectors.
fn scroll() {
    let shift = ROW_BYTES * 8;
    unsafe {
        wgc(0x01, 0x00); // set/reset OFF: this pass moves real data bytes
        wgc(0x05, 0x00); // read mode 0, write mode 0
        wgc(0x08, 0xFF);
        let p = FB as *mut u8;
        for plane in 0u8..4 {
            wgc(0x04, plane); // read from this plane
            wseq(0x02, 1 << plane); // write to the same plane
            core::ptr::copy(p.add(shift), p, PLANE_BYTES - shift);
            for i in (PLANE_BYTES - shift)..PLANE_BYTES {
                *p.add(i) = 0x00;
            }
        }
        wseq(0x02, 0x0F);
        wgc(0x04, 0x00);
    }
}
