//! Component E — `hr_runtime` (SPEC §2.2E): mmap code loader, runtime context
//! (`Rt`, passed to generated code in r15), bump arena, string interning,
//! function/global registries, print builtin, trap & exception trampolines,
//! and the 64 MiB-stack worker thread that executes generated units.
//!
//! Rules honored here: never panic on guest-triggered paths (Part 7.5); every
//! guest failure surfaces as a diagnostic + unit abort, never a crash.

use alloc::{
    boxed::Box,
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::mem::{offset_of, transmute};

use crate::compat::Map;
use crate::hr_out;

// ---------------- mmap (declared manually — std links libc; zero crates, D005) ----------------

// D043 (Windows port): v1 allocated arena + code memory with POSIX `mmap`.
// The JIT target is x86-64 + SysV either way, so the ONLY platform-specific
// piece is "give me anonymous RW pages" and "make them executable". `code_mem`
// isolates exactly those two operations; every other line of the runtime is
// untouched. Linux keeps the identical mmap/mprotect pair it always had.

#[cfg(all(feature = "host", unix))]
mod code_mem {
    extern "C" {
        fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8;
        fn mprotect(addr: *mut u8, len: usize, prot: i32) -> i32;
    }
    const PROT_READ: i32 = 1;
    const PROT_WRITE: i32 = 2;
    const PROT_EXEC: i32 = 4;
    const MAP_PRIVATE: i32 = 2;
    const MAP_ANONYMOUS: i32 = 0x20;

    /// Anonymous RW mapping. Failure is a null or `MAP_FAILED` (-1) pointer.
    pub fn alloc_rw(len: usize) -> *mut u8 {
        unsafe {
            mmap(
                core::ptr::null_mut(),
                len,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
        }
    }
    /// Make an RW region readable + executable. `true` on success.
    pub fn make_rx(p: *mut u8, len: usize) -> bool {
        unsafe { mprotect(p, len, PROT_READ | PROT_EXEC) == 0 }
    }
}

#[cfg(all(feature = "host", windows))]
mod code_mem {
    use core::ffi::c_void;
    extern "system" {
        fn VirtualAlloc(lp: *mut c_void, size: usize, alloc_type: u32, protect: u32) -> *mut c_void;
        fn VirtualProtect(lp: *mut c_void, size: usize, new_protect: u32, old: *mut u32) -> i32;
    }
    const MEM_COMMIT_RESERVE: u32 = 0x1000 | 0x2000;
    const PAGE_READWRITE: u32 = 0x04;
    const PAGE_EXECUTE_READ: u32 = 0x20;

    /// Anonymous RW pages. Failure is a null pointer.
    pub fn alloc_rw(len: usize) -> *mut u8 {
        unsafe {
            VirtualAlloc(
                core::ptr::null_mut(),
                len,
                MEM_COMMIT_RESERVE,
                PAGE_READWRITE,
            ) as *mut u8
        }
    }
    /// Make an RW region readable + executable. `true` on success.
    pub fn make_rx(p: *mut u8, len: usize) -> bool {
        let mut old = 0u32;
        unsafe { VirtualProtect(p as *mut c_void, len, PAGE_EXECUTE_READ, &mut old) != 0 }
    }
}

// ----- kernel hooks (set at boot; `--no-default-features` builds only) -----

/// Kernel physical-memory chunk provider (identity-mapped RWX in v1).
#[cfg(not(feature = "host"))]
pub static mut KERNEL_CHUNK: Option<fn(usize) -> *mut u8> = None;
/// Kernel fatal-halt provider.
#[cfg(not(feature = "host"))]
pub static mut KERNEL_HALT: Option<fn() -> !> = None;

// ----- graphics device hooks (TempleOS-RS Phase 2) -------------------------
//
// Graphics is a *kernel service*, so the runtime must not contain a VGA driver.
// It defines only the interface guest code sees (`GfxInit`, `GfxClear`, `Plot`,
// `GfxChar`) and forwards to hooks the kernel installs at boot — the same shape
// as `hr_out`'s output hooks and `KERNEL_CHUNK`. On the host no device is
// installed, so the builtins report "no graphics device" instead of lying, and
// the host test suite states plainly what it can and cannot exercise.
//
// Everything above `Plot` (Line, Rect, Circle, TextOut) is HolyRust in
// /TOS/Graphics.HR, per SPEC_TOS §2.3.

#[derive(Clone, Copy)]
pub struct GfxHooks {
    pub init: fn(),
    pub clear: fn(i64),
    pub plot: fn(i64, i64, i64),
    pub glyph: fn(i64, i64, i64, i64),
}

/// Installed once at kernel boot, before any guest code runs.
pub static mut GFX: Option<GfxHooks> = None;

/// The device, or `None` on a build with no display (host, tests).
fn gfx_hooks() -> Option<GfxHooks> {
    unsafe { core::ptr::addr_of!(GFX).read() }
}

// ----- timer hooks (TempleOS-RS Phase 3) -----------------------------------
//
// Same shape as `GfxHooks`: `Sleep`/`Time`/`Date` are the guest-visible names,
// the kernel owns the PIT and the CMOS clock, and a build with neither reports
// honestly instead of inventing a time.

#[derive(Clone, Copy)]
pub struct TimeHooks {
    /// Milliseconds since boot (monotonic).
    pub millis: fn() -> i64,
    /// Block for at least this many milliseconds.
    pub sleep_ms: fn(i64),
    /// Calendar date as YYYYMMDD, or 0 when there is no calendar clock.
    pub date: fn() -> i64,
}

pub static mut TIME: Option<TimeHooks> = None;

fn time_hooks() -> Option<TimeHooks> {
    unsafe { core::ptr::addr_of!(TIME).read() }
}

// ----- mouse hooks (TempleOS-RS Phase 3) -----------------------------------

#[derive(Clone, Copy)]
pub struct MouseHooks {
    /// Enable the pointing device; 1 on success.
    pub init: fn() -> i64,
    pub x: fn() -> i64,
    pub y: fn() -> i64,
}

pub static mut MOUSE: Option<MouseHooks> = None;

fn mouse_hooks() -> Option<MouseHooks> {
    unsafe { core::ptr::addr_of!(MOUSE).read() }
}

// ----- sound & randomness hooks (TempleOS-RS Phase 4) ----------------------

#[derive(Clone, Copy)]
pub struct SoundHooks {
    /// Play `freq` Hz for `ms` milliseconds (0 Hz silences).
    pub beep: fn(i64, i64),
    /// Silence the speaker.
    pub off: fn(),
}

pub static mut SOUND: Option<SoundHooks> = None;

fn sound_hooks() -> Option<SoundHooks> {
    unsafe { core::ptr::addr_of!(SOUND).read() }
}

#[derive(Clone, Copy)]
pub struct RngHooks {
    /// A value in `0..n` (n <= 0 returns 0).
    pub below: fn(i64) -> i64,
}

pub static mut RNG: Option<RngHooks> = None;

fn rng_hooks() -> Option<RngHooks> {
    unsafe { core::ptr::addr_of!(RNG).read() }
}

// ----- keyboard hooks (TempleOS-RS Phase 7) --------------------------------
//
// Phase 6 gave guest code `SimKey()` for *simulated* keystrokes, but there was no
// way to read the real keyboard: the only caller of `kbd::poll()` was the shell's
// own line reader. That was invisible until a guest loop needed input while it
// ran — a flight simulator cannot steer with keys that only the shell can see.
// Rather than invent a second key builtin, `SimKey()` gains a fallback source, so
// "the next key" means the next key from wherever the machine has one.

#[derive(Clone, Copy)]
pub struct KeyHooks {
    /// Next key as a character code, or -1 when none is pending.
    pub poll: fn() -> i64,
}

pub static mut KEYS: Option<KeyHooks> = None;

fn key_hooks() -> Option<KeyHooks> {
    unsafe { core::ptr::addr_of!(KEYS).read() }
}

/// Last-resort exit: host → process::exit(3); kernel → KERNEL_HALT (or spin).
/// Only reached when no abort buffer exists (a broken invariant, never on
/// normal guest paths).
fn fatal(msg: &str) -> ! {
    hr_out::err_line(msg);
    #[cfg(feature = "host")]
    std::process::exit(3);
    #[cfg(not(feature = "host"))]
    unsafe {
        match core::ptr::addr_of!(KERNEL_HALT).read() {
            Some(f) => f(),
            None => loop {
                core::hint::spin_loop()
            },
        }
    }
}

// ---------------- trap kinds & value tags ----------------

pub const K_STACK_OVERFLOW: u64 = 1;
pub const K_DIV_ZERO: u64 = 2;
pub const K_INDEX_OOB: u64 = 3;
pub const K_UNCAUGHT_THROW: u64 = 4;
pub const K_USE_AFTER_MOVE: u64 = 10;
pub const K_BORROW_CONFLICT: u64 = 11;
pub const K_WRITE_THROUGH_SHARED: u64 = 12;
pub const K_DEREF_INVALID: u64 = 13;

pub fn trap_message(kind: u64) -> &'static str {
    match kind {
        K_STACK_OVERFLOW => "stack overflow (infinite recursion?)",
        K_DIV_ZERO => "division by zero",
        K_INDEX_OOB => "index out of bounds",
        K_UNCAUGHT_THROW => "uncaught throw",
        K_USE_AFTER_MOVE => "use of a moved value",
        K_BORROW_CONFLICT => "borrow conflict (one mutable XOR many immutable)",
        K_WRITE_THROUGH_SHARED => "write through a shared borrow (needs &mut)",
        K_DEREF_INVALID => "dereference of an invalid pointer",
        _ => "unknown runtime trap",
    }
}

// print-value tags (shared with hr_check/hr_codegen)
pub const T_I64: u8 = 0;
pub const T_U64: u8 = 1;
pub const T_F64: u8 = 2;
pub const T_STR: u8 = 3;
pub const T_U0: u8 = 4;
pub const T_OBJ: u8 = 5; // class value pointer (Phase 4 prints fields)

// Str/heap block layout (uniform 32-byte header, D007/Phase 4)
pub const H_STATE: usize = 0;
pub const H_TYPE: usize = 8;
pub const H_BORROW: usize = 16; // low32: shared count, high32: exclusive count
pub const H_LEN: usize = 24;
pub const H_DATA: usize = 32;
pub const ST_MOVED: u64 = 0;
pub const ST_OWNED: u64 = 1;
pub const ST_IMMORTAL: u64 = 2; // literals: assignment copies, never invalidates
pub const TYPE_STR: u64 = 1;
pub const CLASS_TYPE_BASE: u64 = 0x1000;

// ---------------- TempleOS-RS filesystem store (SPEC_TOS Phase 1, T1.1) --------

/// One entry of the flat namespace. Names are absolute (`/TOS/Files.HR`), which
/// is exactly what makes the OS self-describing: a path *is* the name of a file
/// inside the filesystem the OS is currently running from.
#[derive(Clone)]
pub struct FsEntry {
    pub name: String,
    pub data: Vec<u8>,
}

/// Flat, in-memory filesystem — the modern echo of TempleOS's RAM disk.
///
/// This holds **storage only**. Naming policy and the user-facing layer (`Dir`,
/// `FWrite`, `FRead`, `FDel`, `Cat`, …) are HolyRust source in `/TOS/Files.HR`,
/// so the OS's filesystem behaviour is editable like any other file
/// (SPEC_TOS §2.3). Nothing here knows what `/TOS` means.
pub struct Fs {
    pub entries: Vec<FsEntry>,
    /// Str blocks handed back to guest code by `FsRead`/`FsList`. They live in
    /// the FS's *own* arena, so a returned value can never be confused with —
    /// or invalidated by — the running unit's allocations, and the builtins
    /// need no `&mut Session` (which would alias while the guest runs).
    strs: Arena,
}

pub const FS_MAX_FILES: usize = 64;
pub const FS_MAX_FILE_BYTES: usize = 16 * 1024;

impl Fs {
    pub fn new() -> Fs {
        Fs { entries: Vec::new(), strs: Arena::new() }
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.name == name)
    }

    /// Create or overwrite. Returns `false` (never a silent truncation) when
    /// the flat namespace is full or the content exceeds one file's budget.
    pub fn write(&mut self, name: &str, data: &[u8]) -> bool {
        if data.len() > FS_MAX_FILE_BYTES {
            return false;
        }
        match self.find(name) {
            Some(i) => {
                self.entries[i].data.clear();
                self.entries[i].data.extend_from_slice(data);
                true
            }
            None => {
                if self.entries.len() >= FS_MAX_FILES {
                    return false;
                }
                self.entries.push(FsEntry { name: name.to_string(), data: data.to_vec() });
                true
            }
        }
    }

    pub fn read(&self, name: &str) -> Option<&[u8]> {
        self.find(name).map(|i| self.entries[i].data.as_slice())
    }

    pub fn delete(&mut self, name: &str) -> bool {
        match self.find(name) {
            Some(i) => {
                self.entries.remove(i);
                true
            }
            None => false,
        }
    }

    /// Index-stable within a single traversal: the guest walks 0, 1, 2 … until
    /// `FsList` reports the empty name (past the end).
    pub fn name_at(&self, i: usize) -> Option<&str> {
        self.entries.get(i).map(|e| e.name.as_str())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Mint an OWNED Str block (the representation guest code manipulates) from
    /// this FS's arena — the storage-side twin of `Session::new_str_block`.
    fn mint(&mut self, s: &str) -> u64 {
        let bytes = s.as_bytes();
        let total = H_DATA + bytes.len() + 1;
        let p = self.strs.alloc_block(total);
        unsafe {
            *(p.add(H_STATE) as *mut u64) = ST_OWNED;
            *(p.add(H_TYPE) as *mut u64) = TYPE_STR;
            *(p.add(H_LEN) as *mut u64) = bytes.len() as u64;
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), p.add(H_DATA), bytes.len());
            *p.add(H_DATA + bytes.len()) = 0;
        }
        p as u64
    }
}

