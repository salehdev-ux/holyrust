//! Phase 2 exit criteria:
//!  * all demos/*.hr parse WITHOUT errors;
//!  * all tests/bad/*.hr produce ≥1 diagnostic, never a crash (catch_unwind),
//!    and every diagnostic carries file + line:col.

use holyrust::hr_parser::parse_program;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

fn list_hr(dir: &str) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "hr").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn parse_file(path: &Path) -> Result<bool, String> {
    // returns Ok(has_errors); Err = panicked
    let p = path.to_string_lossy().into_owned();
    let src = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {}", p, e));
    let res = catch_unwind(AssertUnwindSafe(|| parse_program(&p, &src)));
    match res {
        Ok((_, diags)) => {
            for d in &diags.items {
                assert_eq!(d.file, p, "diagnostic file mismatch");
                assert!(d.pos.line >= 1 && d.pos.col >= 1, "bad position in {}", d);
            }
            Ok(diags.has_errors())
        }
        Err(_) => Err(p),
    }
}

#[test]
fn demos_parse_without_errors() {
    let demos = list_hr("demos");
    assert!(demos.len() >= 4, "expected the 4 spec demos, found {:?}", demos);
    for d in demos {
        let src = std::fs::read_to_string(&d).unwrap_or_else(|e| panic!("{}: {}", d.display(), e));
        let (_, diags) = parse_program(&d.to_string_lossy(), &src);
        assert!(
            !diags.has_errors(),
            "demo {} must parse clean:\n{}",
            d.display(),
            diags.render(&src)
        );
    }
}

#[test]
fn bad_files_diagnose_never_crash() {
    let bad = list_hr("tests/bad");
    assert!(bad.len() >= 10, "expected the bad corpus, found {:?}", bad);
    for b in bad {
        match parse_file(&b) {
            Err(name) => panic!("parser CRASHED on {}", name),
            Ok(has_errors) => assert!(has_errors, "{} produced no diagnostic", b.display()),
        }
    }
}
