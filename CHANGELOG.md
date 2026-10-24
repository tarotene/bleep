# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
## [0.2.0] — 2026-10-24

### Added

- **bleep**: Repo-ref の拒否文に、一致した欄と行番号を添える ([#119](https://github.com/tarotene/bleep/pull/119))

- **gh**: 変数のコマンド名とシェルへ流すテキストを opaque-exec で deny にする ([#110](https://github.com/tarotene/bleep/pull/110))

- **bleep**: ツール名で repo-ref の検査から外す allow-tools.txt を足す ([#107](https://github.com/tarotene/bleep/pull/107))

- **lex**: コード位置とデータ位置を、シェルの構文の構造で決める ([#80](https://github.com/tarotene/bleep/pull/80)) ([#82](https://github.com/tarotene/bleep/pull/82))

- **gh**: 本文ファイルを持つ投稿を単独のコマンドに限り、TOCTOU を閉じる ([#78](https://github.com/tarotene/bleep/pull/78)) ([#79](https://github.com/tarotene/bleep/pull/79))

- **gh**: Gh の投稿を正準形に限り、文法の外は deny する ([#75](https://github.com/tarotene/bleep/pull/75))

- **push**: Pre-push の入力で push を判定し、PreToolUse は迂回の形だけを止める ([#74](https://github.com/tarotene/bleep/pull/74))

- **doctor**: Bleep doctor の最小版と lex protocol の版検査 ([#56](https://github.com/tarotene/bleep/pull/56)) ([#63](https://github.com/tarotene/bleep/pull/63))

- **intent**: Gh の投稿の意図を JSON で返す bleep-hook intent と ADR-0002 ([#56](https://github.com/tarotene/bleep/pull/56)) ([#62](https://github.com/tarotene/bleep/pull/62))

- **scan**: Gh の本文をファイル・API 入力から解決して照合する ([#55](https://github.com/tarotene/bleep/pull/55)) ([#60](https://github.com/tarotene/bleep/pull/60))

- **scan**: 短い private リポ名を deny から ask に降格する([#44](https://github.com/tarotene/bleep/pull/44)) ([#52](https://github.com/tarotene/bleep/pull/52))

- **explain**: Term_hash から denylist の一致語を人間の端末で復元する([#44](https://github.com/tarotene/bleep/pull/44)) ([#51](https://github.com/tarotene/bleep/pull/51))

- **lex**: 未展開のシェル変数参照を検出して ask にエスカレートする ([#48](https://github.com/tarotene/bleep/pull/48))

- **ledger**: Denied/asked 判定をローカルの判定レッジャーに記録する ([#32](https://github.com/tarotene/bleep/pull/32))

- **hooks**: 3 adapter を publish-guard-hook + pg-hook.sh に切り替える ([#27](https://github.com/tarotene/bleep/pull/27))

- **hook**: Adapter 3本と字句解析を publish-guard-hook(Rust)に移植する ([#25](https://github.com/tarotene/bleep/pull/25))

- **audit**: 履歴モードと配線レシピを追加する ([#20](https://github.com/tarotene/bleep/pull/20))

- **scan**: Gh の公開面サブコマンドを網羅する ([#18](https://github.com/tarotene/bleep/pull/18))

- **cli**: PreToolUse payload の cwd を --cwd で受け取る ([#16](https://github.com/tarotene/bleep/pull/16))

- Codex CLI / Copilot CLI adapter を追加する(実機で deny/pass を実測)

- **claude**: Claude Code plugin adapter を追加する(MCP カバレッジ込み)

- **cli**: 汎用判定エンジン publish-guard を実装する


### Fixed

- **gate**: Opaque-exec を gh の違反から切り離し、reason_id を分ける ([#118](https://github.com/tarotene/bleep/pull/118))

- **bleep**: 相対パスの BLEEP_LEX_BIN でも selftest が通るようにする ([#117](https://github.com/tarotene/bleep/pull/117))

- **bleep**: Private-repos-cache の更新が殺されても一時ファイルを残さない ([#109](https://github.com/tarotene/bleep/pull/109))

- **gh**: Gh api graphql の読み取り query を投稿として扱わない ([#105](https://github.com/tarotene/bleep/pull/105))

- **bleep**: Repo-ref の拒否文に term_hash・実行できる explain・次の一手を添える ([#104](https://github.com/tarotene/bleep/pull/104))

- **gh**: --help と 2>&1 を投稿の偽陽性から外し、拒否文に単独規則を足す ([#103](https://github.com/tarotene/bleep/pull/103))

- **lex**: --body-file のパスを照合対象から外し、拒否の理由文に一致した場所を示す ([#86](https://github.com/tarotene/bleep/pull/86))

- **lex**: 二重引用符の中の置換を、内側の引用符・演算子で切らない ([#93](https://github.com/tarotene/bleep/pull/93))

- **scan**: Push の範囲を実際の ref から求め、失敗理由をレッジャーに残す ([#54](https://github.com/tarotene/bleep/pull/54)) ([#61](https://github.com/tarotene/bleep/pull/61))

- **lex**: Gh api の暗黙の POST と API パス由来の宛先を扱う (#57, #58) ([#59](https://github.com/tarotene/bleep/pull/59))

- **scan**: WORD_HARD/WORD_WARN の単語境界で拡張子・ハイフンを非境界にする ([#46](https://github.com/tarotene/bleep/pull/46))

- **lex**: Gh --repo/-R を位置非依存で検出し、git -C の相対パスを cd 履歴に連結する ([#45](https://github.com/tarotene/bleep/pull/45))

- **lex**: Cd/-C の ~ と $HOME を hook プロセスの HOME に展開する ([#40](https://github.com/tarotene/bleep/pull/40))

- **scan**: 未設定の理由文を「生成元の再適用・人間の判断」に寄せる ([#36](https://github.com/tarotene/bleep/pull/36))

- **scan**: Orgs.txt が無いとき scan* を無言 pass させず ask にする ([#31](https://github.com/tarotene/bleep/pull/31))

- **scan-bash-command**: コマンド字句解析にバックスラッシュエスケープを実装する ([#24](https://github.com/tarotene/bleep/pull/24))

- **scan**: コマンドのセグメントを走査して実効対象リポジトリを解決する ([#17](https://github.com/tarotene/bleep/pull/17))

- **scan-push**: 削除行を denylist スキャン対象から除外する ([#8](https://github.com/tarotene/bleep/pull/8))


### Other

- **release**: Release-plz の版を git tag から決め、tag を切れるようにする ([#108](https://github.com/tarotene/bleep/pull/108))

- **renovate**: アカウント共通の自動マージ方針を継承する ([#100](https://github.com/tarotene/bleep/pull/100))

- **docs**: API ドキュメントの strict ビルドを CI に足す ([#99](https://github.com/tarotene/bleep/pull/99))

- **readme**: README を charter のスキーマに合わせる ([#98](https://github.com/tarotene/bleep/pull/98))

- インタプリタの heredoc 本文はデータ扱いである回帰例を足す ([#91](https://github.com/tarotene/bleep/pull/91))

- **ci**: CI の命名基準 (ADR-591) に合わせ、集約 job と ruleset 宣言を足す ([#87](https://github.com/tarotene/bleep/pull/87))

- **cargo**: Bleep-hook を crates.io へ publish しない ([#72](https://github.com/tarotene/bleep/pull/72))

- **ci**: Create-github-app-token の app-id を client-id に移行する ([#66](https://github.com/tarotene/bleep/pull/66))

- **scan**: Unconfigured の理由文に .backup 検知のヒントを追加する ([#47](https://github.com/tarotene/bleep/pull/47))

- **hooks**: Selftest の判定レッジャーを一時ディレクトリに隔離する ([#37](https://github.com/tarotene/bleep/pull/37))

- Add PR title check workflow ([#33](https://github.com/tarotene/bleep/pull/33))

- **rename**: Publish-guard を bleep に改名する ([#28](https://github.com/tarotene/bleep/pull/28))

- **rust**: CI に cargo job を追加し、release-plz/Renovate を配線する ([#26](https://github.com/tarotene/bleep/pull/26))

- **config**: Allow-stopwords の効力範囲を明記し回帰を固定する ([#19](https://github.com/tarotene/bleep/pull/19))

- Translate README/CONTRIBUTING to English ([#13](https://github.com/tarotene/bleep/pull/13))

- Bring charter into ADR-0016/0017 schema ([#12](https://github.com/tarotene/bleep/pull/12))

- README に charter(Scope / Issue litmus)を追加する ([#6](https://github.com/tarotene/bleep/pull/6))

- README を完成させ、self-marketplace 化して v0.1.0 を切る

- **repo**: Scaffold LICENSE / README / .gitignore

