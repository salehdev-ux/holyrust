//! `hr` — the HolyRust shell/driver.
//! Modes: `hr FILE.hr` (compile+run), `hr --dump-tokens FILE`,
//! `hr --dump-ast FILE`, `hr --bench` (Phase 3 latency harness),
//! `hr` (interactive shell — Phase 5).

use std::process::ExitCode;
use std::time::Instant;

fn usage() -> ExitCode {
    eprintln!(
        "hr — HolyRust shell\n\
         usage:\n  \
         hr FILE.hr            compile and run a file\n  \
         hr --dump-tokens FILE print the token stream\n  \
         hr --dump-ast FILE    print the AST (stable snapshot format)\n  \
         hr --bench            run the latency benchmark harness\n  \
         hr                    start the interactive shell"
    );
    ExitCode::from(2)
}

fn read_file(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read `{}`: {}", path, e))
}

fn run_bench() -> ExitCode {
    use holyrust::hr_runtime::Session;
    use holyrust::pipeline;

    println!("# HolyRust benchmarks (hr --bench)");
    println!();
    println!("Machine: {} CPUs, rustc-built release binary", std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0));
    println!();

    // ---- metric 1: compile+execute latency for a typical statement (≤50ms gate)
    let n = 200;
    let src = "\"Hello, world\\n\";";
    let mut sess = Session::new();
    sess.rt_mut().capture = Some(Vec::new()); // silence guest output
    // warmup
    let _ = pipeline::compile_and_run(&mut sess, "bench", src);
    let mut full: Vec<u128> = Vec::new();
    let mut comp: Vec<u128> = Vec::new();
    for _ in 0..n {
        let t0 = Instant::now();
        let r = pipeline::compile_only(&mut sess, "bench", src);
        let t1 = Instant::now();
        comp.push(t1.duration_since(t0).as_micros());
        if let Ok((entry, _, _)) = r {
            let _ = sess.run_entry(entry);
        }
        full.push(t0.elapsed().as_micros());
    }
    full.sort_unstable();
    comp.sort_unstable();
    let pct = |v: &Vec<u128>, p: f64| v[((v.len() as f64 * p) as usize).min(v.len() - 1)];
    println!("| Metric | Value | Gate | Phase |");
    println!("| :-- | --: | :-- | :-- |");
    println!(
        "| `\"Hello, world\\n\";` compile+exec (median of {}) | {:.3} ms | ≤ 50 ms | 3 |",
        n,
        pct(&full, 0.5) as f64 / 1000.0
    );
    println!(
        "| `\"Hello, world\\n\";` compile+exec (p95) | {:.3} ms | ≤ 50 ms | 3 |",
        pct(&full, 0.95) as f64 / 1000.0
    );
    println!(
        "| compile only (median) | {:.3} ms | — | 3 |",
        pct(&comp, 0.5) as f64 / 1000.0
    );

    // ---- metric 2: a heavier statement (function + loop)
    let src2 = "I64 Sum(...) { I64 t = 0; for (I64 i = 0; i < argc; i++) { t += argv[i]; } return t; } Print(Sum(1,2,3));";
    let t0 = Instant::now();
    let _ = pipeline::compile_and_run(&mut sess, "bench", src2);
    println!(
        "| variadic fn def + call compile+exec | {:.3} ms | ≤ 50 ms | 3 |",
        t0.elapsed().as_micros() as f64 / 1000.0
    );
    // ---- metric 3: ownership-check overhead, strict vs off (Phase 4 exit) ----
    let body = "Str s; Str b; I64 t = 0; for (I64 i = 0; i < 200000; i++) { s = \"x\"; b = s; t += b[0]; } Print(t, \"\\n\");";
    let off = format!("#![ownership(off)]\n{}", body);
    let time_it = |src: &str| -> (u128, String) {
        let mut best = u128::MAX;
        let mut out = String::new();
        for _ in 0..3 {
            let mut s2 = Session::new();
            s2.rt_mut().capture = Some(Vec::new());
            let t0 = Instant::now();
            let _ = pipeline::compile_and_run(&mut s2, "bench", src);
            let el = t0.elapsed().as_micros();
            out = String::from_utf8_lossy(&s2.rt_mut().capture.take().unwrap_or_default()).into_owned();
            if el < best {
                best = el;
            }
        }
        (best, out)
    };
    let (t_strict, o1) = time_it(body);
    let (t_off, o2) = time_it(&off);
    if o1.trim() != o2.trim() {
        eprintln!("hr --bench: WARNING: strict({:?}) and off({:?}) disagree", o1.trim(), o2.trim());
    }
    println!(
        "| ownership overhead: 200k-iteration move+index loop, strict vs off | {:.1} ms vs {:.1} ms ({:.2}x) | — | 4 |",
        t_strict as f64 / 1000.0,
        t_off as f64 / 1000.0,
        t_strict as f64 / (t_off as f64).max(0.001)
    );
    // ---- metric 4: Phase 6 kernel-sim demo end-to-end (incl. live hot-patch) ----
    {
        let ksrc = include_str!("../demos/kernel_sim.hr");
        let mut s3 = Session::new();
        s3.rt_mut().capture = Some(Vec::new());
        let t0 = Instant::now();
        let _ = pipeline::compile_and_run(&mut s3, "bench-kernel", ksrc);
        let el = t0.elapsed();
        println!(
            "| kernel-sim demo: boot → 4 events → live hot-patch → integrity check | {:.3} ms | (QEMU boot gate N/A in simulation, Q5) | 6 |",
            el.as_micros() as f64 / 1000.0
        );
    }
    sess.rt_mut().capture = None;
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => {
            holyrust::hr_shell::repl();
            ExitCode::SUCCESS
        }
        [b] if b == "--bench" => run_bench(),
        [flag, path] if flag == "--dump-code" => {
            let src = match read_file(path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("hr: {}", e);
                    return ExitCode::from(1);
                }
            };
            let mut sess = holyrust::hr_runtime::Session::new();
            match holyrust::pipeline::compile_only(&mut sess, path, &src) {
                Ok((_entry, diags, _len)) => {
                    if diags.has_errors() {
                        eprint!("{}", diags.render(&src));
                    }
                    match sess.blobs.last() {
                        Some(blob) => {
                            let bytes =
                                unsafe { std::slice::from_raw_parts(blob.base, blob.len) };
                            let trimmed = &bytes[.._len.min(bytes.len())];
                            let blob_path = std::env::temp_dir().join("blob.bin");
                            if let Err(e) = std::fs::write(&blob_path, trimmed) {
                                eprintln!("hr: cannot write {}: {}", blob_path.display(), e);
                                return ExitCode::from(1);
                            }
                            println!(
                                "wrote {} ({} bytes); base = {:p}; entry offset = {}",
                                blob_path.display(),
                                _len,
                                blob.base,
                                _entry - blob.base as u64
                            );
                        }
                        None => eprintln!("hr: no code blob was loaded"),
                    }
                }
                Err(diags) => {
                    eprint!("{}", diags.render(&src));
                    return ExitCode::from(1);
                }
            }
            ExitCode::SUCCESS
        }
        [flag, path] if flag == "--dump-tokens" || flag == "--dump-ast" => {
            let src = match read_file(path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("hr: {}", e);
                    return ExitCode::from(1);
                }
            };
            if flag == "--dump-tokens" {
                let (toks, diags) = holyrust::hr_lexer::tokenize(path, &src);
                for t in &toks {
                    println!(
                        "{:>4}:{:<3} {:>6}..{:<6} {}",
                        t.pos.line, t.pos.col, t.pos.offset, t.end_off, t.tok.name()
                    );
                }
                if diags.has_errors() {
                    eprint!("{}", diags.render(&src));
                    return ExitCode::from(1);
                }
            } else {
                let (prog, diags) = holyrust::hr_parser::parse_program(path, &src);
                print!("{}", holyrust::hr_parser::ast::dump_ast(&prog));
                if diags.has_errors() {
                    eprint!("{}", diags.render(&src));
                    return ExitCode::from(1);
                }
            }
            ExitCode::SUCCESS
        }
        [path] => {
            let src = match read_file(path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("hr: {}", e);
                    return ExitCode::from(1);
                }
            };
            let mut sess = holyrust::hr_runtime::Session::new();
            let out = holyrust::pipeline::compile_and_run(&mut sess, path, &src);
            if out.diags.has_errors() {
                eprint!("{}", out.diags.render(&src));
            }
            if out.aborted {
                return ExitCode::from(1);
            }
            if out.diags.has_errors() {
                return ExitCode::from(1);
            }
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}
