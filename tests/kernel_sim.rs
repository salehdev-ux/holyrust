//! Phase 6 (simulation per D016/Q5) — the flagship demo as a CI test:
//! "Live kernel function edit demo runs without reboot and without memory
//! corruption (verified by a post-edit heap integrity check)."

use holyrust::hr_runtime::Session;
use holyrust::pipeline;

fn run_capture(src: &str, file: &str) -> (String, bool, Vec<String>, String) {
    let mut s = Session::new();
    s.rt_mut().capture = Some(Vec::new());
    s.rt_mut().diag_log.clear();
    let oc = pipeline::compile_and_run(&mut s, file, src);
    let cap = s.rt_mut().capture.take().unwrap_or_default();
    (
        String::from_utf8_lossy(&cap).into_owned(),
        oc.aborted,
        s.rt().diag_log.clone(),
        oc.diags.render(src),
    )
}

#[test]
fn kernel_sim_live_hotpatch() {
    let src = include_str!("../demos/kernel_sim.hr");
    let (out, aborted, rdiags, cdiags) = run_capture(src, "kernel_sim");
    assert!(!aborted, "kernel aborted: {:?}", rdiags);
    assert!(!cdiags.contains("error"), "compile errors:\n{}", cdiags);
    assert_eq!(
        out,
        "kernel: heap integrity pre-boot = 1\n\
         kernel: boot complete, pumping events...\n\
         [echo-v1] key 104\n\
         [echo-v1] key 105\n\
         kernel: >>> live-editing KbdEcho while the event loop runs <<<\n\
         kernel: hot-patch rc = 0\n\
         [ECHO-v2!!] key 33 (patched live)\n\
         [ECHO-v2!!] key 63 (patched live)\n\
         kernel: heap integrity post-patch = 1\n\
         kernel: still running — processed 4 keys, no reboot\n"
    );
}

#[test]
fn kernel_survives_bad_patch() {
    // A patch that fails to compile must NOT kill the running kernel:
    // rc = -1, diagnostic on stderr, event loop continues with the OLD body.
    let src = "I64 F(I64 c) { return c; }\nSimPush(7);\nI64 rc = HotPatch(\"I64 F( { broken\");\nPrint(\"rc=\", rc, \"\\n\");\nPrint(\"alive, next key = \", F(SimKey()), \"\\n\");\n";
    let (out, aborted, rdiags, cdiags) = run_capture(src, "kernel_sim_bad");
    assert!(!aborted, "kernel aborted: {:?}", rdiags);
    assert!(!cdiags.contains("error["), "kernel itself must compile:\n{}", cdiags);
    assert_eq!(out, "rc=-1\nalive, next key = 7\n");
}

#[test]
fn hotpatch_rejects_exe_and_keeps_running() {
    let src = "I64 F() { return 1; }\nI64 rc = HotPatch(\"#exe { Print(\\\"x\\\"); }\");\nPrint(\"rc=\", rc, \" \", F(), \"\\n\");\n";
    let (out, aborted, _, _) = run_capture(src, "kernel_sim_exe");
    assert!(!aborted);
    assert_eq!(out, "rc=-1 1\n");
}
