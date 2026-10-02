//! コマンド文字列の字句解析(D1)。純粋関数のみ — I/O・gh・キャッシュは一切
//! 触らない。`match_verdict` / `build_patterns` / `resolve_repo_nwo` は正本が
//! Bash 実装のまま(判断を運ばない継ぎ目)。
//!
//! コマンドを `;` `&&` `||` `|` 改行でセグメントに分け、セグメントごとに
//! `git push`(pre-push を迂回する形かどうか)と `gh` の投稿(正準形かどうか、
//! `grammar.rs`)を分類する。`env`・`sudo`・`timeout` などの前置、
//! `sh -c '…'`・`eval '…'`、`$(…)`・バッククォートの中身は、同じ分類に
//! 再帰して通す(docs/adr/0003-constructive-grammar.md)。cd の追跡や変数の
//! 解決はしない — 解決が要る形(相対パス、変数)は、そもそも文法の外にする。

use crate::grammar::{self, GhPost};

/// split_command_segments の移植。演算子(`;` `&&` `||` `|&` 単独の `&`/`|`
/// および改行)でコマンドをセグメントに分割する。クォート境界の判定は
/// POSIX のエスケープ規則(#22 で Bash 側に実装した規則と同一)に従う —
/// これはシェル演算子の文法そのものであり、標準的な word-splitting クレート
/// (shell-words 等)の担当範囲外なので自前で持つ。
///
/// クォートが閉じないまま終わった場合は分割せず、コマンド全体を1セグメント
/// にする(Bash 版と同じ、パーサの失敗を緩和側の分割として使わない設計)。
pub fn split_command_segments(cmd: &str) -> Vec<String> {
    let chars: Vec<char> = cmd.chars().collect();
    let n = chars.len();
    let mut segments = Vec::new();
    let mut buf = String::new();
    let mut i = 0usize;
    let mut quote: Option<char> = None;

    while i < n {
        let c = chars[i];
        match quote {
            Some('\'') => {
                // 単一引用符内は \ を含め一切のエスケープが働かない(POSIX)。
                buf.push(c);
                if c == '\'' {
                    quote = None;
                }
                i += 1;
                continue;
            }
            Some('"') => {
                // 二重引用符内は \ が $ ` " \ および改行の直前でだけ
                // エスケープとして働く。\" をここで閉じ扱いにしてはいけない。
                if c == '\\' {
                    if let Some(&nc) = chars.get(i + 1) {
                        if matches!(nc, '$' | '`' | '"' | '\\' | '\n') {
                            buf.push(c);
                            buf.push(nc);
                            i += 2;
                            continue;
                        }
                    }
                }
                buf.push(c);
                if c == '"' {
                    quote = None;
                }
                i += 1;
                continue;
            }
            None => {}
            Some(_) => unreachable!("quote is only ever set to ' or \""),
        }

        match c {
            '\\' => {
                // クォート外の \ は直後の1文字をエスケープする(POSIX)。
                // raw text は変更しない(このセグメント自体は unescape
                // しない設計を維持 — 両文字とも buf に積む)。
                if let Some(&nc) = chars.get(i + 1) {
                    buf.push(c);
                    buf.push(nc);
                    i += 2;
                } else {
                    buf.push(c); // 末尾の孤立した \ はリテラル扱い
                    i += 1;
                }
                continue;
            }
            '\'' | '"' => {
                quote = Some(c);
                buf.push(c);
                i += 1;
                continue;
            }
            ';' | '\n' => {
                segments.push(std::mem::take(&mut buf));
                i += 1;
                continue;
            }
            '&' | '|' => {
                let two: String = chars[i..(i + 2).min(n)].iter().collect();
                if two == "&&" || two == "||" || two == "|&" {
                    segments.push(std::mem::take(&mut buf));
                    i += 2;
                    continue;
                }
                // 単独の & (background) / | (pipe) も区切りとして扱う。
                segments.push(std::mem::take(&mut buf));
                i += 1;
                continue;
            }
            _ => {}
        }
        buf.push(c);
        i += 1;
    }

    if quote.is_some() {
        return vec![cmd.to_string()]; // クォート閉じ忘れ = パース不能。分割しない。
    }
    segments.push(buf);
    segments
}

