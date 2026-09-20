# Contributing

See [README](README.md) for what this repository is and how to set up a
development environment.

## Issues

Judging question: その変更は、既に仲介できている PreToolUse 面での「誤操作
による名前漏れ」検出・判定を改善するか?

Accepted:
- fix(scan-push): refspec 解析が force-push 形式を取りこぼす
- feat(adapter): Copilot CLI の MCP tool 命名に matcher を追従

Rejected:
- feat: ブラウザ経由の投稿も監視する
- feat(audit): API トークン漏洩の検出を追加

## Pull requests

Fork this repository, create a topic branch, and open a pull request against
`main`. Before submitting, run the checks listed in the README's
[Development](README.md#development) section (`selftest` and `shellcheck -S
error`) and make sure both pass.

**サニタイズ規則(公開リポジトリの正本)**。この規則は
[tarotene/dotfiles](https://github.com/tarotene/dotfiles) の
`config/claude/skills/skill-gardening/SKILL.md` §3 と同一のものを、この
リポジトリ用に複製している。この規則自体が「denylist は一切コミットしない」
という本ツールの設計原則の適用例であり、他のどの寄稿にも適用される。

このリポジトリは公開なので、社内固有の情報は一切載せない。コード・README・
Issue/PR・commit message を書くときは次を満たす:

- **固有名詞ゼロ**: 社名・製品名・機体名・プロジェクト名・実在するリポジトリ名・
  人名・Issue/PR 番号を書かない。テスト・例に登場する組織名/リポジトリ名は
  `acme` / `secret-project` のような明らかな架空名のみを使う。
- **URL ゼロ**: 社内リソースへの URL は書かない(公式ドキュメントなど一般に
  公開された参照先は例外)。
- **実物の持ち込み禁止**: スクリーンショット・実ファイルのコピー・実データを
  貼らない。
- **denylist / allowlist の設定ファイルはコミットしない**: `orgs.txt` /
  `repos.txt` / `allow-*.txt` はこのツールが読む対象であり、このリポジトリの
  `.gitignore` にも入れる。テストフィクスチャは selftest 内の `mktemp -d` に
  閉じる。
- **コミット前の最終確認**: 差分を固有名詞と URL の観点で読み直す。迷ったら
  書かない側に倒す。

**このツールに変更を加える前に**。`publish-guard` は README の「これは
security boundary ではない」を前提に設計されている(Lampson 1973, Saltzer
& Schroeder 1975, CWE-184)。新しい検査ロジックを追加するときは:

- 判定不能を無言の pass にしない(fail-loud)。既存の `PUSH_DIFF_FAIL_REASON`
  や `cmd_scan`/`cmd_scan_push` の読み取り失敗処理を参考にする。
- deny の理由文にバイパス手段(`PUBLISH_GUARD_ALLOW=1`)を書かない。
- `./publish-guard selftest` と `shellcheck -S error publish-guard` の両方が
  通ること。
