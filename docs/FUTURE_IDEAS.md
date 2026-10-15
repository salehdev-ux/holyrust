# FUTURE_IDEAS.md — Scope-Creep Quarantine (SPEC Part 0 rule 7)

Temptations noticed while building v1. NONE of these may enter v1 (frozen table §2.4).

| # | Idea | Where the temptation came from | Why deferred |
| :-- | :-- | :-- | :-- |
| F1 | Fixed arrays `I64 a[10];` + indexing | argv/Str indexing exists (D021) | Not in L1–L11 |
| F2 | Pointer arithmetic `p+1` | Pointers exist for borrows | Not in table; unsafe-adjacent |
| F3 | `new`/`delete` heap objects; refcount auto-free | Class value types feel limited | L10 mandates only the keyword |
| F4 | `sizeof` operator | U0 zero-size verification wants it | Tests can use checker API instead |
| F5 | doc/graphics builtins (`Text`, `Plot`, VGA screen buffer) | §2.1 diagram mentions "doc/graphics" | OPEN_QUESTIONS Q4 ruled it illustrative |
| F6 | Real-kernel hardening: SMP/AP bring-up, UEFI boot, W^X + NX mappings, real drivers (AHCI/NVMe, timer PIT/HPET), paging on demand | P6b proved the boot+shell+hot-patch core; the rest is OS depth | v1 kernel is single-core, identity-mapped RWX, fixed memory map (D041) |
| F7 | Self-hosting: compiler written in HolyRust | "self-hosting later" (Phase 0 Task 2) | Explicitly "later" |
| F8 | Generics/traits/enums/pattern matching | Rust envy | Non-goal §1.3 (no full Rust compat) |
| F9 | String interpolation `"x=%d" % x` sugar | Print formatting | L5 only mandates bare-string auto-print |
| F10 | REPL niceties: readline editing, syntax colors, tab-completion | Shell polish | Phase 5 mandates line-read + history + log only |
| F11 | Closure/function values as first-class data | Hot-patch registry hints at it | Not in table; registry is internal |
| F12 | Modules / `use` / multiple files per unit | Real projects need them | RAM-disk single-file world for v1 |
| F13 | GC or region-per-function auto-free of `Str` | Leaks on throw (D012) | Arena reset per session bounds it |
| F14 | `#exe` producing diagnostics back into the host unit's line numbering | Splice positions are synthetic | Nice-to-have ergonomics |
| F15 | ARM64 backend | Portability | Non-goal §1.3 (x86-64 only) |
| F16 | IDT + PIC remap + IRQ1 keyboard interrupts (replacing polling) | SPEC Phase 6 Task 1 lists "interrupt handlers"; polling chosen for demo determinism (D040) | Quarantined to protect the flagship demo's reliability |
| F17 | Kernel-side `#![ownership]`-aware slab allocator with real free() | Bump-only kernel never reclaims (fine for demo sessions) | v1 arena philosophy (D017/D041) |

## TempleOS-RS quarantine (SPEC_TOS Part 0 rule 5 — honest degradation)

| # | Idea | Where the temptation came from | Why deferred |
| :-- | :-- | :-- | :-- |
| F18 | `Str` **building** primitives: `StrCat`, `StrSlice`, `StrCopy`, `StrFromI64` | T1.4 wants a command processor in HolyRust, so `cat /TOS/x.HR` (unquoted, space-separated) and `Sh` parsing its own argument list both need new strings — and `Str` values cannot currently be created except from a literal or `FsRead`. `StrCopy` would also delete the globals in `/TOS/Files.HR` (D047 rule 2: a `Str` variable is consumed when passed to a user function). | Not required by T1.6/T1.7, whose own CI snippets call `Dir("...")`/`FWrite("...")` as functions; adding string construction is a language-surface decision that deserves its own D-number rather than being smuggled in under Phase 1. Degradation recorded: `/TOS/Shell.HR` dispatches argument-less command words, and argument-taking commands are called as HolyRust functions. |
| F19 | `?` and character classes in `/TOS` globs | Every real shell has them | Phase 1 only ever needs `*`; `/TOS/Files.HR` supports exactly one `*` and documents the limit in-file and in `MatchCurrent`'s comment. |

### F20 — AfterEgypt-style flight simulator (`/TOS/Flight.HR`) — **PROMOTED, Phase 7**

| # | Idea | Where the temptation came from | Status |
| :-- | :-- | :-- | :-- |
| F20 | A first-person wireframe flight simulator in the spirit of TempleOS's *AfterEgypt*: fly over a desert of wireframe pyramids, with the horizon tilting as you bank and dropping as you pitch. Written **entirely in HolyRust** as `/TOS/Flight.HR`, using only the existing graphics builtins (`GfxInit`/`GfxClear`/`Plot`/`GfxChar` plus the HolyRust shape layer), mouse X = roll, mouse Y = pitch, throttle on keys. | Phase 2 built the shape vocabulary and Phase 3 supplied the mouse; the combination is exactly the kind of demo a system that is *written in itself* should be able to host — and `Line`-based 3D projection is the cheapest convincing proof that the drawing layer is real rather than a toy. | **Deferred in Phase 5, then promoted to Phase 7 on request.** F20 is recorded here first (per SPEC_TOS Part 4.4) and then implemented in the same change, so the idea and its realisation share one history. Two Rust defects blocked it and were fixed as defects rather than as features: the guest had **no way to read the keyboard** while running (D057), and `MouseX()`/`MouseY()` returned a position that stopped updating as soon as guest code took over the machine (D058). |

**What promotion did *not* buy.** F20 as specified stays inside the frozen scope
(SPEC_TOS §1.4): no SMP, no networking, no new hardware. It is guest code in
`/TOS/`, compiled by the in-kernel JIT at boot like every other module, and the
two Rust changes it needed are the smallest ones that unblock *any* interactive
guest loop — not flight-specific affordances. Anything beyond that (filled
polygons, a textured ground, sound-driven engine note) is a further idea, not
part of this one.
