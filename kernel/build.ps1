#!/usr/bin/env pwsh
# D043 (Windows port): build the HolyRust kernel image on a stock Windows host.
#
# `build.sh` (Linux) drives GNU `as --64` + `ld -T linker.ld` + `objcopy`.
# None of those exist here, and a Windows MinGW `as`/`ld` targets PE/COFF, not
# the ELF64 the multiboot loader path needs. Everything required is already
# shipped by rustup:
#
#   boot.S   -> rustc's integrated LLVM assembler via kernel/bootasm.rs
#               (`global_asm!(..., options(att_syntax))`; Rust defaults to
#                Intel syntax, which is failure mode #1)
#   *.a      -> rustc --target x86_64-unknown-none
#   link     -> rust-lld -flavor gnu (honours linker.ld) + --oformat=binary
#
# Produces the same artefacts as build.sh: kernel.elf + kernel.bin.
$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot

function Resolve-Tool([string]$exe) {
    $c = Get-Command $exe -ErrorAction SilentlyContinue
    if ($c) { return $c.Source }
    $p = Join-Path $env:USERPROFILE ".cargo\bin\$exe"
    if (Test-Path $p) { return $p }
    throw "$exe not found (install rustup: https://rustup.rs)"
}

$cargo = Resolve-Tool 'cargo.exe'
$rustc = Resolve-Tool 'rustc.exe'

# This host's wall clock jumps BACKWARDS: build outputs can end up timestamped
# in the future (measured: a target file at 17:08 while the clock read 16:22).
# Cargo then judges every source older than its own outputs and reports "Fresh
# hr-kernel" forever — three different gfx.rs implementations produced
# byte-identical kernels before this was found (PROGRESS TOS-P2.3). Touching the
# sources cannot fix that; the future-dated target has to go.
$kTarget = Join-Path $PSScriptRoot 'target'
if (Test-Path $kTarget) {
    $nowChk = Get-Date
    $future = Get-ChildItem -LiteralPath $kTarget -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTime -gt $nowChk.AddMinutes(1) } | Select-Object -First 1
    if ($future) {
        Write-Host "build.ps1: future-dated artefact ($($future.Name) at $($future.LastWriteTime)); clearing target for a real rebuild"
        Remove-Item -LiteralPath $kTarget -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# rust-lld ships inside every rustup toolchain at lib/rustlib/<target>/bin/.
$lld = Get-ChildItem "$env:USERPROFILE\.rustup\toolchains\*\lib\rustlib\*\bin\rust-lld.exe" `
        -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
if (-not $lld) { throw "rust-lld.exe not found under ~/.rustup/toolchains" }

# 1. compiler + kernel services as a staticlib for bare metal
& $cargo build -q --release --target x86_64-unknown-none
if ($LASTEXITCODE -ne 0) { throw 'kernel staticlib build failed' }

# 2. assemble boot.S (GAS/AT&T) into ELF64
& $rustc --target x86_64-unknown-none --edition 2021 --crate-type lib `
        --emit=obj -C opt-level=2 bootasm.rs -o boot.o
if ($LASTEXITCODE -ne 0) { throw 'boot.S assembly failed' }

$lib = 'target\x86_64-unknown-none\release\libhr_kernel.a'
if (-not (Test-Path $lib)) { throw "missing $lib" }

# 3. link (kept for symbol/debug inspection, exactly like build.sh)
& $lld -flavor gnu -T linker.ld -o kernel.elf boot.o --start-group $lib --end-group
if ($LASTEXITCODE -ne 0) { throw 'kernel link (elf) failed' }

# 4. flat multiboot binary (replace for `objcopy -O binary`)
& $lld -flavor gnu -T linker.ld --oformat=binary -o kernel.bin boot.o --start-group $lib --end-group
if ($LASTEXITCODE -ne 0) { throw 'kernel link (binary) failed' }

$size = (Get-Item kernel.bin).Length
Write-Host "built kernel/kernel.bin ($size bytes)"
