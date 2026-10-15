//! The in-kernel HolyRust shell (SPEC Component F, running bare-metal):
//! reads lines from PS/2 or serial, compiles them with the IN-KERNEL compiler,
//! JIT-executes them on the kernel's identity-mapped memory, and survives any
//! runtime diagnostic — the live-editable-kernel experience of Phase 6.

use alloc::{boxed::Box, string::String};

use holyrust::hr_lexer::{tokenize, Tok};
use holyrust::hr_out;
use holyrust::hr_runtime::{Session, T_F64, T_I64, T_OBJ, T_STR, T_U0, T_U64};
use holyrust::pipeline;

/// Lines executed automatically at boot, before the interactive prompt.
/// They stage the flagship demo: a KbdEcho "driver", a key pump, and the
/// pre-patch heap-integrity check.
const BOOT_SCRIPT: &[&str] = &[
    "Print(\"HolyRust kernel shell — the compiler runs IN the kernel\\n\");",
    "Print(\"heap integrity: \", HeapCheck(), \"\\n\");",
    "I64 KbdEcho(I64 c) { Print(\"[echo-v1] \", c, \"\\n\"); return 0; }",
    "I64 PumpKeys() { I64 c = 0; I64 n = 0; while ((c = SimKey()) != -1) { KbdEcho(c); n = n + 1; } return n; }",
    "SimPush(104); SimPush(105);",
    "Print(\"pumped \", PumpKeys(), \" boot keys\\n\");",
    // TempleOS-RS Phase 3: bring the pointing device and clock up at boot, so a
    // mouse exists before anyone asks for one.
    "Print(\"mouse: \", MouseInit(), \"  clock: \", Time(), \"ms  date: \", Date(), \"\\n\");",
    "\"kernel: ready — type HolyRust (serial or PS/2 keyboard)\\n\";",
];

/// TempleOS-RS system files (SPEC_TOS T1.5). The `.HR` ones are written into
/// the in-memory filesystem and then *evaluated* before the prompt, so the OS's
/// own source defines its shell and filesystem from the moment it is usable.
/// The `.txt` one is pure data — the OS's documentation, living inside the OS.
const SYSTEM_FILES: &[(&str, &str)] = &[
    ("/TOS/Files.HR", include_str!("../../tos/Files.HR")),
    ("/TOS/Graphics.HR", include_str!("../../tos/Graphics.HR")),
    ("/TOS/Sound.HR", include_str!("../../tos/Sound.HR")),
    ("/TOS/Oracle.HR", include_str!("../../tos/Oracle.HR")),
    ("/TOS/Editor.HR", include_str!("../../tos/Editor.HR")),
    ("/TOS/Flight.HR", include_str!("../../tos/Flight.HR")),
    ("/TOS/Shell.HR", include_str!("../../tos/Shell.HR")),
    ("/TOS/Doc/TempleOS-RS.txt", include_str!("../../tos/Doc/TempleOS-RS.txt")),
];

/// Seed one `/TOS` file and, for HolyRust sources, evaluate it immediately.
fn load_system_file(sess: &mut Session, path: &str, body: &str, is_source: bool) {
    if !sess.rt_mut().fs.write(path, body.as_bytes()) {
        hr_out::err_line("kernel: /TOS namespace full — system files not loaded");
        return;
    }
    if !is_source {
        return;
    }
    // Name each file as it is compiled. The boot transcript is the only window
    // into a kernel that dies before the prompt, and "which file was it?" is the
    // first question any such failure raises.
    hr_out::out_bytes(b"TOS: compiling ");
    hr_out::out_bytes(path.as_bytes());
    hr_out::out_bytes(b"\n");
    let out = pipeline::compile_and_run(sess, path, body);
    if !out.diags.items.is_empty() {
        let rendered = out.diags.render(body);
        hr_out::err_bytes(rendered.as_bytes());
    }
}

/// A line that is exactly one bare identifier is a *shell command word*, not
/// HolyRust source. Deciding this is purely lexical, so the command *policy*
/// stays in /TOS/Shell.HR (`Sh`); the kernel only routes. The word is a lexed
/// identifier, so quoting it into a call is injection-free by construction.
fn bare_command_word(src: &str) -> Option<&str> {
    let (toks, diags) = tokenize("cmd", src);
    if diags.has_errors() {
        return None;
    }
    // The stream always ends in exactly one Eof (hr_lexer §615), so it must be
    // filtered out before counting: "ls" lexes as [Ident, Eof].
    let mut it = toks.iter().filter(|t| !matches!(t.tok, Tok::Eof));
    let first = it.next()?;
    if it.next().is_some() {
        return None; // more than one token: this is source, not a command word
    }
    match &first.tok {
        Tok::Ident(_) => Some(&src[first.pos.offset as usize..first.end_off as usize]),
        _ => None,
    }
}

