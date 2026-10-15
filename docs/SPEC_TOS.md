# 📋 SPEC_TOS.md — TempleOS-RS: Building TempleOS Using HolyRust

## Technical Specification & Agent Protocol Document — v1.0 (FROZEN SCOPE)

> **Provenance (D043):** this is the TempleOS-RS build specification supplied by the
> project owner. It is the **SSOT for the TempleOS-RS work**. The repository's
> pre-existing `SPEC.md` remains the SSOT for **HolyRust v1 itself** (the compiler,
> the language, the P6b kernel) and is not superseded by this document.

---

## Part 0: Document Conventions (MANDATORY READING)

> **⚠️ AGENT INSTRUCTIONS — READ BEFORE ANY ACTION**
>
> 1. This document is the **Single Source of Truth (SSOT)**. Any conflict between your understanding and this document resolves in favor of this document.
> 2. **Work in phases only.** Each phase has entry criteria, tasks, exit gates, and a DECIDE point. Never skip a gate.
> 3. **The loop is inviolable**: THINK → EXECUTE → TEST → REVIEW → DECIDE. Log every iteration in `docs/PROGRESS.md` with evidence.
> 4. **Never guess.** If diagnostics are silent or ambiguous, that's a bug in your instrumentation — fix instrumentation first, then re-test.
> 5. **Honest degradation over silent scope creep.** If something can't be built, document it in `docs/FUTURE_IDEAS.md` and proceed.
> 6. **Never break a passing gate.** A green test suite is a contract. If your change breaks it, revert or fix before proceeding.
> 7. **One language, one compiler, one system.** TempleOS-RS is written IN HolyRust, FOR HolyRust, running AS HolyRust.

---

## Part 1: Project Definition & Vision

### 1.1 The Mission

Build **TempleOS-RS**: a complete, bootable, biblically-themed operating system in the spirit of Terry A. Davis's TempleOS — but implemented **entirely in HolyRust**, leveraging its unique properties:

- **In-kernel JIT compilation** (the shell IS the compiler)
- **Live hot-patching** (modify kernel functions while running)
- **Memory-safe-by-construction** runtime (ownership tracking, heap canaries)
- **Zero external dependencies** (self-contained toolchain)

### 1.2 The Thesis

> *"A 640x480, 16-color, single-tasking operating system where any file is source code, any command is compilation, and any kernel function can be rewritten while the system runs — without ever crashing, without ever rebooting, with memory safety guaranteed by construction."*

This is **not** a TempleOS clone. It is **what TempleOS would have been if Terry had access to a memory-safe language and modern engineering discipline.**

### 1.3 Positioning vs. Existing Systems

| System | Boot | Language | Safety | Live-edit | Multi-tasking |
| :--- | :--- | :--- | :--- | :--- | :--- |
| TempleOS (original) | Real | HolyC/JIT | None | Yes | Cooperative |
| TempleOS-RS (this) | Real | HolyRust/JIT | **Full runtime** | **Yes, verified** | Cooperative (planned) |
| Linux + Rust | Real | Mixed | Partial (modules) | No | Preemptive |

### 1.4 Non-Goals (frozen)

- **NO SMP, NO UEFI, NO networking, NO virtualization, NO multi-user.**
- **NO 64-bit beyond what HolyRust kernel already provides.**
- **NO attempt to replicate Terry's specific UI/UX pixel-for-pixel.**
- **NO backward compatibility with original TempleOS files.**

---

## Part 2: Architecture & System Design

### 2.1 System Layout (Target State)