// ---------------- Rt: the context generated code sees via r15 ----------------

#[repr(C)]
pub struct FnTable {
    pub unit_enter: extern "sysv64" fn(*mut Rt, u64, u64, u64),
    pub unit_exit: extern "sysv64" fn(*mut Rt),
    pub try_enter: extern "sysv64" fn(*mut Rt, u64, u64, u64),
    pub try_pop: extern "sysv64" fn(*mut Rt),
    pub do_throw: extern "sysv64" fn(*mut Rt, i64, u64),
    pub trap: extern "sysv64" fn(*mut Rt, u64, u64),
    pub print_values: extern "sysv64" fn(*mut Rt, i64, *const i64, *const u8),
    // ownership hooks — no-op memcpy/passthrough in Phase 3, real checks in Phase 4
    pub move_value: extern "sysv64" fn(*mut Rt, u64, u64, u64, u64, u64), // dst, src, size, is_str, loc
    pub check_read: extern "sysv64" fn(*mut Rt, u64, u64, u64),        // addr, kind, loc
    pub check_write: extern "sysv64" fn(*mut Rt, u64, u64, u64, u64),  // addr, kind, is_mut_ptr, loc
    pub borrow_shared: extern "sysv64" fn(*mut Rt, u64, u64) -> u64,   // addr, loc -> ptr
    pub borrow_excl: extern "sysv64" fn(*mut Rt, u64, u64) -> u64,     // addr, loc -> ptr
    pub borrow_release: extern "sysv64" fn(*mut Rt, u64, u64),         // addr, was_excl
    // Phase 6 (kernel simulation, D016) guest builtins — LANG_SPEC §14
    pub sim_push: extern "sysv64" fn(*mut Rt, i64) -> i64,
    pub sim_key: extern "sysv64" fn(*mut Rt) -> i64,
    pub hot_patch: extern "sysv64" fn(*mut Rt, u64) -> i64,
    pub heap_check: extern "sysv64" fn(*mut Rt) -> i64,
    // TempleOS-RS Phase 1 (T1.2): filesystem primitives. Raw storage only —
    // every user-facing filesystem command is HolyRust in /TOS/Files.HR.
    pub fs_write: extern "sysv64" fn(*mut Rt, u64, u64) -> i64, // (path, content) -> 1|0
    pub fs_read: extern "sysv64" fn(*mut Rt, u64) -> u64,       // path -> Str block
    pub fs_list: extern "sysv64" fn(*mut Rt, i64) -> u64,       // index -> Str block
    pub fs_delete: extern "sysv64" fn(*mut Rt, u64) -> i64,     // path -> 1|0
    /// (Str) -> I64. Added in Phase 1: `Str[i]` traps at `i == len`, so without
    /// this guest code had no way to find the end of a string.
    pub str_len: extern "sysv64" fn(*mut Rt, u64) -> i64,
    /// (Str, Str) -> I64. Non-consuming equality: a `Str` *variable* is moved
    /// when passed to a user function, so guest code could not compare one
    /// string against several literals without this.
    pub str_eq: extern "sysv64" fn(*mut Rt, u64, u64) -> i64,
    // TempleOS-RS Phase 2 (T2.1/T2.2): the graphics primitives. These are the
    // ONLY display operations in Rust — every shape above them is HolyRust.
    pub gfx_init: extern "sysv64" fn(*mut Rt) -> i64,
    pub gfx_clear: extern "sysv64" fn(*mut Rt, i64) -> i64,
    pub gfx_plot: extern "sysv64" fn(*mut Rt, i64, i64, i64) -> i64,
    pub gfx_char: extern "sysv64" fn(*mut Rt, i64, i64, i64, i64) -> i64,
    // TempleOS-RS Phase 3 (T3.x): timer and pointing device.
    pub sleep_ms: extern "sysv64" fn(*mut Rt, i64) -> i64,
    pub time_ms: extern "sysv64" fn(*mut Rt) -> i64,
    pub date: extern "sysv64" fn(*mut Rt) -> i64,
    pub mouse_init: extern "sysv64" fn(*mut Rt) -> i64,
    pub mouse_x: extern "sysv64" fn(*mut Rt) -> i64,
    pub mouse_y: extern "sysv64" fn(*mut Rt) -> i64,
    // TempleOS-RS Phase 4 (T4.x): PC speaker and the oracle's entropy source.
    pub beep: extern "sysv64" fn(*mut Rt, i64, i64) -> i64,
    pub sound_off: extern "sysv64" fn(*mut Rt) -> i64,
    pub random: extern "sysv64" fn(*mut Rt, i64) -> i64,
    // TempleOS-RS Phase 5 (T5.x): the line editor over the filesystem.
    pub ed_open: extern "sysv64" fn(*mut Rt, u64) -> i64,
    pub ed_lines: extern "sysv64" fn(*mut Rt) -> i64,
    pub ed_line: extern "sysv64" fn(*mut Rt, i64) -> u64,
    pub ed_set: extern "sysv64" fn(*mut Rt, i64, u64) -> i64,
    pub ed_find: extern "sysv64" fn(*mut Rt, u64) -> i64,
    pub ed_save: extern "sysv64" fn(*mut Rt) -> i64,
    pub ed_close: extern "sysv64" fn(*mut Rt) -> i64,
}

