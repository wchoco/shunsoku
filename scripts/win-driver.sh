#!/usr/bin/env bash
# 実機の手動確認の道具（tools/win-driver）を WSL から組み、呼ぶ。使い方と決まりは docs/agents/manual-check.md。
# usage: scripts/win-driver.sh build                 # 組む（scripts/win-build.sh と同じ流れ）
#        scripts/win-driver.sh <コマンド> [引数...]   # 呼ぶ。コマンドの一覧は `scripts/win-driver.sh --help`
#   env: DRV_ROOT（必須。テスト用のフォルダの Windows のパス。例 C:\Users\<you>\shunsoku-build\check-m5）
set -euo pipefail
REPO=$(realpath "$(dirname "$0")/..")
NAME=$(basename "$REPO")

if [ "${1:-}" = build ]; then
  shift
  exec env PKG=win-driver PROFILE=release "$REPO/scripts/win-build.sh" "$REPO" "$@"
fi

export PATH="$PATH:/mnt/c/Windows/System32"
WIN_HOME=$(cmd.exe /c "echo %USERPROFILE%" 2>/dev/null | tr -d '\r')
EXE="$(wslpath "$WIN_HOME")/shunsoku-build/$NAME/win-driver.exe"
[ -x "$EXE" ] || { echo "win-driver.exe がない。先に scripts/win-driver.sh build" >&2; exit 2; }
if [ "${1:-}" = --help ] || [ "${1:-}" = -h ] || [ $# -eq 0 ]; then
  exec "$EXE" --help
fi
: "${DRV_ROOT:?DRV_ROOT にテスト用のフォルダ（Windows のパス）を入れる}"
exec "$EXE" --root "$DRV_ROOT" "$@"
