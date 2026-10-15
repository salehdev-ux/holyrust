//! Component F — `hr_shell` (SPEC §2.2F): the interactive shell. The shell IS
//! the compiler (TempleOS liveness): every line feeds the full pipeline and
//! executes immediately on one persistent Session, so definitions, globals and
//! hot-swapped function cells accumulate across lines.
//!
//! Streams (D036): guest output + value echo → stdout; prompts/banner/
//! diagnostics → stderr (keeps `hr < script | diff` honest). Every session
//! appends to `session.log` in the cwd: input lines, rendered diagnostics and
//! per-statement latency (the Phase 5 p95 gate reads these).

use std::io::{BufRead, Write};
use std::time::Instant;

use crate::hr_lexer::{tokenize, Tok};
use crate::hr_runtime::{Session, T_F64, T_I64, T_OBJ, T_STR, T_U0, T_U64};
use crate::pipeline;

// D043: `isatty(0/1)` was the last POSIX call outside the JIT loader.
// `std::io::IsTerminal` (stable since 1.70) is the same query, portable.
fn interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

struct Log(Option<std::fs::File>);

impl Log {
    fn open() -> Log {
        match std::fs::OpenOptions::new().create(true).append(true).open("session.log") {
            Ok(f) => Log(Some(f)),
            Err(e) => {
                eprintln!("hr: cannot open session.log: {} (continuing without log)", e);
                Log(None)
            }
        }
    }
    fn line(&mut self, s: &str) {
        if let Some(f) = self.0.as_mut() {
            let _ = writeln!(f, "{}", s);
            let _ = f.flush();
        }
    }
}

fn brace_balance(src: &str) -> i32 {
    let (toks, _) = tokenize("repl", src);
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

fn print_echo(sess: &Session) {
    let rt = sess.rt();
    if rt.last_kind < 0 {
        return;
    }
    let mut out = std::io::stdout();
    let line = match rt.last_kind as u8 {
        T_I64 => format!("{}\n", rt.last_val),
        T_U64 => format!("{}\n", rt.last_val as u64),
        T_F64 => format!("{}\n", f64::from_bits(rt.last_val as u64)),
        T_STR => {
            // Str assignment echo (bare Str statements auto-print instead, §4)
            let s = unsafe { crate::hr_runtime::debug_str_block(rt.last_val as u64) };
            if s.ends_with('\n') {
                s
            } else {
                format!("{}\n", s)
            }
        }
        T_OBJ => "<object>\n".to_string(),
        T_U0 => return,
        _ => format!("<?{}>\n", rt.last_kind),
    };
    let _ = out.write_all(line.as_bytes());
    let _ = out.flush();
}

fn help() {
    eprintln!(
        "HolyRust shell commands:\n  \
         #help      this text\n  \
         #history   list statements entered this session\n  \
         #reset     start a fresh session (new globals/functions)\n  \
         #quit      exit (also: #exit, Ctrl-D)\n\
         Anything else is HolyRust: statements execute as you type them;\n\
         multi-line input continues while braces are unbalanced.\n\
         See docs/LANG_SPEC.md for the language."
    );
}

pub fn repl() {
    let tty = interactive();
    let mut log = Log::open();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    log.line(&format!(
        "=== HolyRust session start (unix {}, pid {}) ===",
        stamp,
        std::process::id()
    ));
    eprintln!("HolyRust 0.1 — the shell is the compiler. #help for help, #quit to exit.");

    let mut sess = Session::new();
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut pending = String::new();
    let mut history: Vec<String> = Vec::new();
    let mut stmt_no = 0u64;
    let mut line_buf = String::new();

    loop {
        if tty {
            eprint!("{}", if pending.is_empty() { "hr> " } else { "... " });
            let _ = std::io::stderr().flush();
        }
        line_buf.clear();
        let n = match reader.read_line(&mut line_buf) {
            Ok(0) => break, // EOF
            Ok(n) => n,
            Err(e) => {
                eprintln!("hr: stdin error: {}", e);
                break;
            }
        };
        let _ = n;
        let line = line_buf.trim_end_matches(['\n', '\r']).to_string();
        log.line(&format!("hr> {}", line));

        // commands: exact matches, only at buffer start (never shadow #exe/#![])
        if pending.is_empty() {
            let t = line.trim();
            match t {
                "#quit" | "#exit" => {
                    log.line("=== session end (#quit) ===");
                    return;
                }
                "#help" => {
                    help();
                    continue;
                }
                "#history" => {
                    for (i, h) in history.iter().enumerate() {
                        eprintln!("{:>4}  {}", i + 1, h);
                    }
                    continue;
                }
                "#reset" => {
                    sess = Session::new();
                    eprintln!("hr: session reset (fresh globals/functions)");
                    log.line("=== #reset: fresh session ===");
                    continue;
                }
                "" => continue,
                _ => {}
            }
        }

        history.push(line.clone());
        pending.push_str(&line);
        pending.push('\n');

        if brace_balance(&pending) > 0 {
            continue; // multi-line continuation
        }

        stmt_no += 1;
        let t0 = Instant::now();
        let out = pipeline::compile_and_run(&mut sess, "repl", &pending);
        let ms = t0.elapsed().as_micros() as f64 / 1000.0;

        if out.diags.has_errors() || !out.diags.items.is_empty() {
            let rendered = out.diags.render(&pending);
            eprint!("{}", rendered);
            for l in rendered.lines() {
                log.line(&format!("! {}", l));
            }
        }
        if out.aborted {
            eprintln!("hr: statement aborted by a runtime diagnostic (session continues)");
            log.line("! aborted");
        } else {
            print_echo(&sess);
        }
        log.line(&format!("[stmt {}] {:.3}ms", stmt_no, ms));
        pending.clear();
    }
    log.line("=== session end (EOF) ===");
    if tty {
        eprintln!("\nhr: bye");
    }
}
