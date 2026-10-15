//! SPEC Part 5 item 8 / OPEN_QUESTIONS Q9 (strictest reading): no
//! `unwrap()/expect()/panic!/unreachable!/todo!` anywhere in src/ production
//! code. Test modules (after `#[cfg(test)]`) and the Parser's own
//! `self.expect(token, ctx)` recovery method are excluded.

use std::path::Path;

fn scan(dir: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            scan(&p, out);
            continue;
        }
        if p.extension().map(|x| x != "rs").unwrap_or(true) {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&p) else { continue };
        let idx = src.find("#[cfg(test)]");
        let prod = match idx {
            Some(i) => &src[..i],
            None => &src,
        };
        for (n, line) in prod.lines().enumerate() {
            let l = line.trim();
            if l.starts_with("//") {
                continue;
            }
            let bad = [".unwrap()", "panic!(", "unreachable!(", "todo!(", "assert!(", "assert_eq!(", "assert_ne!("]
                .iter()
                .any(|pat| l.contains(pat))
                || (l.contains(".expect(") && !l.contains("self.expect("));
            if bad {
                out.push(format!("{}:{}: {}", p.display(), n + 1, l));
            }
        }
    }
}

#[test]
fn no_panic_macros_on_user_paths() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    scan(&root, &mut hits);
    assert!(
        hits.is_empty(),
        "production code must never panic on user input (SPEC Part 7.5):\n{}",
        hits.join("\n")
    );
}