/// セグメントが「厳密に `cd <単一トークン>`」だけであれば、そのターゲット
/// 文字列を返す(前後の空白は許容、ターゲット自体に空白は含まない)。
/// scan_subject から cd セグメントを除く判定(#9 — cd のパスに private リポ名が
/// 含まれるだけでは deny しない)。
pub(crate) fn as_single_cd_target(seg: &str) -> Option<&str> {
    let rest = seg.trim_start();
    let rest = rest.strip_prefix("cd")?;
    let rest = rest.strip_prefix(|c: char| c.is_ascii_whitespace())?;
    let target = rest.trim_end();
    if target.is_empty() || target.chars().any(|c| c.is_ascii_whitespace()) {
        None
    } else {
        Some(target)
    }
}

/// 分類の結果。1 つのセグメントから複数出ることがある(`sh -c` や `$(…)` の中)。
#[derive(Debug, Clone)]
pub enum Item {
    /// `git push`。`bypass` は pre-push を無効にする形(`--no-verify`、
    /// `-c core.hooksPath=…`)。
    Push { bypass: bool },
    /// `gh` の投稿(表に載ったコマンド)。正準形かどうかは `noncanonical`。
    Gh(GhPost),
}

/// 再帰の深さの上限(`sh -c` の中の `sh -c` …)。超えた中身は見ない — 検出の
/// 範囲であって、セキュリティ境界ではない(README)。
const MAX_DEPTH: usize = 3;

/// コマンドの前に付けて、後ろのコマンドをそのまま実行する語。
const WRAPPERS: [&str; 13] = [
    "env", "command", "exec", "nohup", "time", "nice", "sudo", "doas", "timeout", "xargs",
    "stdbuf", "setsid", "ionice",
];
const SHELLS: [&str; 5] = ["sh", "bash", "zsh", "dash", "ksh"];

