#!/usr/bin/env bash
# CI entry point: full build + full test suite + scripted demos (SPEC Part 5).
set -euo pipefail
cd "$(dirname "$0")/.."
source tools/env.sh
cargo build --release 2>&1 | tail -1
cargo test --release
# real-kernel scripted QEMU session (self-skips with notice when qemu absent)
python3 tools/kernel_ci.py