```
┌─────────────────────────────────────────────────┐
│  TempleOS-RS                                     │
│  ┌─────────────────────────────────────────┐   │
│  │  HolyRust Shell (user-facing)          │   │
│  │  - interactive REPL                     │   │
│  │  - file browser/editor (HolyRust code)  │   │
│  │  - command-line tools (all .HR files)   │   │
│  └────────────────┬────────────────────────┘   │
│                   │ compile_and_run()             │
│  ┌────────────────▼────────────────────────┐   │
│  │  HolyRust Compiler (JIT, in-kernel)     │   │
│  │  - lexer → parser → checker → codegen   │   │
│  │  - hot-patch engine                     │   │
│  │  - ownership runtime (runtime-check)   │   │
│  └────────────────┬────────────────────────┘   │
│  │  Kernel Services (all in HolyRust)      │   │
│  │  - memory manager (arena + canaries)    │   │
│  │  - filesystem (flat, in-memory)         │   │
│  │  - graphics (VGA 16-color, 640x480)     │   │
│  │  - input (PS/2 keyboard + mouse)        │   │
│  │  - timer (PIT, polled in v1)            │   │
│  │  - sound (PC speaker, single-voice)     │   │
│  └────────────────┬────────────────────────┘   │
│  │  Hardware Layer (Rust, minimal)         │   │
│  │  - port I/O wrappers                    │   │
│  │  - IDT (stub v1, full in v2)           │   │
│  │  - bootloader interface                 │   │
│  └─────────────────────────────────────────┘   │
│  Multiboot bootloader (existing boot.S)         │
└─────────────────────────────────────────────────┘
```

### 2.2 Component Map (1:1 to HolyRust lib modules)

| SPEC Component | HolyRust Module | Role in TempleOS-RS |
| :--- | :--- | :--- |
| **G. Boot & HAL** | `kernel/` (existing) | Multiboot entry, long-mode init, VGA/serial/PS2/allocator — **already built and green** |
| **F. Shell** | `hr_shell` → extended | The user's interface — every command is a HolyRust function call |
| **H. Kernel Services** | NEW: `tos/` modules | FS, graphics, timer, sound — all written as HolyRust builtins |
| **B-E. Compiler** | `hr_lexer` → `hr_codegen` | Unchanged from v1 — this is the "engine" |
| **R. Runtime** | `hr_runtime` | Unchanged, plus new builtins registered at boot |

### 2.3 The Core Innovation: Kernel Services as Guest Code

Unlike traditional OSes where kernel services are compiled C/Rust code, **in TempleOS-RS, most "kernel services" are HolyRust source files compiled at boot and JIT-executed**:

```holyrust
// file: /TOS/Files.HR — a "kernel service" (filesystem layer)
U0 Dir(CStr pattern) {
    // ... iteration over FS entries
    for (I64 i = 0; i < fs_num_entries; i++) {
        Print("%s\n", fs_get_name(i));
    }
}
```

This means:
- **The filesystem layer can be edited like any user file**
- **Hot-patching the FS doesn't crash the kernel** (ownership tracking + canaries)
- **The OS is self-describing**: `cat /TOS/Files.HR` shows you the OS's own source

---

## Part 3: Phase Plan & Checkpoints

### Phase 0: Preparation & Baseline

**Goal**: Ensure the HolyRust compiler is stable and dual-mode.

**Entry criteria**:
- HolyRust repo at commit ≥ `[P6b-K5]`
- `cargo test --release` all green
- `tools/kernel_ci.py` PASS

**Tasks**:
1. Verify baseline with `cargo test` and `python3 tools/kernel_ci.py`.
2. Create `docs/SPEC_TOS.md` (this document), `docs/PROGRESS.md`, `docs/DECISIONS.md`, `docs/OPEN_QUESTIONS.md`, `docs/FUTURE_IDEAS.md`.
3. Freeze scope for v1: **NO** SMP, UEFI, networking, multi-user.

**Exit gate (G0)**:
- Baseline is green, docs exist, scope frozen. DECIDE: ADVANCE or STOP.

### Phase 1: Filesystem (FS)

