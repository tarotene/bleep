# ADR-0001 — `publish-guard` を `bleep` に改名する

- Status: Accepted
- Date: 2026-09-24
- Context: `/grill-me` セッションでの裁定(ユーザー依頼「このリポジトリの
  キャッチーな名前を考えてみて」「OSS であるからにはある程度外部利用を
  奨励したい」)

## Context

このリポジトリは public・MIT・star 0 で、旧名 `publish-guard` は機能の
記述そのもの(`<対象>-<種別語>` 型の `naming-descriptive`)。外部利用を
促すには口コミに乗る固有名が要る、というのが改名の動機。

改名コストが最小になる窓は改名当日(2026-09-24)しかなかった。実測時点で
`publish-guard` / `publish-guard-hook` はどちらも crates.io に未公開
(404)だったが、当時スタック中だった Rust 移植 PR 群の1本の本文に「手動
`cargo publish` + crates.io Trusted Publishing 登録」が実行待ちの
TODO として残っていた。crates.io は publish が恒久的で、`cargo yank`
してもクレート名は解放されない(The Cargo Book, "Publishing on
crates.io" <https://doc.rust-lang.org/cargo/reference/publishing.html>、
取得 2026-09-24: "a publish is generally permanent. The version can
never be overwritten, and the code cannot be deleted")。この TODO を
実行前に踏めば旧名が永久に固定されるため、改名をその TODO の実行より
前のスタック段として積んだ(tarotene/dotfiles の stacked-PR 規範、
docs/adr/0027 系、取得 2026-09-24)。

## Decision

名前を **`bleep`** にする — 生放送で、出てはいけない一語に被せる音。

命名クラスは `naming-descriptive` から `naming-coined` へ移す
(tarotene/dotfiles `docs/adr/0026-naming-lifecycle-axis-and-codename-
split.md`、Date 2026-09-21、取得 2026-09-24: `naming-coined` は単一
トークン `^[a-z0-9]+$`、閉語彙なし、著者固有の有意味な造語)。org 初の
`naming-coined` 宣言になる。`naming-codename` を採らないのは、あちらが
dotfiles の `config/github-audit/codename-registry.tsv` への登録 PR を
default-deny で要求し、リポジトリ名と登録簿という複写+同期を生むため。

### 候補と実測(2026-09-24)

crates.io API(`https://crates.io/api/v1/crates/<name>`)と GitHub
Search API(`search/repositories?q=<name>+in:name`)で名前空間を実測して
足切りした。

| 候補 | crates.io | GitHub 同名件数 / 最大 star | 判定 |
|---|---|---|---|
| `bleep` | 空き | 869 / 181(`oyvindberg/bleep`、Scala ビルドツール、別エコシステム) | **採用** |
| `pokayoke` | 空き | 53 / 27 | 対抗馬 |
| `thwart` | 空き | 51 / 6(名前空間は最も空いている) | 対抗馬 |
| `plimsoll` | 空き | 58 / 12 | 外した(比喩が受動的、絵の素材が無い) |
| `interdict` | 空き | 93 / 8 | 外した(敵対的な遮断を含意し、README の「セキュリティ境界ではない」という自己規定と矛盾) |
| `andon` | 空き | 1531 / 164 | 外した(同名多数で埋もれる) |
| `pratique` | 空き | 4925 / 19 | 外した(仏語一般語で同名多数) |
| `backstop` | 空き | 610 / 7181(`garris/BackstopJS`) | 外した(著名プロジェクトと衝突) |
| `omerta` | 空き | 未計測 | 外した(犯罪の沈黙律という含意が企業導入の体面を損なう) |
| `airlock` `portcullis` `cerberus` `heimdall` `sluice` `bulkhead` ほか | 全て TAKEN | — | 外した(crates.io で取得済み) |

決め手は比喩の質ではなく実測: 選択時点で評価できたのは名前空間の空き
具合と、既存 org 先例(`naming-codename` 8件のうち public な
`telepath` を含む、実在語の比喩・ポートマンテーという2系統の造語脈)への
整合のみ。比喩の良し悪しと口コミ性は事後的にしか評価できない(残りの
private リポジトリの命名例はこの public リポジトリの成果物には書かない
— CONTRIBUTING.md「Zero proper nouns」)。

### アイキャッチ

いらすとや(<https://www.irasutoya.com/>)で `情報漏えい` `口止め`
`秘密` `放送` `通せんぼ` `マイク` を検索した(2026-09-24)。`bleep` を
直撃する「ピー音」素材は無く、近傍(「ON AIR ランプ」「秘密のポーズ」
「マイクを持っている男性会社員」)を合成すれば絵は作れるが、単独では
成立しない。対抗馬だった `pokayoke` は「指差し確認・指差し呼称」が
8点ヒットし1点で意味が完結していた。OpenMoji(CC BY-SA 4.0)・unDraw
(帰属不要)も確認したが同様に直撃素材は無かった。

いらすとやの利用規約(<https://www.irasutoya.com/p/terms.html>、取得
2026-09-24)は商用利用 1 制作物 20 点まで無料、禁止は「素材を主体とした
コンテンツ・商品の再配布・販売」——規約上は使えたが、既成素材を使わず
`docs/img/bleep.svg`(検閲バー)を自作する判断にした。favicon・OG 画像・
README ロゴを同一図形で賄え、外部素材のライセンス制約という恒久的な
同期点を持ち込まずに済む。

## 執行点

この PR で新規追加・変更した実ファイル:

- リポジトリ全体の識別子置換(`publish-guard` → `bleep`、
  `publish-guard-hook` → `bleep-hook`、`PUBLISH_GUARD_*` → `BLEEP_*`):
  `bleep`(旧 `publish-guard`)、`hooks/bleep.sh`(旧 `hooks/pg-hook.sh`)、
  `hooks/hooks.json`、`Cargo.toml`、`Cargo.lock`、`src/main.rs`、
  `src/host.rs`、`src/lex.rs`、`tests/host_test.rs`、
  `tests/fixtures/gh-stub.sh`、`.claude-plugin/plugin.json`、
  `.claude-plugin/marketplace.json`、`release-plz.toml`、
  `.github/workflows/ci.yml`、`.github/workflows/release-plz.yml`、
  `README.md`、`CONTRIBUTING.md`
- `docs/adr/0001-name-bleep.md`(本ファイル)
- `docs/img/bleep.svg`

`gh repo rename`・topic 宣言(`naming-coined`)・description の書き直しは
段2(GitHub 側メタデータ)、dotfiles 側の `home/modules/claude.nix` 等の
追従は段3(companion PR)で行う — いずれもこの PR のマージ後に着手する
別段。

## Consequences

- crates.io に `publish-guard`/`publish-guard-hook` という名前は一度も
  存在しない。
- `~/.config/publish-guard/` からの設定移行はユーザーの手動操作
  (README「Upgrading from `publish-guard`」参照)。後方互換シムは
  置かない — 実ユーザーが本人 1人であるこの時点では、移行検出機構を
  作って後で消すコストの方が大きい。
- dotfiles 側の ADR(0009/0014/0020)本文は書き換えない。ADR-0014 が
  `publish-guard` を `naming-descriptive` の代表例として名指ししている
  箇所も、当時の形の歴史的な記録としてそのまま残す。
