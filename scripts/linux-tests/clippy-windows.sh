#!/bin/sh
# Lints the Windows build from Linux, as CI does (`cargo clippy --all-targets -- -D warnings`),
# without linking: `rustup target add x86_64-pc-windows-msvc` first. ring (TLS, for webhook
# conditions) compiles a little C, so it gets clang-cl with the few C headers it needs
# (msvc-stubs; the object files are never linked). Needs clang and llvm (clang-cl, llvm-lib).
set -e
here=$(cd "$(dirname "$0")" && pwd)
cl=$(command -v clang-cl || command -v clang-cl-18)
lib=$(command -v llvm-lib || command -v llvm-lib-18)
export CC_x86_64_pc_windows_msvc="$cl" AR_x86_64_pc_windows_msvc="$lib"
export CFLAGS_x86_64_pc_windows_msvc="/clang:-ffreestanding /imsvc $here/msvc-stubs"
cd "$here/../../src-tauri"
cargo clippy --target x86_64-pc-windows-msvc --all-targets "$@" -- -D warnings