pub const OFF_SIM_PUSH: usize = OFF_FNS + offset_of!(FnTable, sim_push);
pub const OFF_SIM_KEY: usize = OFF_FNS + offset_of!(FnTable, sim_key);
pub const OFF_HOT_PATCH: usize = OFF_FNS + offset_of!(FnTable, hot_patch);
pub const OFF_HEAP_CHECK: usize = OFF_FNS + offset_of!(FnTable, heap_check);
pub const OFF_FS_WRITE: usize = OFF_FNS + offset_of!(FnTable, fs_write);
pub const OFF_FS_READ: usize = OFF_FNS + offset_of!(FnTable, fs_read);
pub const OFF_FS_LIST: usize = OFF_FNS + offset_of!(FnTable, fs_list);
pub const OFF_FS_DELETE: usize = OFF_FNS + offset_of!(FnTable, fs_delete);
pub const OFF_STR_LEN: usize = OFF_FNS + offset_of!(FnTable, str_len);
pub const OFF_STR_EQ: usize = OFF_FNS + offset_of!(FnTable, str_eq);
pub const OFF_GFX_INIT: usize = OFF_FNS + offset_of!(FnTable, gfx_init);
pub const OFF_GFX_CLEAR: usize = OFF_FNS + offset_of!(FnTable, gfx_clear);
pub const OFF_GFX_PLOT: usize = OFF_FNS + offset_of!(FnTable, gfx_plot);
pub const OFF_GFX_CHAR: usize = OFF_FNS + offset_of!(FnTable, gfx_char);
pub const OFF_SLEEP_MS: usize = OFF_FNS + offset_of!(FnTable, sleep_ms);
pub const OFF_TIME_MS: usize = OFF_FNS + offset_of!(FnTable, time_ms);
pub const OFF_DATE: usize = OFF_FNS + offset_of!(FnTable, date);
pub const OFF_MOUSE_INIT: usize = OFF_FNS + offset_of!(FnTable, mouse_init);
pub const OFF_MOUSE_X: usize = OFF_FNS + offset_of!(FnTable, mouse_x);
pub const OFF_MOUSE_Y: usize = OFF_FNS + offset_of!(FnTable, mouse_y);
pub const OFF_BEEP: usize = OFF_FNS + offset_of!(FnTable, beep);
pub const OFF_SOUND_OFF: usize = OFF_FNS + offset_of!(FnTable, sound_off);
pub const OFF_RANDOM: usize = OFF_FNS + offset_of!(FnTable, random);
pub const OFF_ED_OPEN: usize = OFF_FNS + offset_of!(FnTable, ed_open);
pub const OFF_ED_LINES: usize = OFF_FNS + offset_of!(FnTable, ed_lines);
pub const OFF_ED_LINE: usize = OFF_FNS + offset_of!(FnTable, ed_line);
pub const OFF_ED_SET: usize = OFF_FNS + offset_of!(FnTable, ed_set);
pub const OFF_ED_FIND: usize = OFF_FNS + offset_of!(FnTable, ed_find);
pub const OFF_ED_SAVE: usize = OFF_FNS + offset_of!(FnTable, ed_save);
pub const OFF_ED_CLOSE: usize = OFF_FNS + offset_of!(FnTable, ed_close);
/// Cap on spliced `#include` nesting — the same guard `#exe` uses (D014).
pub const MAX_INCLUDE_DEPTH: u32 = 8;

#[repr(C)]
pub struct Rt {
    pub fns: FnTable,
    pub stack_limit: u64,
    pub abort_stack: Vec<[u64; 3]>, // unit abort buffers {rsp, rbp, target}
    pub jmp_stack: Vec<[u64; 3]>,   // try/catch buffers {rsp, rbp, catch_target}
    pub throw_code: i64,
    pub throw_msg: u64, // Str block ptr or 0
    pub aborted: bool,
    pub capture: Option<Vec<u8>>, // #exe compile-time capture (Phase 5)
    pub last_val: i64,            // REPL echo of last top-level expression
    pub last_kind: i64,           // tag of last_val (T_*), -1 = none
    pub locs: Vec<String>,        // loc_id -> "file:line:col"
    pub diag_log: Vec<String>,    // runtime diagnostics, also mirrored to stderr
    /// borrow tracker (§2.2E "ownership tracker"): addr -> [shared_cnt, excl_cnt] (D030)
    pub track: Map<u64, [u32; 2]>,
    /// pre-interned Str blocks for catchable trap messages (index = kind-10)
    pub own_msgs: [u64; 4],
    /// *const Sender<WorkerMsg> for the guest→host HotPatch builtin (0 = none)
    pub patch_tx: u64,
    /// simulated keystroke queue (FIFO) for the kernel-sim demo
    pub sim_keys: Vec<i64>,
    /// kernel builds: *mut Session for the in-kernel HotPatch fast path (0 on host)
    pub host_sess: u64,
    /// snapshot of arena blocks at unit start, for HeapCheck()
    pub blocks_snapshot: Vec<(u64, usize)>,
    /// TempleOS-RS Phase 1: the flat in-memory filesystem. Field is last so
    /// every codegen-visible offset above is unchanged (offsets come from
    /// `offset_of!`, but keeping additions at the tail keeps that obvious).
    pub fs: Fs,
    /// TempleOS-RS Phase 5: the line editor's buffer — one editing session at a
    /// time, remembering where it came from so `EdSave` can write it back.
    pub ed: Option<EdBuf>,
}

/// A file opened for editing: the whole file as lines, plus its origin.
pub struct EdBuf {
    pub path: String,
    pub lines: Vec<String>,
    pub dirty: bool,
}

// Offsets consumed by hr_codegen (single source of truth; verified by test below).
pub const OFF_FNS: usize = offset_of!(Rt, fns);
pub const OFF_UNIT_ENTER: usize = OFF_FNS + offset_of!(FnTable, unit_enter);
pub const OFF_UNIT_EXIT: usize = OFF_FNS + offset_of!(FnTable, unit_exit);
pub const OFF_TRY_ENTER: usize = OFF_FNS + offset_of!(FnTable, try_enter);
pub const OFF_TRY_POP: usize = OFF_FNS + offset_of!(FnTable, try_pop);
pub const OFF_THROW: usize = OFF_FNS + offset_of!(FnTable, do_throw);
pub const OFF_TRAP: usize = OFF_FNS + offset_of!(FnTable, trap);
pub const OFF_PRINT: usize = OFF_FNS + offset_of!(FnTable, print_values);
pub const OFF_MOVE: usize = OFF_FNS + offset_of!(FnTable, move_value);
pub const OFF_CHECK_READ: usize = OFF_FNS + offset_of!(FnTable, check_read);
pub const OFF_CHECK_WRITE: usize = OFF_FNS + offset_of!(FnTable, check_write);
pub const OFF_BORROW_SHARED: usize = OFF_FNS + offset_of!(FnTable, borrow_shared);
pub const OFF_BORROW_EXCL: usize = OFF_FNS + offset_of!(FnTable, borrow_excl);
pub const OFF_BORROW_RELEASE: usize = OFF_FNS + offset_of!(FnTable, borrow_release);
pub const OFF_STACK_LIMIT: usize = offset_of!(Rt, stack_limit);
pub const OFF_THROW_CODE: usize = offset_of!(Rt, throw_code);
pub const OFF_THROW_MSG: usize = offset_of!(Rt, throw_msg);
pub const OFF_LAST_VAL: usize = offset_of!(Rt, last_val);
pub const OFF_LAST_KIND: usize = offset_of!(Rt, last_kind);

#[cfg(test)]
mod offset_tests {
    use super::*;
    #[test]
    fn rt_offsets_are_sane() {
        assert_eq!(OFF_UNIT_ENTER, 0, "fn table must start at offset 0");
        assert!(OFF_STACK_LIMIT > OFF_FNS);
        assert!(OFF_LAST_KIND > OFF_LAST_VAL);
    }
}

// ---------------- trampolines & builtin implementations ----------------

