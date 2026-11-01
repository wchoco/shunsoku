#!/usr/bin/env bash
# WSL から Windows 側のツールチェーン（cargo.exe）で worktree の exe を組み、
# %USERPROFILE%\shunsoku-build\<worktree 名>\ にコピーする。
# usage: scripts/win-build.sh [worktree-dir=.] [cargo build に足す引数...]
#   env: PKG（既定 shunsoku）、PROFILE（既定 release）
set -euo pipefail
WT=$(realpath "${1:-.}"); [ $# -gt 0 ] && shift
NAME=$(basename "$WT")
PKG=${PKG:-shunsoku}
PROFILE=${PROFILE:-release}

winenv() { cmd.exe /c "echo %$1%" 2>/dev/null | tr -d '\r'; }
export PATH="$PATH:/mnt/c/Windows/System32"
WIN_HOME=$(winenv USERPROFILE)
WIN_LOCAL=$(winenv LOCALAPPDATA)
export PATH="$PATH:$(wslpath "$WIN_HOME")/.cargo/bin"

# 置き場は NTFS に worktree ごと。差分ビルドが速く、worktree 間で cargo のロックを取り合わない。
export CARGO_TARGET_DIR="$WIN_LOCAL\\shunsoku-target\\$NAME"
export WSLENV="${WSLENV:+$WSLENV:}CARGO_TARGET_DIR"
cd "$WT"   # cargo.exe からは \\wsl.localhost\... に見えるが、そのままで組める。

# 動いている exe は出力先をロックし、link が LNK1104 で落ちる。
taskkill.exe /IM "$PKG.exe" /F >/dev/null 2>&1 || true
cargo.exe build --profile "$PROFILE" -p "$PKG" "$@"

SRC="$(wslpath "$CARGO_TARGET_DIR")/$PROFILE/$PKG.exe"
DST_DIR="$(wslpath "$WIN_HOME")/shunsoku-build/$NAME"
mkdir -p "$DST_DIR" && cp "$SRC" "$DST_DIR/"
echo "built: $WIN_HOME\\shunsoku-build\\$NAME\\$PKG.exe"
