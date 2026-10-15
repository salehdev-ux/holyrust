# 📋 HolyRust Project — Technical Specification for AI Agents

**Document Version:** 1.0
**Status:** Ready for Execution
**Classification:** Build Specification / Agent Operating Protocol

---

## 📕 Part 0: Mandatory Reading Instructions for the Agent

> **⚠️ BEFORE EXECUTING ANYTHING, READ THIS:**
>
> 1. Read this document **in full** before executing any instruction. Do not skip sections.
> 2. This document is the **Single Source of Truth (SSOT)**. If your understanding conflicts with this document, the document is correct.
> 3. You must follow the **Agent Execution Loop** defined in Part 4 for every task, without exception.
> 4. Log every decision you make into `DECISIONS.md` with a rationale (one line minimum).
> 5. If any part of this document is ambiguous, **do not guess**. Record the question in `OPEN_QUESTIONS.md` and take the most **conservative** interpretation, documented.
> 6. Never mark a phase complete unless all **Exit Criteria** (Part 5) are verified.
> 7. Scope discipline: If you are tempted to add a feature not listed here, STOP. Record it in `FUTURE_IDEAS.md` instead and continue with the specified scope.

---

## 🎯 Part 1: Project Definition

### 1.1 Vision Statement

Build **HolyRust**: a programming language and minimal operating environment that unifies three principles:

1. **Rust-style memory safety** — ownership and borrow semantics, applied at configurable strictness levels.
2. **TempleOS-style liveness** — Just-In-Time compilation; the shell *is* the compiler; code executes the moment it is written.
3. **Deliberate simplicity** — no LLVM, no external dependencies, one small self-contained compiler.

### 1.2 The Problem Being Solved

| Gap | Current State | HolyRust Solution |
| :--- | :--- | :--- |
| Safety + Interactivity | No language combines memory safety with a fast live REPL | Native REPL with live safety checking |
| Safety + Live system mutation | No OS allows safe hot-editing of running code | Kernel-editable-while-running with memory safety |
| Macro complexity | Rust proc-macros are heavyweight | `#exe {}` compile-time execution blocks |

### 1.3 Non-Goals (Explicitly Out of Scope)

- ❌ Full Rust language compatibility (no std, no cargo, no crates.io).
- ❌ Multi-threading / async runtime in v1.
- ❌ Performance parity with rustc+LLVM (accept 20–30% slower code; developer velocity is the metric).
- ❌ Supporting architectures other than x86-64 in v1.
- ❌ Network stack, filesystem persistence beyond a RAM disk.

### 1.4 Guiding Principles (in priority order)

1. **Correctness over speed.** A working small thing beats a broken big thing.
2. **Liveness over optimization.** Millisecond compile cycles are a hard requirement.
3. **Explicitness over magic.** Every language rule must be documented in `LANG_SPEC.md`.
4. **Testability.** Every subsystem must be testable in isolation.
5. **Self-containment.** The bootstrapped system must build itself with no external tooling.

---

## 🏗️ Part 2: System Architecture

### 2.1 High-Level Component Map

```
┌─────────────────────────────────────────────────────────┐
│                    HolyRust System                       │
│                                                          │
│  ┌──────────────┐   ┌──────────────┐   ┌─────────────┐  │
│  │   Shell/REPL │──▶│   JIT        │──▶│ CodeGen     │  │
│  │  (interactive)│   │ Compiler     │   │ (Cranelift) │  │
│  └──────┬───────┘   └──────┬───────┘   └──────┬──────┘  │
│         │                  │                   │         │
│         ▼                  │                   ▼         │
│  ┌──────────────────────────────────────────────────┐   │
│  │           Runtime Core                            │   │
│  │  (ownership tracker, allocator, doc/graphics)     │   │
│  └──────────────────────────────────────────────────┘   │
│                          │                               │
│                          ▼                               │
│  ┌──────────────────────────────────────────────────┐   │
│  │        Micro-Kernel (Phase 6, optional)           │   │
│  │  (boot, drivers, memory — hot-editable)           │   │
│  └──────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────┘
```

