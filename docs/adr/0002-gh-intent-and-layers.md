# ADR-0002 — gh の「投稿の意図」抽出を共通化し、実行時の層は今は足さない

- Status: Accepted(提案 2 の push に関する部分は ADR-0003 で置き換え)
- Date: 2026-09-30
- Context: tarotene/bleep#56(design Issue)。`/wrapup-chores` での裁定
  (提案1 は bleep 内のサブコマンド + fixture、提案2 は今は不採用、提案3 は
  最小版、提案4 は別件の releaser 整備を待つ)

## Context

`gh` の投稿系コマンドから「投稿先」と「本文」を取り出す処理が、複数の
場所に独立して実装されている。

- bleep: `bleep-hook lex`(`src/lex.rs`)。宛先(`-R`、gh api の API パス)、
  実効ディレクトリ、本文の入力元(#55)を取り出す。
- tarotene/dotfiles 側の bash 製ガード: `attribution-guard.sh` が本文
  (`--body-file`、`-F body=@`、`--input` の中身まで)を取り出し、
  `pr-confirm-guard.sh`・`stack-base-guard.sh` ほかもそれぞれ `gh` の引数を
  読む。

実装がずれるたびに、片方だけが見える形になる。#55 の実例では、本文ファイルの
中身を bleep は見られず、attribution-guard は見られた。#57・#58 は同じ
「抽出の取りこぼし」が `gh api` にあった件で、修正は bleep 側だけに入る。

一次情報(取得 2026-09-30):

- Anthropic, "Configure permissions" <https://code.claude.com/docs/en/permissions>
  の "Bash rule limits": Bash のルールは書かれたコマンド文字列との一致で、
  `sh -c '…'` や絶対パス、変数(`URL=… && curl $URL`)では外れる。
  コマンド文字列に依存しない強制には sandbox を勧め、コマンド文字列を
  自前のロジックで検査するには PreToolUse hook を使うとしている。
- git, "githooks — pre-push" <https://git-scm.com/docs/githooks>: pre-push は
  送信直前に `<local-ref> <local-object-name> <remote-ref> <remote-object-name>`
  の行を stdin で受け取る(remote 側に ref が無ければ all-zeroes)。範囲が
  push される ref 単位で決まる、という #54 の修正(`<src> --not --remotes=<remote>`)
  の根拠でもある。
- 展開後の実 argv を PreToolUse で受け取る仕組みは、調べた範囲(上記の
  permissions / hooks ページ、`gh help environment`)では見つからなかった。

## Decision

#56 の 4 つの提案を独立に裁定する。

1. **「投稿の意図」の共通化 — 採用。** `bleep-hook intent [--cwd DIR] -- CMD`
   を足し、`gh` の投稿セグメントごとに `{surface, action, repo_source, repo,
   cwd, body_sources, unresolved}` の JSON 配列を返す(書式は `src/intent.rs`)。
   抽出だけを共有し、判定(denylist、attribution、PR タイトル等)は各ガードの
   責務のままにする。`lex`(固定行の書式)を原型とし、`lex` と同じ内部関数
   (`classify_segment`、`resolve_body_sources`)を使うので、抽出の正本は
   `src/lex.rs` の 1 箇所に保たれる。
   - 置き場所: bleep 内のサブコマンドと `tests/fixtures/intent/*.json`。crate
     は分割しない。dotfiles 側の Rust 移植(tarotene/dotfiles#415)は、この
     fixture を共有して緑にする(適合の正本は fixture)。
   - fixture には架空の名前だけを使う(bleep は公開リポジトリで、実在の
     非公開名は入れられない)。

2. **実行時の層(`gh` の PATH shim / `bleep exec gh …`)— 今は不採用。**
   コマンド文字列の照合は展開前なので、`$(…)`・変数・インタプリタ経由の
   `gh` は静的には解決できない。だが今の設計は「解決できなければ ask で
   止める」(#34/#48 の `unresolved-var`、#55 の `body-source-unresolved`・
   `body-source-unreadable`)で、取りこぼしを黙って通さない。PATH shim は
   人間の `gh` を壊さない配線、再帰、遅延が要り、`gh` に pre-request の差し
   込み点が無いため shim の内側でも argv までしか見えない。ask で担えない
   仕事が観測されるまで足さない(還元性)。
   - **見直された(2026-10-01、ADR-0003)**: push については、`unresolved-var` /
     `push-diff-failed` の ask が正当な push を妨げるほど多発した(#67/#69/#70/#73)
     ので、見直す条件に当たった。第一候補の sandbox ではなく、git 自身の
     差し込み点(pre-push)に判定を移した。gh については ADR-0003 の正準形で扱う。
   - **見直す条件**: 判定レッジャー(`~/.local/state/agent-verdicts/bleep.jsonl`)
     で `unresolved-var` / `body-source-unresolved` / `body-source-unreadable` の
     ask が、正当な投稿を妨げるほど多発したとき。または、コマンド文字列の
     照合をすり抜けた実例が出たとき(その場合の第一候補は、permissions
     ページが勧める sandbox)。

3. **`bleep doctor` — 最小版を採用。** 登録された hook(matcher が `Bash` と
   `mcp__` を覆っているか)、`bleep-hook` が実行できるか、bash 本体と
   バイナリの lex protocol の版が一致しているかを検査する。合成した
   denylist で実コマンドに canary を通す検査は、残余として後続で扱う。
   protocol の版の不一致は、実行時にも ask にする(#55 で lex の出力形式が
   変わったので、古いバイナリを新しい bash 本体で動かす事故の守りになる)。

4. **tag / release の運用 — 別件の releaser 整備を待つ。** release-plz は
   設定済みだが、release PR を開く GitHub App の secret(`client-id`)が空で
   毎回失敗し、tag は一度も切られていない(`Cargo.toml` は 0.2.0 のまま)。
   修復は PR の外(リポジトリ設定・org の governance)にあり、別件の
   releaser 整備で扱う。この ADR は方針だけを記録する: 挙動が変わる修正
   (#55/#57/#58 など)を含む版に tag を打ち、利用者側の pin の鮮度を
   検知できるようにする。

## 執行点

- `src/intent.rs` — `intent` の実装(新規)
- `src/main.rs` — `intent` サブコマンドの配線
- `src/lex.rs` — `resolve_body_sources`・`gh_surface`/`gh_action` など、
  `intent` が使う抽出の正本
- `tests/fixtures/intent/issue_body_file_flag_repo.json` — 適合 fixture
  (同じディレクトリに 14 件。コマンド → 期待される意図)
- `tests/intent_test.rs` — fixture を実バイナリに通すテスト(CI の
  `cargo test` で実行される)

提案3(`bleep doctor` の最小版)の執行点:

- `bleep` — `doctor` サブコマンド(`cmd_doctor`)と、`cmd_scan_bash_command` の
  lex protocol 検査(`lex-protocol-mismatch`)、selftest のケース
- `src/main.rs` — `--protocol` と、`lex` 出力の先頭行 `#lex <版>`
- `tests/lex_protocol_test.rs` — 版の期待値と、bash 本体の `LEX_PROTOCOL` との
  一致を検査するテスト

## Consequences

- 抽出の取りこぼし(#55/#57/#58 型)は、fixture に 1 行足せば bleep と
  dotfiles 側の両方の回帰テストになる。
- `intent` は本文の中身を読まない(パスまで)。読む・照合する責務は
  呼び出し側に残る。
- 実行時の層を足さないので、`python -c 'subprocess.run(["gh", …])'` や
  `curl` での API 直叩きは従来どおり対象外(README の「セキュリティ境界
  ではない」の範囲)。
- tag が無い間は、dotfiles 側が固定する bleep の版と main の間の差を、利用者
  が機械的に知る手段が無い。提案3 の protocol の版は、その一部だけを埋める
  (バイナリと bash 本体のずれの検出)。