/// Restore rsp/rbp and jump into generated code. Used by traps (unit abort)
/// and throw (catch landing). Never returns.
///
/// SAFETY/ABI notes (both learned the hard way in P3 testing):
///  * operands are PINNED (rax/rdx/rcx/r8): free allocation can put the jump
///    target in a register a preceding `mov` clobbers;
///  * r15 MUST be restored: generated code keeps the runtime context in r15
///    permanently, but Rust (rt_trap/rt_do_throw and the formatting they call)
///    uses r15 as callee-saved scratch. Jumping back without restoring it
///    hands the guest a garbage context pointer.
unsafe fn trampoline(rsp: u64, rbp: u64, target: u64, ctx: u64) -> ! {
    core::arch::asm!(
        "mov rsp, rax",
        "mov rbp, rdx",
        "mov r15, r8",
        "jmp rcx",
        in("rax") rsp,
        in("rdx") rbp,
        in("rcx") target,
        in("r8") ctx,
        options(noreturn, nostack)
    );
}

fn diag_line(ctx: &mut Rt, loc: u64, msg: &str) {
    let where_ = ctx
        .locs
        .get(loc as usize)
        .cloned()
        .unwrap_or_else(|| "<unknown loc>".to_string());
    let line = format!("runtime diagnostic: {} at {}", msg, where_);
    ctx.diag_log.push(line.clone());
    hr_out::err_line(&line);
}

extern "sysv64" fn rt_unit_enter(ctx: *mut Rt, rsp: u64, rbp: u64, target: u64) {
    let ctx = unsafe { &mut *ctx };
    ctx.aborted = false;
    // borrows never live across units (lexical scoping ends at unit exit, and
    // an aborted unit leaves stale entries) — start each unit with a clean tracker
    ctx.track.clear();
    ctx.abort_stack.push([rsp, rbp, target]);
}
extern "sysv64" fn rt_unit_exit(ctx: *mut Rt) {
    let ctx = unsafe { &mut *ctx };
    ctx.abort_stack.pop();
}
extern "sysv64" fn rt_try_enter(ctx: *mut Rt, rsp: u64, rbp: u64, target: u64) {
    let ctx = unsafe { &mut *ctx };
    ctx.jmp_stack.push([rsp, rbp, target]);
}
extern "sysv64" fn rt_try_pop(ctx: *mut Rt) {
    let ctx = unsafe { &mut *ctx };
    ctx.jmp_stack.pop();
}
extern "sysv64" fn rt_do_throw(ctx: *mut Rt, code: i64, msg: u64) {
    let ctx = unsafe { &mut *ctx };
    if let Some(buf) = ctx.jmp_stack.pop() {
        ctx.throw_code = code;
        ctx.throw_msg = msg;
        let ctx_addr = ctx as *mut Rt as u64;
        unsafe { trampoline(buf[0], buf[1], buf[2], ctx_addr) };
    }
    // uncaught: unit-level diagnostic + abort
    ctx.aborted = true;
    let extra = if msg != 0 {
        format!(" (code {}, message {:?})", code, unsafe { str_from_block(msg) })
    } else {
        format!(" (code {})", code)
    };
    let line = format!("runtime diagnostic: uncaught throw{}", extra);
    ctx.diag_log.push(line.clone());
    hr_out::err_line(&line);
    if let Some(buf) = ctx.abort_stack.last().copied() {
        let ctx_addr = ctx as *mut Rt as u64;
        unsafe { trampoline(buf[0], buf[1], buf[2], ctx_addr) };
    }
    fatal("runtime diagnostic: abort with no active unit — exiting (3)");
}
extern "sysv64" fn rt_trap(ctx: *mut Rt, loc: u64, kind: u64) {
    let ctx = unsafe { &mut *ctx };
    // Ownership traps are *catchable* by guest try/catch (LANG_SPEC §7).
    // `aborted` is only set when the trap actually escapes to unit level:
    if kind >= K_USE_AFTER_MOVE {
        if let Some(buf) = ctx.jmp_stack.pop() {
            ctx.throw_code = -(kind as i64);
            let idx = (kind - K_USE_AFTER_MOVE).min(3) as usize;
            ctx.throw_msg = ctx.own_msgs[idx];
            // note: catchable ownership traps do NOT print — the handler decides
            let ctx_addr = ctx as *mut Rt as u64;
            unsafe { trampoline(buf[0], buf[1], buf[2], ctx_addr) };
        }
    }
    ctx.aborted = true;
    diag_line(ctx, loc, trap_message(kind));
    if let Some(buf) = ctx.abort_stack.last().copied() {
        let ctx_addr = ctx as *mut Rt as u64;
        unsafe { trampoline(buf[0], buf[1], buf[2], ctx_addr) };
    }
    fatal("runtime diagnostic: abort with no active unit — exiting (3)");
}

/// Public read of a Str block for shell echo purposes (defensive: corrupt or
/// null pointers yield placeholders, never a deref crash).
pub unsafe fn debug_str_block(ptr: u64) -> String {
    str_from_block(ptr)
}

/// Read an interned/arena Str block into an owned String (defensive: bounds
/// are trusted only for sane lengths; corrupt len → placeholder).
unsafe fn str_from_block(ptr: u64) -> String {
    if ptr == 0 {
        return String::new();
    }
    let len = *(ptr as *const u64).add(H_LEN / 8);
    if len > (1 << 28) {
        return "<corrupt Str>".to_string();
    }
    let bytes = core::slice::from_raw_parts((ptr as *const u8).add(H_DATA), len as usize);
    String::from_utf8_lossy(bytes).into_owned()
}

fn write_out(ctx: &mut Rt, bytes: &[u8]) {
    if let Some(cap) = ctx.capture.as_mut() {
        cap.extend_from_slice(bytes);
    } else {
        hr_out::out_bytes(bytes);
    }
}

extern "sysv64" fn rt_print_values(ctx: *mut Rt, argc: i64, argv: *const i64, tags: *const u8) {
    let ctx = unsafe { &mut *ctx };
    let mut buf: Vec<u8> = Vec::new();
    let n = argc.clamp(0, 4096);
    for i in 0..n {
        let tag = unsafe { *tags.add(i as usize) };
        let v = unsafe { *argv.add(i as usize) };
        match tag {
            T_I64 => buf.extend_from_slice(format!("{}", v).as_bytes()),
            T_U64 => buf.extend_from_slice(format!("{}", v as u64).as_bytes()),
            T_F64 => buf.extend_from_slice(format!("{}", f64::from_bits(v as u64)).as_bytes()),
            T_STR => {
                let s = unsafe { str_from_block(v as u64) };
                buf.extend_from_slice(s.as_bytes());
            }
            T_U0 => {}
            T_OBJ => {
                // Phase 4 will print fields via the type table; keep stable output.
                if v == 0 {
                    buf.extend_from_slice(b"<null object>");
                } else {
                    let state = unsafe { *(v as *const u64) };
                    if state == ST_MOVED {
                        buf.extend_from_slice(b"<moved object>");
                    } else {
                        buf.extend_from_slice(b"<object>");
                    }
                }
            }
            _ => buf.extend_from_slice(b"<bad tag>"),
        }
    }
    write_out(ctx, &buf);
}

// ---- ownership hooks (Phase 4): header state machine + borrow tracker. ----
// All hooks are TOTAL: null/unknown addresses are no-ops or traps, never
// wild dereferences (Part 7.5). Off/loose levels skip the CALLS in codegen,
// so these run only when checks are armed.

fn trap_from(ctx: *mut Rt, loc: u64, kind: u64) -> ! {
    rt_trap(ctx, loc, kind); // never returns (trampoline or fatal)
    fatal("runtime diagnostic: trap returned unexpectedly"); // belt-and-braces for the type checker
}

extern "sysv64" fn rt_move_value(ctx: *mut Rt, dst: u64, src: u64, size: u64, is_str: u64, loc: u64) {
    if dst == 0 || src == 0 {
        trap_from(ctx, loc, K_DEREF_INVALID);
    }
    if dst == src {
        return; // self-assignment: no-op
    }
    if is_str == 1 {
        // Str slot move: transfer the block pointer, invalidate the source slot
        let ptr = unsafe { *(src as *const u64) };
        if ptr == 0 {
            trap_from(ctx, loc, K_USE_AFTER_MOVE);
        }
        unsafe {
            *(dst as *mut u64) = ptr;
            *(src as *mut u64) = 0;
        }
    } else {
        // inline (class) value move: copy bytes, flip header states
        let state = unsafe { *(src as *const u64) };
        if state == ST_MOVED {
            trap_from(ctx, loc, K_USE_AFTER_MOVE);
        }
        unsafe {
            core::ptr::copy_nonoverlapping(src as *const u8, dst as *mut u8, size as usize);
            *(dst as *mut u64) = ST_OWNED;
            *(src as *mut u64) = ST_MOVED;
        }
    }
}

extern "sysv64" fn rt_check_read(ctx: *mut Rt, addr: u64, kind: u64, loc: u64) {
    if addr == 0 {
        trap_from(ctx, loc, K_DEREF_INVALID);
    }
    unsafe {
        match kind {
            0 => {
                // Str slot: null == moved (or never initialized)
                if *(addr as *const u64) == 0 {
                    trap_from(ctx, loc, K_USE_AFTER_MOVE);
                }
            }
            _ => {
                // in-band header: state at offset 0
                if *(addr as *const u64) == ST_MOVED {
                    trap_from(ctx, loc, K_USE_AFTER_MOVE);
                }
            }
        }
    }
}

