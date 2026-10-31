## 開発フロー

### ブランチと PR

- main から `<type>/<issue番号>-<slug>` でブランチを切る（例: `feat/12-login-form`）。issue に紐づかない作業は番号を省略してよい。
- type は `feat` / `fix` / `chore` / `docs` / `refactor` / `test`。
- commit メッセージは Conventional Commits（`<type>: <要約>`、type はブランチと同じ語彙）。本文は日本語でよい。
- main への直接 push は禁止。必ず PR を作り、**merge commit** で main に戻す（squash / rebase は無効化済み）。マージ後のブランチは自動削除される。
- PR のマージはユーザーの指示があった場合のみ `gh pr merge <番号> --merge` で行う。自己判断でマージしない。

### セキュリティゲート（必須ステータス `claude-security`）

main の Ruleset が PR の HEAD に `claude-security` ステータスの success を要求する。HEAD が変われば付け直しが必要。

1. **スキャン要否の判定**: `git diff --name-only main...HEAD` の全ファイルが `*.md` / `docs/**` / `LICENSE` のいずれかならスキャン不要。それ以外（`.github/`、`.claude/`、依存定義・lockfile・設定ファイル含む）は必須。
2. **スキャン種別**: 基本は claude-security の差分スキャン（ブランチの変更）。コードを含む最初の PR、および依存関係・認証まわりの大きな変更ではフルスキャン。
3. **指摘の扱い**: 重大度にかかわらず全件を「修正」か「受け入れ（PR 本文に理由を記載）」にする。未対応が残る間は failure、0 件になったら success。
4. **修正後の再スキャン**: 指摘の修正で追加した commit のみを差分（commit）スキャンする。
5. **ステータスの付与**:

   ```sh
   gh api repos/wchoco/shunsoku/statuses/$(git rev-parse HEAD) \
     -f state=success -f context=claude-security \
     -f description="diff scan: 0 open findings"   # スキャン不要時は "skipped: docs-only"
   ```

   `state` は `success` / `failure`。push 済みの HEAD に対して付ける。
- スキャン結果 `CLAUDE-SECURITY-*/` は public リポジトリに脆弱性情報を出さないため `.gitignore` 済み。コミットしない。

## Agent skills

### Issue tracker

Issues live in GitHub Issues for wchoco/shunsoku (via `gh`). See `docs/agents/issue-tracker.md`.

### Triage labels

Default five canonical labels (needs-triage, needs-info, ready-for-agent, ready-for-human, wontfix). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one root `GLOSSARY.md` + `docs/adr/`. See `docs/agents/domain.md`.