**Goal**: A flat, in-memory filesystem (like TempleOS's RAM-disk approach).

**Tasks (T1.x)**:
| Task | Description |
| :--- | :--- |
| T1.1 | Design FS layout: flat namespace, in-memory, `/TOS/` prefix for system files |
| T1.2 | Implement `fs_read`, `fs_write`, `fs_list` as HolyRust builtins |
| T1.3 | Write `/TOS/Files.HR` in HolyRust — the user-facing FS layer |
| T1.4 | Write `/TOS/Shell.HR` — the command processor (replaces C-era `hr_shell`) |
| T1.5 | Update boot script to load these files from a compiled-in blob or initial RAM disk |
| T1.6 | TEST: boot, then run `Dir("/TOS/*.HR");` → must list all system files |
| T1.7 | TEST: create file, write, read, delete, list — all via the in-kernel shell |

**Exit gate (G1)**:
- FS works entirely from HolyRust source, no C/Rust hardcoded FS logic.
- Boot script loads `/TOS/*.HR` and evaluates them before the interactive prompt.
- Hot-patching `/TOS/Files.HR` at runtime doesn't break the FS (verify with a patch + FS use).

**DECIDE point**: If hot-patching FS breaks anything, revert to built-in FS (Rust-level) and document as D05x.

### Phase 2: Graphics (VGA 16-color, 640x480)

**Goal**: Text and graphics modes, 16 colors, matching TempleOS's visual identity.

**Tasks (T2.x)**:
| Task | Description |
| :--- | :--- |
| T2.1 | VGA mode set (mode 12h — 640x480x16, banked) — port I/O + BIOS calls via VBE in boot.S |
| T2.2 | Implement `Plot`, `Line`, `Rect`, `Circle`, `TextOut` as HolyRust builtins |
| BC1 | Boot, enter graphics mode, clear screen, draw primitives from the shell |
| BC2 | Text overlay in graphics mode — system must remain bootable after mode change |

**Exit gate (G2)**: Graphics commands from the shell work; VGA screendump shows drawn content.

### Phase 3: Input & Timer

**Goal**: PS/2 mouse support, PIT timer for time-of-day, background tick.

**Tasks (T3.x)**:
- Mouse driver (PS/2, polling), cursor position → `Plot` at cursor.
- PIT programming (channel 0, polled tick counter, no interrupts v1).
- `Time()`, `Date()`, `Sleep(ms)` builtins.

**Exit gate (G3)**: `Sleep(100); Time();` returns without hanging; mouse cursor visible and movable via monitor `mouse_move` in CI.

### Phase 4: Sound & Extras

**Goal**: PC speaker music, additional TempleOS-flavored builtins.

**Tasks (T4.x)**:
- PC speaker: `Beep(freq, ms)`, `Music(notes)` — port 0x61 toggling.
- God-speaker random oracle (Terry's oracle — random word/verse selection).
- Dolphin/AfterEgypt style demos as `.HR` files.

**Exit gate (G4)**: Sound produces audible output (or at least serial log of frequency changes); oracle picks from a word list.

### Phase 5: Self-Hosting & Documentation Close-Out

**Goal**: The system boots into a HolyRust REPL that can edit its own source files.

**Tasks (T5.x)**:
- Editor builtin: `Ed("file.HR")` — line editor over the FS.
- `#include "file.HR"` — compile and execute a file, in-context of current session.
- Documentation updates: all files in `/TOS/` documented as "the OS's source code".
- BENCHMARKS: boot time, JIT compile latency, memory usage snapshot.

**Exit gate (G5 — FINAL)**:
- **The E2E test**: boot → `Ed("/TOS/Files.HR")` → change a function → `#include` it back → FS still works.
- Full `cargo test` (host + kernel CI) green.
- README documents: "This OS is written in itself. Edit any file, re-include it, the OS changes."

**DECIDE**: Project complete or future work documented.

---

## Part 4: Agent Protocol (Engineering the Loop)

### 4.1 The Inviolable Loop

Every task, no matter how small, must pass through this loop:

```
THINK
  │  "What do I need to achieve, what could go wrong, what's the test?"
  ▼
EXECUTE
  │  "Write the code / build the thing / run the command."
  ▼
TEST
  │  "Does it match the expected output? Run the gates."
  ▼
REVIEW
  │  "If it failed: WHY? Diagnose with evidence, not guesswork."
  │  "If it passed: Is this robust? Any edge cases?"
  ▼
DECIDE
  │  "Advance to next task? Fix something? Revert? Record in PROGRESS.md."
  ▼
(loop)
```

### 4.2 Logging Requirements

Every iteration must be logged in `docs/PROGRESS.md` as:

```markdown
### Phase X.Y Task Z — [task name] (THINK/EXECUTE/TEST/REVIEW/DECIDE)
- THINK: [reasoning about the task, anticipated risks]
- EXECUTE: [what was built/changed]
- TEST: [what was run, what was measured]
- REVIEW: [analysis of results, root causes of any failure]
- DECIDE: ✅ ADVANCE | 🔁 FIX-AND-RETEST | ⏪ REVERT | 🛑 STOP
  - Reason: [why]
  - Next: [what's next]
```

### 4.3 Decision Documentation (DECISIONS.md)

Every non-trivial architectural choice gets a D-number:

```markdown
| D0xx | [Decision] | [Rationale, including what was rejected and why] |
```

### 4.4 Degradation Protocol

If something proves infeasible:
1. Do NOT silently skip it.
2. Record in `docs/FUTURE_IDEAS.md` with a F-number.
3. Note the degradation in `docs/PROGRESS.md` and `docs/DECISIONS.md`.
4. Choose a simpler path that meets the spirit of the spec.

### 4.5 Risk Management

| Risk | Mitigation |
| :--- | :--- |
| Boot regression (kernel breaks) | Keep a "last known good" kernel.bin; test after every change |
| Hot-patch corrupts memory | Ownership runtime + canaries catch it; revert if HeapCheck fails |
| Scope creep into SMP/UEFI | FUTURE_IDEAS.md quarantine; spec is frozen |
| Agent hallucinates API | All builtins documented in `/TOS/Doc/`; grep before using |

---

## Part 5: Testing & Verification Gates

### 5.1 Test Pyramid

| Level | What | When |
| :--- | :--- | :--- |
| **Unit** | Host-side tests for each builtin (mock the hardware) | Every task |
| **Integration** | QEMU session script (boot → run commands → verify serial/VGA) | Every phase gate |
| **E66E** | Full system test: boot → edit own source → re-include → verify | Phase 5 exit |

### 5.2 Specific Test Scripts

**Per-phase** (added to `tools/tos_ci.py`):
```python
# Phase 1: FS
type_line('Dir("/TOS/*.HR");')
read_until(b"Shell.HR")   # system files listed
type_line('FWrite("/Tmp/x.HR", "Print(42);");')
type_line('#include "/Tmp/x.HR"')
read_until(b"42")

# Phase 2: Graphics
type_line('GraphicsClear(0); Line(0,0,640,480,15);')
# + screendump pixel assert

# Phase 3: Timer
type_line('Print(Time(), "\n");')  # must return a number
# + Sleep(50) must not hang

# Phase 5: E2E self-edit
type_line('Ed("/TOS/Files.HR")')  # enter editor
# modify a function, save
type_line('#include "/TOS/Files.HR"')
type_line('Dir("*")')  # FS still works
```

### 6.2 Success Criteria (Final)

- **Boot time** < 5s (target: < 1s)
- **All CI gates** green (host + kernel + TOS sessions)
- **Self-editing demo** works: OS edits its own source, survives, continues running
- **Memory safety**: HeapCheck() = 1 after all operations
- **Documentation**: every file in `/TOS/` is documented, D-numbers cover all decisions

---

## Part 6: Documentation Deliverables

| Document | Purpose |
| :--- | :--- |
| `SPEC_TOS.md` (this doc) | SSOT for the OS build |
| `docs/LANG_SPEC.md` | HolyRust language definition (existing, extended) |
| replica of v1 docs set | PROGRESS, DECISIONS, OPEN_QUESTIONS, FUTURE_IDEAS, BENCHMARKS |
| `/TOS/Doc/` in the OS itself | The OS's documentation lives inside the OS (like TempleOS's Doc/ directory) |

---

## Part 7: Closing Note to the Agent

> You are not just building an OS. You are building the first memory-safe, self-editing operating system in history. The compiler you already built (HolyRust) is your tool. The kernel you already booted (P6b) is your foundation.
>
> Work the loop. Log everything. Never guess. Never break a green gate.
>
> **The system you build will be one where a user can open a file, change a kernel function, save it, and watch the OS change behavior — without a reboot, without a crash, with memory integrity verified at every step.**
>
> This is Terry's dream, rebuilt with modern tools. Make it work. Make it safe. Make it documented.

---

**END OF SPEC — v1.0 — FROZEN SCOPE — DO NOT EXTEND WITHOUT A NEW D-NUMBER AND PHASE ADDITION**