pub fn run() {
    // Session is leaked-by-design: it IS the machine's state (globals, fn
    // cells, arena). Single-core kernel → one &mut at a time, by construction.
    let sess: &mut Session = unsafe { &mut *Box::into_raw(Box::new(Session::new())) };

    // TempleOS-RS Phase 1: the OS boots its own source before anything else.
    for (path, body) in SYSTEM_FILES {
        load_system_file(sess, path, body, path.ends_with(".HR"));
    }
    hr_out::out_bytes(b"TOS: /TOS loaded - this OS is written in itself\n");

    // One compilation unit rather than seven: the boot script is compiled at
    // every startup, and under emulation each separate compilation is
    // measurable against the < 5 s boot gate (PROGRESS TOS-P5.2). These lines
    // share no state that requires separate units — definitions register as they
    // are checked — and their order is preserved exactly.
    let boot_src = BOOT_SCRIPT.join("\n");
    feed(sess, &boot_src);

    let mut pending = String::new();
    loop {
        prompt(!pending.is_empty());
        match read_line() {
            None => continue,
            Some(line) => {
                let t = line.trim();
                if t == "#quit" || t == "#reboot" {
                    hr_out::out_bytes(b"kernel: halting (reset)\n");
                    return;
                }
                if t == "#help" {
                    hr_out::out_bytes(b"HolyRust kernel shell: type HolyRust code, a command word (`help`), or #quit.\n");
                    continue;
                }
                if pending.is_empty() && t.is_empty() {
                    continue;
                }
                // A bare command word is dispatched by the HolyRust shell layer.
                if pending.is_empty() {
                    if let Some(word) = bare_command_word(t) {
                        let call = alloc::format!("Sh(\"{}\");", word);
                        feed(sess, &call);
                        continue;
                    }
                }
                pending.push_str(&line);
                pending.push('\n');
                if brace_balance(&pending) > 0 {
                    continue; // multi-line continuation
                }
                let src = pending.clone();
                pending.clear();
                feed(sess, &src);
            }
        }
    }
}

fn prompt(cont: bool) {
    if cont {
        hr_out::out_bytes(b"... ");
    } else {
        hr_out::out_bytes(b"hr> ");
    }
}

/// Compile + execute one unit; diagnostics never kill the kernel (§1 thesis).
fn feed(sess: &mut Session, src: &str) {
    let out = pipeline::compile_and_run(sess, "kernel", src);
    if !out.diags.items.is_empty() {
        let rendered = out.diags.render(src);
        hr_out::err_bytes(rendered.as_bytes());
    }
    if out.aborted {
        hr_out::err_line("kernel: statement aborted by a runtime diagnostic (kernel continues)");
    } else {
        print_echo(sess);
    }
}

fn print_echo(sess: &Session) {
    let rt = sess.rt();
    if rt.last_kind < 0 {
        return;
    }
    let line: String = match rt.last_kind as u8 {
        T_I64 => format!("{}\n", rt.last_val),
        T_U64 => format!("{}\n", rt.last_val as u64),
        T_F64 => format!("{}\n", f64::from_bits(rt.last_val as u64)),
        T_STR => {
            let s = unsafe { holyrust::hr_runtime::debug_str_block(rt.last_val as u64) };
            if s.ends_with('\n') {
                s
            } else {
                alloc::format!("{}\n", s)
            }
        }
        T_OBJ => alloc::string::String::from("<object>\n"),
        T_U0 => return,
        _ => alloc::format!("<?{}>\n", rt.last_kind),
    };
    hr_out::out_bytes(line.as_bytes());
}

fn brace_balance(src: &str) -> i32 {
    let (toks, _) = tokenize("kernel", src);
    let mut bal = 0i32;
    for t in &toks {
        match t.tok {
            Tok::LBrace => bal += 1,
            Tok::RBrace => bal -= 1,
            _ => {}
        }
    }
    bal
}

/// Blocking line read; polls PS/2 AND serial (serial is how scripted CI types).
fn read_line() -> Option<String> {
    let mut line = String::new();
    loop {
        let ch: Option<char> = crate::kbd::poll().or_else(|| {
            crate::serial::rx_poll().and_then(|b| match b {
                b'\r' => Some('\n'),
                0x7F => Some('\u{8}'),
                0x08 => Some('\u{8}'),
                0x20..=0x7E => Some(b as char),
                b'\n' => Some('\n'),
                _ => None,
            })
        });
        let Some(c) = ch else {
            // Idle: keep the millisecond clock accumulating and let the mouse
            // cursor follow the device while we wait for a key.
            crate::timer::poll();
            crate::mouse::poll();
            core::hint::spin_loop();
            continue;
        };
        if c == '\n' {
            hr_out::out_bytes(b"\n");
            return Some(line);
        }
        if c == '\u{8}' {
            if line.pop().is_some() {
                hr_out::out_bytes(b"\x08 \x08");
            }
            continue;
        }
        line.push(c);
        let mut buf = [0u8; 4];
        hr_out::out_bytes(c.encode_utf8(&mut buf).as_bytes());
    }
}

// format!/String helpers referenced above
use alloc::format;