### 2.2 Component Specifications

#### Component A: The Lexer (`hr_lexer`)
- Input: UTF-8 source text.
- Output: token stream with positions (line, column).
- Requirements: full position tracking for error messages; zero external dependencies.

#### Component B: The Parser (`hr_parser`)
- Input: token stream.
- Output: Abstract Syntax Tree (AST).
- Requirement: **error recovery** — a parse error must report the location and continue parsing (critical for REPL usability).

#### Component C: The Semantic Checker (`hr_check`)
- Input: AST.
- Output: validated AST + diagnostics.
- This is the **configurable strictness layer** (see 2.4).

#### Component D: The JIT Code Generator (`hr_codegen`)
- Input: validated AST.
- Output: executable machine code in memory.
- Backend: **Cranelift** (small, fast, no LLVM). If Cranelift integration fails in Phase 3, fallback: emit x86-64 assembly via `cc` and invoke `mmap` + execution.
- Hard requirement: **compile-to-executable in under 50ms for typical statements.**

#### Component E: The Runtime Core (`hr_runtime`)
- Ownership tracker, bump/region allocator, minimal print/alloc/free builtins.
- `U0` as a true zero-sized type (not Rust's `()` semantics — verify zero size in tests).

#### Component F: The Shell/REPL (`hr_shell`)
- Reads lines; feeds the lexer→parser→checker→codegen pipeline line-by-line.
- Statements outside functions execute immediately (HolyC semantics).
- Every shell session is logged to `session.log` for debugging.

#### Component G: The Kernel (`hr_kernel`) — Phase 6 only
- Boots on QEMU (x86-64), VGA text mode, PS/2 keyboard.
- Kernel functions editable from a live shell (hot-patch: recompile function, swap pointer, preserve state).
- **This is the flagship demo feature.** If it works, the project's thesis is proven.

### 2.3 The Ownership Model (the core design decision)

Implement **runtime ownership verification** instead of compile-time-only checking:

- Every value carries a header: `{ refcount_or_owner_id, state: Owned|Borrowed|Moved, type_id }`.
- Moving invalidates the old binding **at runtime** — accessing a moved value raises a live diagnostic in the shell instead of a crash or silent UB.
- Borrow rules (one mutable XOR many immutable) are checked at pointer-dereference time, not compile time.

**Rationale:** This is the deliberate inversion of Rust: "if it compiles, it's safe" becomes "it always runs; errors are caught the instant they occur." Document this tradeoff explicitly in `LANG_SPEC.md` §1.

**Performance note:** Runtime checking has overhead. Provide `#![ownership(off)]` at file/block level for hot paths (kernel code, benchmarked loops). Strictness levels: `off | loose | strict`.

### 2.4 Language Feature Set (v1 — frozen scope)

| # | Feature | Source | Status |
| :- | :--- | :--- | :--- |
| L1 | Types: `U0, U8..U64, I8..I64, F64` | HolyC | Required |
| L2 | Functions, default args at any position `Foo(,6)` | HolyC | Required |
| L3 | Variadic functions via `(...)` + `argc/argv` | HolyC | Required |
| L4 | No mandatory `main()`; top-level statements execute | HolyC | Required |
| L5 | Bare string expressions auto-`print` (`"hi\n";`) | HolyC | Required |
| L6 | `switch` with implicit cases + ranges (`case 4...8`) | HolyC | Required |
| L7 | `#exe { }` compile-time execution blocks | HolyC | Required |
| L8 | Ownership/move/borrow semantics (runtime) | Rust | Required |
| L9 | `try / catch / throw` | HolyC | Required |
| L10 | `class` keyword (replaces `typedef`) | HolyC | Required |
| L11 | Explicit `unsafe` blocks that disable runtime checks | Rust | Required |

Anything not in this table is **out of scope for v1**.

---

## 🧭 Part 3: Execution Phases and Deliverables

Each phase has: **Goal, Tasks, Deliverables, Exit Criteria.** Do not start Phase N+1 until Phase N's exit criteria are met and recorded in `PROGRESS.md`.

---

### **Phase 0 — Project Bootstrap**
**Goal:** A working repository skeleton with CI-ready test harness.

**Tasks:**
1. Create repo structure:
```
holyrust/
├── src/           # compiler + runtime source
├── tests/         # integration tests
├── docs/          # LANG_SPEC.md, DECISIONS.md, PROGRESS.md, OPEN_QUESTIONS.md
├── demos/         # example .hr files
└── tools/         # dev scripts
```
2. Choose implementation language for the **host compiler**: Rust (self-hosting later) — decide Cranelift vs. `cc` fallback and record in `DECISIONS.md`.
3. Set up `cargo test` as the test runner.

**Exit Criteria:**
- [ ] `cargo build && cargo test` runs green with a placeholder test.
- [ ] `docs/` contains all four tracking files initialized.

---

### **Phase 1 — Lexer**
**Goal:** Complete tokenization of the full v1 grammar.

**Tasks:**
1. Implement the lexer with position tracking.
2. Write the golden test suite: for every construct in table 2.4, a source snippet → expected token stream.
3. Property test: lexer never panics on arbitrary byte input (fuzz with 10k random inputs).

**Exit Criteria:**
- [ ] 100% of v1 grammar constructs tokenize correctly.
- [ ] Fuzz test passes with zero panics.
- [ ] Error messages include line:column.

---

### **Phase 2 — Parser + AST**
**Goal:** Full AST generation with error recovery.

**Tasks:**
1. Implement recursive-descent parser (LL-ish) for all L1–L11 constructs.
2. Error recovery: on parse error, report, skip to next `;`, continue.
3. AST dump utility (`--dump-ast`) for debugging and test assertions.

**Exit Criteria:**
- [ ] All `demos/*.hr` files parse without error (excluding intentionally-bad files in `tests/bad/`).
- [ ] Files in `tests/bad/` produce a **diagnostic**, never a crash.
- [ ] AST dump is stable enough to use in regression tests (snapshot testing).

---

### **Phase 3 — JIT Code Generation**
**Goal:** Execute HolyRust code from source, in-process, in < 50ms.

**Tasks:**
1. Integrate Cranelift; if blocked > 1 working session, switch to the `cc`+`mmap` fallback (document the switch).
2. Implement codegen for: integer/float arithmetic, function calls, control flow (if/while/for/switch), string literals, print builtin.
3. Benchmark harness: measure per-statement compile+execute latency.

**Exit Criteria:**
- [ ] `"Hello, world\n";` compiles and prints in ≤ 50ms on the test machine.
- [ ] All Phase-2 demos execute and produce correct output.
- [ ] Benchmark results committed to `docs/BENCHMARKS.md`.

---

### **Phase 4 — Runtime Ownership System**
**Goal:** Live move/borrow checking with live diagnostics.

**Tasks:**
1. Value headers + ownership states (`Owned|Borrowed|Moved`).
2. Runtime move invalidation: accessing a moved value → catchable diagnostic, **not** a segfault.
3. Borrow conflict detection at dereference.
4. `unsafe` blocks and `#![ownership(off|loose|strict)]` levels.
5. Test matrix: 30 cases covering legal moves, illegal use-after-move, borrow conflicts, unsafe escapes.

**Exit Criteria:**
- [ ] Zero segfaults across the entire test matrix (diagnostics only).
- [ ] Illegal use-after-move is **always** caught (100% of test cases).
- [ ] `ownership(off)` measurably reduces overhead (record numbers).

---

### **Phase 5 — Shell/REPL + `#exe {}`**
**Goal:** The interactive experience — the project's visible soul.

**Tasks:**
1. REPL: line-by-line compile+execute, statement history, `session.log`.
2. Function definition and later invocation from the same session.
3. `#exe { }`: execute a block at compile time; its output (as source text) is spliced into the surrounding program. Implement via invoking the same JIT on a sub-AST.
4. Error presentation: diagnostics must point at the exact line/column and suggest the fix where possible.

**Exit Criteria:**
- [ ] A scripted REPL session (piped input) defines a function, calls it, mutates a value, and triggers one intentional ownership error — all in one session, non-interactive.
- [ ] `#exe { }` demo: generate a constant table at compile time and use it in runtime code.
- [ ] REPL latency per statement ≤ 100ms p95.

---

### **Phase 6 — Micro-Kernel (Flagship)**
**Goal:** Bootable QEMU image with a live-editable, memory-safe kernel.

**Tasks:**
1. Minimal boot: VGA text output, PS/2 keyboard input, interrupt handlers.
2. Port the HolyRust compiler to run **inside** the kernel (this proves self-hosting of the runtime path).
3. Hot-patching: from the in-kernel shell, edit a kernel function source, recompile, swap the function pointer atomically, verify system continues running.
4. Demo: modify the keyboard echo handler live while typing.

**Exit Criteria:**
- [ ] QEMU boots to a HolyRust shell in < 5 seconds.
- [ ] Live kernel function edit demo runs without reboot and without memory corruption (verified by a post-edit heap integrity check).
- [ ] Demo recorded as a scripted QEMU session in CI.

**⚠️ Risk note:** Phase 6 is the highest-risk phase. If blocked for more than 3 working sessions, produce a **host-side simulation** of hot-patching (swap function pointers in a running host process) and record the limitation in `OPEN_QUESTIONS.md`. A working simulation is an acceptable v1 deliverable; a broken kernel is not.

---

## 🔄 Part 4: The Agent Execution Loop (MANDATORY)

Every task — no matter how small — must pass through this five-stage loop. This is the core behavioral contract of this document.

```
┌─────────┐    ┌──────────┐    ┌─────────┐    ┌──────────┐    ┌──────────┐
│ 1.THINK │───▶│ 2.EXECUTE│───▶│ 3.TEST  │───▶│ 4.REVIEW │───▶│ 5.DECIDE │
└────┬────┘    └──────────┘    └─────────┘    └──────────┘    └─────┬────┘
     ▲                                                              │
     └──────────────────────◀───────────────────────────────────────┘
                              (retry / escalate)
```

### Stage 1 — THINK (before writing any code)

Produce, in your working notes / `PROGRESS.md`, a short structured answer to:

1. **What am I building?** (one sentence, referencing the phase task ID)
2. **What does the spec say?** (quote the relevant section — do not paraphrase from memory)
3. **What are the failure modes?** (list at least two)
4. **What tests prove success?** (define them BEFORE implementing — TDD)
5. **Am I inside scope?** (check Part 1.3 Non-Goals; if unsure → `OPEN_QUESTIONS.md`)

**Gate:** Do not proceed to Stage 2 until all five answers exist in writing.

### Stage 2 — EXECUTE

- Implement the smallest unit that satisfies the tests defined in Stage 1.
- Commit with a message format: `[P<phase>-<task>] <imperative summary>`.
- If implementation reveals a spec problem: **stop**, write the conflict into `OPEN_QUESTIONS.md`, and continue with the conservative interpretation (Part 0, rule 5).

### Stage 3 — TEST

- Run the full test suite (`cargo test`), not only new tests.
- If any **previously green** test regresses, this is a **hard failure** — you may not proceed.
- Fuzz/property tests for parser and lexer must run at every phase touching them.
- Performance gates (50ms/100ms) are tests, not aspirations.

### Stage 4 — REVIEW (self-critique before deciding)

Answer explicitly, in writing:

1. **Did the tests actually test the requirement, or did I write tests that pass trivially?** (If a test would pass with a broken implementation, rewrite it.)
2. **Is there a simpler implementation I skipped?**
3. **Did I introduce hidden state, globals, or ordering dependencies?**
4. **Does the code match the spec's frozen scope (Part 2.4)?**
5. **Error handling:** does every failure path produce a *diagnostic*, never a panic/segfault?
6. **Diff review:** re-read the full diff. Would I approve this as a reviewer?

### Stage 5 — DECIDE (one of exactly four outcomes)

| Outcome | Condition | Required Action |
| :--- | :--- | :--- |
| ✅ **ADVANCE** | All exit criteria for the task pass, review is clean | Log in `PROGRESS.md`, move to next task |
| 🔁 **REVISE** | Review found fixable flaws | Return to Stage 2 with a specific fix list (max 3 revision cycles per task) |
| 📉 **DEGRADE** | A hard requirement is unreachable as specified | Implement the documented fallback, log in `DECISIONS.md` + `OPEN_QUESTIONS.md` |
| ⛔ **ESCALATE** | After 3 REVISE cycles, or a spec conflict you cannot resolve conservatively | Stop work on the task, write full context in `OPEN_QUESTIONS.md`, move to the next independent task |

**Hard rule:** You may never mark ADVANCE without a written review. You may never loop THINK→EXECUTE more than 3 times on the same task without ESCALATE or DEGRADE.

---

## 🚦 Part 5: Exit Criteria Master Checklist (Project-Level "Done")

The project is complete when **all** of the following are true:

1. [ ] `demos/hello.hr`, `demos/fizzbuzz.hr`, `demos/ownership_demo.hr`, `demos/exe_block.hr` all run from the shell with correct output.
2. [ ] The scripted REPL session (Phase 5) passes in CI.
3. [ ] The 30-case ownership test matrix passes with zero segfaults.
4. [ ] Benchmarks committed: statement latency ≤ 50ms compile, ≤ 100ms REPL p95.
5. [ ] Phase 6 demo (real kernel **or** documented simulation) passes in CI.
6. [ ] `LANG_SPEC.md` fully documents L1–L11 including the ownership tradeoff rationale.
7. [ ] `DECISIONS.md`, `PROGRESS.md`, `OPEN_QUESTIONS.md` are complete and consistent.
8. [ ] No `unwrap()` on user-controlled input paths anywhere in the compiler.

---

## 📝 Part 6: File Conventions and Documentation Duties

| File | Purpose | Update Trigger |
| :--- | :--- | :--- |
| `PROGRESS.md` | Phase/task status, ADVANCE/REVISE/DEGRADE log | Every Stage 5 decision |
| `DECISIONS.md` | Architectural choices + one-line rationale | Every non-obvious choice |
| `OPEN_QUESTIONS.md` | Ambiguities, spec conflicts, blockers | When discovered |
| `LANG_SPEC.md` | The language specification (user-facing) | Every language feature change |
| `FUTURE_IDEAS.md` | Scope-creep quarantine | Every temptation to add features |
| `docs/BENCHMARKS.md` | Latency numbers per phase | Phase 3, 5, 6 |

**Rule:** A feature that is not in `LANG_SPEC.md` does not exist. A decision not in `DECISIONS.md` did not happen.

---

## 🧠 Part 7: Behavioral Guardrails for the Agent

1. **Never invent requirements.** If the spec is silent, choose the simplest option and log it.
2. **Never silently broaden scope.** New ideas go to `FUTURE_IDEAS.md`.
3. **Never suppress a failing test** (`#[ignore]`, deletion, weakened assertions). If a test is wrong, justify its replacement in `DECISIONS.md`.
4. **Never claim completion without evidence.** Every ADVANCE must cite the passing test names.
5. **Diagnostics over crashes — always.** A panic anywhere in the compiler or runtime on user input is a defect, not a edge case.
6. **Latency is a feature.** If a change makes compile latency worse, it needs justification in `DECISIONS.md`.
7. **Honest failure is preferred over fake success.** An ESCALATE with full context is a good outcome; a green build hiding a broken demo is not.

---

## 🚀 Part 8: Kickoff Prompt (verbatim, to start the project)

> You are the build agent for the HolyRust project. Read `SPEC.md` (this document) in full, then execute Phase 0. Before writing any code, complete Stage 1 (THINK) for the Phase 0 tasks and record it in `PROGRESS.md`. Follow the Agent Execution Loop (Part 4) for every task. Begin.

---

**End of specification.**
