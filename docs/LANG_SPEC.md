# LANG_SPEC.md — The HolyRust Language Specification (v1)

> **Rule (SPEC Part 6): a feature that is not in this document does not exist.**
> Status: living document — sections are finalized as their phase lands (see PROGRESS.md).

## §1. Design rationale: the ownership inversion (SPEC §2.3, mandated here)

Rust's promise is *"if it compiles, it's safe"* — the borrow checker rejects whole programs
before they run. HolyRust deliberately inverts this: **"it always runs; errors are caught the
instant they occur."** Ownership, move, and borrow rules are enforced by the *runtime*: every
movable value carries a header `{state: Owned|Borrowed|Moved, type_id, borrow counts}`, moves
invalidate the source at runtime, and touching an invalidated value raises a **live diagnostic
in the shell** — pointing at the exact line:column — instead of refusing to compile, crashing,
or silently corrupting memory.

Why: HolyRust is a *live* system (TempleOS lineage). In a shell where the program under
development mutates while it runs (and in a kernel edited while it boots — Phase 6),
compile-time-only rejection is hostile: you cannot hot-edit your way out of a type error you
can't even run. Runtime verification trades a small, *configurable* performance cost (§8:
`#![ownership(off|loose|strict)]`) for total liveness. The cost is bounded and measurable
(docs/BENCHMARKS.md), and `unsafe` (§10) plus `off` strictness provide documented escape
hatches for hot paths.

Consequence you must accept: a HolyRust program that would fail `rustc`'s borrow checker
**still starts running** — the first *invalid action* traps with a diagnostic, and (in the
REPL) the session survives. Correctness is observational, not a priori.

## §2. Lexical structure (finalized — Phase 1)

- **Charset:** UTF-8. Columns count characters (a tab is one column). Non-ASCII characters
  are allowed inside string/char literals and comments only; elsewhere they are a
  diagnostic (`L040`) and are skipped (lexing continues).
- **Comments:** `// line` and `/* block */` (non-nesting). Unterminated block comment =
  diagnostic `L001` at its start; the rest of input is consumed.
- **Identifiers:** `[A-Za-z_][A-Za-z0-9_]*`. Keywords (reserved): `if else while do for
  switch case default break continue return try catch throw unsafe class mut` and the type
  names `U0 U8 U16 U32 U64 I8 I16 I32 I64 F64 Str`.
- **Integer literals:** decimal, `0x`/`0X` hex, `0b`/`0B` binary. Values ≤ `I64::MAX` are
  signed; values up to `U64::MAX` are stored as two's-complement bit patterns for unsigned
  contexts; larger = diagnostic `L013`. Identifier characters glued to a number (`123abc`)
  = diagnostic `L014`.
- **Float literals (F64):** require digits before `.`; `.` joins the number **only when
  immediately followed by a digit** (this is what makes `case 4...8` lex as range, and `4.`
  lex as `IntLit(4) Dot`). Exponents `1e3`, `1.5e-3`; `1e` (no exponent digits) lexes as
  `IntLit(1) Ident(e)`.
- **Char literals:** `'a'`, escapes below; hold the Unicode code point as an integer value
  (v1: single character only, else diagnostic `L020`).
- **String literals:** escapes `\n \t \r \0 \\ \" \' \xHH` (exactly 2 hex digits).
  Unknown escape = diagnostic `L005` (character kept literally, with a suggestion listing
  valid escapes). Strings cannot span lines: newline before closing quote = diagnostic
  `L002` and the literal ends there.
- **Operators (longest match):** `+ - * / % & | ^ ~ ! << >> && || == != < <= > >= =
  += -= *= /= %= &= |= ^= <<= >>= ++ -- ( ) { } [ ] ; , : . ... # #!`.
  `...` is one token with two roles (variadic marker L3 / case range L6) — the parser
  disambiguates by position. `..` is a diagnostic `L030` with the suggestion
  "did you mean the range operator `...`?" and recovers as `...`.
- **Error behavior:** the lexer never panics and never stops early: every malformed
  construct produces a diagnostic carrying `file line:col`, the offending material is
  skipped or recovered, and the token stream always ends with exactly one `EOF`.
