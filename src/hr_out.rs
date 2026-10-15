//! Output hooks (dual-mode). Guest program output and runtime diagnostics go
//! through function pointers so the SAME runtime code prints to stdout/stderr
//! on the host and to VGA+serial inside the kernel (P6b).
//!
//! Host default: std streams (set-nothing, just works). Kernel: `set_hooks`
//! at boot. Hooks are write-once before any guest code runs; afterwards they
//! are only read (host: set before worker spawn; kernel: single-core, set
//! before the shell starts). Accessed via addr_of to avoid static-mut refs.

pub type WriteHook = fn(&[u8]);

static mut OUT_HOOK: Option<WriteHook> = None;
static mut ERR_HOOK: Option<WriteHook> = None;

/// Install output sinks (kernel boot). Passing a no-op fn mutes a stream.
pub fn set_hooks(out: WriteHook, err: WriteHook) {
    unsafe {
        core::ptr::addr_of_mut!(OUT_HOOK).write(Some(out));
        core::ptr::addr_of_mut!(ERR_HOOK).write(Some(err));
    }
}

fn out_hook() -> Option<WriteHook> {
    unsafe { core::ptr::addr_of!(OUT_HOOK).read() }
}
fn err_hook() -> Option<WriteHook> {
    unsafe { core::ptr::addr_of!(ERR_HOOK).read() }
}

pub fn out_bytes(b: &[u8]) {
    match out_hook() {
        Some(f) => f(b),
        None => {
            #[cfg(feature = "host")]
            host_out(b);
        }
    }
}

pub fn err_bytes(b: &[u8]) {
    match err_hook() {
        Some(f) => f(b),
        None => {
            #[cfg(feature = "host")]
            host_err(b);
        }
    }
}

/// Line-oriented diagnostic helper (replaces eprintln! in lib code).
pub fn err_line(s: &str) {
    err_bytes(s.as_bytes());
    err_bytes(b"\n");
}

#[cfg(feature = "host")]
fn host_out(b: &[u8]) {
    use std::io::Write;
    let s = std::io::stdout();
    let mut lk = s.lock();
    let _ = lk.write_all(b);
    let _ = lk.flush(); // liveness: output appears the moment it is produced
}

#[cfg(feature = "host")]
fn host_err(b: &[u8]) {
    use std::io::Write;
    let s = std::io::stderr();
    let mut lk = s.lock();
    let _ = lk.write_all(b);
    let _ = lk.flush();
}
