//! Phase 0 placeholder test (exit criterion: `cargo build && cargo test` green).

#[test]
fn toolchain_and_crate_skeleton_build() {
    let d = holyrust::diag::Diagnostic::error(
        "smoke.hr",
        holyrust::diag::Pos { line: 1, col: 2, offset: 1 },
        "E000",
        "placeholder",
    );
    assert_eq!(format!("{}", d), "error[E000]: placeholder (smoke.hr 1:2)");
}
