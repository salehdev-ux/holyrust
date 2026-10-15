#!/usr/bin/env bash
# Check that this machine can build the project — and say what is missing.
#
# This used to be an unattended bootstrap that ran `apt-get` as root and
# installed a second Rust toolchain into /opt. That is wrong for a published
# repository: most people already have Rust, nobody expects a project's setup
# script to require root, and silently downloading an installer is not something
# a reader can audit. So this checks, reports, and lets you decide.
#
# Windows users do not need this at all: use tools/cargo.ps1 and
# kernel/build.ps1 (see README.md).
set -euo pipefail
cd "$(dirname "$0")/.."

missing=0
need() {
  if command -v "$1" >/dev/null 2>&1; then
    printf '  %-22s ok        %s\n' "$1" "$("$1" --version 2>&1 | head -1)"
  else
    printf '  %-22s MISSING   %s\n' "$1" "$2"
    missing=1
  fi
}
opt() {
  if command -v "$1" >/dev/null 2>&1; then
    printf '  %-22s ok        %s\n' "$1" "$("$1" --version 2>&1 | head -1)"
  else
    printf '  %-22s absent    %s\n' "$1" "$2"
  fi
}

echo "Required to build the host compiler:"
need cargo "install Rust — https://rustup.rs"
need gcc   "needed to LINK on Linux — apt-get install gcc"

echo
echo "Required to build and boot the operating system:"
opt qemu-system-x86_64 "boots the OS — apt-get install qemu-system-x86"
opt python3             "runs tools/kernel_ci.py — apt-get install python3"

# The OS needs the bare-metal target; check without failing the required set.
if command -v rustup >/dev/null 2>&1; then
  if rustup target list --installed 2>/dev/null | grep -qx x86_64-unknown-none; then
    echo "  x86_64-unknown-none    ok        bare-metal target installed"
  else
    echo "  x86_64-unknown-none    MISSING   rustup target add x86_64-unknown-none"
    missing=1
  fi
fi

echo
if [ "$missing" -eq 0 ]; then
  echo "Ready. Next:  cargo test --release   then   python3 tools/kernel_ci.py"
else
  echo "Something required is missing (see above). Install it and re-run."
fi
exit "$missing"
