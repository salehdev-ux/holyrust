# PROGRESS.md — Phase/Task Status & Agent Execution Loop Log

Every Stage 1 (THINK) gate and Stage 5 (DECIDE) outcome is recorded here (SPEC Part 4).
Format per task: THINK (5 answers) → tests → REVIEW answers → DECIDE outcome + evidence.

Legend: ✅ ADVANCE · 🔁 REVISE · 📉 DEGRADE · ⛔ ESCALATE

---

## Phase 0 — Project Bootstrap

### P0-T1..T3 THINK (Stage 1 gate — recorded BEFORE any code, per Part 8 kickoff)

1. **What am I building?** Repo skeleton (`src/ tests/ docs/ demos/ tools/`), host-compiler
   language + JIT backend decision, and `cargo test` harness with a placeholder test
   (Phase 0 Tasks 1–3).
2. **What does the spec say?** §Phase 0: "Create repo structure … Choose implementation
   language for the host compiler: Rust (self-hosting later) — decide Cranelift vs. `cc`
   fallback and record in `DECISIONS.md`. Set up `cargo test` as the test runner."
   Exit: "`cargo build && cargo test` runs green with a placeholder test" and
   "`docs/` contains all four tracking files initialized." §2.2D: "Backend: **Cranelift**
   … If Cranelift integration fails in Phase 3, fallback: emit x86-64 assembly via `cc`
   and invoke `mmap` + execution." §1.1(3): "no LLVM, no external dependencies, one small
   self-contained compiler." Principle 5: "The bootstrapped system must build itself with
   no external tooling."
3. **Failure modes:** (a) toolchain unavailable/misinstalled in sandbox → build never runs;
   (b) backend decision made blindly now → costly rework in Phase 3; (c) docs initialized
   but empty → violates "all four tracking files initialized" in spirit.
4. **Tests proving success (defined before implementing):** `tests/smoke.rs::
   toolchain_and_crate_skeleton_build` passes under `cargo test`; `cargo build` exits 0;
   the four docs exist with real initial content (verified by `ls`/git commit).
5. **Inside scope?** Yes — Phase 0 exactly. Environment findings that tempt extra work
   (installing QEMU now, adding crates) are deferred/quarantined (FUTURE_IDEAS / OPEN_QUESTIONS).

**Environment survey (evidence for the backend decision):** sandbox is x86-64 Linux, root;
network works (static.rust-lang.org, deb.debian.org reachable); initially no rustc/cargo/cc/qemu.
Installed: gcc 12.2 (apt, needed as Rust's link driver), Rust 1.99.0 stable (rustup, installed
under `/opt` to keep the workspace snapshot small; `tools/setup_env.sh` re-bootstraps).
`qemu-system-x86_64`: absent (relevant to Phase 6, see OPEN_QUESTIONS Q5).

**Backend decision (Phase 0 Task 2):** self-contained direct x86-64 machine-code emitter +
`mmap` execution (the spec's fallback *family*), **not** Cranelift. Full rationale:
DECISIONS.md D003; spec-conflict note: OPEN_QUESTIONS.md Q1. Recorded as 📉 DEGRADE from the
spec's first choice per Part 4 Stage 5 (fallback documented; contingency: literal `cc`+asm
path remains available since gcc is installed, if the emitter proves unworkable).

### P0 EXECUTE/TEST/REVIEW/DECIDE

- EXECUTE: skeleton + `SPEC.md` (verbatim SSOT) + docs + tools created; commits
  `[P0-1] scaffold repo`, `[P0-2] record backend + toolchain decisions`, `[P0-3] cargo test harness`.
- TEST: `cargo build && cargo test` → see evidence below.
- REVIEW (Stage 4): (1) placeholder test exercises the real crate (diag module), not a
  trivial `assert!(true)`. (2) Simpler impl? No — minimal already. (3) Hidden state? None;
  toolchain lives outside the repo by design (documented in tools/setup_env.sh).
  (4) Frozen scope respected. (5) No failure paths yet beyond diag formatting (no panics).
  (6) Diff reviewed: only skeleton + docs.
- DECIDE: ✅ ADVANCE (evidence: `cargo test` output below + exit-criteria checkboxes).

**Exit criteria:**
- [x] `cargo build && cargo test` green with placeholder test — evidence: `test result: ok. 1 passed` (smoke.rs), recorded at commit `[P0-3]`.
- [x] `docs/` contains PROGRESS.md, DECISIONS.md, OPEN_QUESTIONS.md, LANG_SPEC.md (+ FUTURE_IDEAS.md, BENCHMARKS.md per Part 6).

---

## Phase 1 — Lexer

### P1-T1..T3 THINK (Stage 1 gate)

1. **What am I building?** `hr_lexer`: UTF-8 source → token stream with (line,col) positions,
   covering every construct of frozen table §2.4, plus golden + 10k-input fuzz tests
   (Phase 1 Tasks 1–3).
2. **What does the spec say?** §2.2A: "Input: UTF-8 source text. Output: token stream with
   positions (line, column). Requirements: full position tracking for error messages; zero
   external dependencies." Phase 1 exit: "100% of v1 grammar constructs tokenize correctly;
   fuzz test passes with zero panics; error messages include line:column." Phase 1 Task 3:
   "Property test: lexer never panics on arbitrary byte input (fuzz with 10k random inputs)."
3. **Failure modes:** (a) panics on malformed input (slicing, unwrap, arithmetic overflow in
   debug builds — literal parsing!); (b) ambiguous lexing at `4...8` (range) vs `4.5` (float)
   vs `..` typo; (c) wrong columns on non-ASCII (bytes vs chars); (d) unterminated
   strings/comments consuming the stream silently.
4. **Tests proving success (written first):** `tests/lexer_golden.rs` — one snippet per
   feature L1–L11 asserting the exact token-name sequence; position assertions (line:col);
   malformed-input cases assert diagnostics contain line:col and lexing continues.
   `tests/lexer_fuzz.rs` — seeded PRNG, 10k inputs of arbitrary bytes (utf8-lossy), run under
   `catch_unwind`: zero panics, stream always ends with exactly one Eof, positions monotonic.
5. **Inside scope?** Yes. Temptations deferred to FUTURE_IDEAS: unicode escapes (F-none yet),
   HolyC comma-separated hex literals, string interpolation (F9).

Design notes recorded while executing: number rule "`.` joins a float only when followed by a
digit" resolves `4...8` vs `4.5`; `..` → diagnostic with `...` suggestion (feeds Phase 5
"suggest the fix"); literals overflowing I64 are diagnostics, not panics (u64 bit-patterns
allowed for unsigned contexts, DECISIONS to follow if needed).

### P1 EXECUTE/TEST/REVIEW/DECIDE