extern "sysv64" fn rt_check_write(ctx: *mut Rt, addr: u64, _kind: u64, is_mut: u64, loc: u64) {
    if is_mut == 0 {
        // statically-shared pointer write: rule violation regardless of tracker
        trap_from(ctx, loc, K_WRITE_THROUGH_SHARED);
    }
    let ctxr = unsafe { &mut *ctx };
    if let Some(e) = ctxr.track.get(&addr) {
        if e[0] > 0 {
            // live shared borrows XOR this exclusive write (D032)
            trap_from(ctx, loc, K_BORROW_CONFLICT);
        }
    }
}

extern "sysv64" fn rt_borrow_shared(ctx: *mut Rt, addr: u64, loc: u64) -> u64 {
    if addr == 0 {
        trap_from(ctx, loc, K_DEREF_INVALID);
    }
    let ctxr = unsafe { &mut *ctx };
    let e = ctxr.track.entry_or_insert(addr, [0, 0]);
    if e[1] > 0 {
        trap_from(ctx, loc, K_BORROW_CONFLICT);
    }
    e[0] = e[0].saturating_add(1);
    addr
}

extern "sysv64" fn rt_borrow_excl(ctx: *mut Rt, addr: u64, loc: u64) -> u64 {
    if addr == 0 {
        trap_from(ctx, loc, K_DEREF_INVALID);
    }
    let ctxr = unsafe { &mut *ctx };
    let e = ctxr.track.entry_or_insert(addr, [0, 0]);
    if e[0] > 0 || e[1] > 0 {
        trap_from(ctx, loc, K_BORROW_CONFLICT);
    }
    e[1] = 1;
    addr
}

extern "sysv64" fn rt_borrow_release(ctx: *mut Rt, addr: u64, was_excl: u64) {
    if addr == 0 {
        return;
    }
    let ctxr = unsafe { &mut *ctx };
    if let Some(e) = ctxr.track.get_mut(&addr) {
        if was_excl == 1 {
            e[1] = e[1].saturating_sub(1);
        } else {
            e[0] = e[0].saturating_sub(1);
        }
        if e[0] == 0 && e[1] == 0 {
            ctxr.track.remove(&addr);
        }
    }
}

// ---------------- Phase 6 builtins (kernel simulation, LANG_SPEC §14) ----------------

extern "sysv64" fn rt_sim_push(ctx: *mut Rt, c: i64) -> i64 {
    let ctx = unsafe { &mut *ctx };
    ctx.sim_keys.push(c);
    0
}
/// `SimKey()` -> the next pending key, or -1.
///
/// Simulated keys (the Phase 6 demo's `SimPush`) are drained first so a scripted
/// demo stays deterministic; when that queue is empty the kernel's real keyboard
/// is consulted, which is what lets a guest loop read the keyboard at all.
extern "sysv64" fn rt_sim_key(ctx: *mut Rt) -> i64 {
    let ctx = unsafe { &mut *ctx };
    if !ctx.sim_keys.is_empty() {
        return ctx.sim_keys.remove(0);
    }
    match key_hooks() {
        Some(h) => (h.poll)(),
        None => -1,
    }
}
extern "sysv64" fn rt_hot_patch(ctx: *mut Rt, src_block: u64) -> i64 {
    let src = unsafe { str_from_block(src_block) };
    #[cfg(feature = "host")]
    {
        // host: hand the source to the main thread over the control channel;
        // the worker blocks on the ack while the host compiles (D037).
        let patch_tx = unsafe { (*ctx).patch_tx };
        if patch_tx == 0 {
            hr_out::err_line("runtime diagnostic: HotPatch unavailable (no host control channel)");
            return -1;
        }
        let tx = unsafe { &*(patch_tx as *const std::sync::mpsc::Sender<WorkerMsg>) };
        let (ack_tx, ack_rx) = std::sync::mpsc::channel::<i64>();
        if tx.send(WorkerMsg::Patch { src, ack: ack_tx }).is_err() {
            return -1;
        }
        return match ack_rx.recv() {
            Ok(rc) => rc,
            Err(_) => -1,
        };
    }
    #[cfg(not(feature = "host"))]
    {
        // kernel: single address space, single core — compile directly on the
        // Session pointer stashed by run_entry. No channel, no worker.
        let sess_ptr = unsafe { (*ctx).host_sess };
        if sess_ptr == 0 {
            hr_out::err_line("runtime diagnostic: HotPatch unavailable (no kernel session)");
            return -1;
        }
        let sess = unsafe { &mut *(sess_ptr as *mut Session) };
        sess.hotpatch_compile(&src)
    }
}
extern "sysv64" fn rt_heap_check(ctx: *mut Rt) -> i64 {
    let ctx = unsafe { &mut *ctx };
    for (addr, total) in ctx.blocks_snapshot.iter() {
        let p = *addr as *const u8;
        let can = unsafe { *(p.add(*total - 8) as *const u64) };
        if can != CANARY {
            hr_out::err_line(&format!("runtime diagnostic: heap corruption — smashed canary at block {:#x}", *addr));
            return 0;
        }
        let state = unsafe { *(p as *const u64) };
        if state > ST_IMMORTAL {
            hr_out::err_line(&format!("runtime diagnostic: heap corruption — bad state {} at block {:#x}", state, *addr));
            return 0;
        }
    }
    1
}

// ---------------- Phase 1 builtins: filesystem primitives (SPEC_TOS T1.2) ------
//
// Deliberately dumb: these four functions move bytes in and out of `Rt.fs` and
// nothing else. Every naming rule, wildcard, listing format and command lives in
// HolyRust (`/TOS/Files.HR`, `/TOS/Shell.HR`), which is what makes the OS's
// filesystem layer editable while the system runs.

/// `FsWrite(path, content)` -> 1 on success, 0 when refused (namespace full or
/// content over budget). A refusal is reported to the guest, never truncated.
extern "sysv64" fn rt_fs_write(ctx: *mut Rt, path: u64, content: u64) -> i64 {
    let ctx = unsafe { &mut *ctx };
    let name = unsafe { str_from_block(path) };
    let body = unsafe { str_from_block(content) };
    if name.is_empty() {
        return 0;
    }
    if ctx.fs.write(&name, body.as_bytes()) {
        1
    } else {
        hr_out::err_line(&format!(
            "runtime diagnostic: FsWrite refused ({} entries, {} byte budget) for {}",
            FS_MAX_FILES,
            FS_MAX_FILE_BYTES,
            name
        ));
        0
    }
}

/// `FsRead(path)` -> a fresh Str block (empty when the path does not exist, so
/// the guest can test `Str s = FsRead(p); if (s[0] == 0) { ... }`).
extern "sysv64" fn rt_fs_read(ctx: *mut Rt, path: u64) -> u64 {
    let ctx = unsafe { &mut *ctx };
    let name = unsafe { str_from_block(path) };
    let bytes = match ctx.fs.read(&name) {
        Some(b) => b.to_vec(),
        None => return ctx.fs.mint(""),
    };
    let s = String::from_utf8_lossy(&bytes).into_owned();
    ctx.fs.mint(&s)
}

/// `FsList(i)` -> the i-th name, or "" past the end (the traversal sentinel).
extern "sysv64" fn rt_fs_list(ctx: *mut Rt, index: i64) -> u64 {
    let ctx = unsafe { &mut *ctx };
    if index < 0 || index as usize >= ctx.fs.len() {
        return ctx.fs.mint("");
    }
    let name = ctx.fs.entries[index as usize].name.clone();
    ctx.fs.mint(&name)
}

/// `FsDelete(path)` -> 1 when removed, 0 when it was not there.
extern "sysv64" fn rt_fs_delete(ctx: *mut Rt, path: u64) -> i64 {
    let ctx = unsafe { &mut *ctx };
    let name = unsafe { str_from_block(path) };
    if ctx.fs.delete(&name) {
        1
    } else {
        0
    }
}

/// `StrLen(s)` -> the block's byte length.
///
/// Discovered while writing `/TOS/Files.HR`: v1 let guest code index a `Str`
/// but gave it no way to ask how long one is, and the (correct) bounds check
/// rejects `i == len`, so the usual `while (s[i] != 0)` scan traps instead of
/// terminating. Every string helper in `/TOS/Files.HR` was unbuildable until
/// this primitive existed.
extern "sysv64" fn rt_str_len(_ctx: *mut Rt, s: u64) -> i64 {
    if s == 0 {
        return 0;
    }
    let len = unsafe { *(s as *const u64).add(H_LEN / 8) };
    if len > (1 << 28) {
        return 0; // corrupt header: report empty rather than a wild length
    }
    len as i64
}

/// `StrEq(a, b)` -> 1 when both blocks hold the same bytes (0 == 0 is equal).
///
/// Non-consuming by construction: it only reads. That matters because passing a
/// `Str` **variable** to a user-defined function moves it (D008), so guest code
/// otherwise cannot test one string against a set of literals.
extern "sysv64" fn rt_str_eq(_ctx: *mut Rt, a: u64, b: u64) -> i64 {
    if a == b {
        return 1; // same block, or both null
    }
    if a == 0 || b == 0 {
        return 0;
    }
    let (la, lb) = unsafe {
        (
            *(a as *const u64).add(H_LEN / 8),
            *(b as *const u64).add(H_LEN / 8),
        )
    };
    if la > (1 << 28) || lb > (1 << 28) || la != lb {
        return 0;
    }
    let pa = (a as *const u8).wrapping_add(H_DATA);
    let pb = (b as *const u8).wrapping_add(H_DATA);
    let mut i = 0usize;
    while i < la as usize {
        if unsafe { *pa.add(i) != *pb.add(i) } {
            return 0;
        }
        i += 1;
    }
    1
}