fn basename(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

/// 先頭の `(` `{` はサブシェル/グループの開き。語の本体だけを返す。
fn word(tok: &str) -> &str {
    tok.trim_start_matches(['(', '{'])
}

/// `NAME=value` 形の代入か。
fn is_assignment(tok: &str) -> bool {
    match tok.split_once('=') {
        Some((n, _)) => {
            !n.is_empty()
                && n.chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        None => false,
    }
}

/// 代入・サブシェルの開き・`!`・前置の語を読み飛ばして、実際のコマンド語の
/// 位置を返す。前置の語(env、sudo、timeout …)はオプションの形が語ごとに違う
/// ので、後ろに最初に現れる gh / git / シェルの位置までを前置として飛ばす。
/// 見つからなければ `t.len()`。
fn command_start(t: &[String]) -> usize {
    let mut i = 0usize;
    while i < t.len() {
        let w = word(&t[i]);
        if w.is_empty() || w == "!" || is_assignment(w) {
            i += 1;
            continue;
        }
        if WRAPPERS.contains(&basename(w)) {
            let found = (i + 1..t.len()).find(|&j| {
                let b = basename(word(&t[j]));
                b == "gh" || b == "git" || b == "eval" || SHELLS.contains(&b)
            });
            return found.unwrap_or(t.len());
        }
        return i;
    }
    t.len()
}

/// `git [global options] push [args]` の `push` の位置と、`-c core.hooksPath=…`
/// があったか。global option の値を読み飛ばしてサブコマンド位置を確定する
/// (`git -C dir push` を取りこぼさない、#9/#14)。
fn git_push_form(t: &[String], start: usize) -> Option<bool> {
    let n = t.len();
    let mut idx = start + 1;
    let mut hooks_path_overridden = false;
    while idx < n {
        match t[idx].as_str() {
            "-C" | "--git-dir" | "--work-tree" | "--namespace" => idx += 2,
            "-c" => {
                if t.get(idx + 1).is_some_and(|v| config_disables_hooks(v)) {
                    hooks_path_overridden = true;
                }
                idx += 2;
            }
            s if s.starts_with("-c=") => {
                if config_disables_hooks(&s[3..]) {
                    hooks_path_overridden = true;
                }
                idx += 1;
            }
            s if s.starts_with('-') => idx += 1, // 未知の global option。安全側に読み飛ばす
            _ => break,
        }
    }
    if idx < n && t[idx] == "push" {
        let skip_hook = t[idx + 1..].iter().any(|a| a == "--no-verify");
        Some(hooks_path_overridden || skip_hook)
    } else {
        None
    }
}

/// `git -c <key>=<value>` が hook の置き場を差し替える(= pre-push を外す)
/// 設定か。git-config(1) の `core.hooksPath`。キーは大文字小文字を区別しない
/// (`core.hookspath` も同じ設定)。
fn config_disables_hooks(kv: &str) -> bool {
    let key = kv.split('=').next().unwrap_or("");
    key.eq_ignore_ascii_case("core.hooksPath")
}

/// `$(…)` とバッククォートの中身を取り出す(対応する閉じ括弧まで。引用符は
/// 見ない — 過剰に取るだけで、取りこぼしはしない)。
fn substitutions(seg: &str) -> Vec<String> {
    let chars: Vec<char> = seg.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '$' && chars.get(i + 1) == Some(&'(') {
            let mut depth = 1usize;
            let mut j = i + 2;
            while j < chars.len() && depth > 0 {
                match chars[j] {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            let end = if depth == 0 { j - 1 } else { chars.len() };
            out.push(chars[i + 2..end].iter().collect());
            i += 2;
        } else if chars[i] == '`' {
            if let Some(off) = chars[i + 1..].iter().position(|&c| c == '`') {
                out.push(chars[i + 1..i + 1 + off].iter().collect());
                i += off + 2;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

/// トークン化に失敗した(クォートが閉じていない — ヒアドキュメントの本文の
/// アポストロフィなど)セグメントが、gh の投稿の形に見えるか。見えるなら
/// 「解釈できない」として文法の外にする(解釈できないものを素通りにしない)。
fn looks_like_gh_post(seg: &str) -> bool {
    let words: Vec<&str> = seg.split_whitespace().collect();
    words.windows(2).any(|w| {
        basename(word(w[0])) == "gh"
            && matches!(w[1], "pr" | "issue" | "release" | "repo" | "gist" | "api")
    })
}

fn unparsable_post() -> Item {
    Item::Gh(GhPost {
        surface: "unknown".to_string(),
        action: String::new(),
        repo_source: grammar::RepoSource::None,
        repo: None,
        body_sources: Vec::new(),
        noncanonical: Some("unparsable"),
    })
}

/// 本文ファイルを持つ投稿が、単独のコマンドでない位置にあるとき文法の外にする
/// (`not-alone`、docs/adr/0004-body-file-post-stands-alone.md)。照合は
/// PreToolUse の時点のファイルを読むが、投稿されるのは同じコマンドの前段が
/// 書き換えた後の中身になりうる(TOCTOU)。既に別の違反があればそれを残す。
fn require_alone(item: &mut Item) {
    if let Item::Gh(p) = item {
        if p.noncanonical.is_none() && !p.body_sources.is_empty() {
            p.noncanonical = Some("not-alone");
        }
    }
}

/// `sh -c`・`eval`・`$(…)` の中で見つけた項目。トップレベルの単独のコマンドでは
/// ないので、`require_alone` を通す。
fn nested(mut items: Vec<Item>) -> Vec<Item> {
    items.iter_mut().for_each(require_alone);
    items
}

fn classify_tokens(t: &[String], depth: usize) -> Vec<Item> {
    let mut out = Vec::new();
    let i = command_start(t);
    if i >= t.len() {
        return out;
    }
    let cmd = basename(word(&t[i]));
    match cmd {
        "git" => {
            if let Some(bypass) = git_push_form(t, i) {
                out.push(Item::Push { bypass });
            }
        }
        "gh" => {
            let mut g: Vec<String> = vec!["gh".to_string()];
            g.extend(t[i + 1..].iter().cloned());
            if let Some(post) = grammar::recognize(&g) {
                out.push(Item::Gh(post));
            }
        }
        c if SHELLS.contains(&c) && depth < MAX_DEPTH => {
            // sh -c '…' / bash -lc '…': -c を含む短いオプションの束の次が本文。
            let mut j = i + 1;
            while j < t.len() {
                let a = t[j].as_str();
                if a.starts_with('-') && !a.starts_with("--") && a.contains('c') {
                    if let Some(script) = t.get(j + 1) {
                        out.extend(nested(classify_command_depth(script, depth + 1)));
                    }
                    break;
                } else if a.starts_with('-') {
                    j += 1;
                } else {
                    break;
                }
            }
        }
        "eval" if depth < MAX_DEPTH => {
            let script = t[i + 1..].join(" ");
            out.extend(nested(classify_command_depth(&script, depth + 1)));
        }
        _ => {}
    }
    out
}

fn classify_segment_depth(seg: &str, depth: usize) -> Vec<Item> {
    let mut out = Vec::new();
    match shell_words::split(seg) {
        Ok(t) => out.extend(classify_tokens(&t, depth)),
        Err(_) => {
            if looks_like_gh_post(seg) {
                out.push(unparsable_post());
            }
        }
    }
    if depth < MAX_DEPTH {
        for inner in substitutions(seg) {
            out.extend(nested(classify_command_depth(&inner, depth + 1)));
        }
    }
    out
}

fn classify_command_depth(cmd: &str, depth: usize) -> Vec<Item> {
    split_command_segments(cmd)
        .iter()
        .flat_map(|seg| classify_segment_depth(seg, depth))
        .collect()
}

/// セグメント 1 つを分類する(`sh -c` や `$(…)` の中身も含む)。
pub fn classify_segment(seg: &str) -> Vec<Item> {
    classify_segment_depth(seg, 0)
}

/// 解析結果。
#[derive(Debug, Default)]
pub struct AnalyzeResult {
    /// denylist の照合対象。CMD 全文から `cd <単一トークン>` セグメントを除き、
    /// 宛先を `-R`/`--repo` で明示した gh セグメントからはその宛先トークンを
    /// 除いたもの(#9、#43 — 宛先そのものは漏洩ではない)。
    pub scan_subject: String,
    pub found_push: bool,
    /// いずれかの `git push` が pre-push を無効にする形。
    pub push_bypass: bool,
    /// gh の投稿(表に載ったコマンド)。コマンドの出現順。
    pub posts: Vec<GhPost>,
}

/// gh セグメントの生テキストから `--repo`/`-R`(値を伴う2トークン形)と
/// `--repo=値`(1トークン形)を取り除き、残りのトークンを
/// `shell_words::join`(`split` の逆演算、必要最小限の再クォート)で
/// 再結合したテキストを返す(#43、D2)。
fn strip_repo_override_tokens(seg: &str) -> String {
    let t = shell_words::split(seg).unwrap_or_default();
    let mut kept: Vec<String> = Vec::with_capacity(t.len());
    let mut j = 0usize;
    while j < t.len() {
        match t[j].as_str() {
            "--repo" | "-R" => {
                j += 2; // フラグ+値の2トークンを両方取り除く
            }
            s if s.starts_with("--repo=") => j += 1,
            _ => {
                kept.push(t[j].clone());
                j += 1;
            }
        }
    }
    shell_words::join(kept)
}

pub fn analyze(cmd: &str) -> AnalyzeResult {
    let mut result = AnalyzeResult::default();
    let segments = split_command_segments(cmd);
    let alone = segments.iter().filter(|s| !s.trim().is_empty()).count() <= 1;
    for seg in segments {
        let mut items = classify_segment(&seg);
        if !alone {
            items.iter_mut().for_each(require_alone);
        }

        if as_single_cd_target(&seg).is_none() {
            let strips = items
                .iter()
                .any(|it| matches!(it, Item::Gh(p) if p.repo_source == grammar::RepoSource::Flag));
            if strips {
                result
                    .scan_subject
                    .push_str(&strip_repo_override_tokens(&seg));
            } else {
                result.scan_subject.push_str(&seg);
            }
            result.scan_subject.push('\n');
        }

        for it in items {
            match it {
                Item::Push { bypass } => {
                    result.found_push = true;
                    result.push_bypass |= bypass;
                }
                Item::Gh(p) => result.posts.push(p),
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn posts(cmd: &str) -> Vec<GhPost> {
        analyze(cmd).posts
    }

    #[test]
    fn backslash_escaped_quote_does_not_close_segment_early() {
        // #22 の回帰1と同型: \" が && の区切りを壊さないこと。
        let segs = split_command_segments(r#"echo \" && gh pr create --title t"#);
        assert_eq!(segs.len(), 2);
        assert!(segs[1].trim_start().starts_with("gh pr create"));
    }

    #[test]
    fn repo_value_with_escaped_quote_is_not_a_valid_destination() {
        // #22 の回帰2と同型: --repo の値に \" があっても、引用符の解釈は崩れない。
        // 値は owner/repo の形ではないので、宛先として受理しない。
        let p = &posts(r#"gh --repo "x\"y" pr create --title t --body-file /tmp/b.md"#)[0];
        assert_eq!(p.noncanonical, Some("bad-repo"));
    }

    #[test]
    fn repo_flag_forms_after_subcommand_are_detected() {
        // #23: --repo/-R/--repo=値 は gh の persistent flag で位置に依存しない。
        for cmd in [
            "gh pr create --repo acme/other --body-file /tmp/b.md",
            "gh issue create -R acme/other --body-file /tmp/b.md",
            "gh pr create --repo=acme/other --body-file /tmp/b.md",
            "gh --repo acme/other pr create --body-file /tmp/b.md",
        ] {
            let p = &posts(cmd)[0];
            assert_eq!(p.repo.as_deref(), Some("acme/other"), "{cmd}");
            assert_eq!(p.noncanonical, None, "{cmd}");
        }
    }

    #[test]
    fn repo_override_token_excluded_from_scan_subject() {
        // #43(D2): 明示された --repo の宛先トークン自体は scan_subject から
        // 除外される(#9 の cd 除外と同じ考え方)。タイトルの内容は残る。
        let r = analyze(
            r#"gh pr create --repo acme/public-oss --title unrelated --body-file /tmp/b.md"#,
        );
        assert_eq!(r.posts.len(), 1);
        assert!(!r.scan_subject.contains("acme/public-oss"));
        assert!(r.scan_subject.contains("unrelated"));
    }

    #[test]
    fn cd_segment_excluded_from_scan_subject() {
        let r = analyze(r#"cd /x/secret-repo-work && gh pr create --title t"#);
        assert!(!r.scan_subject.contains("secret-repo-work"));
    }

    #[test]
    fn prefixes_and_wrappers_do_not_hide_a_post() {
        for cmd in [
            "env X=1 gh issue create -R a/b --body hi",
            "X=1 gh issue create -R a/b --body hi",
            "/usr/bin/gh issue create -R a/b --body hi",
            "sudo -u me gh issue create -R a/b --body hi",
            "timeout 30 gh issue create -R a/b --body hi",
            "nohup gh issue create -R a/b --body hi",
            "(gh issue create -R a/b --body hi)",
            "{ gh issue create -R a/b --body hi; }",
            "sh -c 'gh issue create -R a/b --body hi'",
            "bash -lc \"gh issue create -R a/b --body hi\"",
            "eval gh issue create -R a/b --body hi",
            "env sh -c 'gh issue create -R a/b --body hi'",
            "echo $(gh issue create -R a/b --body hi)",
            "x=`gh issue create -R a/b --body hi`",
            "sh -c 'sh -c \"gh issue create -R a/b --body hi\"'",
        ] {
            let ps = posts(cmd);
            assert_eq!(ps.len(), 1, "{cmd}");
            assert_eq!(ps[0].noncanonical, Some("inline-body"), "{cmd}");
        }
    }

    #[test]
    fn nesting_depth_is_bounded() {
        // 深すぎる入れ子は見ない(検出の範囲。セキュリティ境界ではない)。
        let deep = "sh -c 'sh -c \"sh -c \\\"sh -c gh\\\\ issue\\\\ create\\\"\"'";
        let _ = analyze(deep); // 無限再帰しないこと
    }

    #[test]
    fn unparsable_gh_post_is_noncanonical() {
        // ヒアドキュメント本文のアポストロフィでクォートが閉じない形。解釈できない
        // gh の投稿を素通りにしない。
        let ps = posts("gh issue create -R a/b --body-file /dev/stdin <<EOF\nit's here\nEOF");
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].noncanonical, Some("unparsable"));
        // gh の投稿に見えなければ、解釈できなくても何も出さない。
        assert!(posts("echo it's fine").is_empty());
    }

    #[test]
    fn multiple_posts_are_all_reported_in_order() {
        let ps = posts(
            "gh issue create -R pub/one --body-file /tmp/a.md && gh issue comment 2 -R pub/two -F /tmp/c.md",
        );
        let repos: Vec<_> = ps.iter().map(|p| p.repo.as_deref()).collect();
        assert_eq!(repos, [Some("pub/one"), Some("pub/two")]);
    }

    #[test]
    fn body_file_post_must_stand_alone() {
        // #78 の F2: 前段が本文ファイルを書き換えると、照合した中身と投稿される
        // 中身がずれる。本文ファイルを持つ投稿は単独のコマンドに限る。
        let alone = "gh issue create -R pub/r --body-file /tmp/x.md";
        assert_eq!(posts(alone)[0].noncanonical, None);
        for cmd in [
            "cp /tmp/a.md /tmp/x.md && gh issue create -R pub/r --body-file /tmp/x.md",
            "cd /x && gh issue create -R pub/r --body-file /tmp/x.md",
            "git push && gh issue create -R pub/r --body-file /tmp/x.md",
            "gh issue create -R pub/r --body-file /tmp/x.md; rm /tmp/x.md",
            "gh issue create -R pub/r --body-file /tmp/x.md | tee log",
            "printf x > /tmp/x.md\ngh issue create -R pub/r --body-file /tmp/x.md",
            "sh -c 'gh issue create -R pub/r --body-file /tmp/x.md'",
            "eval gh issue create -R pub/r --body-file /tmp/x.md",
            "echo $(gh issue create -R pub/r --body-file /tmp/x.md)",
        ] {
            let ps = posts(cmd);
            assert_eq!(ps.len(), 1, "{cmd}");
            assert_eq!(ps[0].noncanonical, Some("not-alone"), "{cmd}");
        }
        // 前置は単独のコマンドのまま。
        for cmd in [
            "env X=1 gh issue create -R pub/r --body-file /tmp/x.md",
            "timeout 30 gh issue create -R pub/r --body-file /tmp/x.md",
            "gh issue create -R pub/r --body-file /tmp/x.md\n",
            "gh issue create -R pub/r --body-file /tmp/x.md;",
        ] {
            assert_eq!(posts(cmd)[0].noncanonical, None, "{cmd}");
        }
        // 本文ファイルを持たない投稿と、別の違反が先に立つ投稿は影響を受けない。
        assert_eq!(
            posts("echo hi && gh issue close 5 -R pub/r")[0].noncanonical,
            None
        );
        assert_eq!(
            posts("cd /x && gh issue create -R pub/r --body hi")[0].noncanonical,
            Some("inline-body")
        );
        // 複数の投稿は、並べた時点でどれも単独ではない。
        let ps = posts("gh issue create -R a/b --body-file /tmp/a.md && gh issue comment 2 -R c/d -F /tmp/c.md");
        assert!(ps.iter().all(|p| p.noncanonical == Some("not-alone")));
    }

    #[test]
    fn push_bypass_detects_the_forms_that_skip_pre_push() {
        // pre-push を外す形だけを拾う。普通の push は found_push のみ。
        let bypass = |cmd: &str| {
            let r = analyze(cmd);
            assert!(r.found_push, "{cmd}");
            r.push_bypass
        };
        assert!(!bypass("git push origin main"));
        assert!(!bypass("git -C /x push -u origin HEAD"));
        assert!(bypass("git push --no-verify origin main"));
        assert!(bypass("git push origin main --no-verify"));
        assert!(bypass("git -c core.hooksPath=/dev/null push origin main"));
        assert!(bypass("git -c core.hookspath=/dev/null push origin main"));
        assert!(bypass("git -c=core.hooksPath=/dev/null push origin main"));
        // 前置・包みの中でも拾う。
        assert!(bypass("env X=1 git push --no-verify"));
        assert!(bypass("sh -c 'git push --no-verify'"));
        assert!(bypass("echo $(git push --no-verify)"));
        // 無関係な -c は迂回ではない。
        assert!(!bypass("git -c user.name=x push origin main"));
        // push 以外のサブコマンドは found_push にならない。
        assert!(!analyze("git -c core.hooksPath=/dev/null status").found_push);
    }

    #[test]
    fn unterminated_quote_segment_is_not_classified_as_push() {
        let r = analyze(r#"git push "unterminated"#);
        assert!(!r.found_push);
        assert!(r.posts.is_empty());
    }
}
