//! TempleOS-RS Phase 2 host-side tests (SPEC_TOS T2.2 / BC1 / BC2).
//!
//! The host build has no display: `GfxInit` reports "no graphics device" and
//! every drawing call is a no-op. The *pixel* assertions therefore live in QEMU
//! (`tools/kernel_ci.py` screendump), and what is provable here is the part that
//! does not need a framebuffer — that the drawing layer compiles, that its loops
//! terminate and clip, that off-screen coordinates are ignored rather than fatal,
//! and that the API's arity contract is enforced by the checker.

use holyrust::pipeline;

mod tos_common;
use tos_common::{boot, eval};

#[test]
fn tos_graphics_reports_honestly_when_there_is_no_device() {
    let mut sess = boot();
    // /TOS/Graphics.HR was evaluated by boot(); the primitives are callable and
    // tell the truth about this build instead of pretending to draw.
    assert_eq!(eval(&mut sess, "Print(\"init=\", GfxInit(), \"\\n\");"), "init=0\n");
    assert_eq!(eval(&mut sess, "Print(\"clear=\", GfxClear(3), \"\\n\");"), "clear=0\n");
    assert_eq!(eval(&mut sess, "Print(\"plot=\", Plot(10, 10, 15), \"\\n\");"), "plot=0\n");
    assert_eq!(
        eval(&mut sess, "Print(\"char=\", GfxChar(0, 0, 15, 65), \"\\n\");"),
        "char=0\n"
    );
}

#[test]
fn tos_shapes_terminate_clip_and_survive() {
    let mut sess = boot();
    // Every shape is HolyRust on top of Plot. With no device each Plot is a
    // no-op but the LOOPS still execute, so a bounds bug or a non-terminating
    // Bresenham/midpoint loop shows up here instead of inside QEMU.
    eval(&mut sess, "Line(0, 0, 639, 479, 15);");
    eval(&mut sess, "Line(639, 0, 0, 479, 15);");
    eval(&mut sess, "Rect(10, 10, 100, 50, 12);");
    eval(&mut sess, "FillRect(0, 0, 20, 20, 1);");
    eval(&mut sess, "Circle(320, 240, 60, 14);");
    eval(&mut sess, "FillCircle(100, 100, 10, 5);");
    eval(&mut sess, "TextOut(0, 0, 15, \"TempleOS-RS\");");
    eval(&mut sess, "TextCentered(200, 15, \"centred\");");
    eval(&mut sess, "GfxDemo();");

    // Off-screen coordinates must clip silently: one stray coordinate must not
    // be able to kill the shell.
    eval(&mut sess, "Plot(-1, -1, 15); Plot(640, 480, 15); Plot(100000, 100000, 15);");
    // Degenerate shapes are legitimate drawing calls, not errors.
    eval(&mut sess, "Line(5, 5, 5, 5, 15); Circle(5, 5, 0, 15); Rect(0, 0, 0, 0, 15);");
    // Horizontal and vertical lines are the classic Bresenham edge cases.
    eval(&mut sess, "Line(0, 100, 639, 100, 7); Line(100, 0, 100, 479, 7);");

    assert_eq!(
        eval(&mut sess, "Print(\"still alive: \", HeapCheck(), \"\\n\");"),
        "still alive: 1\n"
    );
}

#[test]
fn tos_plot_arity_is_enforced_by_the_checker() {
    let mut sess = boot();
    // The drawing API's argument counts are a contract; assert it rather than
    // trusting that a wrong call would be noticed.
    sess.rt_mut().capture = Some(Vec::new());
    let out = pipeline::compile_and_run(&mut sess, "tos", "Plot(1, 2);");
    assert!(out.diags.has_errors(), "Plot with two arguments must not compile");

    sess.rt_mut().capture = Some(Vec::new());
    let out = pipeline::compile_and_run(&mut sess, "tos", "GfxChar(0, 0, 15);");
    assert!(out.diags.has_errors(), "GfxChar with three arguments must not compile");

    sess.rt_mut().capture = Some(Vec::new());
    let out = pipeline::compile_and_run(&mut sess, "tos", "Plot(1, 2, 3, 4);");
    assert!(out.diags.has_errors(), "Plot with four arguments must not compile");
}