// ---------------- Phase 2 builtins: graphics primitives (SPEC_TOS T2.2) --------
//
// Interface only. Each forwards to the device the kernel installed at boot
// (`GfxHooks`) and returns 1 when the operation reached a device, 0 when this
// build has no display — a diagnosable answer, not a silent no-op.

extern "sysv64" fn rt_gfx_init(_ctx: *mut Rt) -> i64 {
    match gfx_hooks() {
        Some(h) => {
            (h.init)();
            1
        }
        None => {
            hr_out::err_line(
                "runtime diagnostic: GfxInit — no graphics device on this build (host has none; \
                 the kernel installs one at boot)",
            );
            0
        }
    }
}

extern "sysv64" fn rt_gfx_clear(_ctx: *mut Rt, color: i64) -> i64 {
    match gfx_hooks() {
        Some(h) => {
            (h.clear)(color);
            1
        }
        None => 0,
    }
}

extern "sysv64" fn rt_gfx_plot(_ctx: *mut Rt, x: i64, y: i64, color: i64) -> i64 {
    match gfx_hooks() {
        Some(h) => {
            (h.plot)(x, y, color);
            1
        }
        None => 0,
    }
}

extern "sysv64" fn rt_gfx_char(_ctx: *mut Rt, x: i64, y: i64, color: i64, ch: i64) -> i64 {
    match gfx_hooks() {
        Some(h) => {
            (h.glyph)(x, y, color, ch);
            1
        }
        None => 0,
    }
}

// ---------------- Phase 3 builtins: timer & mouse (SPEC_TOS T3.x) --------------

/// `Sleep(ms)` -> the milliseconds actually waited (0 when this build has no
/// timer). Blocking is deliberate: v1 has no scheduler — SPEC_TOS §1.4 freezes
/// multi-tasking out — so "sleep" is a busy wait on a real time base.
extern "sysv64" fn rt_sleep_ms(_ctx: *mut Rt, ms: i64) -> i64 {
    if ms <= 0 {
        return 0;
    }
    match time_hooks() {
        Some(h) => {
            (h.sleep_ms)(ms);
            ms
        }
        None => 0,
    }
}

/// `Time()` -> milliseconds since boot (monotonic).
extern "sysv64" fn rt_time_ms(_ctx: *mut Rt) -> i64 {
    match time_hooks() {
        Some(h) => (h.millis)(),
        None => 0,
    }
}

/// `Date()` -> calendar date as YYYYMMDD, or 0 when there is no clock.
extern "sysv64" fn rt_date(_ctx: *mut Rt) -> i64 {
    match time_hooks() {
        Some(h) => (h.date)(),
        None => 0,
    }
}

/// `MouseInit()` -> 1 when a pointing device was enabled.
extern "sysv64" fn rt_mouse_init(_ctx: *mut Rt) -> i64 {
    match mouse_hooks() {
        Some(h) => (h.init)(),
        None => {
            hr_out::err_line(
                "runtime diagnostic: MouseInit — no pointing device on this build \
                 (the kernel installs one at boot)",
            );
            0
        }
    }
}

extern "sysv64" fn rt_mouse_x(_ctx: *mut Rt) -> i64 {
    match mouse_hooks() {
        Some(h) => (h.x)(),
        None => 0,
    }
}

extern "sysv64" fn rt_mouse_y(_ctx: *mut Rt) -> i64 {
    match mouse_hooks() {
        Some(h) => (h.y)(),
        None => 0,
    }
}

// ---------------- Phase 4 builtins: speaker & entropy (SPEC_TOS T4.x) ----------

/// `Beep(freq, ms)` -> 1 when a tone was played. A build with no speaker reports
/// 0 rather than pretending; non-positive frequency or duration is a no-op.
extern "sysv64" fn rt_beep(_ctx: *mut Rt, freq: i64, ms: i64) -> i64 {
    if freq <= 0 || ms <= 0 {
        return 0;
    }
    match sound_hooks() {
        Some(h) => {
            (h.beep)(freq, ms);
            1
        }
        None => 0,
    }
}

/// `SoundOff()` -> 1 when the speaker was silenced.
extern "sysv64" fn rt_sound_off(_ctx: *mut Rt) -> i64 {
    match sound_hooks() {
        Some(h) => {
            (h.off)();
            1
        }
        None => 0,
    }
}

/// `Random(n)` -> a value in `0..n`, or 0 when this build has no entropy source.
/// The result is clamped to the contract, so a caller can never index out of
/// range with it — the oracle must not be able to crash the OS.
extern "sysv64" fn rt_random(_ctx: *mut Rt, n: i64) -> i64 {
    if n <= 0 {
        return 0;
    }
    match rng_hooks() {
        Some(h) => {
            let v = (h.below)(n);
            if v < 0 {
                0
            } else if v >= n {
                n - 1
            } else {
                v
            }
        }
        None => 0,
    }
}

// ---------------- Phase 5 builtins: the line editor (SPEC_TOS T5.x) -------------
//
// The buffer lives on `Rt` next to the filesystem, so "the file you are editing"
// is part of the live session — the same object the OS boots from. Only
// line-oriented plumbing is here; the commands built on top are HolyRust in
// /TOS/Editor.HR.

/// `EdOpen(path)` -> number of lines, or -1 when the file is not in the
/// filesystem. Opening a second file abandons the first (one session at a time).
extern "sysv64" fn rt_ed_open(ctx: *mut Rt, path: u64) -> i64 {
    let ctx = unsafe { &mut *ctx };
    let name = unsafe { str_from_block(path) };
    let text = match ctx.fs.read(&name) {
        Some(b) => String::from_utf8_lossy(b).into_owned(),
        None => {
            hr_out::err_line(&format!("runtime diagnostic: EdOpen — no such file: {}", name));
            return -1;
        }
    };
    // Keep line endings out of the buffer: they are re-added on save, so a
    // platform's newline convention cannot leak into the OS's own source.
    let mut lines: Vec<String> = Vec::new();
    for l in text.split('\n') {
        lines.push(l.trim_end_matches('\r').to_string());
    }
    if lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop(); // a trailing newline is a terminator, not an empty line
    }
    let n = lines.len() as i64;
    ctx.ed = Some(EdBuf { path: name, lines, dirty: false });
    n
}

extern "sysv64" fn rt_ed_lines(ctx: *mut Rt) -> i64 {
    let ctx = unsafe { &mut *ctx };
    match ctx.ed.as_ref() {
        Some(e) => e.lines.len() as i64,
        None => 0,
    }
}

/// `EdLine(i)` -> that line as a Str, or "" when out of range.
extern "sysv64" fn rt_ed_line(ctx: *mut Rt, index: i64) -> u64 {
    let ctx = unsafe { &mut *ctx };
    let text = match ctx.ed.as_ref() {
        Some(e) if index >= 0 => e.lines.get(index as usize).cloned(),
        _ => None,
    };
    match text {
        Some(t) => ctx.fs.mint(&t),
        None => ctx.fs.mint(""),
    }
}

/// `EdSet(i, text)` -> 1 when the line was replaced. Out-of-range is refused
/// rather than extending the file by accident.
extern "sysv64" fn rt_ed_set(ctx: *mut Rt, index: i64, text: u64) -> i64 {
    let ctx = unsafe { &mut *ctx };
    let new = unsafe { str_from_block(text) };
    match ctx.ed.as_mut() {
        Some(e) if index >= 0 && (index as usize) < e.lines.len() => {
            e.lines[index as usize] = new;
            e.dirty = true;
            1
        }
        _ => 0,
    }
}

/// `EdFind(text)` -> index of the first line containing `text`, or -1.
extern "sysv64" fn rt_ed_find(ctx: *mut Rt, needle: u64) -> i64 {
    let ctx = unsafe { &mut *ctx };
    let n = unsafe { str_from_block(needle) };
    match ctx.ed.as_ref() {
        Some(e) => match e.lines.iter().position(|l| l.contains(&n)) {
            Some(i) => i as i64,
            None => -1,
        },
        None => -1,
    }
}

/// `EdSave()` -> 1 when written back, 0 when there was nothing to save or the
/// write was refused. This is the step that writes the OS's own source.
extern "sysv64" fn rt_ed_save(ctx: *mut Rt) -> i64 {
    let ctx = unsafe { &mut *ctx };
    let (path, body) = match ctx.ed.as_ref() {
        Some(e) => {
            let mut s = String::new();
            for l in e.lines.iter() {
                s.push_str(l);
                s.push('\n');
            }
            (e.path.clone(), s)
        }
        None => return 0,
    };
    if ctx.fs.write(&path, body.as_bytes()) {
        if let Some(e) = ctx.ed.as_mut() {
            e.dirty = false;
        }
        1
    } else {
        hr_out::err_line("runtime diagnostic: EdSave refused — file over budget");
        0
    }
}

