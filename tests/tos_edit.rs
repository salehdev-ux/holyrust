//! TempleOS-RS Phase 5 host tests (SPEC_TOS T5.x): `#include` and the line
//! editor, including the self-edit loop that exit gate G5 is about — the OS
//! opens its own source, rewrites a function, saves, re-includes it, and the new
//! definition takes effect while the filesystem keeps working.

use holyrust::pipeline;

mod tos_common;
use tos_common::{boot, eval};

#[test]
fn include_splices_filesystem_source_into_the_unit() {
    let mut sess = boot();

    // A file that lives only in the OS's own filesystem — which is the point:
    // the OS includes *its own* storage, not a host directory.
    assert!(sess
        .rt_mut()
        .fs
        .write("/Tmp/inc.HR", b"Print(\"from the include\\n\");\nI64 Inc42() { return 42; }\n"));

    let out = eval(&mut sess, "#include \"/Tmp/inc.HR\"\nPrint(\"Inc42=\", Inc42(), \"\\n\");");
    assert_eq!(out, "from the include\nInc42=42\n");

    // A missing file is a diagnosed error, never a silent no-op.
    sess.rt_mut().capture = Some(Vec::new());
    let missing = pipeline::compile_and_run(&mut sess, "t", "#include \"/Tmp/nope.HR\"\n");
    assert!(missing.diags.has_errors(), "a missing #include must be an error");
    let rendered = missing.diags.render("");
    assert!(rendered.contains("C094"), "expected the #include diagnostic, got:\n{rendered}");

    // A cycle is bounded rather than fatal: the depth cap reports an error.
    assert!(sess.rt_mut().fs.write("/Tmp/self.HR", b"#include \"/Tmp/self.HR\"\n"));
    sess.rt_mut().capture = Some(Vec::new());
    let cyclic = pipeline::compile_and_run(&mut sess, "t", "#include \"/Tmp/self.HR\"\n");
    assert!(cyclic.diags.has_errors(), "an include cycle must be reported, not run forever");

    // Ordinary source that merely *mentions* the word is untouched.
    assert_eq!(eval(&mut sess, "Print(\"#include is a directive\\n\");"), "#include is a directive\n");
}

#[test]
fn editor_rewrites_the_os_own_source_and_the_change_takes_effect() {
    let mut sess = boot();

    // Baseline: the function as shipped in /TOS/Files.HR.
    assert_eq!(eval(&mut sess, "Print(OsName(), \"\\n\");"), "TempleOS-RS\n");

    // G5's loop, on the host: open the OS's own source with the editor...
    let lines: i64 = eval(&mut sess, "Print(EdOpen(\"/TOS/Files.HR\"), \"\\n\");")
        .trim()
        .parse()
        .expect("EdOpen must report a line count");
    assert!(lines > 50, "Files.HR should be a substantial file, got {lines} lines");

    // ...find the one-line function, rewrite exactly that line, and save...
    let idx: i64 = eval(&mut sess, "Print(EdFind(\"Str OsName\"), \"\\n\");")
        .trim()
        .parse()
        .expect("EdFind must report a line index");
    assert!(idx >= 0, "the edit target must be present in the file");
    assert_eq!(
        eval(
            &mut sess,
            &format!("Print(EdSet({idx}, \"Str OsName() {{ return \\\"self-edited\\\"; }}\"), \"\\n\");")
        ),
        "1\n"
    );
    assert_eq!(eval(&mut sess, "Print(EdSave(), \"\\n\");"), "1\n");

    // The edit really reached storage: reopening the file finds the new text.
    eval(&mut sess, "EdOpen(\"/TOS/Files.HR\");");
    assert_eq!(eval(&mut sess, "Print(EdFind(\"self-edited\") >= 0, \"\\n\");"), "1\n");

    // ...re-include the file, and the OS's behaviour changes.
    eval(&mut sess, "#include \"/TOS/Files.HR\"");
    assert_eq!(eval(&mut sess, "Print(OsName(), \"\\n\");"), "self-edited\n");

    // ...while the filesystem keeps working and memory integrity holds.
    assert_eq!(
        eval(&mut sess, "Print(\"fs ok=\", Exists(\"/TOS/Shell.HR\"), \" count=\", Count(), \" heap=\", HeapCheck(), \"\\n\");"),
        "fs ok=1 count=8 heap=1\n"
    );

    // The editor refuses to invent lines, so a bad index cannot corrupt a file.
    assert_eq!(eval(&mut sess, "Print(EdSet(999999, \"x\"), EdSet(-1, \"x\"), \"\\n\");"), "00\n");
    assert_eq!(eval(&mut sess, "Print(EdClose(), EdSave(), \"\\n\");"), "10\n", "save after close is a no-op");
}