## §3. Types & literals (final — Phase 3)

- **Scalar types (L1):** `U0` (zero-sized — occupies no storage, D011), `U8 U16 U32 U64`,
  `I8 I16 I32 I64`, `F64`. Built-in support type `Str` (heap string with ownership header,
  D007). Class types via `class` (§11). Pointer/reference types `T *` (single level in v1;
  `T **` is diagnostic P030).
- **Literals:** see §2 (integers incl. hex/bin, floats, chars, strings). Integer literals up
  to `U64::MAX` are accepted and stored as two's-complement bit patterns; the checker rejects
  values that do not fit the *context* type.
- **Conversions (D018):** implicit, HolyC-style, between all scalar numeric types in
  expressions, assignments and argument passing (int↔float included). Explicit cast `(T)expr`
  for scalars (not to `U0`, `Str`, pointers, or classes). `Str` never converts to/from scalars.
- **Arithmetic promotion (C usual conversions, simplified):** if either operand is `F64` →
  `F64`; else if both are narrower than 64 bits → `I64`; else 64-bit: mixed/unsigned → `U64`,
  otherwise `I64`. Assignment to a narrower type truncates modulo 2^width (no diagnostic for
  computed values; literals are range-checked at compile time, C002).
- **Integer division/modulus:** truncating toward zero; `%` is integer-only (C007); division or
  modulus by zero is a runtime trap (§9) at `strict` level. Shift counts mask to the low 6 bits.
- **F64 semantics:** IEEE-754 double. `F64→I64` truncates; out-of-range and NaN give the
  platform "indefinite" value; `U64→F64` for values ≥ 2^63 rounds (halve-convert-double).
  NaN compares false against everything except `!=` (C semantics); NaN is truthy in conditions.
- **Values:** expression results travel as 64-bit quantities; `F64` as IEEE bits. Printing a
  value formats per its static type (D023).
- **Declarations:** `Type name [= init];`. Uninitialized locals/globals are zero-filled.
  Ambiguity note: at statement position `Ident * Ident ;` parses as a *pointer declaration*
  (`Point *p;`), not multiplication — name your classes Capitalized.

## §4. Statements & control flow (syntax final — Phase 2)

- **Top-level statements (L4):** a program is a sequence of items (functions, classes) and
  statements; statements execute immediately in source order when the unit is loaded. There
  is no `main()`.
- **Bare strings (L5):** an expression statement whose value is `Str` prints the string
  (no trailing newline added) instead of echoing.
- **Blocks:** `{ … }`. **if/else**, **while**, **do…while**, **for** `(init; cond; step)`
  with all three parts optional (`for(;;)` = infinite loop); init may be a declaration.
- **break / continue:** innermost enclosing loop (or switch, for `break`).
- **return [expr];** — `return;` only in `U0` functions.
- **Expression statements** end with `;` (forgiven directly before `}` or EOF for REPL
  ergonomics; missing elsewhere = diagnostic P063 + recovery).
- **Empty statement:** a lone `;`.
- **Error recovery:** every parse error reports `file line:col`, skips to the next `;`
  (or stops before `}`), and parsing continues; placeholder nodes keep the tree well-formed.

## §5. Functions, default args, variadics (final — Phase 3)

- **Definition (only at top level in v1):** `RetType Name(params) { body }`. Prototypes
  (`I64 f();`) are rejected (P042) — the shell compiles definitions directly (liveness).
- **Default args at ANY position (L2):** `I64 Foo(I64 a = 1, I64 b = 2)`. Defaults are
  constant expressions. Call-site omission with empty slots: `Foo(,6)` → `a=1, b=6`;
  `Foo()` → all defaults. Omitting a parameter that has no default is a compile diagnostic.
- **Variadics (L3):** `...` as the LAST parameter marker: `I64 Sum(...)` or
  `I64 Foo(I64 a, ...)`. Inside the body, implicit `argc` (total number of passed args,
  named + variadic) and `argv` (indexable I64 view of the 8-byte arg slots): `argv[i]`.
  v1 limitation (Q6): F64 args passed to variadics are visible only as raw bits.
