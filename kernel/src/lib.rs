//! HolyRust micro-kernel (SPEC Component G / Phase 6, real-boot track P6b).
//! `#![no_std]`, single-core, identity-mapped first 1 GiB (2 MiB pages),
//! fixed physical heap 32..224 MiB (CI pins `-m 256M`).
//!
//! The HolyRust compiler (the whole `holyrust` lib, no-default-features) is
//! linked IN — the shell compiles and JIT-executes code inside the kernel,
//! and `HotPatch` recompiles a running kernel function with no reboot
//! (SPEC Phase 6 Tasks 2+3).

#![no_std]

extern crate alloc;

use alloc::format;
use core::alloc::GlobalAlloc;
use core::panic::PanicInfo;

mod font8x8;
mod gfx;
mod kbd;
mod mouse;
mod rng;
mod serial;
mod shell;
mod sound;
mod timer;
mod vga;

use holyrust::hr_runtime;

// ---------------- port I/O ----------------

#[inline(always)]
pub unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    core::arch::asm!("in al, dx", out("al") v, in("dx") port,
        options(nomem, nostack, preserves_flags));
    v
}

#[inline(always)]
pub unsafe fn outb(port: u16, v: u8) {
    core::arch::asm!("out dx, al", in("al") v, in("dx") port,
        options(nomem, nostack, preserves_flags));
}

// ---------------- bump allocator (v1: no free — arena philosophy, D017) ----------------

const HEAP_START: usize = 32 * 1024 * 1024;
const HEAP_END: usize = 224 * 1024 * 1024;
static mut HEAP_NEXT: usize = HEAP_START;

struct Bump;

unsafe impl core::alloc::GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let align = layout.align().max(16);
        let next = core::ptr::addr_of_mut!(HEAP_NEXT);
        let start = ((*next).wrapping_add(align - 1)) & !(align - 1);
        let end = match start.checked_add(layout.size()) {
            Some(e) => e,
            None => return core::ptr::null_mut(),
        };
        if end > HEAP_END || start < *next {
            return core::ptr::null_mut(); // OOM: null → Rust alloc-error → panic handler
        }
        *next = end;
        start as *mut u8
    }
    unsafe fn dealloc(&self, _p: *mut u8, _l: core::alloc::Layout) {
        // bump allocator: memory is never returned (v1, documented)
    }
    unsafe fn alloc_zeroed(&self, layout: core::alloc::Layout) -> *mut u8 {
        // RAM is zero at boot and blocks are never reused → alloc is zeroed
        self.alloc(layout)
    }
}

#[global_allocator]
static ALLOC: Bump = Bump;

/// Chunk provider for the HolyRust runtime (arena + code blobs). Identity
/// mapping makes everything RWX in v1 (documented simplification).
fn chunk_alloc(size: usize) -> *mut u8 {
    unsafe {
        match core::alloc::Layout::from_size_align(size.max(64), 16) {
            Ok(l) => ALLOC.alloc(l),
            Err(_) => core::ptr::null_mut(),
        }
    }
}

/// Halt: keyboard-controller reset pulse. Under `qemu -no-reboot` this exits
/// QEMU — that is how the scripted CI session terminates deterministically.
fn halt() -> ! {
    unsafe { outb(0x64, 0xFE) };
    loop {
        unsafe { core::arch::asm!("cli; hlt") };
    }
}

fn out_all(b: &[u8]) {
    vga::write(b);
    serial::write(b);
}
fn err_all(b: &[u8]) {
    vga::write_err(b);
    serial::write(b);
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let msg = format!("\nKERNEL PANIC: {}\n", info);
    vga::write_err(msg.as_bytes());
    serial::write(msg.as_bytes());
    halt()
}

/// `SimKey()`'s real-keyboard fallback: one poll of the PS/2 controller,
/// translated to a character code and -1 when nothing is pending. The shell's
/// line reader uses the same poller, so the two never disagree about the device —
/// they simply never run at the same time.
fn key_poll() -> i64 {
    match kbd::poll() {
        Some(c) => c as i64,
        None => -1,
    }
}

/// Enter graphics mode and reveal the mouse cursor — the cursor only makes
/// sense once there is a framebuffer to draw it into, so the two are paired
/// here rather than left for every guest to remember.
fn gfx_init_and_cursor() {
    gfx::init();
    mouse::invalidate();
    mouse::show();
}

/// Fill the screen and drop the cursor's saved background: the old pixels are
/// gone, so restoring them later would paint stale content over the new screen.
fn gfx_clear_and_forget(c: i64) {
    gfx::clear(c);
    mouse::invalidate();
}

#[no_mangle]
pub extern "C" fn kmain() -> ! {
    serial::init();
    vga::clear();
    serial::write(b"S3:rust-kmain\n");
    // hooks BEFORE Session::new (its arena grows immediately)
    unsafe {
        core::ptr::addr_of_mut!(hr_runtime::KERNEL_CHUNK).write(Some(chunk_alloc));
        core::ptr::addr_of_mut!(hr_runtime::KERNEL_HALT).write(Some(halt));
        // TempleOS-RS Phase 2: the display is a kernel service, so the kernel
        // installs it and the HolyRust runtime only forwards (see GfxHooks).
        core::ptr::addr_of_mut!(hr_runtime::GFX).write(Some(hr_runtime::GfxHooks {
            init: gfx_init_and_cursor,
            clear: gfx_clear_and_forget,
            plot: gfx::plot,
            glyph: gfx::glyph,
        }));
        // TempleOS-RS Phase 3: the PIT/CMOS clock and the pointing device.
        core::ptr::addr_of_mut!(hr_runtime::TIME).write(Some(hr_runtime::TimeHooks {
            millis: timer::millis,
            sleep_ms: timer::sleep_ms,
            date: timer::date,
        }));
        core::ptr::addr_of_mut!(hr_runtime::MOUSE).write(Some(hr_runtime::MouseHooks {
            init: mouse::init,
            x: mouse::x,
            y: mouse::y,
        }));
        // TempleOS-RS Phase 4: the speaker and the oracle's entropy source.
        core::ptr::addr_of_mut!(hr_runtime::SOUND).write(Some(hr_runtime::SoundHooks {
            beep: sound::beep,
            off: sound::off,
        }));
        core::ptr::addr_of_mut!(hr_runtime::RNG).write(Some(hr_runtime::RngHooks {
            below: rng::below,
        }));
        // Phase 7: `SimKey()` falls back to the real keyboard, so guest code can
        // read keys while it runs (see KeyHooks).
        core::ptr::addr_of_mut!(hr_runtime::KEYS).write(Some(hr_runtime::KeyHooks {
            poll: key_poll,
        }));
    }
    timer::init();
    holyrust::hr_out::set_hooks(out_all, err_all);
    serial::write(b"S4:hooks-set\n");
    shell::run();
    halt()
}