/// `EdClose()` -> 1 when the buffer was discarded.
extern "sysv64" fn rt_ed_close(ctx: *mut Rt) -> i64 {
    let ctx = unsafe { &mut *ctx };
    if ctx.ed.take().is_some() {
        1
    } else {
        0
    }
}

// ---------------- bump arena with block registry (D017) ----------------

pub struct Arena {
    chunks: Vec<(*mut u8, usize)>,
    cur: *mut u8,
    cur_end: *mut u8,
    pub blocks: Vec<(u64, usize)>, // (addr, total_len) of registered blocks (HeapCheck)
}

const CANARY: u64 = 0x4852_4341_4E52_5931; // "HRCANRY1"

impl Arena {
    pub fn new() -> Arena {
        let mut a = Arena {
            chunks: Vec::new(),
            cur: core::ptr::null_mut(),
            cur_end: core::ptr::null_mut(),
            blocks: Vec::new(),
        };
        a.grow(1 << 20);
        a
    }
    fn grow(&mut self, size: usize) {
        #[cfg(feature = "host")]
        let p = code_mem::alloc_rw(size);
        #[cfg(not(feature = "host"))]
        let p = unsafe {
            match core::ptr::addr_of!(KERNEL_CHUNK).read() {
                Some(f) => f(size),
                None => core::ptr::null_mut(),
            }
        };
        if p.is_null() || p as isize == -1 {
            fatal("runtime diagnostic: arena chunk allocation failed");
        }
        self.chunks.push((p, size));
        self.cur = p;
        self.cur_end = unsafe { p.add(size) };
    }
    pub fn alloc(&mut self, size: usize) -> *mut u8 {
        let size = (size + 15) & !15;
        let need = size;
        if self.cur.is_null() || unsafe { self.cur.add(need) } > self.cur_end {
            self.grow(need.max(1 << 20));
        }
        let p = self.cur;
        self.cur = unsafe { p.add(need) };
        p
    }
    /// Allocate a registered heap block with a trailing canary (for HeapCheck).
    pub fn alloc_block(&mut self, size: usize) -> *mut u8 {
        // canary slot at the END, 8-aligned (p is 16-aligned; total multiple of 8)
        let total = (size + 8 + 7) & !7;
        let p = self.alloc(total);
        unsafe {
            core::ptr::write_bytes(p, 0, total);
            *(p.add(total - 8) as *mut u64) = CANARY;
        }
        self.blocks.push((p as u64, total));
        p
    }
    /// Walk all registered blocks, verify canaries and state fields.
    pub fn heap_check(&self) -> Result<usize, String> {
        for (addr, total) in &self.blocks {
            let p = *addr as *const u8;
            let can = unsafe { *(p.add(*total - 8) as *const u64) };
            if can != CANARY {
                return Err(format!("heap block at {:#x} has a smashed canary", *addr));
            }
            let state = unsafe { *(p as *const u64) };
            if state > ST_IMMORTAL {
                return Err(format!("heap block at {:#x} has corrupt state {}", *addr, state));
            }
        }
        Ok(self.blocks.len())
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        // intentionally leaked-by-design in v1 (process-lifetime arena); chunks
        // stay mapped so absolute addresses embedded in code blobs stay valid.
    }
}

// ---------------- Session ----------------

pub struct Blob {
    pub base: *mut u8,
    pub len: usize,
}

pub struct Session {
    pub rt: *mut Rt,
    pub arena: Arena,
    pub blobs: Vec<Blob>,
    pub fns: Vec<crate::hr_check::FnSig>,
    pub fn_by_name: Map<String, u32>,
    pub classes: Vec<crate::hr_check::ClassInfo>,
    pub class_by_name: Map<String, u32>,
    pub globals: Map<String, crate::hr_check::GlobalInfo>,
    pub lits: Map<String, u64>,
    pub tags: Vec<*const u8>,
    #[cfg(feature = "host")]
    worker: Option<Worker>,
}
unsafe impl Send for Session {}

#[cfg(feature = "host")]
struct Worker {
    tx: std::sync::mpsc::Sender<Job>,
    ctrl: std::sync::mpsc::Receiver<WorkerMsg>,
}
#[cfg(feature = "host")]
struct Job {
    entry: u64,
    ctx: u64,
}

/// Messages from the worker thread back to the host: unit completion, or a
/// guest-initiated HotPatch request (blocks on its ack while the host
/// compiles — the running kernel keeps its state; only a cell is swapped).
#[cfg(feature = "host")]
pub enum WorkerMsg {
    Done {
        value: i64,
        aborted: bool,
    },
    Patch {
        src: String,
        ack: std::sync::mpsc::Sender<i64>,
    },
}

pub struct RunResult {
    pub value: i64,
    pub aborted: bool,
}

impl Session {
    pub fn new() -> Session {
        let rt = Box::new(Rt {
            fns: FnTable {
                unit_enter: rt_unit_enter,
                unit_exit: rt_unit_exit,
                try_enter: rt_try_enter,
                try_pop: rt_try_pop,
                do_throw: rt_do_throw,
                trap: rt_trap,
                print_values: rt_print_values,
                move_value: rt_move_value,
                check_read: rt_check_read,
                check_write: rt_check_write,
                borrow_shared: rt_borrow_shared,
                borrow_excl: rt_borrow_excl,
                borrow_release: rt_borrow_release,
                sim_push: rt_sim_push,
                sim_key: rt_sim_key,
                hot_patch: rt_hot_patch,
                heap_check: rt_heap_check,
                fs_write: rt_fs_write,
                fs_read: rt_fs_read,
                fs_list: rt_fs_list,
                fs_delete: rt_fs_delete,
                str_len: rt_str_len,
                str_eq: rt_str_eq,
                gfx_init: rt_gfx_init,
                gfx_clear: rt_gfx_clear,
                gfx_plot: rt_gfx_plot,
                gfx_char: rt_gfx_char,
                sleep_ms: rt_sleep_ms,
                time_ms: rt_time_ms,
                date: rt_date,
                mouse_init: rt_mouse_init,
                mouse_x: rt_mouse_x,
                mouse_y: rt_mouse_y,
                beep: rt_beep,
                sound_off: rt_sound_off,
                random: rt_random,
                ed_open: rt_ed_open,
                ed_lines: rt_ed_lines,
                ed_line: rt_ed_line,
                ed_set: rt_ed_set,
                ed_find: rt_ed_find,
                ed_save: rt_ed_save,
                ed_close: rt_ed_close,
            },
            stack_limit: 0,
            abort_stack: Vec::new(),
            jmp_stack: Vec::new(),
            throw_code: 0,
            throw_msg: 0,
            aborted: false,
            capture: None,
            last_val: 0,
            last_kind: -1,
            locs: Vec::new(),
            diag_log: Vec::new(),
            track: Map::new(),
            own_msgs: [0; 4],
            patch_tx: 0,
            sim_keys: Vec::new(),
            host_sess: 0,
            blocks_snapshot: Vec::new(),
            fs: Fs::new(),
            ed: None,
        });
        let mut sess = Session {
            rt: Box::into_raw(rt),
            arena: Arena::new(),
            blobs: Vec::new(),
            fns: Vec::new(),
            fn_by_name: Map::new(),
            classes: Vec::new(),
            class_by_name: Map::new(),
            globals: Map::new(),
            lits: Map::new(),
            tags: Vec::new(),
            #[cfg(feature = "host")]
            worker: None,
        };
        // pre-intern the catchable-trap messages (LANG_SPEC §7/§9)
        let msgs = [
            sess.intern_str("use of a moved value (moved or never initialized)"),
            sess.intern_str("borrow conflict (rule: one mutable XOR many immutable)"),
            sess.intern_str("write through a shared borrow (needs &mut)"),
            sess.intern_str("dereference of an invalid pointer"),
        ];
        sess.rt_mut().own_msgs = msgs;
        sess
    }
    pub fn rt(&self) -> &Rt {
        unsafe { &*self.rt }
    }
    pub fn rt_mut(&mut self) -> &mut Rt {
        unsafe { &mut *self.rt }
    }