- **Calls:** callee is a plain function name (first-class function values: FUTURE_IDEAS F11).
- **Recursion:** allowed. Overloading: not in v1.
- **Evaluation order:** call arguments evaluate strictly left-to-right (D024).
- **Returns:** scalar/`Str` values return in-place; returning class values is rejected (C016,
  D025); returning a borrow of a local is rejected (C006). Missing `return` yields 0/U0.
- **Redefinition:** defining a function with an existing name recompiles it and hot-swaps the
  call target (indirection cell, D015) — later calls in the session see the new body.
- **Deep recursion:** each frame checks the 64 MiB guest stack limit; overflow is a runtime
  trap (§9), never a host crash.

## §6. switch (final — Phase 3)

- `switch (expr) { case …: stmts … default: stmts }`.
- **No fallthrough** (HolyC semantics): each arm's statements run, then control exits the
  switch — an explicit `break` is legal but unnecessary.
- **Case labels (v1):** integer/char literals with optional unary minus, comma lists
  (`case 3, 6, 9:`), and inclusive ranges (`case 4...8:`). Non-literal labels are a
  diagnostic (P072) — no constant-expression evaluator in v1.
- `default:` is optional; unmatched value with no default = no-op.
- `break` inside an arm exits the switch; `continue` targets the enclosing loop.
- Implementation: cascade of compares (D026); overlapping labels resolve first-match.
- The scrutinee is promoted to `I64`; negative and >32-bit labels supported.
## §7. Ownership & borrows (final — Phase 4)

**Copy vs Move (D008).** Scalars (`U0`, `U8..U64`, `I8..I64`, `F64`) are **Copy**:
assignment/passing copies, the source stays live. `Str` and `class` values are **Move**:
assignment, pass-by-value, and declaration-init *transfer* the value and invalidate the
source **at runtime**:
- a moved `Str` slot reads as null; any read/index/pass/borrow of it traps;
- a moved class value keeps its bytes but its header flips to `Moved`; reads trap.
- Re-initializing a moved binding (`a = "new";` / `a = other_class;`) is legal and revives it.
- Literals are immortal (D007): `Str b = "lit";` never invalidates anything.
- Passing a Move value to a function consumes it (the parameter owns it).

**Borrows (D009/D032).** `&place` creates a *shared* borrow; `&mut place` an *exclusive* one;
both yield `T *` pointers; `*p` dereferences. The runtime borrow tracker enforces
**one mutable XOR many immutable**, checked at borrow *creation* and at *writes*:
- `&mut x` while any borrow of `x` is live → trap `borrow conflict`;
- `&x` while an exclusive borrow is live → trap `borrow conflict`;
- writing through a statically-shared pointer (`*p` where `p` came from `&`) → trap
  `write through a shared borrow`;
- writing through `&mut` while shared borrows are live → trap `borrow conflict`.
Reads through shared pointers while an exclusive borrow exists are *permitted* in v1 (D032
documents this deliberate relaxation — the tracker keys places, not individual pointers).
For `Str`, borrows key on the underlying block (so borrows survive slot moves of *other*
slots correctly); for everything else, on the place address.

**Lifetimes of borrows (lexical, conservative).** A borrow registered by `T *p = &x;`
(declaration or pointer assignment) lives to the **end of the enclosing block**; a borrow
created anywhere else (e.g. a call argument `f(&x)`) lives to the **end of the statement**.
`break`/`continue` release the borrows of the loop body they leave; `return` releases all
borrows of the function. Pointer *mutability* is inherited from the initializer
(`I64 *q = &mut n;` makes `q` exclusive; declared types carry no `mut` marker). Passing
`&mut` where `&` is expected demotes silently; the reverse is compile error C045. Function
parameters are always shared pointers.

**Traps are catchable.** Ownership violations raise the diagnostic through the exception
mechanism (§9): an enclosing `try/catch` receives `catch_code` ∈ {-10 use-after-move,
-11 borrow conflict, -12 shared write, -13 invalid pointer} and a human-readable
`catch_msg`. Uncaught, they print `runtime diagnostic: <message> at file:line:col` and abort
the unit — the session always survives (§1).

