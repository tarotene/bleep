//! `bleep-hook intent` — `gh` の投稿コマンドから「投稿の意図」を取り出して
//! JSON で返す(#56 提案1、docs/adr/0002-gh-intent-and-layers.md)。
//!
//! 抽出だけを共有し、判定(denylist、attribution、PR タイトル等)は各ガードの
//! 責務のままにする。`lex` と同じく純粋関数で、I/O・gh は一切行わない —
//! 本文の入力元はパスまでを返し、中身を読むのは呼び出し側。
//!
//! 抽出の正本は `grammar.rs`(gh の投稿の正準形、
//! docs/adr/0003-constructive-grammar.md)。投稿(自由記述を公開面へ運ぶ
//! 既知のコマンド)は、正準形に入っていれば `canonical: true`、外れていれば
//! 最初の違反の閉じたコードを `noncanonical_reason` に持つ。読み取りや
//! 本文を持たない操作は、投稿ではないので出さない。
//!
//! 出力は投稿ごとの JSON オブジェクトの配列(投稿が無ければ `[]`)。各要素:
//!
//! - `surface`: `pr` / `issue` / `release` / `repo` / `gist` / `api`
//!   (解釈できないセグメントは `unknown`)
//! - `action`: `create` / `edit` / `comment` / `review` / `close` / `merge` /
//!   `reopen`、`gh api` は小文字のメソッド
//! - `repo_source`: `flag`(-R/--repo)/ `url`(PR・Issue の URL)/
//!   `positional`(`gh repo edit` の位置引数)/ `api-path`(gh api の API
//!   パス)/ `none`(静的に決められない)
//! - `repo`: リテラルで取れた `owner/repo`、取れなければ null
//! - `body_sources`: 本文を読む入力元の絶対パス(stdin は含めない)
//! - `canonical`: 正準形か
//! - `noncanonical_reason`: 違反コード(`src/grammar.rs` の `NONCANONICAL_CODES`)、
//!   正準形なら null
//!
//! 適合 fixture は tests/fixtures/intent/*.json(コマンド → 期待される
//! 意図)。dotfiles 側の Rust 移植(tarotene/dotfiles#415)は、これを共有して
//! 緑にする。

use crate::lex;
use serde_json::{json, Value};

pub fn intents(cmd: &str) -> Value {
    let out: Vec<Value> = lex::analyze(cmd)
        .posts
        .into_iter()
        .map(|p| {
            json!({
                "surface": p.surface,
                "action": p.action,
                "repo_source": p.repo_source.as_str(),
                "repo": p.repo,
                "body_sources": p.body_sources,
                "canonical": p.noncanonical.is_none(),
                "noncanonical_reason": p.noncanonical,
            })
        })
        .collect();
    Value::Array(out)
}