    /// Intern a string literal into a heap block: immortal, never moved.
    pub fn intern_str(&mut self, s: &str) -> u64 {
        if let Some(a) = self.lits.get(s) {
            return *a;
        }
        let bytes = s.as_bytes();
        let total = H_DATA + bytes.len() + 1;
        let p = self.arena.alloc_block(total);
        unsafe {
            *(p.add(H_STATE) as *mut u64) = ST_IMMORTAL;
            *(p.add(H_TYPE) as *mut u64) = TYPE_STR;
            *(p.add(H_LEN) as *mut u64) = bytes.len() as u64;
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), p.add(H_DATA), bytes.len());
            *p.add(H_DATA + bytes.len()) = 0;
        }
        let addr = p as u64;
        self.lits.insert(s.to_string(), addr);
        addr
    }

    /// Mutable (non-immortal) Str block, e.g. created by runtime ops in later phases.
    pub fn new_str_block(&mut self, s: &str) -> u64 {
        let bytes = s.as_bytes();
        let total = H_DATA + bytes.len() + 1;
        let p = self.arena.alloc_block(total);
        unsafe {
            *(p.add(H_STATE) as *mut u64) = ST_OWNED;
            *(p.add(H_TYPE) as *mut u64) = TYPE_STR;
            *(p.add(H_LEN) as *mut u64) = bytes.len() as u64;
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), p.add(H_DATA), bytes.len());
            *p.add(H_DATA + bytes.len()) = 0;
        }
        p as u64
    }

    pub fn alloc_zeroed(&mut self, size: usize) -> u64 {
        let p = self.arena.alloc(size);
        unsafe { core::ptr::write_bytes(p, 0, size) };
        p as u64
    }

    pub fn make_tags(&mut self, t: &[u8]) -> u64 {
        let boxed: Box<[u8]> = t.to_vec().into_boxed_slice();
        let p = Box::into_raw(boxed) as *const u8;
        self.tags.push(p);
        p as u64
    }

    pub fn loc_id(&mut self, file: &str, line: u32, col: u32) -> u32 {
        let rt = self.rt_mut();
        rt.locs.push(format!("{}:{}:{}", file, line, col));
        (rt.locs.len() - 1) as u32
    }

    /// Register a function signature; returns its index. Allocates the
    /// hot-patch indirection cell (initially 0 = "not yet compiled").
    pub fn register_fn(&mut self, mut sig: crate::hr_check::FnSig) -> u32 {
        let cell_box = Box::new(0u64);
        sig.cell = Box::into_raw(cell_box) as u64;
        let idx = self.fns.len() as u32;
        self.fn_by_name.insert(sig.name.clone(), idx);
        self.fns.push(sig);
        idx
    }
    pub fn set_fn_code(&mut self, idx: u32, addr: u64) {
        if let Some(f) = self.fns.get(idx as usize) {
            unsafe { *(f.cell as *mut u64) = addr };
        }
    }

    /// Load machine code: mmap RW → copy → patch Abs64 fixups with
    /// base+label_offset → mprotect RX. Returns (base, patch of entry).
    pub fn load_code(&mut self, code: &[u8], abs_fixups: &[(usize, usize)]) -> Result<u64, String> {
        if code.is_empty() {
            return Err("internal error: empty code blob".into());
        }
        let len = (code.len() + 4095) & !4095;
        #[cfg(feature = "host")]
        let base = code_mem::alloc_rw(len);
        #[cfg(not(feature = "host"))]
        let base = unsafe {
            match core::ptr::addr_of!(KERNEL_CHUNK).read() {
                Some(f) => f(len),
                None => core::ptr::null_mut(),
            }
        };
        if base.is_null() || base as isize == -1 {
            return Err("cannot allocate executable memory for code blob".into());
        }
        unsafe { core::ptr::copy_nonoverlapping(code.as_ptr(), base, code.len()) };
        for (at, label_off) in abs_fixups {
            if at + 8 > code.len() {
                return Err("internal error: abs fixup out of range".into());
            }
            let v = base as u64 + *label_off as u64;
            unsafe { core::ptr::copy_nonoverlapping(v.to_le_bytes().as_ptr(), base.add(*at), 8) };
        }
        #[cfg(feature = "host")]
        if !code_mem::make_rx(base, len) {
            return Err("mprotect(RX) failed for code blob".into());
        }
        // icache coherence on x86-64: serializing instruction not required for
        // cross-modifying code we never executed before; still, keep a fence.
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.blobs.push(Blob { base, len });
        Ok(base as u64)
    }

    #[cfg(feature = "host")]
    fn ensure_worker(&mut self) {
        if self.worker.is_some() {
            return;
        }
        let (tx, job_rx) = std::sync::mpsc::channel::<Job>();
        let (ctrl_tx, ctrl) = std::sync::mpsc::channel::<WorkerMsg>();
        // leak a Sender clone for the guest builtin (Rt holds its address; the
        // Session outlives every execution — see Drop note)
        let boxed = Box::new(ctrl_tx.clone());
        self.rt_mut().patch_tx = Box::into_raw(boxed) as u64;
        let ctx = self.rt as u64;
        let builder = std::thread::Builder::new()
            .name("hr-worker".into())
            .stack_size(64 << 20);
        let spawned = builder.spawn(move || {
            // set the guest stack limit relative to this thread's stack
            let probe = 0u64;
            let near_top = &probe as *const u64 as u64;
            let limit = near_top.saturating_sub(48 << 20);
            unsafe { (*(ctx as *mut Rt)).stack_limit = limit };
            while let Ok(job) = job_rx.recv() {
                let f: extern "sysv64" fn(u64) -> i64 = unsafe { transmute(job.entry as usize) };
                let value = f(job.ctx);
                let aborted = unsafe { (*(ctx as *mut Rt)).aborted };
                if ctrl_tx.send(WorkerMsg::Done { value, aborted }).is_err() {
                    break;
                }
            }
        });
        match spawned {
            Ok(_) => self.worker = Some(Worker { tx, ctrl }),
            Err(e) => hr_out::err_line(&format!("runtime diagnostic: cannot spawn worker thread: {}", e)),
        }
    }

    /// Execute a loaded entry point.
    /// Host: on the 64 MiB-stack worker thread, servicing HotPatch requests.
    /// Kernel: directly on the kernel stack (single address space, no threads).
    #[cfg(feature = "host")]
    pub fn run_entry(&mut self, entry: u64) -> RunResult {
        self.ensure_worker();
        // snapshot heap blocks for HeapCheck() during this run
        self.rt_mut().blocks_snapshot = self.arena.blocks.clone();
        let ctx = self.rt as u64;
        let worker = match self.worker.as_ref() {
            Some(w) => w,
            None => return RunResult { value: 0, aborted: true },
        };
        if worker.tx.send(Job { entry, ctx }).is_err() {
            return RunResult { value: 0, aborted: true };
        }
        // borrow split: the control loop needs &mut self for compiling patches
        // while `worker` borrows it — the receiver is accessed via raw pointer
        // (single-threaded use: the worker is blocked whenever we touch self).
        let ctrl_ptr: *const std::sync::mpsc::Receiver<WorkerMsg> = &worker.ctrl;
        loop {
            let msg = match unsafe { (*ctrl_ptr).recv() } {
                Ok(m) => m,
                Err(_) => return RunResult { value: 0, aborted: true },
            };
            match msg {
                WorkerMsg::Done { value, aborted } => return RunResult { value, aborted },
                WorkerMsg::Patch { src, ack } => {
                    let rc = self.hotpatch_compile(&src);
                    if rc == 0 {
                        // keep HeapCheck() honest for blocks created by the patch
                        self.rt_mut().blocks_snapshot = self.arena.blocks.clone();
                    }
                    let _ = ack.send(rc);
                }
            }
        }
    }

    #[cfg(not(feature = "host"))]
    pub fn run_entry(&mut self, entry: u64) -> RunResult {
        self.rt_mut().blocks_snapshot = self.arena.blocks.clone();
        // stash the session pointer so the in-guest HotPatch builtin can
        // compile directly (single-core kernel; no &mut aliasing at runtime:
        // no other borrow of *self is live while the guest runs).
        self.rt_mut().host_sess = self as *mut Session as u64;
        let ctx = self.rt as u64;
        let f: extern "sysv64" fn(u64) -> i64 = unsafe { transmute(entry as usize) };
        let value = f(ctx);
        let aborted = unsafe { (*(ctx as *mut Rt)).aborted };
        self.rt_mut().host_sess = 0;
        if aborted {
            // same hygiene as the host: an aborted unit leaves no live borrows
            self.rt_mut().track.clear();
        }
        RunResult { value, aborted }
    }

    /// Host side of HotPatch: compile `src` (function definitions) and swap
    /// cells. Never executes the patch source itself (no #exe allowed — that
    /// would need the worker, which is blocked awaiting our ack: deadlock).
    pub fn hotpatch_compile(&mut self, src: &str) -> i64 {
        let (prog, diags) = crate::hr_parser::parse_program("hotpatch", src);
        if diags.has_errors() {
            hr_out::err_bytes(diags.render(src).as_bytes());
            return -1;
        }
        let has_exe = prog.items.iter().any(|it| {
            matches!(it, crate::hr_parser::ast::Item::Stmt(crate::hr_parser::ast::Stmt::Exe(..)))
        });
        if has_exe {
            hr_out::err_line("runtime diagnostic: #exe is not allowed inside HotPatch source (v1)");
            return -1;
        }
        match crate::pipeline::compile_only(self, "hotpatch", src) {
            Ok((_entry, diags2, _len)) => {
                for d in &diags2.items {
                    hr_out::err_line(&format!("{}", d));
                }
                0
            }
            Err(diags2) => {
                hr_out::err_bytes(diags2.render(src).as_bytes());
                -1
            }
        }
    }

    pub fn heap_check(&self) -> Result<usize, String> {
        self.arena.heap_check()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // code blobs & arena intentionally stay mapped for process lifetime
        // (absolute addresses are embedded in live code; freeing = UB).
        if let Some(rt) = (!self.rt.is_null()).then_some(unsafe { Box::from_raw(self.rt) }) {
            drop(rt);
        }
        self.rt = core::ptr::null_mut();
    }
}
