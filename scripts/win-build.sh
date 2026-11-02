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
export PATH="$PATH:/mnt/c/Windows/System32:/mnt/c/Windows/System32/WindowsPowerShell/v1.0"
WIN_HOME=$(winenv USERPROFILE)
WIN_LOCAL=$(winenv LOCALAPPDATA)
export PATH="$PATH:$(wslpath "$WIN_HOME")/.cargo/bin"

# 置き場は NTFS に worktree ごと。差分ビルドが速く、worktree 間で cargo のロックを取り合わない。
export CARGO_TARGET_DIR="$WIN_LOCAL\\shunsoku-target\\$NAME"
export WSLENV="${WSLENV:+$WSLENV:}CARGO_TARGET_DIR"
cd "$WT"   # cargo.exe からは \\wsl.localhost\... に見えるが、そのままで組める。

SRC_WIN="$CARGO_TARGET_DIR\\$PROFILE\\$PKG.exe"
DST_WIN="$WIN_HOME\\shunsoku-build\\$NAME\\$PKG.exe"

# 動いている exe は自分のファイルをロックし、link（LNK1104）やコピーが落ちる。
# このスクリプトが上書きする 2 つのパスで動いているものだけを止める。
# 名前で止めると、常駐中の shunsoku や他の worktree の exe まで止まる。
powershell.exe -NoProfile -Command "Get-Process -Name '$PKG' -ErrorAction SilentlyContinue | Where-Object { \$_.Path -in @('$SRC_WIN', '$DST_WIN') } | Stop-Process -Force" || true
cargo.exe build --profile "$PROFILE" -p "$PKG" "$@"

SRC="$(wslpath "$SRC_WIN")"
DST_DIR="$(wslpath "$WIN_HOME")/shunsoku-build/$NAME"
mkdir -p "$DST_DIR" && cp "$SRC" "$DST_DIR/"
echo "built: $WIN_HOME\\shunsoku-build\\$NAME\\$PKG.exe"
