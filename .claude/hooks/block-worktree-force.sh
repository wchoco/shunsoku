#!/usr/bin/env bash
# PreToolUse(Bash): git worktree remove に --force / -f が付いたコマンドを止める。
# 強制削除は、別セッションが使っている worktree を未コミットの変更ごと消すおそれがある。
# 引数は ASCII の語だけをたどる。commit メッセージなどの文中の言及では止めない。
cmd=$(jq -r '.tool_input.command // empty')
arg="[[:space:]]+[[:alnum:]_./~\$\"'{}=:@+%-]+"
re="git(${arg})*[[:space:]]+worktree[[:space:]]+remove(${arg})*[[:space:]]+(--force|-f+)([[:space:];&|)]|\$)"
if printf '%s\n' "$cmd" | LC_ALL=C grep -Eq "$re"; then
  echo "git worktree remove の --force / -f は止めている。--force なしで消すこと。消せなければ git -C <worktree> status と、別セッションが使っていないかを確かめ、ユーザーに聞く。" >&2
  exit 2
fi
exit 0
