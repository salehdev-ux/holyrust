#!/usr/bin/env bash
# Build the HolyRust kernel image: Rust staticlib + asm boot → flat multiboot bin.
set -euo pipefail
cd "$(dirname "$0")"
source ../tools/env.sh
cargo build -q --release --target x86_64-unknown-none
LIB=target/x86_64-unknown-none/release/libhr_kernel.a
as --64 -o /tmp/hr_boot.o boot.S
ld -T linker.ld -o /tmp/hr_kernel.elf /tmp/hr_boot.o --start-group "$LIB" --end-group
objcopy -O binary /tmp/hr_kernel.elf kernel.bin
echo "built kernel/kernel.bin ($(stat -c%s kernel.bin) bytes)"
