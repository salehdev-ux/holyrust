//! TempleOS-RS Phase 1 (SPEC_TOS T1.6 / T1.7) — host-side integration test for
//! the filesystem.
//!
//! This is the Test Pyramid's *unit* level (SPEC_TOS §5.1): the very same
//! HolyRust source the kernel boots is compiled and executed here, so a
//! regression in `/TOS/*.HR` is caught in milliseconds instead of a QEMU boot
//! cycle. `tools/kernel_ci.py` then re-asserts the same behaviour bare-metal,
//! which is what makes the fast test trustworthy rather than merely convenient.

use holyrust::pipeline;

mod tos_common;
use tos_common::{boot, eval};

#[test]
fn tos_boot_loads_tos_source_and_dir_lists_it() {
    // T1.5 + T1.6: the boot script loads /TOS/*.HR and `Dir("/TOS/*.HR")` lists
    // every system file — without the storage layer knowing what "/TOS" means.
    let mut sess = boot();

    // The seeded namespace is visible through the OS's own listing command.
    let listed = eval(&mut sess, "Dir(\"/TOS/*.HR\");");
    assert!(listed.contains("/TOS/Files.HR"), "listing missed Files.HR:\n{listed}");
    assert!(listed.contains("/TOS/Graphics.HR"), "listing missed Graphics.HR:\n{listed}");
    assert!(listed.contains("/TOS/Sound.HR"), "listing missed Sound.HR:\n{listed}");
    assert!(listed.contains("/TOS/Oracle.HR"), "listing missed Oracle.HR:\n{listed}");
    assert!(listed.contains("/TOS/Editor.HR"), "listing missed Editor.HR:\n{listed}");
    assert!(listed.contains("/TOS/Flight.HR"), "listing missed Flight.HR:\n{listed}");
    assert!(listed.contains("/TOS/Shell.HR"), "listing missed Shell.HR:\n{listed}");
    assert!(listed.contains("7 entries"), "expected exactly seven .HR files:\n{listed}");
    // The doc is NOT a .HR file, so the glob must exclude it.
    assert!(!listed.contains("TempleOS-RS.txt"), "glob over-matched:\n{listed}");

    // `files` (a bare shell command, dispatched by HolyRust) lists /TOS/*.
    let files = eval(&mut sess, "Sh(\"files\");");
    assert!(files.contains("/TOS/Files.HR"), "Sh(files) missed Files.HR:\n{files}");
    assert!(files.contains("/TOS/Doc/TempleOS-RS.txt"), "Sh(files) missed the doc:\n{files}");

    // The OS is self-describing: reading its own source through its own FS layer.
    let cat = eval(&mut sess, "Cat(\"/TOS/Shell.HR\");");
    assert!(cat.contains("U0 Sh(Str word)"), "Cat did not return Shell.HR:\n{cat}");
}

#[test]
fn tos_file_lifecycle_create_read_list_delete() {
    // T1.7: create, write, read, delete, list — all through the in-OS shell.
    let mut sess = boot();

    let created = eval(&mut sess, "FWrite(\"/Tmp/x.HR\", \"Print(42);\");");
    assert!(created.contains("wrote /Tmp/x.HR (10 bytes)"), "unexpected write reply:\n{created}");

    // A file the OS wrote is immediately readable, matchable and measurable.
    assert_eq!(eval(&mut sess, "Print(Exists(\"/Tmp/x.HR\"), \"\\n\");"), "1\n");
    assert_eq!(eval(&mut sess, "Print(Size(\"/Tmp/x.HR\"), \"\\n\");"), "10\n");
    assert_eq!(eval(&mut sess, "Print(FsRead(\"/Tmp/x.HR\")[0], \"\\n\");"), "80\n"); // 'P'

    let listed = eval(&mut sess, "Dir(\"/Tmp/*\");");
    assert!(listed.contains("/Tmp/x.HR"), "created file missing from listing:\n{listed}");

    // Overwrite replaces, it does not append.
    eval(&mut sess, "FWrite(\"/Tmp/x.HR\", \"Print(7);\");");
    assert_eq!(eval(&mut sess, "Print(Size(\"/Tmp/x.HR\"), \"\\n\");"), "9\n");

    // Delete, then confirm it is really gone by three independent routes.
    let deleted = eval(&mut sess, "FDel(\"/Tmp/x.HR\");");
    assert!(deleted.contains("deleted /Tmp/x.HR"), "unexpected delete reply:\n{deleted}");
    assert_eq!(eval(&mut sess, "Print(Exists(\"/Tmp/x.HR\"), \"\\n\");"), "0\n");
    assert_eq!(eval(&mut sess, "Print(Size(\"/Tmp/x.HR\"), \"\\n\");"), "-1\n");
    // Deleting something that is already gone reports the refusal *and* still
    // returns 0 — the notice is part of the contract, not an accident.
    assert_eq!(
        eval(&mut sess, "Print(FDel(\"/Tmp/x.HR\"), \"\\n\");"),
        "(no such file) /Tmp/x.HR\n0\n"
    );

    // The system files survived all of it — Count() sees exactly /TOS's eight.
    assert_eq!(eval(&mut sess, "Print(Count(), \"\\n\");"), "8\n");
}

