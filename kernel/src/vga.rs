//! VGA console (SPEC Component G). Guest stdout is white-on-black;
//! diagnostics are bright-red so a human at the VGA console sees them too.
//!
//! TempleOS-RS Phase 2 (BC2): once the guest switches to 640x480x16 graphics
//! the same byte stream is rendered into the framebuffer with the 8x8 font, so
//! boot output, prompts and diagnostics stay visible across the mode change.
//! Serial output is unaffected either way, which is what keeps the scripted CI
//! transcript exact regardless of display mode.

use crate::gfx;

const VGA: *mut u16 = 0xB8000 as *mut u16;
const WIDTH: usize = 80;
const HEIGHT: usize = 25;

static mut ROW: usize = 0;
static mut COL: usize = 0;

pub fn clear() {
    unsafe {
        for i in 0..WIDTH * HEIGHT {
            *VGA.add(i) = 0x0F20; // ' ' white-on-black
        }
        let r = core::ptr::addr_of_mut!(ROW);
        let c = core::ptr::addr_of_mut!(COL);
        *r = 0;
        *c = 0;
    }
}

pub fn write(bytes: &[u8]) {
    if gfx::is_graphics() {
        gfx::put(bytes, 0x0F);
    } else {
        write_attr(bytes, 0x0F);
    }
}
pub fn write_err(bytes: &[u8]) {
    if gfx::is_graphics() {
        gfx::put(bytes, 0x0C); // bright red on black
    } else {
        write_attr(bytes, 0x0C);
    }
}

fn write_attr(bytes: &[u8], attr: u8) {
    for &b in bytes {
        match b {
            b'\n' => newline(),
            b'\r' => unsafe { *core::ptr::addr_of_mut!(COL) = 0 },
            0x08 => {
                // backspace: move back, blank
                unsafe {
                    let c = core::ptr::addr_of_mut!(COL);
                    if *c > 0 {
                        *c -= 1;
                    }
                    put_cell(*c, *core::ptr::addr_of!(ROW), b' ', attr);
                }
            }
            0x20..=0x7E => {
                unsafe {
                    let (c, r) = (*core::ptr::addr_of!(COL), *core::ptr::addr_of!(ROW));
                    put_cell(c, r, b, attr);
                    *core::ptr::addr_of_mut!(COL) = c + 1;
                    if *core::ptr::addr_of!(COL) >= WIDTH {
                        newline();
                    }
                }
            }
            _ => {
                // non-ASCII/control: render as '.' to keep the grid sane
                unsafe {
                    let (c, r) = (*core::ptr::addr_of!(COL), *core::ptr::addr_of!(ROW));
                    put_cell(c, r, b'.', attr);
                    *core::ptr::addr_of_mut!(COL) = c + 1;
                    if *core::ptr::addr_of!(COL) >= WIDTH {
                        newline();
                    }
                }
            }
        }
    }
}

unsafe fn put_cell(col: usize, row: usize, ch: u8, attr: u8) {
    *VGA.add(row * WIDTH + col) = ((attr as u16) << 8) | (ch as u16);
}

fn newline() {
    unsafe {
        let r = core::ptr::addr_of_mut!(ROW);
        let c = core::ptr::addr_of_mut!(COL);
        *c = 0;
        *r += 1;
        if *r >= HEIGHT {
            // scroll up one row
            core::ptr::copy(VGA.add(WIDTH), VGA, WIDTH * (HEIGHT - 1));
            for i in 0..WIDTH {
                *VGA.add(WIDTH * (HEIGHT - 1) + i) = 0x0F20;
            }
            *r = HEIGHT - 1;
        }
    }
}
