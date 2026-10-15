#!/usr/bin/env bash
# Regenerate docs/BENCHMARKS.md from the in-process harness (SPEC Part 6).
set -euo pipefail
cd "$(dirname "$0")/.."
source tools/env.sh
cargo build -q --release
{
  echo "# BENCHMARKS.md — Latency Numbers per Phase (SPEC Part 6)"
  echo
  echo "Gates (SPEC): compile-to-executable ≤ **50 ms** per typical statement (Phase 3);"
  echo "REPL ≤ **100 ms p95** per statement (Phase 5). Enforced as tests:"
  echo "\`tests/exec_tests.rs::hello_world_50ms_gate\` (+ Phase 5 REPL gate)."
  echo
  echo "Methodology: \`tools/bench.sh\` → \`hr --bench\` (in-process, monotonic clock,"
  echo "release build). Regenerated, never hand-edited."
  echo
  ./target/release/hr --bench | tail -n +3
  # Phase 6 real-kernel metric (only when qemu is available)
  if command -v qemu-system-x86_64 >/dev/null 2>&1; then
    KOUT=$(python3 tools/kernel_ci.py 2>/dev/null | grep -E "^boot to HolyRust shell" || true)
    if [ -n "$KOUT" ]; then
      BOOT_S=$(echo "$KOUT" | sed -E "s/boot to HolyRust shell: ([0-9.]+)s.*/\1/")
      echo "| real-kernel: QEMU boot → in-kernel HolyRust shell | ${BOOT_S} s | < 5 s | 6b |"
    fi
  fi
} > docs/BENCHMARKS.md
echo "--- docs/BENCHMARKS.md ---"
cat docs/BENCHMARKS.md
