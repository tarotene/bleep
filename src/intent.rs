//! `bleep-hook intent` — `gh` の投稿コマンドから「投稿の意図」を取り出して
//! JSON で返す(#56 提案1、docs/adr/0002-gh-intent-and-layers.md)。
//!
//! 抽出だけを共有し、判定(denylist、attribution、PR タイトル等)は各ガードの
//! 責務のままにする。`lex` と同じく純粋関数で、I/O・gh は一切行わない —
//! 本文の入力元はパスまでを返し、中身を読むのは呼び出し側。
//!
//! 出力は投稿セグメントごとの JSON オブジェクトの配列(投稿セグメントが
//! 無ければ `[]`)。各要素:
//!
//! - `surface`: `pr` / `issue` / `release` / `repo` / `gist` / `api`
//! - `action`: `create` / `edit` / `comment`、`gh api` は小文字のメソッド
//! - `repo_source`: `flag`(-R/--repo)/ `api-path`(gh api の API パス)/
//!   `cwd`(cwd の origin。`cwd` に解決済みの実効ディレクトリ)/
//!   `unknown`(静的に決められない)
//! - `repo`: `flag` / `api-path` のときの `owner/repo`、それ以外は null
//! - `cwd`: `repo_source == "cwd"` のときの実効ディレクトリ、それ以外は null
//! - `body_sources`: 本文を読む入力元の絶対パス(stdin は含めない)
//! - `unresolved`: 宛先または本文の入力元を静的に解決できなかった
//!
//! 適合 fixture は tests/fixtures/intent/*.json(コマンド → 期待される
//! 意図)。dotfiles 側の Rust 移植(tarotene/dotfiles#415)は、これを共有して
//! 緑にする。

use crate::lex;
use serde_json::{json, Value};

pub fn intents(cmd: &str, start_dir: &str, home: Option<&str>) -> Value {
    let segments = lex::split_command_segments(cmd);
    let mut out: Vec<Value> = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        if lex::as_single_cd_target(seg).is_some() {
            continue;
        }
        let c = lex::classify_segment(seg);
        if !c.kind_is_gh_publish {
            continue;
        }
        let (body_sources, body_unresolved) =
            lex::resolve_body_sources(&c, &segments, i, start_dir, home);
        let (repo_source, repo, cwd, dest_unresolved) = match (&c.repo_override, c.dest_unknown) {
            (Some(r), _) => (
                if c.repo_from_api_path {
                    "api-path"
                } else {
                    "flag"
                },
                Some(r.clone()),
                None,
                r.contains('$'),
            ),
            (None, true) => ("unknown", None, None, true),
            (None, false) => (
                "cwd",
                None,
                Some(lex::resolve_effective_dir(start_dir, &segments, i, home)),
                lex::cd_chain_has_unresolved_var(&segments, i, home),
            ),
        };
        out.push(json!({
            "surface": c.gh_surface,
            "action": c.gh_action,
            "repo_source": repo_source,
            "repo": repo,
            "cwd": cwd,
            "body_sources": body_sources,
            "unresolved": dest_unresolved || body_unresolved,
        }));
    }
    Value::Array(out)
}
