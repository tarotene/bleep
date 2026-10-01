# ADR-0003 — 生成済みコマンドを検査する代わりに、受理する形を小さな文法に刈り取る

- Status: Accepted
- Date: 2026-10-01
- Context: `/wrapup-chores` での設計の見直し。push の偽陽性(#67/#69/#70/#73)と
  gh の取りこぼし(#65/#71)が、同じ根から出ていた
- 関連: ADR-0002(提案 2「実行時の層」の見直し条件を満たしたので、push については
  この ADR で置き換える)

## Context

bleep は PreToolUse で、エージェントが書いた Bash コマンドの文字列から「何を
どこへ公開するか」を静的に復元して照合してきた。`git push` なら範囲(どの ref の、
どの commit か)、`gh` なら宛先と本文である。

この復元は、書き方の自由度に追いつけない。`git push` は `-C`・`cd`・変数・
refspec の省略形・worktree・shallow clone で範囲の求め方が変わり、`gh` は
`--body`・`--body-file`・`-F`・`--input`・コマンド置換・`sh -c` と、本文の渡し方が
いくつもある。実際に出た不具合は、どれも「ある書き方で復元を取りこぼした/求め損ねた」
形をしている。

- #69/#73: 新規ブランチ・別 worktree が checkout しているブランチ・shallow clone で、
  範囲の基点(merge-base)を求め損ねて `push-diff-failed` の ask を繰り返した
- #70: `W=<path>; cd $W && …` の変数を解決できず `unresolved-var` の ask を繰り返した
- #67: 範囲の計算が、中間 commit の patch を照合するという意図どおりの挙動で止まったが、
  どの commit が原因か分からず、偽陽性と区別できなかった
- #71/#65: `gh issue close --comment`、`--notes-file`、`gist create <file>` が
  抽出の対象外で素通りした

これは終わりのない探索である(README が引く CWE-184 "Incomplete List of Disallowed
Inputs" と同じ構造)。コマンドを後から検査する側は、書き方が 1 つ増えるたびに
抽出を足すことになる。言語理論に基づくセキュリティ(LangSec)の言い方では、
入力言語が大きすぎるのに、全部を認識しきらずに処理(照合)へ進んでいる
(shotgun parsing)。

一次情報(取得 2026-10-01):

- Sassaman, Patterson, Bratus, Locasto, "Security Applications of Formal Language
  Theory", IEEE Systems Journal 7(3), 2013
  <https://langsec.org/papers/sassaman-jsys7-3.pdf>: 入力言語に足りる強さの
  parser を使い、それ以上強くしない("use a sufficiently strong parser for an input
  language, but no stronger")。Dejector は、既知の良いクエリから SQL の
  部分文法を作り、検証を「部分言語への所属判定」に還元する。
- Momot, Bratus, Hallberg, Patterson, "The Seven Turrets of Babel: A Taxonomy of
  LangSec Errors and How to Expunge Them", IEEE SecDev, 2016
  <https://langsec.org/papers/langsec-cwes-secdev2016.pdf>: 場当たり的な検証
  (shotgun parsing)と、必要以上に強い入力言語をアンチパターンとして挙げる。
- OWASP, "SQL Injection Prevention Cheat Sheet"
  <https://cheatsheetseries.owasp.org/cheatsheets/SQL_Injection_Prevention_Cheat_Sheet.html>:
  prepared statement は意図を構造で固定し(escaping は "strongly discouraged")、
  検査ではなく構成で防ぐ。
- git, "githooks — pre-push" <https://git-scm.com/docs/githooks>: pre-push は
  送信直前の `<local-ref> <local-sha> <remote-ref> <remote-sha>` を stdin で
  受け取る。コマンドの書き方に依らない、git が正規化した ref と完全な SHA である。
  迂回は `git push --no-verify`(<https://git-scm.com/docs/git-push>)と
  `-c core.hooksPath=…` で、どちらもコマンドラインに現れる。
- Anthropic, "Configure permissions" <https://code.claude.com/docs/en/permissions>:
  Bash のルールは書かれたコマンド文字列との一致で、`sh -c` や変数などの別の
  書き方では外れる。コマンド文字列を後から検査する方式の限界を、公式が認めている。

## Decision

**受理する形を、検査しやすい小さな文法に刈り取る。** bleep は「その文法に入って
いるか」の判定(認識)と、入っているものの中身の照合だけを担う。文法の外は、
素通りでも ask でもなく deny にして、書き直し方を理由文で案内する。

1. **push — 範囲の推測をやめ、git の pre-push が渡す事実で判定する。**
   `bleep scan-push --pre-push <remote-name> <remote-url>` を足し、stdin の
   `<local-ref> <local-sha> <remote-ref> <remote-sha>` から範囲を求める。
   - remote-sha が全 0 でなく、手元にある commit なら `remote..local`
   - それ以外(新規ブランチ、shallow clone で先端が手元に無い、リモートが先行)は
     `local --not --remotes=<remote>`。追跡 ref が無ければ履歴全体
   - 削除(local-sha が全 0)は公開する内容が無いので飛ばす
   - 可視性は remote の URL から求める(`--repo` やカレントの origin を使わない)

   PreToolUse の `git push` は、コマンド文字列から範囲を求めるのをやめる(lex の
   `push_dir`・`push_spec` を廃止、`LEX_PROTOCOL` を 3 に上げる)。止めるのは、
   pre-push を無効にする形 — `--no-verify` と `-c core.hooksPath=…` — だけ
   (`push-hook-bypass`)。これは git-push(1) と git-config(1) に列挙された
   閉じた集合なので、トークン一致で足りる。環境変数(`GIT_CONFIG_*`)など他の
   迂回は、README の「セキュリティ境界ではない」の範囲に残る(検出のみ)。

   `hooks/pre-push` を同梱し、`bleep doctor` が配線(有効な `core.hooksPath` か
   `.git/hooks/` の pre-push が `scan-push --pre-push` を呼ぶか)を検査する。
   引数なしの `scan-push`(default branch との merge-base 経路)は、配線が切り
   替わるまでの互換経路として残す。

   #67 の「どの commit が止めたか分からない」は、deny/ask の理由文に、最初に
   一致した commit の短い SHA を出して解く(語は出さない)。判定レッジャーの
   `detail` は閉じた語彙(`[a-z-]`)なので、SHA は理由文だけに置く。

2. **gh の投稿 — 受理する正準形を閉じた文法で定義する。**
   文法の対象は、自由記述を公開面へ運ぶ既知のコマンド(`pr create|edit|comment|
   review|close|merge|reopen`、`issue create|edit|comment|close|reopen`、
   `release create|edit`、`repo edit`、`gist create`、書き込みの `gh api`)。
   これらのコマンドが正準形なのは、次を満たすとき:
   - 宛先がリテラルの `-R OWNER/REPO`(`pr|issue` は PR/Issue の URL、`repo edit`
     は位置引数の OWNER/REPO、`gh api` は `repos/<owner>/<repo>/…` のパスでも可。
     gist は宛先なし)。cwd の origin からの推測、`cd` の追跡、変数の解決はしない
   - 本文は絶対パスのリテラルのファイルだけ(`--body-file`・`-F`、release は
     `--notes-file`、gist は位置引数のファイル、`gh api` は `-F k=@path`・`--input`)。
     `--body`・`-b`・`--notes`・`-n`・`close|reopen` の `--comment`・stdin(`-`)・
     相対パス・`~`・変数は文法の外
   - そのほかの値はリテラルだけ(`$(…)`・バッククォート・`$VAR` を含まない)

   文法の外は deny する(`gh-noncanonical`。理由コードは閉じた語彙
   `no-repo`/`bad-repo`/`inline-body`/`inline-comment`/`body-stdin`/`body-path`/
   `dynamic-value`/`unparsable`)。理由文は正準形と書き直し方を案内する。トークン
   化できない投稿(ヒアドキュメント本文のアポストロフィで引用符が閉じない形)も
   `unparsable` で deny する。`env`・`sudo`・`timeout` などの前置、
   `sh -c '…'`・`eval '…'`、`$(…)`・バッククォートの中身も、同じ文法に再帰して
   通す(深さは 3 まで)。`unresolved-var`・`body-source-unresolved` の ask は、
   文法の外になるので発生しなくなった。

   **文法の対象外は pass のまま**: 読み取り(`pr view`・`run watch`・`api` の GET)や、
   自由記述を持たない操作(`pr ready` など)、表に載っていない `gh` のサブコマンドは、
   従来どおり検査しない。表に載っていないものを一律 deny にすると `gh pr merge` の
   ような日常の操作まで止まり、偽陽性の多さで迂回が常態化する(README の
   「偽陽性の高さは、エージェントにも人間にも迂回を常態化させる」)。Dejector も
   SQL の部分文法を検証するだけで、SQL でないものは対象外である。宛先が PRIVATE/
   INTERNAL のリテラルな投稿は、文法も含めて検査しない。

3. **分担 — 組み立てる側は bleep の外。** bleep が持つのは認識器(文法の定義と
   所属判定)と照合器(denylist)だけ。弾かれた側が正準形を組み立てるための
   手順(skill の例示、pre-push の配線、gh の引数を読む各ガードの
   `--body-file` 対応)は、配線と利用者側の手順を持つ tarotene/dotfiles の
   責務とする(README の Scope の既存の線引き)。正準形の定義の正本は bleep の
   認識器と `tests/fixtures/intent/` で、dotfiles 側は複写せず fixture を共有する
   (ADR-0002)。

### 検討して採らなかったもの

- **GitHub の MCP サーバに寄せ、Bash の gh 書き込みをすべて deny する。**
  `issue_write`・`add_issue_comment`・`create_pull_request` などは owner/repo/body が
  構造化された入力で、表現不可能性は最も高い
  (<https://github.com/github/github-mcp-server>、取得 2026-10-01)。ただし
  dotfiles が MCP を retire 済みで、gh の引数を読む既存ガード 8 本の移植が要る。
  正準形は同じ文法の考え方を raw gh のまま採るので、MCP への移行は、この文法の
  上に後から足せる。
- **bleep 自身が構成用 CLI(`bleep post …`)を持つ。** 感触で外した(構成する側の
  道具が検査器に入ると、役割が混ざる感触)。分析ではない。
- **PATH shim で git / gh を包む。** ADR-0002 提案 2 と同じ理由で採らない。push には
  git 自身の差し込み点(pre-push)があり、包む必要が無い。

## 執行点

- `hooks/pre-push` — pre-push の入力を `scan-push --pre-push` に渡す git hook(新規)
- `bleep` — `cmd_scan_pre_push`・`push_range_for_ref`・`attribute_commit`・
  `diff_text_for_range`・`push_dir_check`(新規)、`cmd_scan_bash_command` の
  `push-hook-bypass`、`cmd_doctor` の pre-push 検査、selftest(e2e 節を含む)
- `src/lex.rs` — `push_bypass` の検出(`classify_segment`・`analyze`)。
  `push_dir`・`push_spec` の廃止
- `src/main.rs` — `LEX_PROTOCOL` 3 と lex の出力形式
- `tests/lex_protocol_test.rs` — 版の期待値と、bash 本体の `LEX_PROTOCOL` との一致
- `tests/host_test.rs` — ペイロードの cwd が `--cwd` に渡ること(本文の入力元で検査)

gh の正準形(決定 2)の執行点:

- `src/grammar.rs` — 正準形の認識器(`recognize`)と違反コード(新規)
- `src/lex.rs` — 前置・`sh -c`・`eval`・`$(…)` の再帰的な分類。cd 追跡・
  `unresolved_var`・`gh_effective_dir` の廃止
- `src/intent.rs` — `canonical`・`noncanonical_reason` を返す JSON
- `src/main.rs` — `LEX_PROTOCOL` 4 と lex の出力形式(投稿ごとの違反・宛先・本文)
- `bleep` — `cmd_scan_bash_command`(投稿ごとの可視性判定、`gh-noncanonical` の deny)、
  `noncanonical_reason_text`、selftest
- `tests/fixtures/intent/*.json` — 適合 fixture(正準形と文法外のコマンド →
  期待される意図。31 件)
- `tests/intent_test.rs`・`tests/lex_protocol_test.rs`・`tests/host_test.rs`

## Consequences

- push の偽陽性の類(新規ブランチ・別 worktree・shallow clone・変数付きの
  `cd`/`-C`)は、範囲を推測しないので起きない。
- PreToolUse だけを配線して pre-push を配線しない環境(Codex・Copilot の
  ホストだけを使う場合など)では、push は無検査になる。`bleep doctor` が NG を
  返す。git の hook はホストに依らないので、`core.hooksPath` に置けば全ホストを
  一度に覆える。
- `--no-verify` は PreToolUse で deny される。人間が自分の端末で使うことは
  妨げない(PreToolUse の対象はエージェントの Bash だけ)。
- 判定レッジャーの `reason_id` に `push-hook-bypass`・`gh-noncanonical` が増える。
  消費側の schema(tarotene/dotfiles)の閉語彙の拡張が要る。
- gh の投稿の正準形を組み立てる側(skill の例示、`--body` を読む各ガードの
  `--body-file` 対応)は dotfiles の責務で、切り替えのタイミングは dotfiles が
  bleep の pin を上げる版で握る。bleep 側は、`bleep-hook intent` の JSON(旧
  `cwd`・`unresolved` を廃止し、`canonical`・`noncanonical_reason` を追加)と
  fixture を、適合の正本として提供する。
- 文法の外は deny なので、書き直しが必要になる。書き直しの方法は理由文に出る。