#[test]
fn tos_glob_semantics_are_honest() {
    let mut sess = boot();
    // '*' only: prefix*suffix. No '?' or classes in v1 — assert the limits
    // rather than leaving them to be discovered.
    assert_eq!(eval(&mut sess, "Print(GlobMatch(\"*\", \"/TOS/Files.HR\"), \"\\n\");"), "1\n");
    assert_eq!(eval(&mut sess, "Print(GlobMatch(\"/TOS/*.HR\", \"/TOS/Files.HR\"), \"\\n\");"), "1\n");
    assert_eq!(eval(&mut sess, "Print(GlobMatch(\"/TOS/*.HR\", \"/TOS/Doc/x.txt\"), \"\\n\");"), "0\n");
    assert_eq!(eval(&mut sess, "Print(GlobMatch(\"/TOS/Files.HR\", \"/TOS/Files.HR\"), \"\\n\");"), "1\n");
    assert_eq!(eval(&mut sess, "Print(GlobMatch(\"/TOS/Files.HR\", \"/TOS/Shell.HR\"), \"\\n\");"), "0\n");
    // The bounds rule that shaped this whole layer: `Str[i]` traps at i == len,
    // so the NUL terminator is NOT readable and every string walk must be
    // bounded by StrLen. Pin that down here so nobody "simplifies" it back.
    assert_eq!(eval(&mut sess, "Print(\"PH\"[0], \" \", \"PH\"[1], \" \", StrLen(\"PH\"), \"\\n\");"), "80 72 2\n");
    assert_eq!(eval(&mut sess, "Print(StrLen(\"\"), \"\\n\");"), "0\n");

    sess.rt_mut().capture = Some(Vec::new());
    let oob = pipeline::compile_and_run(&mut sess, "oob", "Str s = \"ab\"; Print(s[2], \"\\n\");");
    assert!(oob.aborted, "index == len must trap, not silently read the NUL");
}

#[test]
fn tos_hotpatching_the_fs_layer_keeps_the_filesystem_working() {
    // Exit gate G1: "Hot-patching /TOS/Files.HR at runtime doesn't break the
    // FS (verify with a patch + FS use)." Recompile one function of the layer
    // mid-session, then keep using the filesystem through the *other* paths.
    let mut sess = boot();

    // Baseline: the real implementation.
    assert_eq!(eval(&mut sess, "Print(Count(), \"\\n\");"), "8\n");

    // Hot-patch a function defined in /TOS/Files.HR, exactly as a user would
    // by editing the file and re-including it (Phase 5).
    let patched = eval(&mut sess, "Print(\"patch rc = \", HotPatch(\"I64 Count() { return 4242; }\"), \"\\n\");");
    assert!(patched.contains("patch rc = 0"), "hot-patch failed:\n{patched}");
    assert_eq!(eval(&mut sess, "Print(Count(), \"\\n\");"), "4242\n");

    // ...and the filesystem still works, byte-for-byte, through every other
    // entry point. Memory integrity holds across the patch.
    let listed = eval(&mut sess, "Dir(\"/TOS/*.HR\");");
    assert!(listed.contains("/TOS/Files.HR") && listed.contains("7 entries"), "listing broke after patch:\n{listed}");
    let cat = eval(&mut sess, "Cat(\"/TOS/Files.HR\");");
    assert!(cat.contains("U0 Dir(Str pattern)"), "read broke after patch:\n{cat}");
    eval(&mut sess, "FWrite(\"/Tmp/after.hr\", \"ok\");");
    assert_eq!(eval(&mut sess, "Print(Exists(\"/Tmp/after.hr\"), Size(\"/Tmp/after.hr\"), \"\\n\");"), "12\n");
    assert_eq!(eval(&mut sess, "Print(\"heap integrity: \", HeapCheck(), \"\\n\");"), "heap integrity: 1\n");

    // The patch did not corrupt the namespace: restore the real Count() and
    // confirm the layer is fully functional again.
    let restored = eval(&mut sess, &format!("Print(\"restore rc = \", HotPatch(\"{}\"), \"\\n\");", COUNT_SRC));
    assert!(restored.contains("restore rc = 0"), "restore failed:\n{restored}");
    assert_eq!(eval(&mut sess, "Print(Count(), \"\\n\");"), "9\n");
}

/// The real `Count()` body from /TOS/Files.HR, used to restore the live patch.
/// Kept as a source literal because v1 has no way to take a function's address.
const COUNT_SRC: &str = "I64 Count() { I64 i = 0; while (1) { Str name = FsList(i); if (StrLen(name) == 0) { break; } i = i + 1; } return i; }";