**Known v1 limitations (honest list).** (a) A borrow still registered when a `throw` unwinds
past its scope leaks in the tracker until the unit ends — conservative: it can only cause an
extra conflict trap later, never a missed one (D033). (b) Dangling borrows of dead *frames*
are prevented statically only for `return &local` (C006); other cross-frame pointer escapes
are not tracked. (c) Reads-during-exclusive are permitted (D032).
## §8. Strictness levels (final — Phase 4)

`#![ownership(level)]` — as the first thing in a file (applies to the whole unit and to
functions defined in it) or as a statement inside a block (applies to the remainder of that
block; D010/Q7). Default: **strict**.

| Level | Move tracking (invalidation + use-after-move traps) | Borrow XOR + write checks | div-by-zero / index bounds |
| :-- | :-- | :-- | :-- |
| `strict` | ✓ | ✓ | ✓ |
| `loose`  | ✓ | – | – |
| `off`    | – (moves become plain copies) | – | – |

`unsafe { … }` (L11, §10) applies `off` semantics to its body regardless of the level.
The stack-overflow guard is **always** on (host-process survival is not negotiable).
Measured cost (docs/BENCHMARKS.md, 200k-iteration move+index loop): strict ≈ 1.9× the
runtime of `off` — the price of §1, and why the level exists.
## §9. try / catch / throw (final — Phase 3)

- `try <stmt> catch <stmt>` — the handler is usually a block. `throw <expr>;` or
  `throw <code>, <msg>;` where code is an integer and msg a `Str` (Q8/D012).
- Inside a handler: implicit `catch_code: I64` (0 if only a message was thrown; negative for
  runtime ownership traps — see §7) and `catch_msg: Str` (empty if none).
- Mechanics: `try` pushes a jump buffer {rsp, rbp, handler}; `throw` pops and resumes at the
  handler. `break`/`continue`/`return` crossing out of a `try` pop the buffers they leave
  (checker-computed), keeping the stack balanced.
- **Uncaught throw:** runtime diagnostic (`uncaught throw (code …)`) + the current unit aborts;
  the shell/REPL survives (§1 philosophy). Nothing is freed during unwinding — v1 has no
  destructors; arena allocation bounds any leak (D012).
- **Runtime traps** (div/mod by zero at `strict`, index out of bounds at `strict`, stack
  overflow always, and — from Phase 4 — ownership violations) behave like throws: they are
  catchable by an enclosing `try` (ownership traps surface as negative `catch_code`), and
  otherwise abort the unit with a positioned diagnostic. A HolyRust program never segfaults
  on these paths (SPEC Part 7.5).
## §10. unsafe (final — Phase 3 semantics; ownership interplay Phase 4)

`unsafe <stmt>` (usually a block) disables runtime checks for its body (L11): no ownership
checks, no borrow registration, no div-by-zero/index bounds checks. Writes through shared
borrows, reads of moved values, etc. are permitted — consequences are the programmer's
(the stack-overflow guard still applies: the host process must survive). `#![ownership(off)]`
(§8) is the file/block-wide equivalent.
## §11. class (syntax final — Phase 2; layout/move semantics Phase 4)

- `class Point { I64 x; I64 y; U0 tag; };` — the L10 replacement for `typedef`.
- Fields are declarations without initializers (field initializers = diagnostic P021);
  instances are zero-initialized at declaration.
- v1: classes are **value types** (stack locals/globals; no `new`/`delete` — FUTURE_IDEAS
  F3). Member access `p.x` auto-dereferences pointers (`p.x` works when `p` is `Point`
  or `Point *`, HolyC style). All fields occupy 8-byte slots (D022); `U0` fields occupy
  zero bytes.
- Classes are **Move types** under L8 (§7): assignment moves and invalidates the source.
## §12. `#exe {}` compile-time execution (final — Phase 3 engine, Phase 5 exit demo)