- EXECUTE: `src/hr_lexer/mod.rs` (token set for all L1–L11 surface syntax, char-based
  positions with byte-offset map, longest-match operators, number rule "`.` joins a float
  only when followed by a digit" → `4...8` vs `4.5` resolved deterministically).
  Commits: `[P1-1] lexer core`, `[P1-2] golden suite L1–L11`, `[P1-3] 10k fuzz`.
- TEST (full suite, Stage 3): `cargo test` →
  lib unit 5 passed (`operators_longest_match`, `range_vs_float`, `dotdot_suggests_range`,
  `bad_char_has_line_col`, `never_panics_on_garbage`);
  `tests/lexer_golden.rs` 18 passed (l1_all_type_names, l1_declarations_and_literals,
  l2_default_args_any_position, l3_variadic_argc_argv, l4_toplevel_statements,
  l5_bare_string, l6_switch_ranges, l7_exe_block_and_attribute, l8_borrow_move_syntax,
  l9_try_catch_throw, l10_class, l11_unsafe_block, operators_full_set,
  comments_are_skipped, positions_track_line_and_column, errors_include_line_column,
  malformed_input_recovers, dotdot_suggestion_present);
  `tests/lexer_fuzz.rs::lexer_fuzz_10k_no_panics` passed (10k inputs, zero panics,
  invariants: single trailing EOF, monotonic offsets, well-formed positions);
  `tests/smoke.rs` still green (no regression).
- REVIEW (Stage 4): (1) Non-trivial: goldens pin exact token sequences (a dropped or
  misclassified token fails), positions verified against exact line:col AND byte slices;
  fuzz asserts structural invariants, not just no-panic. (2) Simpler impl? byte-slice
  lexing possible but Vec<char>+offset-map removes a class of UTF-8 boundary bugs —
  worth the memory at these sizes. (3) No hidden state: Lexer is per-call, no globals.
  (4) Scope: token set covers exactly L1–L11 needs (+ `mut` for &mut, char/hex/bin
  literals — documented in LANG_SPEC §2/§3). (5) Every malformed path yields a
  Diagnostic; removed one `debug_assert!` that could panic in debug builds (Part 7.5);
  progress guard prevents infinite loops. (6) Diff re-read: approve.
- REVISE cycles used: 2 (test-expectation bugs only: missing trailing EOF in 2 unit
  tests; non-ASCII in raw byte string). Implementation unchanged after review.
- DECIDE: ✅ **ADVANCE** — exit criteria:
  - [x] 100% of v1 grammar constructs tokenize correctly (18 golden tests, one per feature + cross-cutting).
  - [x] Fuzz: zero panics across 10k arbitrary-byte inputs (`lexer_fuzz_10k_no_panics`).
  - [x] Error messages include line:column (`errors_include_line_column`, `positions_track_line_and_column`).

## Phase 2 — Parser + AST

### P2-T1..T3 THINK (Stage 1 gate)

1. **What am I building?** `hr_parser`: recursive-descent (LL-ish) parser producing an AST
   for all L1–L11 constructs, with error recovery (report → skip to next `;` → continue),
   a stable `--dump-ast` utility, demos that parse clean, and `tests/bad/` files that
   produce diagnostics but never crash (Phase 2 Tasks 1–3).
2. **What does the spec say?** §2.2B: "Input: token stream. Output: AST. Requirement:
   **error recovery** — a parse error must report the location and continue parsing
   (critical for REPL usability)." Phase 2 exit: "All `demos/*.hr` files parse without
   error …; Files in `tests/bad/` produce a **diagnostic**, never a crash; AST dump is
   stable enough to use in regression tests (snapshot testing)."
3. **Failure modes:** (a) infinite loop when a construct fails without consuming tokens
   (recovery must guarantee progress); (b) stack overflow on adversarial deep nesting
   `((((…` → explicit depth limit; (c) grammar ambiguities: `Foo(,6)` holes vs arg lists,
   unary `&`/`*` vs binary, `(I64)x` cast vs paren-group, `Ident Ident` declaration vs
   expression statement; (d) unstable snapshots (addresses/hash iteration) → dump only
   structural data via derived Debug, no pointers, no maps.
4. **Tests proving success (written first):** `tests/parser_snapshots.rs` — AST dumps of all
   `demos/*.hr` + one snippet per L-feature compared byte-for-byte against committed
   `tests/snapshots/*.ast` (regenerate via UPDATE_SNAPSHOTS=1); `tests/bad_files.rs` —
   every `tests/bad/*.hr` yields ≥1 diagnostic, no panic (catch_unwind), terminates;
   `tests/parser_fuzz.rs` — same 10k-input generator, full lex+parse, zero panics;
   parser unit tests for holes/ranges/defaults/recovery-progress.
5. **Inside scope?** Yes. Deferred temptations: ternary `?:`, nested function definitions,
   first-class call targets (`f()` where f is an expression) → FUTURE_IDEAS (F11 covers fn
   values); case labels restricted to integer/char literals (no const-eval expressions).


### P2 EXECUTE/TEST/REVIEW/DECIDE

- EXECUTE: `src/hr_parser/{ast.rs,mod.rs}` — recursive-descent parser for all L1–L11
  constructs: precedence climbing (11 levels), assignment right-assoc, default-arg holes,
  variadic `...`, case lists/ranges, casts vs parens, unary `&/&mut/*` vs binary,
  `#exe`/`#![ownership]` directives, class defs, `unsafe`, `try/catch/throw`. Recovery:
  diagnostic → skip-to-`;`/stop-before-`}`; progress guards at every loop level;
  depth limit 96 (P002) prevents stack overflow on adversarial nesting.
  `hr --dump-tokens/--dump-ast` CLI. Demos written (4), bad corpus written (13 files).
  Commits: `[P2-1] parser core`, `[P2-2] demos + bad corpus`, `[P2-3] snapshots + fuzz`.
- TEST (full suite, Stage 3): `cargo test` → lib 15 passed (parser unit:
  default_holes_parse, case_range_parses, unary_vs_binary_ambiguity, cast_vs_paren,
  recovery_continues_after_error, deep_nesting_is_diagnostic_not_crash,
  fn_with_defaults_and_variadic, exe_block_and_ownership_attr, class_def, borrow_syntax);
  `tests/bad_files.rs` 2 passed (demos_parse_without_errors — all 4 demos clean;
  bad_files_diagnose_never_crash — all 13 bad files diagnose, zero crashes under
  catch_unwind); `tests/parser_snapshots.rs` 3 passed (18 snapshots: 4 demos + 14 feature
  snippets, determinism check); `tests/parser_fuzz.rs::parser_fuzz_10k_no_panics` passed
  (10k inputs, 0.97s); lexer suites still green (no regression).
- REVIEW (Stage 4): (1) Non-trivial? Mutation-checked: corrupting one committed snapshot
  (`Answer`→`Broken`) makes parser_snapshots FAIL with SNAPSHOT MISMATCH — verified, then
  regenerated. Found a real bug via the bad corpus during EXECUTE: `f(..., I64 b)` was
  silently accepted; fixed (P040 now fires — verified `exit=1 diags≥1`). (2) Simpler impl?
  Single-file table-driven precedence climbing — yes, minimal. (3) Hidden state? Parser is
  per-call; no globals; snapshots deterministic (verified by dumps_are_deterministic).
  (4) Scope: constructs limited to L1–L11 (no ternary, no nested fns, no fn values —
  diagnostics point at FUTURE_IDEAS). (5) Every failure path → Diagnostic (P001–P092,
  L001–L099); progress guards + depth limit make hangs/stack-overflows impossible on fuzz
  input. (6) Diff re-read: approve.
- REVISE cycles used: 1 (P040 variadic-position bug found by my own bad-file corpus).
- DECIDE: ✅ **ADVANCE** — exit criteria:
  - [x] All `demos/*.hr` parse without error (`demos_parse_without_errors`).
  - [x] All `tests/bad/*.hr` produce diagnostics, never crash (`bad_files_diagnose_never_crash`).
  - [x] AST dump stable for regression testing (18 committed snapshots + determinism test +
        mutation check evidence above).

## Phase 3 — JIT Code Generation

### P3-T1..T3 THINK (Stage 1 gate)

1. **What am I building?** The JIT: `hr_check` (semantic checker → typed AST), `hr_codegen`
   (self-contained x86-64 emitter, D003) and `hr_runtime` (mmap loader, worker thread,
   print/trap builtins, exception trampoline) so HolyRust source executes in-process:
   int/float arithmetic, calls, control flow, strings, print, defaults L2, variadics L3,
   top-level execution L4, bare-string print L5, switch L6, try/catch/throw L9 (pulled
   forward: the abort trampoline is needed for stack-overflow/div-zero diagnostics anyway),
   benchmark harness (Phase 3 Tasks 1–3).
2. **What does the spec say?** §2.2D: "Output: executable machine code in memory …
   **compile-to-executable in under 50ms**." Phase 3 Task 1: "Integrate Cranelift; if
   blocked > 1 working session, switch to the `cc`+`mmap` fallback (document the switch)."
   Exit: "`Hello, world\n";` compiles and prints in ≤ 50ms; All Phase-2 demos execute and
   produce correct output; Benchmark results committed to `docs/BENCHMARKS.md`."
   Backend decision was made in Phase 0 (D003, Q1): direct emitter + mmap — Cranelift would
   violate Principle 5 and §1.1(3). Re-verified this phase: gcc IS installed as the
   emergency fallback should the emitter prove unworkable (not needed so far).
3. **Failure modes:** (a) wrong instruction encodings → SIGSEGV/SIGILL or silent
   miscompiles (mitigate: encoder unit tests + objdump cross-check + integration tests);
   (b) ABI violations at extern "C" boundary: 16-byte stack alignment for SSE, callee-saved
   r15/r14 discipline; (c) runaway recursion → stack overflow crash (mitigate: prologue
   stack-limit check → diagnostic trap); (d) div-by-zero SIGFPE (mitigate: checked division
   → trap); (e) infinite loops in generated code (accepted: same as C — v1 has no watchdog;
   noted in LANG_SPEC).
4. **Tests proving success:** `tests/exec_tests.rs` — end-to-end programs with exact stdout
   comparison: hello world (timed ≤50ms), arithmetic int/float incl. all operators,
   precedence, comparisons, control flow (if/while/do/for/switch incl. ranges+lists),
   functions + recursion, default-arg omission `Foo(,6)`, variadic argc/argv sum, bare
   string auto-print, Print value formatting, string escapes, classes (fields, member
   auto-deref), try/catch/throw (code+msg, uncaught → diagnostic), div-zero + stack-overflow
   + index-oob traps produce diagnostics NOT crashes, U0 zero-size (checker layout test).
   `tools/bench.sh` → docs/BENCHMARKS.md.
5. **Inside scope?** Yes; explicitly deferred: ownership *checks* (Phase 4 — headers and
   traps are laid in now), `#exe` execution (Phase 5 — parser support exists), REPL (Phase 5),
   jump-table switch optimization (cascade of compares is fine per §1.3 speed non-goal).

Interpretation note (Part 0 rule 5, conservative): the exit criterion "All Phase-2 demos
execute and produce correct output" is satisfied *incrementally*: hello.hr + fizzbuzz.hr in
Phase 3; ownership_demo.hr requires Phase 4 (ownership); exe_block.hr requires Phase 5
(#exe). Each phase's ADVANCE cites the demos it proves; the full set is proven by Part 5.

New decisions recorded while executing: D023 (Print semantics), D024 (arg eval order),
D025 (movable returns), D026 (switch codegen strategy), D027 (exceptions pulled into P3).

### P3 EXECUTE/TEST/REVIEW/DECIDE

- EXECUTE: `hr_codegen/emitter.rs` (x86-64 encoder, byte-exact unit tests incl. SIB/disp
  ordering fixture), `hr_codegen` (stack-machine compiler over the typed AST; D006/D028
  conventions; indirection-cell calls D015), `hr_check` (types, promotion, places,
  defaults/variadic resolution, C0xx diagnostics), `hr_runtime` (mmap loader, Rt+r15,
  arena+canaries, worker thread, traps/throw trampoline, print), `pipeline` (incl. the whole
  `#exe` engine, D014 — lands early, its exit criterion stays in Phase 5). Commits `[P3-1..3]`.
- TEST (Stage 3, full suite, both profiles): release 88 tests green / debug 11 suites green:
  `exec_tests` 36 (hello_world_50ms_gate, int/float/bitwise/widths, control flow,
  switch_full_semantics, functions_recursion_forward_ref, default_args_any_position,
  variadic_argc_argv, fn_redefinition_swaps_cell, strings+OOB trap, classes, u0_is_zero_sized,
  deref_read_write_and_unsafe, try/catch forms + loop balance + uncaught abort, div_zero /
  stack_overflow / argv_oob traps, globals_persist_across_units, exe_block splice+trap,
  demos hello/fizzbuzz/exe_block exact-output); emitter 25; lexer 23; parser 13; fuzz 2×10k;
  snapshots 3 (+18 files); bad_files 2; no_unwrap 1 (Part 5 item 8 enforced); smoke 1.
  Benchmarks: median compile+exec 0.031 ms (gate ≤50 ms — ~1600× margin), p95 0.040 ms,
  variadic fn def+call 0.094 ms → docs/BENCHMARKS.md via tools/bench.sh.
- REVIEW (Stage 4): (1) Tests non-trivial — they caught SEVEN real defects during EXECUTE:
  guest clobbering callee-saved r14 across the host boundary (entry now preserves
  rbx/r12–r15); ModRM SIB-before-disp ordering (fixture added); trampoline operand-register
  clobber + missing r15 restore (pinned rax/rdx/rcx/r8 + explicit ctx restore); parser
  `at()` comparing Kw discriminants so `} I64 …` parsed as `else`; argv bounds-check
  clobbering idx; `#exe` capture clobbering outer capture; deref-store address in rcx
  clobbered by check args (→ rbx). A suite that finds seven bugs is not passing trivially.
  (2) Simpler impl? Stack-machine + uniform rax values is the smallest correct design;
  cc/dlopen rejected (subprocess + external tooling). (3) Hidden state: Session explicit;
  r15 convention documented at module head; worker isolated behind channels. (4) Frozen
  scope: exactly L1–L11 (+ exceptions pulled from P4 per D027; #exe engine early per THINK
  note). (5) Every failure path → diagnostic: compile errors positioned; runtime traps abort
  units; debug-profile run green (canary alignment fixed). (6) Diff re-read: approve.
- REVISE cycles used: 3 (within cap; each driven by test evidence).
- DECIDE: ✅ **ADVANCE** — exit criteria:
  - [x] `"Hello, world\n";` compiles+prints ≤50 ms: 0.031 ms median (`hello_world_50ms_gate` = the spec gate as a test).
  - [x] Phase-2 demos execute with correct output — incrementally per THINK note: hello ✓, fizzbuzz ✓ (exact 1–30), exe_block ✓; ownership_demo's move-traps fire in Phase 4 (its try/catch/unsafe paths already correct).
  - [x] Benchmarks committed to docs/BENCHMARKS.md (regenerated by tools/bench.sh).

## Phase 4 — Runtime Ownership System
_(THINK below; execution log appended after it)_

### P4-T1..T5 THINK (Stage 1 gate)

1. **What am I building?** Real semantics behind the Phase-3 hooks: value-header state
   machines, move invalidation, the borrow tracker (XOR rule), scope-exact borrow release,
   `unsafe` + `#![ownership(off|loose|strict)]` behavior, and the 30-case test matrix
   (Phase 4 Tasks 1–5).
2. **What does the spec say?** §2.3: "Every value carries a header: { refcount_or_owner_id,
   state: Owned|Borrowed|Moved, type_id }. Moving invalidates the old binding **at runtime** —
   accessing a moved value raises a live diagnostic in the shell instead of a crash or silent
   UB. Borrow rules (one mutable XOR many immutable) are checked at pointer-dereference time…
   Provide `#![ownership(off)]` … Strictness levels: off | loose | strict." Phase 4 exit:
   "Zero segfaults across the entire test matrix (diagnostics only); Illegal use-after-move is
   **always** caught (100% of test cases); `ownership(off)` measurably reduces overhead
   (record numbers)."
3. **Failure modes:** (a) false-positive traps from missed/late borrow releases (break /
   continue / return / throw paths); (b) stale tracker entries after frame reuse → spurious
   conflicts (mitigations: tracker cleared at unit_enter; releases at block/loop/return
   boundaries; throw-leak documented as D033); (c) `off` still paying hook overhead (must
   bypass EMission, not just runtime logic); (d) UB inside hooks (null/unaligned reads) —
   hooks stay total: null/missing keys are no-ops or traps, never wild derefs.
4. **Tests proving success:** `tests/ownership_matrix.rs` — ≥30 numbered cases across legal
   moves / use-after-move (100% caught) / borrow conflicts / unsafe escapes / strictness
   levels, each asserting exact output OR exact diagnostic, zero segfaults; catchability of
   ownership traps via try/catch (negative catch_code, real catch_msg); strict-vs-off overhead
   numbers in BENCHMARKS.md; ownership_demo.hr full expected output pinned in exec_tests.
5. **Inside scope?** Yes — no GC (F13), no per-pointer borrower identities (D032 documents
   enforcement points), no compile-time borrow checker (that is the anti-goal, §1).


### P4 EXECUTE/TEST/REVIEW/DECIDE

- EXECUTE: real semantics behind the Phase-3 hooks — `hr_runtime`: Str-slot null-sentinel +
  class in-band header state machines (Owned/Moved/Immortal), move invalidation, borrow
  tracker (HashMap side table, D030) with XOR enforcement at creation+writes (D032),
  catchable ownership traps (negative catch_code + pre-interned catch_msg), per-unit tracker
  reset. `hr_codegen`: scope-marked borrow records with lexical release emission
  (statement/block/loop-exit/return/function-end), `off`/unsafe bypass moves to inline copies
  (no hook calls at all — the measurable overhead delta). `hr_check`: pointer-mutability
  inheritance + C045 (D034), assignment-silence rule (D035). Commits `[P4-1..3]`.
- TEST (Stage 3, full suite both profiles): `ownership_matrix` **36 cases** (≥30 required):
  legal moves 1–8, use-after-move 9–16 + 33/35 (10/10 caught — 100% criterion), borrow XOR
  17–26, unsafe escapes 27–29, catchability 30–31, strictness levels 32–36; every case also
  asserts `heap_check()` canary integrity afterwards; zero segfaults (any crash fails the
  process). `demo_ownership_runs` pins the full showcase output. Suite totals: release 91
  tests green, debug 12 suites green — no regressions (all Phase 1–3 tests still pass).
  Overhead recorded: **strict 2.3ms vs off 1.2ms = 1.89×** on the 200k move+index loop
  (`hr --bench` → BENCHMARKS.md) — "ownership(off) measurably reduces overhead" ✓.
- REVIEW (Stage 4): (1) Non-trivial tests: the matrix caught three real bugs during EXECUTE —
  ptr-mutability lost through decl coercion (writes trapped as shared), global records storing
  pre-inheritance types, and `aborted` set before the catchable-trap jump (caught traps
  reported as aborts); the auto-print-of-assignment semantic bug was caught by case 3's exact
  output. (2) Simpler impl? Side-table tracker avoided headers-on-every-integer (D030
  documents the §2.3 interpretation); release emission reuses place materialization.
  (3) Hidden state: tracker lives in Rt, cleared deterministically per unit (no cross-unit
  leaks); borrow records are compiler-local. (4) Scope: exactly L8/L11 + §2.3; no GC (F13),
  no NLL (D032 documented). (5) All failure paths → diagnostics; hooks are total functions
  (null keys trap as K_DEREF_INVALID, missing keys no-op). (6) Diff re-read: approve.
- REVISE cycles used: 3 (mut inheritance, global type record, aborted-flag ordering — each
  fixed with a pinned regression case in the matrix/exec suite).
- DECIDE: ✅ **ADVANCE** — exit criteria:
  - [x] Zero segfaults across the matrix (36 cases, diagnostics only; heap canaries verified per case).
  - [x] Illegal use-after-move caught in 100% of cases (10/10: `ownership_matrix` asserts the exact diagnostic each time).
  - [x] `ownership(off)` measurably reduces overhead: 1.89× (2.3ms → 1.2ms, 200k iterations) recorded in BENCHMARKS.md.

## Phase 5 — Shell/REPL + `#exe {}`

### P5-T1..T4 THINK (Stage 1 gate)

1. **What am I building?** `hr_shell`: the interactive REPL — line-by-line
   compile+execute over one persistent Session, brace-balanced multi-line continuation,
   in-memory statement history + `session.log`, value echo, positioned diagnostics with
   suggestions, shell commands; plus the scripted-session CI test and the ≤100ms p95
   latency gate (Phase 5 Tasks 1–4; Task 3's `#exe` engine already landed and is tested in
   Phase 3 — this phase keeps its exit-criterion demo).
2. **What does the spec says?** §2.2F: "Reads lines; feeds the lexer→parser→checker→codegen
   pipeline line-by-line. Statements outside functions execute immediately (HolyC
   semantics). Every shell session is logged to `session.log` for debugging." Phase 5 exit:
   "A scripted REPL session (piped input) defines a function, calls it, mutates a value, and
   triggers one intentional ownership error — all in one session, non-interactive; `#exe {}`
   demo: generate a constant table at compile time and use it in runtime code; REPL latency
   per statement ≤ 100ms p95." Phase 5 Task 4: "diagnostics must point at the exact
   line/column and suggest the fix where possible."
3. **Failure modes:** (a) REPL dies or corrupts session state after a runtime trap (must
   isolate per-unit — Session/abort machinery already designed for this, but the loop must
   never propagate panics); (b) `#`-commands colliding with `#exe`/`#![ownership]` source
   syntax (exact-match commands only, and only at buffer start); (c) latency gate measured
   dishonestly (must include compile+execute per statement, recorded in session.log);
   (d) parallel test races on session.log (single scripted-session test; parse last block).
4. **Tests proving success:** `tests/repl_scripted.rs` — spawns the real `hr` binary with
   piped stdin (CARGO_BIN_EXE_hr): defines fn with default arg, calls it, mutates a global,
   triggers a caught+uncaught ownership error, proves the session survives, splices an
   `#exe`-generated constant, `#quit`s cleanly; asserts exact stdout ordering + stderr
   diagnostic + session.log per-statement latencies p95 ≤ 100 ms. `demo_exe_block_runs`
   (already green) is the #exe constant-table exit criterion.
5. **Inside scope?** Yes — readline editing/colors/completion stay quarantined (F10);
   history = in-memory list + session.log (spec's words), not a ~/.histfile.

### P5 EXECUTE/TEST/REVIEW/DECIDE

- EXECUTE: `hr_shell` REPL (persistent Session, brace-balanced continuation on real tokens,
  exact-match `#help/#history/#reset/#quit` commands that never shadow `#exe`/`#![…]`,
  HolyC-style value echo, per-statement latency + inputs + diagnostics into `session.log`,
  stdout/stderr discipline per D036). `#exe` exit-demo already green since P3.
  Commits `[P5-1..3]`.
- TEST (Stage 3, full suite both profiles): `tests/repl_scripted.rs::scripted_repl_session`
  — the spec's exact scenario piped non-interactively into the real `hr` binary: defines
  `Add` (with default arg), calls it (15), mutates a global (echoes 41, 42), triggers an
  intentional use-after-move (`Print(s)` after move) — diagnostic on stderr, shell prints
  "session continues" and SURVIVES (subsequent statements run: `session alive`, `#exe`-
  generated `sq7` → 49, echo `5+5` → 10), `#quit` exits 0. Latency gate parsed from
  session.log: statements ~0.05–0.06 ms, p95 ≤ 100 ms ✓ (asserted in-test). Full suite:
  release 93 tests green; debug 13 suites green — no regressions.
- REVIEW (Stage 4): (1) Non-trivial: asserts exact stdout ordering incl. echo semantics,
  stderr contents, exit code, log presence/format, latency parsed from the real log — a
  broken REPL fails on any axis. (2) Simpler? Single loop + token-based brace counting;
  no external readline (F10). (3) Hidden state: session.log append-only with session
  headers (test parses last block; parallel-test race analyzed — single consumer).
  (4) Scope: commands minimal, no colors/editing (F10). (5) All I/O failures handled with
  diagnostics; isatty declared manually (no crates); no_unwrap still green. (6) Approve.
- REVISE cycles used: 0 (assignment-echo semantics discovered in P4 were already documented
  as D035/§13 before this phase — no rework needed).
- DECIDE: ✅ **ADVANCE** — exit criteria:
  - [x] Scripted piped session: fn def + call + mutation + intentional ownership error, one session, non-interactive (`scripted_repl_session`).
  - [x] `#exe {}` constant-table demo (`demo_exe_block_runs`, `exe_block_splices_generated_source`, and `sq7` inside the scripted session).
  - [x] REPL latency ≤ 100 ms p95 — measured ~0.06 ms (gate asserted in `scripted_repl_session`).

## Phase 6 — Micro-Kernel (Flagship)

### P6 THINK (Stage 1 gate)

1. **What am I building?** The flagship demo per D016 (spec-sanctioned host-side
   simulation, Q5): a live "kernel" — a running HolyRust event loop processing scripted
   keystrokes — whose `KbdEcho` driver function is recompiled and hot-swapped **while the
   loop is running** (mid-execution, via a control channel from guest builtin to the host
   compiler), with heap-integrity verification before/after (Phase 6 Tasks 2–4 adapted;
   Task 1 boot/VGA/PS2 is N/A in simulation and recorded in OPEN_QUESTIONS).
2. **What does the spec say?** Component G: "Kernel functions editable from a live shell
   (hot-patch: recompile function, swap pointer, preserve state). **This is the flagship
   demo feature.**" Risk note: "If blocked … produce a **host-side simulation** of
   hot-patching (swap function pointers in a running host process) … A working simulation
   is an acceptable v1 deliverable; a broken kernel is not." Exit: "Live kernel function
   edit demo runs without reboot and without memory corruption (verified by a post-edit
   heap integrity check); Demo recorded as a scripted … session in CI."
3. **Failure modes:** (a) deadlock between worker (blocked in HotPatch builtin) and main
   thread (blocked on Done) — solved with ONE control channel carrying Done|Patch{ack}
   messages; (b) swapping a cell while generated code is mid-call — the swap is a single
   aligned 8-byte store and old code stays mapped (D015), so in-flight calls finish safely;
   (c) patch source that fails to compile must NOT kill the running kernel (diagnostic +
   rc=-1, loop continues); (d) state preservation: globals/frame state of the running loop
   must be untouched by the patch (it only replaces a function body).
4. **Tests proving success:** `tests/kernel_sim.rs` runs `demos/kernel_sim.hr` end-to-end:
   exact output shows v1 echo for the first keys, the patch applied mid-loop (rc=0), v2
   echo for subsequent keys, `HeapCheck()`=1 before AND after, and "still running" at the
   end; plus a bad-patch case (kernel survives, rc=-1, diagnostic on stderr). CI = this
   test (the "scripted session in CI" criterion, adapted per Q5).
5. **Inside scope?** Simulation is explicitly sanctioned (risk note). New guest surface is
   minimal and documented: builtins `SimPush/SimKey/HotPatch/HeapCheck` (§14) exist to make
   the demo honest — recorded in DECISIONS (D037) and quarantined-from-vocabulary in
   FUTURE_IDEAS (real kernel = F6).

## Phase 6 execution log

### P6 EXECUTE/TEST/REVIEW/DECIDE

- EXECUTE: control-channel hot-patching (`WorkerMsg::Patch` + host-side
  `Session::hotpatch_compile` with deadlock guard against `#exe`), guest builtins
  `SimPush/SimKey/HotPatch/HeapCheck` (LANG_SPEC §14), arena block snapshotting for the
  post-edit integrity check, `demos/kernel_sim.hr` (flagship), `hr --bench` metric 4.
  Commits `[P6-1..3]`.
- TEST (Stage 3): `tests/kernel_sim.rs` 3/3 — `kernel_sim_live_hotpatch` pins the EXACT demo
  transcript: v1 echo for keys 104/105 → mid-loop patch (rc=0) → **v2 echo for keys 33/63
  from the swapped cell** → `HeapCheck()=1` pre-boot AND post-patch → "still running —
  processed 4 keys, no reboot"; `kernel_survives_bad_patch` (rc=-1, old body keeps serving,
  kernel not aborted); `hotpatch_rejects_exe_and_keeps_running` (deadlock guard). Full suite
  re-run: release 96 tests green (13 suites), debug 13 suites green — zero regressions.
- REVIEW (Stage 4): (1) Non-trivial: exact-transcript assertion would fail on any ordering,
  swap-visibility, or state-preservation defect; bad-patch cases prove the failure paths.
  (2) Simpler? Reuses the existing cell indirection (D015) — the patch mechanism is
  literally one 8-byte store; the control channel replaces what would otherwise be unsafe
  cross-thread &mut Session sharing. (3) Hidden state: sim queue + blocks snapshot live in
  Rt, documented; leaked ctrl_tx Box is process-lifetime by design (session outlives runs).
  (4) Scope: simulation per the risk note; real boot stack NOT faked — honestly recorded
  (Q5 resolved-as-simulation; F6 quarantined). (5) Failure paths: bad patch → diagnostic +
  rc=-1, never abort/crash; HeapCheck reports instead of trapping. (6) Diff re-read:
  approve.
- REVISE cycles used: 0 for the patch machinery (worked first run — the P3 trampoline/cell
  groundwork paid off); 1 cleanup cycle on the test file (removed a confusing draft).
- DECIDE: ✅ **ADVANCE (simulation deliverable, per SPEC Phase 6 risk note)** — exit criteria:
  - [~] "QEMU boots in <5s" — N/A in simulation (Q5, D016): simulated boot→shell is immediate; demo end-to-end measured in BENCHMARKS.md (metric 4).
  - [x] Live kernel function edit without reboot, no memory corruption — verified by post-edit `HeapCheck()` (in-demo AND asserted in `kernel_sim_live_hotpatch`).
  - [x] Demo recorded as a scripted session in CI — `tests/kernel_sim.rs` (adaptation documented in Q5/D016).

---

## 🚦 Part 5 Master Checklist — Project-Level Verification (final)

Date: 2026-10-14 · Machine: sandbox x86-64 Linux, 2 CPUs, rustc 1.99.0 release ·
Full suite: **95 tests green in release AND debug profiles, zero warnings.**

1. [x] **All demos run from the shell with correct output** — `hr demos/{hello,fizzbuzz,
   ownership_demo,exe_block}.hr` all exit 0 with exact expected output (byte-checked here +
   pinned in `exec_tests::demo_hello_runs / demo_fizzbuzz_runs / demo_ownership_runs /
   demo_exe_block_runs`); hello additionally verified through the piped REPL (fn defined in
   one line, called in the next — session persistence). `demos/kernel_sim.hr` exit 0.
2. [x] **Scripted REPL session passes in CI** — `tests/repl_scripted.rs::
   scripted_repl_session` (real `hr` binary, piped stdin: fn def w/ default arg → call →
   global mutation → intentional ownership error survived → `#exe` splice → echo → #quit,
   exit 0, exact stdout, stderr diagnostic asserted).
3. [x] **30-case ownership matrix, zero segfaults** — `tests/ownership_matrix.rs` runs
   **36** cases (legal moves 1–8, use-after-move 9–16+33/35 → **10/10 caught = 100%**,
   borrow XOR 17–26, unsafe escapes 27–29, catchability 30–31, strictness levels 32–36);
   heap canary integrity asserted after every case; any segfault would kill the process and
   fail the suite — none did, in either profile.
4. [x] **Benchmarks committed** — docs/BENCHMARKS.md (regenerated by tools/bench.sh):
   compile+exec median **0.031 ms** / p95 0.040 ms (gate ≤50 ms — enforced as test
   `hello_world_50ms_gate`); REPL per-statement ~**0.06 ms**, p95 ≤100 ms gate asserted
   in `scripted_repl_session` from session.log; ownership overhead strict-vs-off **1.87×**
   (Phase 4 exit); kernel-sim demo end-to-end **0.29 ms** (Phase 6).
5. [x] **Phase 6 demo passes in CI — BOTH tracks**:
   (a) documented simulation `tests/kernel_sim.rs` (3 tests): mid-loop hot-patch visible on
   the next event, HeapCheck()=1 pre/post, bad-patch and `#exe`-patch rejection survive
   without killing the kernel; (b) REAL kernel (P6b): `tests/kernel_boot.rs` drives a
   scripted QEMU session — boot→shell 0.16s (<5s gate), typed statements JIT-compiled
   IN the kernel, KbdEcho live-patched mid-demo, post-edit HeapCheck()=1, PS/2 sendkey
   path, VGA screendump, ordered transcript; honest SKIP without qemu.
6. [x] **LANG_SPEC.md fully documents L1–L11 + the ownership tradeoff** — §1 rationale
   (the deliberate inversion, mandated by §2.3), §2–§14 all marked final; every v1 rule,
   limitation and diagnostic code referenced from the spec text; "a feature not in
   LANG_SPEC does not exist" holds (builtins §14, echo rules §13, borrow lifetimes §7).
7. [x] **DECISIONS / PROGRESS / OPEN_QUESTIONS complete & consistent** — D001–D037 each
   with rationale; every phase has THINK → EXECUTE → TEST → REVIEW → DECIDE with cited
   test names and REVISE-cycle counts (never >3); Q1–Q9 all resolved with the conservative
   reading taken; FUTURE_IDEAS F1–F15 quarantined (nothing leaked into v1).
8. [x] **No unwrap() on user-controlled input paths** — `tests/no_unwrap.rs` scans all of
   src/ production code for unwrap/expect/panic!/unreachable!/assert and fails the build on
   any hit (strictest reading, Q9); green in both profiles.

### Honest limitations (Part 7.7 — recorded, not hidden)
- Phase 6 is the sanctioned **simulation** (no QEMU in the environment); the boot/VGA/PS2
  stack was not built (Q5/F6).
- Borrow enforcement points: creation + writes (reads-during-exclusive permitted, D032);
  borrows alive across a `throw` leak conservatively until unit end (D033); dangling
  frame-borrows are statically blocked only for `return &local` (C006).
- F64 args inside variadic `argv` are raw bits (Q6); case labels are literals only;
  `U64→F64` ≥2^63 rounds (LANG_SPEC §3); no arrays/pointer-arith/closures (F1/F2/F11).
- Generated code is stack-machine quality (~2–3× slower than optimized native) — within
  the explicit non-goal (§1.3: developer velocity is the metric; gates are ~1600× under).

**Project status: v1 COMPLETE per SPEC Part 5 (all 8 items verified with evidence) —
and Phase 6 upgraded from simulation-only to simulation + real bootable kernel (P6b).**

---

## Phase 6b — Real-Kernel Attempt (deferred stretch; Q5/D016 contingency)

### P6b THINK (Stage 1 gate)

1. **What am I building?** The SPEC Phase 6 *primary* deliverable, attempted only because
   the sanctioned simulation landed early and green (D016 pre-condition): a bootable QEMU
   x86-64 kernel (VGA text, PS/2 or serial keyboard, event loop) that runs the HolyRust
   compiler **inside** the kernel, culminating in the flagship demo — live-editing the
   keyboard-echo handler while keys are being processed, with post-edit heap integrity
   check. Staged checkpoints K0–K6, each independently committable.
2. **What does the spec say?** Component G: "Boots on QEMU (x86-64), VGA text mode, PS/2
   keyboard. Kernel functions editable from a live shell (hot-patch: recompile function,
   swap pointer, preserve state). **This is the flagship demo feature.** If it works, the
   project's thesis is proven." Phase 6 exit: "QEMU boots to a HolyRust shell in < 5
   seconds; live kernel function edit demo runs without reboot and without memory
   corruption (verified by a post-edit heap integrity check); demo recorded as a scripted
   QEMU session in CI." Risk note: "A working simulation is an acceptable v1 deliverable;
   a broken kernel is not." — so: the committed simulation STAYS the CI deliverable unless
   the kernel passes its own scripted test; kernel work must never regress the host suite
   (Part 4 Stage 3: previously-green test regressing = hard failure).
3. **Failure modes:** (a) QEMU absent/unrunnable in this sandbox → verify at K0 before any
   code; (b) multiboot/long-mode bring-up without a debugger → mitigate with serial
   tracing at every boot stage + `qemu -d int` triage; if QEMU rejects multiboot ELF64
   `-kernel`, fall back to grub-mkrescue ISO (nasm+xorriso installed for this); (c) the
   no_std compat refactor destabilizes the host build → feature-gated shims, run the full
   95-test host suite after every refactor commit; (d) budget exhaustion → checkpoints are
   independently shippable; after 3 REVISE cycles on a checkpoint, DEGRADE: stop there,
   keep what's green, document (the simulation remains the deliverable).
4. **Tests proving success:** K1 boot transcript (serial log contains banner; wall-clock
   boot < 5 s); K2 key-echo transcript; K4 in-kernel compile+execute of a typed statement;
   K5 hot-patch transcript: v1 echo → typed redefinition → v2 echo, `HeapCheck()`=1 after;
   scripted via QEMU (serial stdin and/or monitor `sendkey`) in `tools/kernel_ci.sh` +
   cargo test wrapper; host suite 95/95 green at every commit.
5. **Inside scope?** Yes — Phase 6 primary deliverable, explicitly contemplated by the
   risk note and my own Phase 0 survey ("attempt QEMU afterwards only if simulation lands
   early"). Interrupts vs polling for the keyboard: spec says "interrupt handlers" — attempt
   IDT+IRQ1; if blocked >3 cycles, polling fallback recorded as DEGRADE with rationale.

### P6b-K1 checkpoint — multiboot → long-mode boot ✅ (REVISE ×1)

- EXECUTE: `kernel/boot.S` (multiboot1 header with a.out-kludge fields, 32-bit entry,
  serial traces S1/S1.5/S2, identity 1 GiB 2 MiB-page tables, PAE+LME+PG, ljmp to 64-bit,
  VGA marker, hlt), `kernel/linker.ld` (1M base, `_kernel_*` symbols for the kludge header),
  `kernel/build_boot.sh` (as → ld → objcopy -O binary).
- Empirical findings recorded: QEMU 7.2 `-kernel` REJECTS multiboot ELF64 ("Cannot load
  x86-64 image") → flat-binary a.out-kludge route chosen (ELF64 link, objcopy to binary);
  this keeps one toolchain for asm boot + 64-bit Rust kernel later.
- TEST: `qemu-system-x86_64 -kernel kernel.bin -m 256M -smp 1` (TCG) → serial transcript
  `S1 → S1.5 → S2 → BOOT-OK-64BIT`; monitor `screendump` → VGA non-blank (3634 px, glyphs
  visible); boot wall-time < 1 s (gate: < 5 s).
- REVIEW: first attempt triple-faulted; `qemu -d int` + monitor `xp` memory dump pinpointed
  PML4[0]/PDPT[0] self-loops (typo: stored `pml4|3` into PML4[0] instead of `pdpt|3`).
  Fixed; second attempt booted. Diagnostics-over-guessing worked exactly as intended.
- DECIDE: ✅ ADVANCE (K1). Next: K3 lib no_std refactor BEFORE kernel Rust, so the kernel
  can link the real compiler (host 95-test suite must stay green through the refactor).

### P6b-K2..K5 checkpoints — real kernel ✅ (REVISE ×3, all diagnosis-driven)

- EXECUTE:
  - K2 (`kernel/` crate): `#![no_std]` staticlib for `x86_64-unknown-none` — bump global
    allocator (32–224 MiB), panic handler → serial+VGA → reset, VGA 80×25 writer (white
    stdout / red diagnostics), COM1 driver (tx + non-blocking rx), PS/2 poll driver
    (scancode set 1, shift tracking), kernel shell (boot script + interactive loop, serial
    OR PS/2 input, HolyC-style echo). `kmain` wired from boot.S on an 8 MiB kernel stack.
  - K3 (prereq, done first): the whole `holyrust` lib is dual-mode — `--no-default-features`
    compiles it `no_std + alloc`: `compat::Map` (Vec-backed, Borrow-ergonomic) replaces
    HashMap, `hr_out` write hooks replace std io, cfg(host) gates worker/channels/mmap/exit,
    kernel gets direct `run_entry` + `KERNEL_CHUNK/KERNEL_HALT` hooks + `Rt.host_sess`
    in-place HotPatch (D042). **Host suite stayed green at every step** (Part 4 Stage 3).
  - K4: linked kernel boots QEMU → S1..S4 traces → in-kernel compiler executes the boot
    script (`heap integrity: 1`, KbdEcho/PumpKeys defined, `[echo-v1] 104/105`).
  - K5: scripted CI (`tools/kernel_ci.py` + `tests/kernel_boot.rs`, runs inside `cargo test`):
    serial socket transcript with ordered markers, `<5s` boot gate, live hot-patch of
    KbdEcho mid-demo (`[ECHO-v2!!] 33 (live-patched)`), `post-patch heap = 1`, PS/2 path
    via monitor `sendkey 4/2/ret` → `42`, VGA `screendump` non-blank (10098 px), `#quit`
    reset-pulse exit (D042). `tools/ci.sh` runs it; self-skips honestly without qemu.
- Boot crash postmortems (all found with `qemu -d int` + monitor `xp` + symbol math, never
  by guesswork): (1) QEMU rejects multiboot ELF64 `-kernel` → flat-binary a.out-kludge
  route (D038); (2) orphaned `.got` linked past `_kernel_load_end` → never loaded → wild
  indirect jumps → linker script now pins `.got*` inside the loaded image (D038); (3) bss
  zero-loop erased its own page tables (tables lived in merged `.bss`) → moved to
  `.boot_tables` NOBITS outside the zero range (D039); (4) `prompt(pending.is_empty())`
  polarity inverted → `!`. One-char bugs and layout bugs alike: each pinned by a trace or
  a CI marker before/after the fix.
- TEST (Stage 3): release suite **97 tests green** incl. `kernel_boot_and_live_patch_
  scripted_session` (7.2s, real QEMU); debug suite 15/15 green with `HR_SKIP_KERNEL_CI=1`
  (kernel test also passes in debug when run directly); `tools/kernel_ci.py` → PASS
  (boot 0.16–0.18s vs <5s gate); host regression gate held through K3 (95→96→97 monotonic,
  zero previously-green test ever regressed).
- REVIEW (Stage 4): (1) Non-trivial: ordered-transcript + timing + screendump + PS/2-path
  assertions; three real boot crashes were caught by these instruments, not by luck.
  (2) Simpler? Reused the ENTIRE host compiler via no_std feature-gating instead of a
  kernel-only interpreter — the in-kernel shell compiles the same language with the same
  diagnostics (that IS the spec's Task 2). (3) Hidden state: fixed physical map + single
  static Session (documented D041); hooks set exactly once at boot before any use.
  (4) Scope: everything from the frozen table works in-kernel (demos run identically);
  IRQ interrupts consciously DEGRADED to polling (D040, F16) per the risk note's
  "working > broken". (5) Failure paths: alloc-OOM → panic handler → red screen + reset;
  guest traps → diagnostics, kernel continues (thesis holds bare-metal). (6) Approve.
- DECIDE: ✅ **ADVANCE** — Phase 6 exit criteria, real-kernel track:
  - [x] "QEMU boots to a HolyRust shell in < 5 seconds" — **0.16 s** (gate asserted in CI).
  - [x] "Live kernel function edit demo runs without reboot and without memory corruption (post-edit heap integrity check)" — KbdEcho v1→v2 mid-event-stream, `HeapCheck()=1` before and after, asserted in transcript order.
  - [x] "Demo recorded as a scripted QEMU session in CI" — `tests/kernel_boot.rs` in `cargo test`; `tools/ci.sh` includes it; honest SKIP when qemu absent (simulation `tests/kernel_sim.rs` remains the portable deliverable).

---

# TempleOS-RS (SPEC_TOS.md v1.0) — build log

New SSOT: `docs/SPEC_TOS.md`. Everything below is the TOS work; the v1 log above is
the frozen foundation it builds on.

## Phase 0 — Preparation & Baseline

### TOS-P0.0 — Workspace recovery (unplanned, blocking) ✅

- THINK: Phase 0 entry requires a green baseline, so the first action must be to
  *establish* the baseline rather than write new code. Assumption to verify first:
  "the HolyRust repo I built" is present and intact.
- EXECUTE: none (discovery step). Inspected the whole tree byte-wise.
- TEST: **every one of the 90 non-empty files was NUL-filled** — byte lengths
  preserved exactly, content 100 % zero, no `.git` directory present (classic
  unclean-shutdown NTFS data loss). `C:\Users\ROUYA\Desktop\holyrust-v1.0.tar.gz`
  (155,340 B) decoded as gzip (magic `1f 8b`), SHA-256
  `B11C68DD600AB769F40D8D436B7C5159DD741462153EF1C82FF3FC077AAEBD57`, and held the
  *same* 91-file tree with real content (`Compare-Object` over both file lists:
  zero differences in either direction).
- REVIEW: **The first restore silently did nothing.** `robocopy` reported exit 0
  having changed no file, because its default "same file" test is size+timestamp
  and the corrupted copies had *identical sizes* — so it skipped all 91. Caught by
  verifying the restored bytes instead of trusting the exit code (Part 0 rule 4:
  fix the instrumentation, then re-test). Fixed with a forced per-file copy; the
  zeroed tree was preserved as `holyrust.corrupted-20261015.zip` for forensics.
- DECIDE: ✅ ADVANCE — 91/91 files restored, 0 files still zeroed. The corruption
  was host filesystem damage, not a project defect.

### TOS-P0.1 — Entry criteria: dual-mode compiler present, baseline green ✅

- THINK: Phase 0 entry demands (a) HolyRust ≥ `[P6b-K5]`, (b) `cargo test --release`
  green, (c) `tools/kernel_ci.py` PASS. Two hard environment facts contradicted the
  plan: v1 is **x86-64-Linux-only** (`mmap` + SysV ABI), and this host is Windows 10
  with no WSL distro, no in-process elevation and 4 GB RAM — so INSTALL.md's
  documented path needed an elevated install **and a reboot** first.
  Risks anticipated: the Windows x64 ABI differs from SysV (RCX/RDX/R8/R9 vs
  RDI/RSI/RDX/RCX/R8/R9), which would corrupt every JIT→runtime call; the kernel
  link step depends on ELF-capable GNU `as`/`ld`/`objcopy`, none of which exist here.
- EXECUTE:
  - Toolchain: rustup `stable-x86_64-pc-windows-gnu` (existing install was
    interrupted — "missing manifest" — and only had `cargo.exe`); its `rust-mingw`
    component links with no external gcc. `x86_64-unknown-none` target +
    `llvm-tools-preview` added. VS 2022 is installed **without** the C++ workload
    (no `link.exe`, no SDK) and there is no MinGW, so the MSVC path was rejected.
  - Host port (D043): `code_mem` (`VirtualAlloc`/`VirtualProtect`), all JIT-facing
    callbacks `extern "sysv64" fn`, `std::io::IsTerminal`, temp-dir blob path,
    working-python probe in `tests/kernel_boot.rs`.
  - Kernel port (D044): `kernel/bootasm.rs` + `kernel/build.ps1`; portable
    `tools/kernel_ci.py`.
  - QEMU 11.1.0 installed via winget.
- TEST:
  - `cargo build --release` → `hr.exe` 1,852,668 B; `hr demos/hello.hr` →
    `Hello, world` / `The answer is 42` (the JIT executes x86-64 SysV code on
    Windows).
  - `cargo test --release` → **97 passed / 0 failed across 15 targets, exit 0,
    zero warnings** (26+0+2+38+1+3+1+18+1+1+1+3+1+1). Identical total to v1's
    claimed 97 — no test was lost in the port.
  - Kernel: `cargo build --target x86_64-unknown-none` → `libhr_kernel.a`
    (12,848,484 B); `build.ps1` → `kernel.bin` 518,320 B with multiboot magic
    `0x1BADB002` at **offset 0** (< 8192 required) and all five serial traces
    present.
  - QEMU boot → `S1 → S1.5 → S2 → S2.1 → S2.2 → S3 → S4`, `heap integrity: 1`,
    `[echo-v1] 104/105`, `kernel: ready`, `hr>` prompt.
- REVIEW: two porting failure modes found by instrumentation, not guesswork:
  (1) rustc's `global_asm!` defaults to **Intel** syntax, so `boot.S` produced 74
  assembler errors until `options(att_syntax)` was added; (2) `python3` on Windows
  resolves to a Microsoft Store *stub* that spawns fine but exits 9009, which turned
  the kernel test's honest skip into a hard failure — hence the version-probing
  `find_python()`. Both were diagnosed from the actual error text.
- DECIDE: ✅ ADVANCE — entry criteria met in substance on the native Windows host,
  with the platform change recorded as D043/D044 rather than left implicit.

### TOS-P0.2 — G0 gate: scripted kernel CI + docs ✅

- THINK: Phase 0's third criterion is `tools/kernel_ci.py` PASS, which exercises the
  boot gate (< 5 s), in-kernel JIT, live hot-patch, post-patch heap integrity, the
  PS/2 path and a non-blank VGA screendump. Risks: QEMU's Windows build has no
  `AF_UNIX` (v1's transport), and the guest's final reset line is emitted
  microseconds before QEMU exits.
- EXECUTE: rebuilt the driver on **TCP loopback** chardevs, `wait=on` so the guest
  cannot write before the recorder attaches, system-temp scratch files, and a
  chardev `logfile` mirror.
- TEST: `python tools/kernel_ci.py` → `KERNEL-CI: PASS (boot 2.80s, qemu rc=0)`,
  VGA screendump 10,098 lit pixels, all ten transcript markers present **in order**
  (`S2:64bit-longmode` … `[ECHO-v2!!] 33 (live-patched)` … `post-patch heap = 1`
  … `halting (reset)`). In the wired-in `cargo test --release` run the
  `kernel_boot` target passes in 14.26 s.
- REVIEW: the `halting (reset)` marker failed **twice** before passing: first with
  a post-`wait()` drain, then with a concurrent drain — because a loopback TCP
  teardown (RST) discards the peer's receive queue, so no amount of read timing
  recovers data once QEMU dies. The fix was to capture the stream where it cannot
  be lost (QEMU's synchronous chardev `logfile`). Also noted: boot time on this
  2011 dual-core under TCG measured 1.46–2.80 s against the < 5 s gate, versus
  v1's 0.16 s on the original sandbox — same code, slower emulation host, and the
  CI asserts the gate rather than a fixed number.
- DECIDE: ✅ **ADVANCE to Phase 1.** G0 checklist:
  - [x] "Baseline is green" — 97/97 release tests, exit 0, zero warnings.
  - [x] "`tools/kernel_ci.py` PASS" — PASS on Windows, wired into `cargo test`.
  - [x] "Docs exist" — `docs/SPEC_TOS.md` (SSOT) created; PROGRESS/DECISIONS/
        OPEN_QUESTIONS/FUTURE_IDEAS/BENCHMARKS all present from v1 and extended
        here (D043, D044, Q10).
  - [x] "Scope frozen" — no SMP, UEFI, networking, virtualization or multi-user;
        carried forward verbatim from §1.4.

## Phase 1 — Filesystem (FS) ✅

### TOS-P1.1 — FS design + storage primitives ✅

- THINK: T1.1 wants a flat, in-memory namespace with a `/TOS/` convention;
  T1.2 wants `fs_read`/`fs_write`/`fs_list` as builtins; G1 wants "no C/Rust
  hardcoded FS logic". So the split must be: raw *storage* in the runtime, all
  *policy* (naming, globs, listing format, messages) in HolyRust. Where does
  the storage live? A process-global would be racy — `cargo test` runs tests on
  parallel threads — so the store is a field on `Rt`, i.e. one filesystem per
  `Session`. The kernel has exactly one Session, so that *is* the system's
  filesystem; host tests get isolation for free. Risk: `Rt` is read by
  generated code via `offset_of!`, so the new field must go last.
- EXECUTE: `Fs`/`FsEntry` (`name: String`, `data: Vec<u8>`, 64 entries × 16 KiB
  budget) as the last field of `Rt`; four builtins `FsWrite` / `FsRead` /
  `FsList` / `FsDelete` wired through `BuiltinId` → `FnTable` → `OFF_FS_*`.
  Codegen had documented "max one arg in v1", so the builtin call path was
  extended to two SysV argument registers, with arguments evaluated into a
  reserved stack block first (a later argument's code can clobber a register
  already holding an earlier one — the same reason `Print` does it).
  `FsRead`/`FsList` return `Str` blocks minted from an arena the FS owns, so no
  builtin needs `&mut Session` while the guest is running.
- TEST: `cargo build --release` clean; host suite unchanged at that point.
- REVIEW: returning a `Str` from a builtin required fixing a latent bug —
  `check_call` hardcoded `ret: Type::I64` for every builtin, so
  `Str s = FsRead(p);` would have been rejected. Now driven by
  `BuiltinId::ret_ty()`. Caught by writing the HolyRust layer, not by luck.
- DECIDE: ✅ ADVANCE (D045).

### TOS-P1.2 — two real v1 gaps, found by writing the OS in its own language ✅

- THINK: writing `/TOS/Files.HR` is the honest test of "is HolyRust expressive
  enough to host kernel services?". Build the layer first, keep the storage
  dumb, and let the language complain.
- EXECUTE: none — this is the diagnosis step. Four failures, each pinned by a
  minimal reproduction (probe scripts driving the real `hr` binary), never by
  guesswork:
  1. **`Str[i]` traps at `i == len`** (correct: the NUL terminator is not an
     addressable element). So the classic `while (s[i] != 0)` scan traps
     instead of terminating — and v1 exposed **no** way to ask a `Str` its
     length. **FIX:** new builtin `StrLen(Str) -> I64`.
  2. **Passing a `Str` variable/parameter to a *user-defined* function moves
     it** (D008 move semantics) — the caller cannot read or re-pass it. Literals
     are immune (immortal), builtins are immune, plain reads are immune. So one
     `Str` cannot be compared against several literals in HolyRust, and a glob
     pattern cannot be handed to a matcher once per loop iteration.
     **FIX:** new builtin `StrEq(Str, Str) -> I64` (non-consuming), plus
     restructuring `/TOS/Files.HR` to hold the current traversal's pattern and
     entry in globals and match with a **zero-argument** helper.
  3. `Str *p` (borrow) parameters **crash** (0xC0000005) — a genuine v1 defect,
     unrelated to this phase; recorded rather than worked around silently
     (OPEN_QUESTIONS Q12).
  4. `robocopy`-style silent no-ops have no counterpart here, but the same
     lesson recurred: the first probe "passed" for the wrong reason until the
     assertions were made specific.
- TEST: each hypothesis reproduced in isolation, e.g.
  `Str s = "hello"; I64 n = 0; while (s[n] != 0) { n = n + 1; }` → trap;
  `Str v = "abc"; Print(F(v), " ", v[0]);` → trap, while
  `Print(F("abc"), " ", F("abc"));` → `3 3`; `I64 G(Str *p)` → access violation.
- REVIEW: the two new primitives are not scope creep — they are the minimum a
  language with a `Str` type must offer before *any* program can be written in
  it, and both mirror TempleOS (which has `StrLen`/`StrCmp` builtins). They are
  primitives, not policy: no filesystem or shell behaviour moved into Rust.
- DECIDE: ✅ ADVANCE (D047).

### TOS-P1.3 — `/TOS/Files.HR`, `/TOS/Shell.HR`, boot load, tests ✅ (G1)

- THINK: T1.3/T1.4 want the user-facing filesystem and command processor to be
  HolyRust *source*; T1.5 wants the boot script to load `/TOS/*.HR` and
  evaluate them before the prompt; T1.6/T1.7 want listing and lifecycle tests;
  G1 wants hot-patching the FS layer to leave the FS working. Risk: the kernel
  shell must still own reading a line and compiling it — that is what a JIT
  host *is* — so T1.4 has to be honest about which half moves.
- EXECUTE:
  - `tos/Files.HR`: `Dir`, `FWrite`, `FRead`, `FDel`, `Cat`, `Exists`, `Count`,
    `Size`, plus the byte toolbox (`StartsWith`, `EndsWith`, `StrFind`,
    `MatchCurrent`, `GlobMatch`).
  - `tos/Shell.HR`: `Help` and `Sh(word)` — the command table and every message.
  - `tos/Doc/TempleOS-RS.txt`: the OS's own documentation, stored *in* the OS.
  - `kernel/src/shell.rs`: `SYSTEM_FILES` (`include_str!`) written into the FS
    and evaluated at boot; a lexically-detected bare command word is routed to
    `Sh("<word>")`. Only lexing decides "command vs. source", so all command
    policy stays in HolyRust; the word is a validated identifier, so the
    generated call cannot be injected into.
  - `tests/tos_fs.rs`: 4 host tests that boot the same source the kernel boots.
- TEST:
  - Host: `cargo test --release` → **101 passed / 0 failed, exit 0, zero
    warnings** (97 v1 + 4 new; every previously-green test still green).
  - Bare metal: `tools/kernel_ci.py` → `KERNEL-CI: PASS (boot 1.18s)` with
    Phase 1 markers asserted in-transcript —
    `Dir("/TOS/*.HR")` → `2 entries` (both system sources),
    bare `ls` → all three entries including the doc,
    `FWrite`/`Size`/`Exists`/`FDel` lifecycle (`size=10 exists=1` →
    `gone=0 size=-1`),
    **G1 hot-patch**: `Count()` recompiled live to `4242` while
    `Exists("/TOS/Shell.HR")=1`, `Cat` still returned the doc, and
    `HeapCheck()=1` after the patch.
- REVIEW: three boot failures were fixed from evidence, not guesswork:
  (a) a non-ASCII em dash in a `b"..."` literal and a `u32` index into `&str`
  (both caught by the compiler); (b) the bare-word router never fired because
  the token stream always ends in `Eof`, so "one token" meant "one token plus
  Eof" — the serial transcript showed `error[C004]: unknown identifier 'ls'`,
  which located it immediately; (c) a failed CI session leaked its QEMU process
  and locked the chardev logfile for the next run — `fail()` now always reaps
  QEMU. Duplication is confined to one documented place (`Dir` cannot call
  `GlobMatch` per iteration under the move rule, so `GlobMatch` is the one-shot
  wrapper over `SetPattern`/`MatchCurrent`).
  Honest boundary: the interactive read/compile loop stays in Rust (D046) —
  that IS the JIT host, and only the *command policy* moved to HolyRust.
- DECIDE: ✅ **ADVANCE to Phase 2.** G1 checklist:
  - [x] "FS works entirely from HolyRust source, no hardcoded FS logic" —
        the runtime knows only bytes: 4 storage + 2 string primitives; every
        naming/glob/listing/message rule is in `/TOS/Files.HR`+`Shell.HR`.
  - [x] "Boot script loads /TOS/*.HR and evaluates them before the prompt" —
        `TOS: /TOS loaded` precedes `kernel: ready` in the transcript.
  - [x] "Hot-patching /TOS/Files.HR at runtime doesn't break the FS" —
        live `Count()` patch + continued `Exists`/`Cat` + `HeapCheck()=1`,
        asserted in CI and in `tests/tos_fs.rs`.

## Phase 2 — Graphics (VGA 640x480x16) ✅

### TOS-P2.1 — Mode 12h without a BIOS ✅

- THINK: T2.1 says "VGA mode set (mode 12h) — port I/O + BIOS calls via VBE in
  boot.S". But `boot.S` is entered by the multiboot loader in **32-bit protected
  mode**, so `INT 10h` is unreachable without a real-mode excursion, and
  `boot.S` is also the file that already boots — Phase 0's rule is never to break
  a passing gate. Risk: mode 12h is *planar*, so every write depends on the
  Graphics Controller being configured correctly, and a wrong guess shows up as a
  blank or garbage screen with no debugger available.
- EXECUTE: `kernel/src/gfx.rs` programs the mode directly — miscellaneous output,
  Sequencer, CRTC (releasing and re-asserting CR0-CR7 write protection), Graphics
  Controller, Attribute Controller, then the DAC with the 16 standard EGA colours
  written out explicitly so the palette is what the source says. The console is
  routed through the framebuffer once graphics is active (`vga::write` →
  `gfx::put`), which is what BC2 asks for.
- TEST: `KERNEL-CI` boots, `GfxInit()` returns 1, the screendump is **640x480**
  (text mode was 720x400), and `GfxClear(15)` fills 306,906 pixels with
  (255,255,255).
- REVIEW: keeping the BIOS out of it makes the whole feature port I/O, and the
  screendump is a ground-truth oracle for every register guess. Recorded as a
  deliberate deviation from the letter of T2.1 (D048).
- DECIDE: ✅ ADVANCE.

### TOS-P2.2 — Three bugs, each found by measurement ✅

- THINK: the exit gate wants "VGA screendump shows drawn content", so the test
  must assert **exact pixels** rather than "the screen is not blank". That single
  decision is what exposed all three bugs below.
- EXECUTE / TEST / REVIEW — the three failures, in the order they were pinned:
  1. **Write mode 0 cannot expand a colour across planes.** In write mode 0 a
     data byte's bit *j* reaches every selected plane at pixel position *j*, so
     only colours 0 and 15 are reachable. Symptom: `GfxClear(15)` filled the
     screen while `GfxClear(0)` and `GfxClear(4)` did nothing visible.
     **FIX:** write **mode 2**, where the data byte is expanded — bit 0 → plane
     0, bit 1 → plane 1, … — so `Plot(x,y,c)` writes any of the 16 colours.
  2. **The VGA latches.** With a bit mask the controller *merges* the write into
     its four plane latches. Stale latches let a masked write also revert bits an
     earlier plot set **in the same byte**, so of two pixels sharing a byte the
     first silently vanished. Isolated with a five-case matrix:
     `Plot(100,100,15); Plot(101,100,4);` → only the second survived, while
     `Plot(200,200,15); Plot(300,300,15);` → both survived (different bytes).
     **FIX:** read the byte first to load all four latches, then do the masked
     write (documented in `gfx::plot`).
  3. **The build was never being recompiled at all** — which is why the first two
     fixes appeared to change nothing: three genuinely different `gfx.rs`
     implementations produced **byte-identical** kernels. `cargo -v` reported
     `Fresh hr-kernel`; the cause is that this host's wall clock **jumps
     backwards**, so `libhr_kernel.a` carried mtime 17:08 while the clock read
     16:22 and cargo judged every freshly edited source older than its own
     outputs, permanently. Touching the sources cannot fix that; the
     future-dated `target` has to go. **FIX:** `kernel/build.ps1` clears a
     future-dated target before building and `tools/cargo.ps1` does the same for
     host builds (D050). The lesson: "the test passed" is worthless until you
     have confirmed the code under test was actually compiled.
- DECIDE: ✅ ADVANCE.

### TOS-P2.3 — `/TOS/Graphics.HR`, tests, gate G2 ✅

- THINK: T2.2 names `Plot`, `Line`, `Rect`, `Circle`, `TextOut`. Only `Plot`,
  `GfxChar`, `GfxClear` and `GfxInit` need to be Rust — the rest is policy and
  belongs in HolyRust, exactly as Phase 1 split `/TOS/Files.HR` from the storage
  primitives (SPEC_TOS §2.3). Graphics is a *kernel service*, so the runtime must
  not contain a driver: it defines `GfxHooks` and forwards, the same shape as
  `hr_out`'s hooks and `KERNEL_CHUNK`.
- EXECUTE:
  - `hr_runtime`: `GfxHooks { init, clear, plot, glyph }` + a `GFX` static;
    builtins `GfxInit`/`GfxClear`/`Plot`/`GfxChar`; codegen extended from two to
    four SysV argument registers (rsi, rdx, rcx, r8).
  - `tos/Graphics.HR`: `Line` (Bresenham), `Rect`, `FillRect`, `Circle`,
    `FillCircle` (midpoint), `TextOut`, `TextCentered`, `GfxDemo` — all HolyRust.
  - `kernel/src/font8x8.rs`, generated by `tools/gen_font8x8.py` from the
    public-domain font8x8 table, so glyph rendering does not depend on what the
    firmware happened to leave in VGA plane 2.
  - `tos/Shell.HR` gains a `demo` command word; `/TOS/Doc/TempleOS-RS.txt`
    documents the graphics layer.
- TEST:
  - Host: **105 passed / 0 failed, exit 0, zero warnings** (97 v1 + 4 Phase 1 +
    4 new). `tests/tos_gfx.rs` proves the layer compiles, that shapes terminate
    and clip, and that the arity contract is enforced; `tests/tos_gfx_args.rs`
    installs **recording** `GfxHooks` and asserts the exact `(x, y, colour)` the
    codegen passes — that turned a hard-to-see codegen suspicion into a passed
    assertion (the codegen was in fact correct; the fault was the latches).
  - Bare metal: `KERNEL-CI: PASS (boot 1.80s)` — `GfxInit()=1`; screendump
    **640x480**; exact pixels (white/red/green placed, neighbours still black);
    `Line`/`Rect`/`FillRect`/`Circle`/`TextOut` from HolyRust visibly drawn;
    BC2 shows `TextOut` at 507 lit px in the requested region while a cleared
    screen stays black, and the console prompt still renders (19 px) after the
    mode change; `HeapCheck()=1` afterwards.
- REVIEW: honest deviations, all recorded rather than hidden — (a) mode set by
  direct registers instead of BIOS/VBE (D048); (b) the font is compiled in rather
  than read from plane 2; (c) `GfxInit` returns 0 with a diagnostic when no
  device exists, so the host suite states what it cannot exercise instead of
  pretending. Pixel-exact screendump assertions are the strongest evidence
  available for "graphics commands work".
- DECIDE: ✅ **ADVANCE to Phase 3.** G2 checklist:
  - [x] "Graphics commands from the shell work" — `GfxInit`, `GfxClear`, `Plot`,
        `GfxChar` builtins plus the whole HolyRust shape layer, driven from the
        in-kernel shell and from the bare `demo` command word.
  - [x] "VGA screendump shows drawn content" — asserted at exact coordinates.
  - [x] BC1 (boot, enter graphics, draw from the shell) and BC2 (text overlay in
        graphics mode; system still bootable — the session continues to `#quit`
        and resets cleanly afterwards).

## Phase 3 — Input & Timer ✅

### TOS-P3.1 — Polled millisecond clock + CMOS date ✅

- THINK: T3.x wants "PIT programming (channel 0, polled tick counter, no
  interrupts v1)" plus `Time()`, `Date()` and `Sleep(ms)`. Polling a down-counter
  has an obvious failure mode: a counter that wraps between samples loses time.
  Risks: reading the counter inconsistently (latch discipline), assuming BCD
  where the RTC is binary, and `Sleep` either hanging or returning instantly.
- EXECUTE: `kernel/src/timer.rs`. Channel 0 is a rate generator with divisor
  1193, so one counter unit is one millisecond; `poll()` accumulates wraps
  (a wrap is simply `now > last`, because the counter counts down) and is called
  both from the shell's idle loop and from `Time()`/`Sleep()` themselves. `Date()`
  reads the CMOS RTC, waits out update-in-progress, honours register 0x0B bit 2
  for BCD-vs-binary, and returns 0 rather than a wrong date if the fields are
  implausible.
- TEST: `Sleep(100)` returns and the guest's own clock advanced by at least
  100 ms; `Date()` returns `20261015`, matching the host's calendar.
- REVIEW: an honest characteristic to record — on this TCG host `Sleep(100)`
  measured **635 ms**. The clock is not wrong; a polled clock's *resolution* is
  bounded by how often it is polled, and each poll is three port-I/O operations
  which are expensive under emulation, so the wait overshoots by a large
  fraction. The gate is "returns without hanging" and the assertion allows up to
  2 s precisely because the tight bound would be a lie on an emulated host.
- DECIDE: ✅ ADVANCE.

### TOS-P3.2 — PS/2 mouse and a non-destructive cursor ✅

- THINK: T3.x wants a polled PS/2 mouse whose position drives a cursor. Two
  hazards stood out: the auxiliary port shares the 8042 with the keyboard, and a
  cursor that "erases" by painting black destroys whatever is underneath.
- EXECUTE: `kernel/src/mouse.rs`. `MouseInit` enables the auxiliary device,
  clears the aux-clock-disable bit in the controller configuration, sets
  defaults and enables data reporting; `poll()` drains whole 3-byte packets,
  resynchronises on byte 0's mandatory bit 3, drops overflow packets, and
  applies dx/dy with the PS/2 sign convention. The cursor is a 5x5 crosshair
  that **saves the pixels it covers** (via the new `gfx::read_pixel`) and
  restores them before moving. `GfxInit` reveals the cursor; anything that
  repaints the screen wholesale calls `mouse::invalidate()` so a stale saved
  background is never restored over a new screen.
- TEST: with the monitor driving `mouse_move 20 12` six times, the OS reports
  `(320,240) -> (440,312)` — exactly `+120,+72`, which validates packet assembly,
  signs and clamping. The cursor is then found on screen at that position.
- REVIEW: **a real defect surfaced while wiring this up**: `kbd::poll()` read the
  data port whenever the output buffer was full, without checking status bit 5.
  That was harmless while no auxiliary device existed, and became a byte-stealing
  race the moment the mouse was enabled — the two pollers would have eaten each
  other's data. Fixed by having the keyboard ignore (and leave in place) any byte
  flagged as auxiliary, and it is exactly the kind of latent bug a new caller
  exposes rather than a new line of code.
- DECIDE: ✅ ADVANCE.

### TOS-P3.3 — Hooks, tests, gate G3 ✅

- THINK: Phase 3 services are hardware-bound, so they follow the Phase 2 pattern
  exactly: the runtime declares `TimeHooks` and `MouseHooks` and forwards, and a
  build with no device reports zero instead of inventing a clock or a position.
- EXECUTE: `TimeHooks { millis, sleep_ms, date }`, `MouseHooks { init, x, y }`,
  builtins `Sleep`/`Time`/`Date`/`MouseInit`/`MouseX`/`MouseY`, installed by the
  kernel at boot.
- TEST:
  - Host: **106 passed / 0 failed, exit 0, zero warnings** (105 + 1 new).
    `tests/tos_time.rs` installs recording hooks and proves the builtins forward
    faithfully — `Time()`, `Sleep(250)` returning 250 and advancing the clock to
    1250, `Date()`, and the mouse position changing between reads — then proves
    that with **no** hooks installed the same builtins report `0|0|0`, so a build
    without a clock never fabricates one.
  - Bare metal: `KERNEL-CI: PASS` with `Sleep(100)` waiting 635 ms of real time
    and returning (G3), `Date() = 20261015`, the mouse moving
    `(320,240) → (440,312)`, and the cursor found on a cleared screen.
- REVIEW: the boot script now brings up the mouse and prints `mouse: 1 clock: Nms
  date: YYYYMMDD`, so every one of these services is exercised on every boot, not
  only when a test asks.
- DECIDE: ✅ **ADVANCE to Phase 4.** G3 checklist:
  - [x] "`Sleep(100); Time();` returns without hanging" — asserted with a real
        elapsed-time floor, not just "it returned".
  - [x] "Mouse cursor visible and movable via monitor `mouse_move` in CI" —
        the OS-reported position tracks six monitor-driven moves exactly, and the
        cursor is then found on the framebuffer.

## Phase 4 — Sound & Extras ✅

### TOS-P4.1 — PC speaker, the oracle, and a demand-driven idle loop ✅

- THINK: T4.x wants `Beep(freq, ms)` and `Music(notes)` on the PC speaker, plus
  the God-speaker oracle. Two things needed deciding up front. First, a headless
  CI cannot hear anything, so the *evidence channel* had to be chosen deliberately
  — G4 permits "at least a serial log of frequency changes", so the kernel logs
  every tone through `hr_out` **before** it waits. Second, the oracle needs
  entropy, and with interrupts out of scope the CPU timestamp counter is the only
  honest source available.
- EXECUTE:
  - `kernel/src/sound.rs`: PIT channel 2 in mode 3 (square wave) with the divisor
    for the requested frequency, gated into the speaker via port 0x61 bits 0-1;
    each tone is logged, then the wait, then silence.
  - `kernel/src/rng.rs`: xorshift64\* seeded and stirred by `rdtsc` on every call.
  - `SoundHooks { beep, off }` and `RngHooks { below }` follow the Phase 2/3
    pattern; builtins `Beep`, `SoundOff`, `Random`.
  - `tos/Sound.HR`: named notes, `Music(...)` (a variadic tune), `Scale`,
    `Fanfare`, `Chirp`. `tos/Oracle.HR`: eight verses, `Oracle()` and `OracleN(k)`.
  - `/TOS/Shell.HR` gains the `beep` and `oracle` command words.
- TEST:
  - Bare metal: `Beep(440,120)` returns 1 and the transcript contains
    `[tone] 440 Hz for 120 ms`; `Music(262,330,392,523)` logs **four distinct
    tones**; the bare `beep` word reaches `Scale()`; `Random(10)` is in range; and
    `OracleN(8)` produced **5 distinct verses** in 8 draws (`KERNEL-CI: PASS`).
  - Host: `tests/tos_sound.rs` installs recording/lying hooks and proves the
    builtins forward faithfully — and that `Random(n)` is **clamped** to `0..n`
    even when the entropy source returns `n+89` or a negative number. The oracle
    indexes a word list with that value, so a misbehaving source must not become
    an out-of-range index in guest code.
- REVIEW: three real problems surfaced, all fixed and recorded rather than
  papered over:
  1. **The boot gate regressed to 6.5 s.** Cause: `Date()`'s
     update-in-progress guard could spin a million port-I/O iterations, and the
     mouse's controller waits could spin 100k. Both bounds are now 10k — tens of
     milliseconds worst case — and boot returned to ~4.2 s against the < 5 s gate.
     A "wait until ready" loop with no tight bound is a boot-time landmine.
  2. **Phase 2 stopped passing** after the mouse existed. The cursor was being
     repainted on *every* idle-loop iteration (~250 port operations each), which
     starved the guest enough that command processing fell behind the CI. The
     cursor is now redrawn only when it is *dirty* — repainting something that has
     not moved is pure cost.
  3. **A screendump raced the guest.** Several CI steps slept a fixed `settle` and
     then captured the screen; the drawing was still in progress often enough to
     fail intermittently. Each graphics step now ends with a guest-printed marker
     that the CI reads before capturing, so the assertion is ordered against the
     guest's own progress instead of against a guess.
  Also recorded: `Sleep`'s measured value on this host varies (635 / 882 / 1509 /
  2069 ms for the same 100 ms request). That is not clock error — a polled clock
  expresses host scheduling as overshoot, and when the emulator is descheduled the
  PIT keeps counting. The CI floor proves time advances; the ceiling only catches
  a wildly wrong clock (D051).
- DECIDE: ✅ **ADVANCE to Phase 5.** G4 checklist:
  - [x] "Sound produces audible output (or at least serial log of frequency
        changes)" — every tone is logged with its exact frequency and duration,
        and distinct frequencies are asserted for a multi-note tune.
  - [x] "The oracle picks from a word list" — eight verses in `/TOS/Oracle.HR`,
        chosen through a real entropy source, verified to vary, and unable to
        index out of range.

## Phase 5 — Self-Hosting & Documentation Close-Out ✅

### TOS-P5.1 — `#include` and a line editor over the OS's own source ✅

- THINK: T5.x needs an editor builtin and `#include "file.HR"`. Neither existed:
  there was no `#include` in the language at all, and an editor needs a buffer
  that belongs to the *running machine* — because the file being edited is
  frequently the OS's own source. Risk: `#include` is a text-level operation that
  could misfire inside a string or comment, and a cycle could recurse forever.
- EXECUTE:
  - `pipeline::expand_source` resolves `#include "path"` **before parsing**,
    because that is what the directive means: the included source *becomes* part
    of this unit. The line shape is strict (the directive must be the whole line,
    with only whitespace or a `//` comment after it), so it cannot match inside a
    string or comment. Nesting is capped at 8 like `#exe` (D014), and a missing
    file is a positioned `C094` error, never a silent no-op. Lookup order is the
    session's filesystem **first** — that is what lets the OS include its own
    source — then, on the host only, the real filesystem.
  - The editor buffer (`EdBuf { path, lines, dirty }`) lives on `Rt` beside the
    filesystem, so "the file you are editing" is part of the live session. Seven
    primitives (`EdOpen`, `EdLines`, `EdLine`, `EdSet`, `EdFind`, `EdSave`,
    `EdClose`) are the plumbing; `/TOS/Editor.HR` supplies the commands (`Ed`,
    `EdShow`, `EdReplaceLine`, `EdSaveAndReport`) in HolyRust.
  - `OsName()` was added to `/TOS/Files.HR` as a deliberately **one-line**
    function, so the self-edit E2E has a target that can be rewritten without
    disturbing the code around it.
- TEST:
  - Host: `tests/tos_edit.rs` proves `#include` splices filesystem source into
    the unit (and that its effects are visible immediately), that a missing
    include is a `C094` error, that a **cycle is caught by the depth cap rather
    than running forever**, and that ordinary source merely *mentioning*
    `#include` is untouched. A second test runs the whole G5 loop on the host:
    open `/TOS/Files.HR`, find the one-line function, rewrite it, save, reopen to
    confirm the text reached storage, re-include, and observe the new behaviour —
    with the filesystem and heap still healthy. It also pins the editor's refusal
    to invent lines (`EdSet` out of range returns 0) so a bad index cannot
    corrupt a file.
  - Bare metal: `KERNEL-CI: PASS` with the full E2E — editor opened
    `/TOS/Files.HR` (236 lines), rewrote line 235, saved, `#include`d the file
    back, and `OsName()` returned `self-edited` while
    `Exists("/TOS/Shell.HR") = 1`, `Count() = 7` and `HeapCheck() = 1`.
- REVIEW: one build failure was instructive rather than annoying: the host build
  accepted `src.to_string()` while the **kernel** build rejected it, because
  `ToString` is in `std`'s prelude and not in `alloc`'s. The dual-mode build is
  therefore doing real work — the no_std target is a second, stricter compiler
  for this code. Also fixed a genuine boot-cost problem: the boot script was
  compiled as seven separate units, which under emulation was measurable against
  the < 5 s gate; it is now one unit (order preserved, definitions still register
  as they are checked), and the best observed boot dropped to **1.77 s**.
- DECIDE: ✅ ADVANCE to close-out.

### TOS-P5.2 — Gate G5, benchmarks, documentation ✅

- THINK: G5 is the whole thesis in one test, plus the requirement that the
  documentation say so. The honest risk is claiming the E2E "works" because a
  script passed once: it has to hold alongside the other four phases, in one
  session, on every run.
- EXECUTE: `tools/kernel_ci.py` gained the Phase 5 block (open → find → rewrite →
  save → re-include → verify behaviour → verify filesystem + heap). `README.md`
  now documents the claim with the runnable loop that demonstrates it, plus the
  layer table that shows exactly which parts are Rust and which are HolyRust.
  `docs/BENCHMARKS.md` was regenerated from measurement and extended with the
  kernel numbers and an honest reading of each. `/TOS/Doc/TempleOS-RS.txt` — the
  documentation that lives *inside* the OS — documents the editor, `#include`,
  every command and every primitive, so `docs` at the prompt is current.
- TEST: `cargo test --release` → **109 passed / 0 failed, exit 0, zero warnings**
  across 21 targets, including the scripted QEMU session that runs all five phase
  gates end to end. Boot best observed 1.77 s against the < 5 s gate.
- REVIEW: the E2E initially "passed" only when the host was idle; the console-text
  assertion in BC2 proved flaky under load because it sampled a fixed screen
  region while the guest was still rendering. That is the same class of mistake
  as the earlier `settle` races — asserting against a guess about time instead of
  against the guest's own progress — and the fix in both cases is a marker the CI
  reads. Recorded rather than re-run until green.
- DECIDE: ✅ **PROJECT CLOSE-OUT.** G5 checklist:
  - [x] "The E2E test: boot → `Ed(\"/TOS/Files.HR\")` → change a function →
        `#include` it back → FS still works" — asserted in QEMU every run.
  - [x] "Full `cargo test` (host + kernel CI) green" — 109/109, exit 0.
  - [x] "README documents: 'This OS is written in itself. Edit any file,
        re-include it, the OS changes.'" — with the executable demonstration
        directly beneath it.
  - [x] "Boot time < 5 s (target < 1 s)" — best observed 1.77 s; the target was
        not met and the gap is explained (the OS compiles its own source at boot)
        rather than hidden.
  - [x] "Memory safety: `HeapCheck() = 1` after all operations" — asserted in
        Phases 1, 2, 3 and 5.
  - [x] "Documentation: every file in `/TOS/` is documented, D-numbers cover all
        decisions" — `/TOS/Doc/TempleOS-RS.txt` covers every file and primitive;
        D001–D056 record every non-trivial choice, including the ones that went
        wrong first.

### Honest scope statement (SPEC_TOS §6.2)

All six success criteria are met or explicitly qualified:

| Criterion | Status |
| :--- | :--- |
| Boot < 5 s (target < 1 s) | **< 5 s met** (1.77 s best); < 1 s target **not met** — the OS JIT-compiles its own six sources at boot, which is the design, not slack. |
| All CI gates green | Host **109/109**, kernel CI `PASS` — every run. |
| Self-editing demo | Verified in QEMU and on the host; the OS rewrites its own source and keeps running. |
| `HeapCheck() = 1` | Asserted after filesystem work, after graphics, after the hot-patch, and after the self-edit. |
| Documentation | `/TOS/Doc/` inside the OS; `docs/` outside; D001–D056. |
| Frozen scope respected | No SMP, UEFI, networking, virtualization or multi-user was added (SPEC_TOS §1.4). |

Deferred and quarantined rather than silently dropped: unquoted shell arguments
and `Str` building primitives (F18), richer globs (F19), IRQ-driven input
(F16), W^X/NX hardening (F6), and the `Str *p` parameter defect found in Phase 1
(OPEN_QUESTIONS Q12).

## Phase 7 — AfterEgypt-Style Flight Simulator ✅

### TOS-P7.1 — F20 recorded, then `/TOS/Flight.HR` implemented ✅

- THINK: the request was an AfterEgypt-style flight simulator, guest code only,
  on existing Gfx builtins. Recording it as F20 *before* implementing (SPEC_TOS
  Part 4.4) mattered here: writing the idea down forced two questions up front —
  can guest code read the keyboard while it runs, and does the mouse position
  update while guest code owns the machine? Both answers turned out to be *no*,
  which reframed the phase from "write a demo" to "fix two defects, then write a
  demo". The third question — what maths can a guest actually do? — was settled
  by measuring rather than assuming: F64 literals, arithmetic, comparison, casts
  and printing all work, so the trigonometry is real floating point, not a
  fixed-point approximation.
- EXECUTE:
  - `tos/Flight.HR` (~250 lines, all HolyRust): `Sin`/`Cos` by range reduction
    plus a 7th-order series (error < 2e-4, far finer than 640x480 can show);
    projection with yaw in world space, pitch as a vertical shift and roll as a
    screen-space rotation; a 3x3 procedural field of wireframe pyramids; a
    tilted, perpendicular-offset horizon and ground grid; a HUD with throttle and
    altitude bars; mouse X = roll, mouse Y = pitch, `w`/`s` = throttle,
    `space` = level, `r` = reset, `q` = land.
  - Frame-driven by design: `FlightStep()` renders exactly one frame and
    returns, because a cooperative OS with no scheduler should not own the
    machine forever, and because one frame is the unit a script can observe.
  - `MouseX()`/`MouseY()` now **sample the device** (D058); `SimKey()` falls back
    to the **real keyboard** via a kernel-installed `KeyHooks` (D057).
  - `/TOS/Shell.HR` gains the `flight` command word; `SYSTEM_FILES` seeds
    `Flight.HR`; the boot transcript now names each file as it compiles
    (`TOS: compiling <path>`), which is what turned a silent triple fault into a
    one-line answer.
- TEST:
  - Host (`tests/tos_flight.rs`, recording graphics + scripted mouse + scripted
    keyboard): a frame draws >2000 plots in all five scene colours; a level
    aircraft draws a **level** horizon (< 3 px difference across 140 px); banking
    tilts it (≥ 5 px); pitching moves it (≥ 5 px); `space` levels it again; `w`
    and `s` move the throttle through the key fallback and it clamps at 100;
    `FlightExit()` leaves the OS healthy (`HeapCheck() = 1`, 8 files).
  - Bare metal (`KERNEL-CI: PASS`): flight mode entered with the horizon level at
    y=240; two monitor `mouse_move 60 0` deltas banked the aircraft and tilted the
    horizon **+72 px**; `mouse_move 0 60` pitched it and moved the horizon
    **−68 px**; exit clean with `heap:count = 1:8`.
- REVIEW: three defects surfaced, two of them *latent since Phase 0* and one of
  them serious.
  1. **The kernel could not do floating-point arithmetic at all** (D059).
     `/TOS/Flight.HR` killed the kernel instantly on boot. Bisecting the file was
     misleading — the only prefix that booted was the one that *failed to
     typecheck*, so no code was generated and nothing could fault. Probing the
     language instead of the file gave the answer in one run: an `F64` global
     printed fine, but `x * 2.0`, an F64 comparison, an F64 loop and an F64→I64
     cast each killed the kernel. Those are exactly the operations that need SSE,
     while copying an F64 as a bit pattern does not. **`boot.S` never set
     `CR4.OSFXSR`**, so every SSE instruction raised #UD, with no IDT to report
     it. Five phases had never noticed because no `/TOS` code had ever done
     arithmetic on a float.
  2. **Every glyph in graphics mode was drawn mirrored** (D060). The flight HUD
     made it visible; the console overlay had been mirrored since Phase 2 and no
     test had ever *read* the text, because every assertion counted lit pixels —
     and a mirrored glyph has exactly the same count. The font table
     (`font8x8_basic`) stores bit 0 as the leftmost pixel; `gfx::glyph` read bit 7
     first, following the table's own header comment, which was wrong. Fixed in
     the renderer and the comment, plus a CI check that reads the *shape*: `/`
     must lean forward.
  3. **CI's `read_value` could read the shell's own echo.** The shell echoes what
     you type, so a `Print("TOS-CI x=", ...)` command puts the prefix in the
     transcript before the value arrives; the reader took the last match and
     returned source text. Now a candidate containing `(`, `)`, `"` or `;` is
     recognised as an echo and skipped. This was latent in Phases 3–5 too and only
     fired when the guest was slow enough to lose the race.
- DECIDE: ✅ **G7 met.** Boot → flight mode → horizon pixels shift with
  simulated mouse deltas (both axes, measured in pixels) → clean exit, with the
  filesystem and heap verified afterwards. F20 is promoted rather than dropped,
  and the two Rust changes it needed are defects fixed, not features added: the
  simulator itself contains no Rust at all.

**What Phase 7 says about the rest of the project.** Two of the three bugs found
here had been sitting in the tree since Phase 0 and were invisible to a suite
that had been green for five phases. Neither was found by adding tests to code
that already had tests; both were found by *using* the system for something new
and looking at the result. The pixel-count assertions were not wrong, they were
just unable to see shape — and the moment the OS had to render something a human
would actually read, the mirroring was obvious in the first screenshot.
