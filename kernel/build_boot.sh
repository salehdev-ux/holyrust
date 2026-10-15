#!/usr/bin/env bash
# K1: build the asm-only smoke kernel (superseded by build.sh once Rust lands).
set -euo pipefail
cd "$(dirname "$0")"
as --64 -o /tmp/hr_boot.o boot.S
ld -T linker.ld -o /tmp/hr_kernel.elf /tmp/hr_boot.o
objcopy -O binary /tmp/hr_kernel.elf /tmp/hr_kernel.bin
echo "built /tmp/hr_kernel.bin"
