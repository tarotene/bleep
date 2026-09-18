# publish-guard

AI コーディングエージェントが public な GitHub 面(git push・PR/Issue 作成・MCP tool call)に会社/private リポジトリの実名を漏らす事故を防ぐ deny/ask gate。

Claude Code の PreToolUse hook として生まれ(元は
[tarotene/dotfiles](https://github.com/tarotene/dotfiles) の
`config/claude/hooks/public-publish-guard.sh`)、判定エンジンを
エージェント非依存の CLI に切り出し、Claude Code plugin / Codex CLI /
Copilot CLI 用の薄い adapter を追加したものがこのリポジトリ。

## Scope

In:
- 判定エンジン CLI(`scan` / `scan-push` / `scan-bash-command` / `audit`)
- Claude Code plugin・Codex CLI・Copilot CLI 用 adapter
- denylist/allowlist 設定ファイル仕様

Out:
- 悪意ある回避を防ぐ security boundary(設計上不可能。詳細は次節)
- secret 検出(gitleaks 等の責務)
- 各ホストでの hook 配線(dotfiles 側の責務)

## Issue litmus

判定問: その変更は、既に仲介できている PreToolUse 面での「誤操作による名前漏れ」検出・判定を改善するか?

採用例:
- fix(scan-push): refspec 解析が force-push 形式を取りこぼす
- feat(adapter): Copilot CLI の MCP tool 命名に matcher を追従

棄却例:
- feat: ブラウザ経由の投稿も監視する
- feat(audit): API トークン漏洩の検出を追加

## これは security boundary ではない

このツールは **事故とエージェントの滑りに対するガードレール**であり、
悪意ある回避を防ぐ仕組みではない。理由は3つある。

1. **制約される当事者(エージェント、または手打ちする人間)が
   github.com に直接ブラウザで打ち込む経路はどう転んでも仲介できない**。
   言い換え・分割・base64・手打ちのいずれでも迂回できる。これは
   local な PreToolUse hook という設計そのものの限界であり、実装で
   解決できるものではない。
   — Lampson, B. W., "A Note on the Confinement Problem", *Communications
   of the ACM* 16(10), 1973, pp.613–615,
   <https://dl.acm.org/doi/10.1145/362375.362389>(2026-09-10 取得)。
2. **complete mediation を満たしていない**。このツールが仲介するのは
   `Bash` ツールと `mcp__*` ツールの PreToolUse イベントだけで、それ以外の
   経路(エージェントが直接 API を叩く、ユーザが別のターミナルで作業する
   等)は一切見ない。
   — Saltzer, J. H. & Schroeder, M. D., "The Protection of Information in
   Computer Systems", 1975,
   <https://www.cs.virginia.edu/~evans/cs551/saltzer/>(2026-09-10 取得)。
3. **denylist ベースの検出は「疑わしい活動の検出」にしか使えない**。
   `audit` サブコマンドはこの前提で設計している(既存ファイル/Issue/PR の
   事後チェック用)。
   — MITRE CWE-184, "Incomplete List of Disallowed Inputs",
   <https://cwe.mitre.org/data/definitions/184.html>(2026-09-10 取得)。

誤検知が多いと、エージェントも人間も bypass を日常化させ、結果として
このツールは無いのと同じになる。誤検知を見つけたら denylist の粒度を
上げるより先に allowlist(下記)を使うこと。
— Rahman, A., Imtiaz, F., Storey, M.-A., Williams, L., "Why secret
detection tools are not enough: It's not just about false positives — An
industrial case study", *Empirical Software Engineering*, 2022,
<https://doi.org/10.1007/s10664-021-10109-y>(2026-09-10 取得)。

### hook のタイムアウトは構造的に防げない

Claude Code の公式ドキュメントは「タイムアウトで停止した hook は tool
call を block しない」と明記している。つまり、hook プロセス自体が
時間内に終了しなければ、判定結果に関わらず操作は素通りする。これは
publish-guard 側で対処できない、ホスト CLI 側の仕様。
— Anthropic, "Hooks reference", <https://code.claude.com/docs/en/hooks>
(2026-09-10 取得)。

## bypass: `PUBLISH_GUARD_ALLOW=1`

`scan`/`scan-push`/`scan-bash-command` は環境変数 `PUBLISH_GUARD_ALLOW=1`
が立っていると即座に pass する。意識的に検査を外したいとき用の
エスケープハッチ。**deny/ask の理由文にはこの env var 名を書いていない**
— 制約される当事者(エージェント)が deny の理由を読んで自分で bypass を
再実行できてしまうため。この bypass の存在自体は、このツールを設定する
人間だけが知っていればよい。

## インストール

denylist データ(org 名・repo 名)は一切このリポジトリにコミットしない。
`$XDG_CONFIG_HOME/publish-guard/`(既定 `~/.config/publish-guard/`)以下に
自分で置く。**手書きが必要なのは実質 `orgs.txt` の org 名 1 行だけ**——
自分の private/internal リポジトリ名は、そこに書いた org 名と(gh
認証ユーザ自身の)owner から、各自の `gh` credential で live 導出する。
リポジトリ名のリストそのものは誰も持ち運ばない。

```
$ mkdir -p ~/.config/publish-guard
$ echo acme > ~/.config/publish-guard/orgs.txt   # 会社の org 名だけ書く
```

任意で追加できる設定ファイル(すべて省略可、1行1件、`#` コメント・空行は
無視):

| ファイル | 用途 |
|---|---|
| `orgs.txt` | 会社/他人の org 名(手書きが必要なのは実質ここだけ) |
| `repos.txt` | `org/repo` 形の明示的な参照(任意) |
| `allow-stopwords.txt` | denylist から除外する単語(組み込みで `.github` を既定値として持つ) |
| `allow-regexes.txt` | この正規表現が scan 対象テキストにマッチしたら **ask だけ**を無効化する(deny は緩めない) |
| `allow-paths.txt` | `audit` から除外するファイルパスの正規表現 |

### Claude Code

```
/plugin marketplace add tarotene/publish-guard
/plugin install publish-guard@publish-guard
```

`.claude-plugin/plugin.json` が `hooks/hooks.json` を読み、`PreToolUse` に
`Bash|mcp__.*` の複合 matcher で `hooks/claude-adapter.sh` を登録する
(`${CLAUDE_PLUGIN_ROOT}` で自己解決)。

**matcher を分けない理由**: Bash 用と MCP 用に2つの hook エントリを
書きたくなるかもしれないが、書かないこと。存在判定が command 文字列の
完全一致でしか行えない登録系(home-manager の `registerHooks` 等)と
組み合わせると、同一 command を2つの matcher で登録した場合に2回目が
早期 return し、片方の経路が無検査のまま残る。複合1本に統一しておけば
どの登録系でも同じ挙動になる。

### Codex CLI

`~/.codex/hooks.json` に手で追記する(dotfiles の `register-codex-hooks`
のような idempotent merger を自分で書いてもよい):

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|mcp__.*",
        "hooks": [
          {"type": "command", "command": "/path/to/publish-guard/adapters/codex-adapter.sh", "timeout": 20}
        ]
      }
    ]
  }
}
```

matcher `Bash` は実機(`codex exec --dangerously-bypass-hook-trust`)で
確認済み。**MCP tool の命名規則(`mcp__server__tool` 形かどうか)は Codex
側で未確認** — MCP server を使っている場合は実際の `tool_name` を確認して
から matcher を調整すること。

### Copilot CLI

`~/.copilot/settings.json` の `"hooks"` キーに追記する:

```json
{
  "hooks": {
    "preToolUse": [
      {"type": "command", "bash": "/path/to/publish-guard/adapters/copilot-adapter.sh", "timeoutSec": 20}
    ]
  }
}
```

Copilot の `preToolUse` には **matcher が無い**(実機確認済み — 全 tool
call で無条件発火する)。絞り込みは adapter 内部(`toolName == "bash"` か
どうか)で行っている。

## exit code の契約(自分でスクリプトを書く人向け)

`publish-guard scan`/`scan-push`/`scan-bash-command` はすべて同じ契約:

| exit code | 意味 | stdout |
|---|---|---|
| 0 | pass(denylist に一致しない) | 無出力 |
| 1 | ask(org 名の裸単体一致 — 正当な用途と衝突しうる) | 理由1行 |
| 2 | deny(org/repo 明示参照・具体リポ名一致 — ほぼ確実に意図しない漏洩) | 理由1行 |

## `audit --remote` の注意

`publish-guard audit --remote` は `gh issue list`/`gh pr list` で対象
リポジトリの **open な Issue/PR の title/body を丸ごと取得**してローカル
プロセスに載せる。会社の private/internal リポジトリに対して実行すると、
社内の Issue/PR 本文がこのプロセスのメモリと(シェル履歴等)に一時的に
残る。実行対象は選んで使うこと。

## 組織単位で一括配布したい場合(Claude Code)

Claude Code の managed settings は、組織が使えるマーケットプレイスを
`strictKnownMarketplaces` で制限し、`enabledPlugins` で全ユーザに
事前導入できる。
— Anthropic, "Plugin marketplaces",
<https://code.claude.com/docs/en/plugin-marketplaces.md>(2026-09-10 取得)。

```json
{
  "strictKnownMarketplaces": [
    {"source": "github", "repo": "tarotene/publish-guard"}
  ],
  "enabledPlugins": {
    "publish-guard@publish-guard": true
  }
}
```

denylist データ(`orgs.txt` 等)はこの配布経路に乗らない——各ユーザが
自分の `~/.config/publish-guard/orgs.txt` に org 名を書く必要がある点は
変わらない(意図的な設計。詳細は「これは security boundary ではない」参照)。

## 開発

- `./publish-guard selftest` / `./hooks/claude-adapter.sh --selftest` /
  `./adapters/codex-adapter.sh --selftest` / `./adapters/copilot-adapter.sh --selftest`
- `shellcheck -S error publish-guard hooks/*.sh adapters/*.sh`
- コミット前のサニタイズ規則: [CONTRIBUTING.md](CONTRIBUTING.md)

## License

MIT — [LICENSE](LICENSE)
