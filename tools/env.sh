# Source this before a cargo/rustc invocation (it is sourced by tools/ci.sh,
# tools/bench.sh and kernel/build.sh).
#
# This project was originally built inside a sandbox where the toolchain lived
# under /opt, and this file used to hard-code that unconditionally — which broke
# every Linux user and CI runner whose rustup lives in the normal place
# (~/.cargo, ~/.rustup), because RUSTUP_HOME=/opt/rustup makes rustup unable to
# find its own toolchains.
#
# So: honour the legacy layout only when it is actually present, and otherwise
# leave the caller's toolchain configuration completely alone.

if [ -d /opt/rustup ] && [ -d /opt/cargo ]; then
  export RUSTUP_HOME=/opt/rustup
  export CARGO_HOME=/opt/cargo
  export PATH=/opt/cargo/bin:$PATH
fi
