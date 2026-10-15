# INSTALL.md — getting HolyRust & TempleOS-RS running

> **The full guide — how to use both halves and how they work — is
> [README.md](README.md).** This file is only the per-platform install detail.

## 1. What you need

| Need | Why | Required? |
| :-- | :-- | :-- |
| **x86-64 CPU** | The JIT emits x86-64 machine code; the kernel is 64-bit | Always |
| **Rust** (rustup, stable ≥ 1.77) | The compiler, runtime and kernel are Rust | Always |
| **`x86_64-unknown-none` target** | `rustup target add x86_64-unknown-none` | To build the OS |
| **QEMU** (`qemu-system-x86_64`) | To boot the OS | To run the OS |
| **Python 3** | `tools/kernel_ci.py`, `tools/gen_font8x8.py` | To run the kernel CI |
| ~2 GB free disk | Toolchain + build artefacts | Always |

**No C compiler is needed.** The kernel is assembled and linked entirely with
tools that ship with rustc (`rust-lld` and rustc's integrated assembler), so you
do **not** need `as`, `ld`, `objcopy`, nasm, or Visual Studio's C++ workload. On
Windows, rustup's bundled `rust-mingw` provides the linker.

The HolyRust compiler itself has **zero external crates** — once Rust is
installed, the project builds offline.

---

## 2. Windows

Verified on Windows 10 with rustup's `stable-x86_64-pc-windows-gnu` toolchain.

```powershell
# 1. Rust (installs %USERPROFILE%\.cargo and .rustup)
#    https://rustup.rs  ->  the default MSVC option is fine; the GNU toolchain
#    also works and needs no Visual Studio C++ workload.

# 2. the bare-metal target for the OS kernel
rustup target add x86_64-unknown-none

# 3. QEMU (optional, for booting the OS): https://www.qemu.org/download/#windows
#    Check:  & "C:\Program Files\qemu\qemu-system-x86_64.exe" --version

# 4. Python 3 (optional, for the kernel CI). NOT the Microsoft Store stub —
#    that one exits 9009 and the kernel test will report "python is a stub".
```

```powershell
cd <project>
powershell -NoProfile -ExecutionPolicy Bypass -File tools\cargo.ps1 test --release
```

**Always build through `tools/cargo.ps1`, not bare `cargo`.** It detects
future-dated build artefacts and clears them. This host's clock jumps backwards,
and without that guard cargo reports `Fresh` forever while your edits are never
compiled — a failure mode that cost three debugging rounds before it was found
([D050](docs/DECISIONS.md)).

Then build and boot the OS:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File kernel\build.ps1
& "C:\Program Files\qemu\qemu-system-x86_64.exe" -kernel kernel\kernel.bin -m 256M -smp 1 -serial stdio
python tools\kernel_ci.py        # the full scripted session -> KERNEL-CI: PASS
```

---

## 3. Linux

Verified path (Debian/Ubuntu/WSL2). The GNU toolchain links with the system
`gcc`, so a C compiler *is* required here:

```bash
sudo apt-get update && sudo apt-get install -y gcc qemu-system-x86 python3
rustup target add x86_64-unknown-none

cargo build --release
cargo test --release
(cd kernel && ./build.sh)
qemu-system-x86_64 -kernel kernel/kernel.bin -m 256M -smp 1 -serial stdio
python3 tools/kernel_ci.py
```

`tools/setup_env.sh` checks the machine and reports what is missing (it installs
nothing and needs no root); `tools/env.sh` is sourced by the shell scripts and
only configures the toolchain if it lives under `/opt`. `tools/ci.sh` and
`tools/bench.sh` are Linux convenience wrappers (the Windows equivalents are
`tools/cargo.ps1`, `kernel/build.ps1` and the commands above). On Linux the kernel
is linked through GNU `as`/`ld`/`objcopy`, so `binutils`/`gcc` are required.

**macOS and ARM Linux are not supported natively** (x86-64 machine code + a
64-bit kernel). You can still work inside an amd64 container, just slower:

```bash
docker run --platform=linux/amd64 -it -v "$PWD:/work" rust:1-bookworm bash
```

---

## 4. What "success" looks like

```
cargo test --release        ->  110 passed; 0 failed   (22 test targets, zero warnings)
hr --bench                  ->  ~0.05 ms vs the 50 ms gate (regenerated numbers in docs/BENCHMARKS.md)
python tools/kernel_ci.py   ->  boot to HolyRust shell: ~1.2-4.2s (<5s gate)
                                ... all phase gates ...
                                KERNEL-CI: PASS
```

The kernel CI **self-skips with a clear message** if QEMU is absent, so a
machine without QEMU still gets a green host suite.

The same split is used by `.github/workflows/ci.yml`: the host suite is the
required gate, and the QEMU job is `continue-on-error` because the boot-time gate
depends on the runner's scheduling.

---

## 5. Troubleshooting

The full table is in [README.md § Troubleshooting](README.md#troubleshooting).
The three that catch people most often:

| Symptom | Fix |
| :-- | :-- |
| Edits appear to do nothing | You used bare `cargo`. Use `tools/cargo.ps1` (Windows) and check `cargo -v` says `Compiling`, not `Fresh` |
| `python` opens the Microsoft Store | That is the Store stub, not Python. Install real Python |
| Kernel dies right after `TOS: compiling <file>` | The transcript names the file; the fault is a language construct, not the file. See [D059](docs/DECISIONS.md) |

---

## 6. Where to read next

- **[README.md](README.md)** — how to use and how it works, for both halves
- [docs/LANG_SPEC.md](docs/LANG_SPEC.md) — the language definition (a feature not in it does not exist)
- [docs/SPEC_TOS.md](docs/SPEC_TOS.md) — the TempleOS-RS build specification
- [docs/PROGRESS.md](docs/PROGRESS.md) — every phase's THINK → EXECUTE → TEST → REVIEW → DECIDE
- [docs/DECISIONS.md](docs/DECISIONS.md) — D001–D060, why each choice was made
- [SPEC.md](SPEC.md) — HolyRust v1's original Single Source of Truth
