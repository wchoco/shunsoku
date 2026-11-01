#!/usr/bin/env bash
# Build settings-dialog.exe with the Windows-side toolchain (cargo.exe) from WSL.
# Output is copied to C:\Users\jiji\shunsoku-build\settings-dialog\ (override with DST_DIR).
# Needs VS 2022 (link.exe) and the Windows SDK (rc.exe; or set RC=<path> and add it to WSLENV).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
PROFILE=${PROFILE:-release}
export PATH="$PATH:/mnt/c/Users/jiji/.cargo/bin:/mnt/c/Windows/System32"
export CARGO_TARGET_DIR="C:\\Users\\jiji\\AppData\\Local\\shunsoku-target\\settings-dialog"
export WSLENV="${WSLENV:+$WSLENV:}CARGO_TARGET_DIR"
cd "$HERE"   # cargo.exe sees cwd as \\wsl.localhost\...; that works.
taskkill.exe /IM settings-dialog.exe /F >/dev/null 2>&1 || true
cargo.exe build --profile "$PROFILE" "$@"
SRC="/mnt/c/Users/jiji/AppData/Local/shunsoku-target/settings-dialog/$PROFILE/settings-dialog.exe"
DST_DIR=${DST_DIR:-/mnt/c/Users/jiji/shunsoku-build/settings-dialog}
mkdir -p "$DST_DIR" && cp "$SRC" "$DST_DIR/"
echo "built: C:\\Users\\jiji\\shunsoku-build\\settings-dialog\\settings-dialog.exe"
