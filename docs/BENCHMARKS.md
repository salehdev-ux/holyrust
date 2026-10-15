# BENCHMARKS.md — Latency Numbers per Phase (SPEC Part 6 / SPEC_TOS Part 5)

Gates (HolyRust v1 SPEC): compile-to-executable ≤ **50 ms** per typical statement
(Phase 3); REPL ≤ **100 ms p95** per statement (Phase 5). Enforced as tests:
`tests/exec_tests.rs::hello_world_50ms_gate` (+ Phase 5 REPL gate).

Gate (TempleOS-RS SPEC_TOS §6.2): boot to a usable shell **< 5 s** (target < 1 s),
asserted by `tools/kernel_ci.py` on every run.

Methodology: `tools/bench.sh` → `hr --bench` (in-process, monotonic clock, release
build). Regenerated, never hand-edited. Kernel numbers are read off the scripted
QEMU session (TCG, `-m 256M -smp 1`, no KVM) on the same host.

Machine: 4 CPUs, rustc-built release binary

## HolyRust host compiler/JIT

| Metric | Value | Gate | Phase |
| :-- | --: | :-- | :-- |
| `"Hello, world\n";` compile+exec (median of 200) | 0.056 ms | ≤ 50 ms | 3 |
| `"Hello, world\n";` compile+exec (p95) | 0.114 ms | ≤ 50 ms | 3 |
| compile only (median) | 0.036 ms | — | 3 |
| variadic fn def + call compile+exec | 0.280 ms | ≤ 50 ms | 3 |
| ownership overhead: 200k-iteration move+index loop, strict vs off | 9.4 ms vs 3.5 ms (2.71x) | — | 4 |
| kernel-sim demo: boot → 4 events → live hot-patch → integrity check | 1.137 ms | (QEMU boot gate N/A in simulation, Q5) | 6 |

## TempleOS-RS real kernel (QEMU, TCG)

| Metric | Value | Gate | Phase |
| :-- | --: | :-- | :-- |
| QEMU start → in-kernel shell (`kernel: ready`), best observed | **1.77 s** | < 5 s | TOS-1 |
| the same, worst observed under parallel test load | 4.74 s | < 5 s | TOS-1 |
| boot work attributable to the OS's own source | 6 HolyRust files (≈800 lines) compiled in-kernel at boot | — | TOS-1 |
| `Sleep(100)` observed elapsed | 0.6 – 2.1 s | returns without hanging | TOS-3 |
| PS/2 mouse tracking, 6 × monitor `mouse_move 20 12` | (320,240) → (440,312), exact | must move | TOS-3 |
| self-edit E2E: open → rewrite line → save → `#include` → new behaviour | passes | G5 | TOS-5 |

**Reading the boot number honestly.** It is the dominant cost of a *cold* boot
that the OS compiles its own six `/TOS/*.HR` sources with its own in-kernel JIT
before the prompt appears — that is a feature, not overhead to hide, and it is
why the number is stated with its range instead of as a single figure. Under
parallel load the emulator is descheduled and the same boot measures up to
4.74 s; the gate is asserted on every CI run rather than assumed.

**Reading `Sleep` honestly.** v1 has no interrupts, so the clock is a *polled*
PIT counter (D051). Its resolution is bounded by how often it is polled, and a
polled clock expresses host scheduling as overshoot: when the emulator is
descheduled the PIT keeps counting, so one poll can absorb a multi-second stall.
The CI therefore asserts a floor (the clock must advance by at least the
requested time) and a loose ceiling (it must not hang), not a tight bound that
would be a lie on an emulated host.