- `#exe { … }` at TOP LEVEL of a unit (file, REPL line, or generated text) is compiled and
  executed **during compilation** of the surrounding unit, through the same pipeline
  (Phase 5 Task 3's "invoke the same JIT on a sub-AST").
- Its program output (everything it prints, including L5 bare strings) is captured and
  **re-lexed, re-parsed and spliced** into the surrounding program at the `#exe` position —
  macro-by-execution, with no separate macro language.
- Nesting: generated text may itself contain `#exe` (recursion capped at 8, diagnostic C091).
- Failure semantics: if the block fails to compile or traps at compile time, a positioned
  diagnostic is issued and NOTHING is spliced (C092); the surrounding unit still compiles.
- Diagnostics inside generated code carry the file label `<file>#exe@line:col`.
- Statements inside `#exe` run with the session state visible so far (they may define and
  call functions; those definitions persist in the session like any other unit's).
## §13. Shell/REPL semantics (final — Phase 5)

- **Persistence:** one Session backs the whole shell: globals, functions (hot-swappable
  cells, D015), classes and interned literals persist across lines. Each entered statement
  group is one compilation *unit* executed immediately (L4 liveness).
- **Redeclaration:** a global redeclared with the SAME type reuses its slot and re-runs the
  initializer; a different type is diagnostic C020. Redefining a function recompiles it and
  swaps its cell — subsequent calls run the new body. Identical class redefinitions are
  accepted silently (D029).
- **Echo:** a top-level expression statement with a non-`U0` value echoes its result on its
  own line (HolyC-style) — including assignments (`x = 5;` echoes `5`). Bare `Str`
  expressions auto-print RAW instead of echoing (§4/L5). Inside functions nothing echoes.
- **Multi-line:** input continues on the `...` prompt while `{` braces are unbalanced
  (counted on real tokens, so braces in strings/comments don't confuse it).
- **Isolation:** a runtime trap aborts only the current unit; the shell prints
  `statement aborted … (session continues)` and keeps going (§1 philosophy).
- **Commands** (exact match, only at buffer start, so `#exe`/`#![…]` are never shadowed):
  `#help`, `#history`, `#reset` (fresh Session), `#quit`/`#exit` (also Ctrl-D/EOF).
- **Streams (D036):** guest output + echo → **stdout**; banner, prompts, diagnostics →
  **stderr**. `session.log` (cwd, append) records: session headers, every input line
  (`hr> …`), rendered diagnostics, aborts, and per-statement latency (`[stmt N] X.XXXms`) —
  the Phase 5 p95 gate reads that log.

## §14. Kernel-simulation builtins (Phase 6, D016/Q5 — simulation deliverable)

These four builtins exist to make the *live-editable kernel* demo honest inside the host
process (the spec's sanctioned simulation). They are part of v1's frozen surface:

| Builtin | Signature | Semantics |
| :-- | :-- | :-- |
| `SimPush(c)` | `I64 → I64` | enqueue a simulated keystroke (returns 0) |
| `SimKey()` | `→ I64` | pop the next simulated key; `-1` when the queue is empty |
| `HotPatch(src)` | `Str → I64` | send `src` to the host compiler **while the calling unit keeps running** (the worker blocks only inside this call); on success the new definitions are loaded and their indirection cells swapped atomically (single aligned 8-byte stores, D015) — subsequent calls run the new bodies with all program state preserved. Returns `0`, or `-1` with diagnostics on stderr. `#exe` inside patch source is rejected (it would deadlock the worker). |
| `HeapCheck()` | `→ I64` | walk every registered heap block's canary + header state; `1` = intact, `0` = corruption (also diagnosed). The snapshot refreshes after each successful hot-patch. |

These builtins work identically in the host shell and the **real kernel shell** (P6b):
the same `holyrust` library is compiled `no_std` into the kernel, so a line typed at the
kernel's `hr>` prompt (PS/2 or serial) is compiled and JIT-executed bare-metal, and
`HotPatch` there recompiles a running kernel function in place (D042). Kernel v1 scope:
single core, polled PS/2 (D040), identity-mapped memory (D041); interrupts/UEFI/SMP are
FUTURE_IDEAS F6/F16. The host-side simulation (`demos/kernel_sim.hr`) remains available
and CI-tested on machines without QEMU.
