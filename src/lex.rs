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

/// コマンドの 1 区切り(セグメント)と、そこに書かれたヒアドキュメント。
#[derive(Debug, Clone, Default)]
pub struct Segment {
    /// セグメントの字面。ヒアドキュメントの本文は含まない(`<<'EOF'` の
    /// 区切りの記法までは含む)。コメントは除く。
    pub text: String,
    pub heredocs: Vec<Heredoc>,
}

#[derive(Debug, Clone)]
pub struct Heredoc {
    /// 区切りの語に引用符(`'…'`・`"…"`・`\`)が 1 つでもあるか。POSIX 2.7.4:
    /// あれば本文は展開されないデータ、なければ本文のパラメータ展開・コマンド
    /// 置換・算術展開が行われる(`$(…)` とバッククォートはコード)。
    pub quoted: bool,
    pub body: String,
}

struct PendingHeredoc {
    /// 本文が属するセグメントの添字(`<<` を読んだ時点の `segments.len()`)。
    seg: usize,
    word: String,
    strip_tabs: bool,
    quoted: bool,
}

fn flush(segments: &mut Vec<Segment>, buf: &mut String) {
    segments.push(Segment {
        text: std::mem::take(buf),
        heredocs: Vec::new(),
    });
}

/// `<<` の直後(`start`)から区切りの語を読む。`(語, 引用符あり, <<- か, 終端)`。
/// 語が空、または引用符が閉じなければ `None`(ヒアドキュメントと見なさない)。
fn parse_heredoc_delimiter(chars: &[char], start: usize) -> Option<(String, bool, bool, usize)> {
    let n = chars.len();
    let mut j = start;
    let strip = chars.get(j) == Some(&'-');
    if strip {
        j += 1;
    }
    while j < n && (chars[j] == ' ' || chars[j] == '\t') {
        j += 1;
    }
    let mut word = String::new();
    let mut quoted = false;
    while j < n {
        let c = chars[j];
        match c {
            '\'' | '"' => {
                quoted = true;
                let s = j + 1;
                let close = chars[s..].iter().position(|&x| x == c)?;
                word.extend(&chars[s..s + close]);
                j = s + close + 1;
            }
            '\\' => {
                quoted = true;
                if let Some(&nc) = chars.get(j + 1) {
                    word.push(nc);
                    j += 2;
                } else {
                    j += 1;
                }
            }
            c if c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')' | '<' | '>') => break,
            _ => {
                word.push(c);
                j += 1;
            }
        }
    }
    if word.is_empty() {
        None
    } else {
        Some((word, quoted, strip, j))
    }
}

/// `start`(改行の次)から本文を読む。終端の行が見つからなければ `None`。
/// 返すのは `(本文, 終端の行の次の位置)`。
fn read_heredoc_body(chars: &[char], start: usize, p: &PendingHeredoc) -> Option<(String, usize)> {
    let n = chars.len();
    let mut pos = start;
    let mut body = String::new();
    while pos < n {
        let eol = chars[pos..]
            .iter()
            .position(|&c| c == '\n')
            .map(|k| pos + k);
        let line: String = chars[pos..eol.unwrap_or(n)].iter().collect();
        let cmp = if p.strip_tabs {
            line.trim_start_matches('\t')
        } else {
            line.as_str()
        };
        if cmp == p.word {
            return Some((body, eol.map_or(n, |e| e + 1)));
        }
        body.push_str(&line);
        body.push('\n');
        pos = eol? + 1;
    }
    None
}

/// コマンドを演算子(`;` `&&` `||` `|&` 単独の `&`/`|` および改行)でセグメントに
/// 分割する。クォート境界の判定は POSIX のエスケープ規則(#22 で Bash 側に実装した
/// 規則と同一)に従う — これはシェル演算子の文法そのものであり、標準的な
/// word-splitting クレート(shell-words 等)の担当範囲外なので自前で持つ。
///
/// ヒアドキュメント(`<<WORD`・`<<-WORD`)は、本文を、書かれたセグメントの
/// 付属物にする(本文の各行を別のコマンドとして分割しない、docs/adr/
/// 0005-code-position-closure.md)。本文は、`<<` を含む行の終わりから、WORD
/// だけの行までである。次の場合は、ヒアドキュメントと見なさず従来どおりに
/// 分割する(コードをデータに取り違える向きの誤りを避けるため): `$((…))`・
/// `((…))` の中、here-string(`<<<`)、区切りの語が空・引用符が閉じない、
/// 終端の行が無い。`#` から行末まではコメントとして捨てる。
///
/// クォートが閉じないまま終わった場合は分割せず、コマンド全体を1セグメント
/// にする(Bash 版と同じ、パーサの失敗を緩和側の分割として使わない設計)。
pub fn split_command_segments(cmd: &str) -> Vec<Segment> {
    let chars: Vec<char> = cmd.chars().collect();
    let n = chars.len();
    let mut segments: Vec<Segment> = Vec::new();
    let mut buf = String::new();
    let mut i = 0usize;
    // `'` 単一引用符、`"` 二重引用符、`$` は ANSI-C 引用符(`$'…'`)。
    let mut quote: Option<char> = None;
    let mut pending: Vec<PendingHeredoc> = Vec::new();
    // `((` の中の、閉じていない括弧の数。0 でなければ算術式の中。
    let mut arith = 0usize;

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
            Some('$') => {
                // ANSI-C 引用符: \ が直後の 1 文字をエスケープし、\' では閉じない。
                buf.push(c);
                if c == '\\' {
                    if let Some(&nc) = chars.get(i + 1) {
                        buf.push(nc);
                        i += 2;
                        continue;
                    }
                } else if c == '\'' {
                    quote = None;
                }
                i += 1;
                continue;
            }
            None => {}
            Some(_) => unreachable!("quote is only ever set to ', \" or $"),
        }

        // here-string(`<<<`)は三文字ごと読み飛ばす(ヒアドキュメントではない)。
        if c == '<' && chars.get(i + 1) == Some(&'<') && chars.get(i + 2) == Some(&'<') {
            buf.push_str("<<<");
            i += 3;
            continue;
        }
        if c == '<' && chars.get(i + 1) == Some(&'<') && arith == 0 {
            if let Some((word, quoted, strip_tabs, end)) = parse_heredoc_delimiter(&chars, i + 2) {
                buf.extend(&chars[i..end]);
                pending.push(PendingHeredoc {
                    seg: segments.len(),
                    word,
                    strip_tabs,
                    quoted,
                });
                i = end;
                continue;
            }
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
            '$' if chars.get(i + 1) == Some(&'\'') => {
                quote = Some('$');
                buf.push_str("$'");
                i += 2;
                continue;
            }
            '#' if buf
                .chars()
                .last()
                .is_none_or(|p| p.is_whitespace() || matches!(p, ';' | '&' | '|' | '(')) =>
            {
                // 語の先頭の # は、行末までコメント(実行も公開もされない)。
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '(' => {
                if arith > 0 {
                    arith += 1;
                } else if chars.get(i + 1) == Some(&'(') {
                    arith = 2;
                    buf.push('(');
                    i += 1;
                }
                buf.push('(');
                i += 1;
                continue;
            }
            ')' => {
                arith = arith.saturating_sub(1);
                buf.push(')');
                i += 1;
                continue;
            }
            '\n' if !pending.is_empty() => {
                flush(&mut segments, &mut buf);
                i += 1;
                for p in pending.drain(..) {
                    if let Some((body, next)) = read_heredoc_body(&chars, i, &p) {
                        segments[p.seg].heredocs.push(Heredoc {
                            quoted: p.quoted,
                            body,
                        });
                        i = next;
                    }
                }
                continue;
            }
            ';' | '\n' => {
                flush(&mut segments, &mut buf);
                i += 1;
                continue;
            }
            '&' | '|' => {
                let two: String = chars[i..(i + 2).min(n)].iter().collect();
                if two == "&&" || two == "||" || two == "|&" {
                    flush(&mut segments, &mut buf);
                    i += 2;
                    continue;
                }
                // 単独の & (background) / | (pipe) も区切りとして扱う。
                flush(&mut segments, &mut buf);
                i += 1;
                continue;
            }
            _ => {}
        }
        buf.push(c);
        i += 1;
    }

    if quote.is_some() {
        // クォート閉じ忘れ = パース不能。分割しない。
        return vec![Segment {
            text: cmd.to_string(),
            heredocs: Vec::new(),
        }];
    }
    flush(&mut segments, &mut buf);
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

/// `$(…)` とバッククォートの中身を取り出す(入れ子も含む。外側が先)。
///
/// `quote_aware` が真なら、シェルの引用規則に従って、展開されない場所を飛ばす:
/// 単一引用符 `'…'`、ANSI-C 引用符 `$'…'`、`\` で打ち消された `$`・バッククォート。
/// 二重引用符の中は展開されるので飛ばさない。偽なら、引用符付きでない
/// ヒアドキュメントの本文(`'` や `"` はただの文字で、`\` だけが効く)として読む。
/// 取りこぼしの向きの誤り(展開されるものを飛ばす)を避けるため、判断に迷う
/// 引用の形(閉じない引用符など)は、飛ばさない側に倒す。
fn substitutions(text: &str, quote_aware: bool) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    scan_substitutions(&chars, 0, false, quote_aware, &mut out);
    out
}

/// `i` から読み、`nested`(`$(` の中)なら対応する `)` の次の位置と閉じたか、
/// そうでなければ末尾を返す。
fn scan_substitutions(
    chars: &[char],
    mut i: usize,
    nested: bool,
    quote_aware: bool,
    out: &mut Vec<String>,
) -> (usize, bool) {
    let n = chars.len();
    let mut dq = false;
    let mut paren = 0usize;
    while i < n {
        let c = chars[i];
        if c == '\\' {
            i += 2;
            continue;
        }
        if quote_aware && !dq {
            if c == '\'' {
                // 閉じない単一引用符は飛ばさない(取りこぼしを避ける)。
                match chars[i + 1..].iter().position(|&x| x == '\'') {
                    Some(k) => i += k + 2,
                    None => i += 1,
                }
                continue;
            }
            if c == '$' && chars.get(i + 1) == Some(&'\'') {
                let mut j = i + 2;
                while j < n && chars[j] != '\'' {
                    j += if chars[j] == '\\' { 2 } else { 1 };
                }
                i = if j < n { j + 1 } else { i + 2 };
                continue;
            }
        }
        if quote_aware && c == '"' {
            dq = !dq;
            i += 1;
            continue;
        }
        if c == '$' && chars.get(i + 1) == Some(&'(') {
            let slot = out.len();
            out.push(String::new());
            let (end, closed) = scan_substitutions(chars, i + 2, true, quote_aware, out);
            let close = if closed { end - 1 } else { end };
            out[slot] = chars[i + 2..close].iter().collect();
            i = end;
            continue;
        }
        if c == '`' {
            let mut j = i + 1;
            while j < n && chars[j] != '`' {
                j += if chars[j] == '\\' { 2 } else { 1 };
            }
            if j < n {
                out.push(chars[i + 1..j].iter().collect());
                i = j + 1;
            } else {
                i += 1;
            }
            continue;
        }
        if nested {
            if c == '(' {
                paren += 1;
            } else if c == ')' {
                if paren == 0 {
                    return (i + 1, true);
                }
                paren -= 1;
            }
        }
        i += 1;
    }
    (n, false)
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

/// ANSI-C 引用符 `$'…'`(`\\` が直後の 1 文字をエスケープし、`\\'` では閉じない)を、
/// 同じ字面の単一引用符の語に直す。shell-words は `$'…'` を知らず、`\\'` で
/// 引用符の対応がずれて、後ろの語を引用符の中と取り違える(取りこぼしの向き)。
/// 中身のエスケープは解釈しない(`\\n` は `n`)— 語の境界が正しければ足りる。
fn normalize_ansi_c(seg: &str) -> String {
    let chars: Vec<char> = seg.chars().collect();
    let mut out = String::with_capacity(seg.len());
    let mut i = 0usize;
    let mut quote: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some(q) => {
                out.push(c);
                if q == '"' && c == '\\' {
                    if let Some(&nc) = chars.get(i + 1) {
                        out.push(nc);
                        i += 2;
                        continue;
                    }
                }
                if c == q {
                    quote = None;
                }
                i += 1;
            }
            None => {
                if c == '\\' {
                    out.push(c);
                    if let Some(&nc) = chars.get(i + 1) {
                        out.push(nc);
                    }
                    i += 2;
                } else if c == '$' && chars.get(i + 1) == Some(&'\'') {
                    let mut j = i + 2;
                    let mut lit = String::new();
                    while j < chars.len() && chars[j] != '\'' {
                        if chars[j] == '\\' && j + 1 < chars.len() {
                            j += 1;
                        }
                        lit.push(chars[j]);
                        j += 1;
                    }
                    if j < chars.len() {
                        out.push_str(&shell_words::quote(&lit));
                        i = j + 1;
                    } else {
                        out.push(c); // 閉じない: そのまま(後段が解釈できないと判断する)
                        i += 1;
                    }
                } else {
                    if c == '\'' || c == '"' {
                        quote = Some(c);
                    }
                    out.push(c);
                    i += 1;
                }
            }
        }
    }
    out
}

/// セグメントを語に分ける(shell-words、ANSI-C 引用符は先に直す)。
fn split_words(seg: &str) -> Result<Vec<String>, shell_words::ParseError> {
    shell_words::split(&normalize_ansi_c(seg))
}

fn classify_segment_depth(seg: &str, depth: usize) -> Vec<Item> {
    let mut out = Vec::new();
    match split_words(seg) {
        Ok(t) => out.extend(classify_tokens(&t, depth)),
        Err(_) => {
            if looks_like_gh_post(seg) {
                out.push(unparsable_post());
            }
        }
    }
    if depth < MAX_DEPTH {
        // 置換の出力が実行される文脈(`eval "$(cat <<'EOF' …)"`)なら、置換の中の
        // ヒアドキュメントは、引用符付きでもコードになる。
        let output_is_run = reads_code_from_stdin(seg);
        for inner in substitutions(seg, true) {
            out.extend(nested(classify_command_ctx(
                &inner,
                depth + 1,
                output_is_run,
            )));
        }
    }
    out
}

fn classify_command_depth(cmd: &str, depth: usize) -> Vec<Item> {
    classify_command_ctx(cmd, depth, false)
}

/// `heredocs_are_code` は、呼び出し側の文脈から、このコマンドのヒアドキュメントを
/// コードとして扱うと決まっているとき真。
fn classify_command_ctx(cmd: &str, depth: usize, heredocs_are_code: bool) -> Vec<Item> {
    let segs = split_command_segments(cmd);
    let stdin_shell = heredocs_are_code || any_reads_code_from_stdin(&segs);
    segs.iter()
        .flat_map(|seg| classify_segment_full(seg, stdin_shell, depth))
        .collect()
}

/// セグメントの標準入力を、シェルがコマンドとして読むか(`bash`・`sh -s`・
/// `. /dev/stdin`)。`-c` があれば標準入力はコードではない。取りこぼしの向きの
/// 誤りを避けるため、`-c` の有無だけを見て、スクリプトの引数は見ない
/// (`bash script.sh <<EOF` もコードとして扱う — 過剰側)。
fn reads_code_from_stdin(seg: &str) -> bool {
    let Ok(t) = split_words(seg) else {
        return false;
    };
    let i = command_start(&t);
    if i >= t.len() {
        return false;
    }
    let cmd = basename(word(&t[i]));
    let args = &t[i + 1..];
    if SHELLS.contains(&cmd) {
        let has_c = args
            .iter()
            .take_while(|a| a.as_str() != "--")
            .any(|a| a.starts_with('-') && !a.starts_with("--") && a.contains('c'));
        return !has_c;
    }
    // `eval "$(cat <<'EOF' … EOF)"` は、ヒアドキュメントの本文を `cat` が読んで
    // 出力し、`eval` がそれを実行する。置換を含む `eval` は、同じコマンドの
    // ヒアドキュメントがコードになりうる形として扱う。
    if cmd == "eval" {
        return args.iter().any(|a| a.contains("$(") || a.contains('`'));
    }
    matches!(cmd, "." | "source")
        && args
            .iter()
            .any(|a| matches!(a.as_str(), "/dev/stdin" | "/dev/fd/0" | "-"))
}

/// 同じコマンドの中に、標準入力をコードとして読むシェルがあるか。あれば、
/// `cat <<'EOF' | bash` のようにヒアドキュメントの本文がそこへ流れうるので、
/// 同じコマンドのヒアドキュメントはすべてコードとして扱う。
fn any_reads_code_from_stdin(segs: &[Segment]) -> bool {
    segs.iter().any(|s| reads_code_from_stdin(&s.text))
}

/// セグメントの字面と、そのヒアドキュメントの本文のうちコードであるもの
/// (標準入力をシェルが読むなら全体、引用符なしの区切りなら本文中の
/// `$(…)`・バッククォート)を分類する。引用符付きの本文でシェルに流れないものは
/// データで、見ない。
fn classify_segment_full(seg: &Segment, stdin_shell: bool, depth: usize) -> Vec<Item> {
    let mut out = classify_segment_depth(&seg.text, depth);
    if depth < MAX_DEPTH {
        for h in &seg.heredocs {
            if stdin_shell {
                out.extend(nested(classify_command_depth(&h.body, depth + 1)));
            } else if !h.quoted {
                for inner in substitutions(&h.body, false) {
                    out.extend(nested(classify_command_depth(&inner, depth + 1)));
                }
            }
        }
    }
    out
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
    let t = split_words(seg).unwrap_or_default();
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
    let alone = segments
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .count()
        <= 1;
    let stdin_shell = any_reads_code_from_stdin(&segments);
    for seg in &segments {
        let mut items = classify_segment_full(seg, stdin_shell, 0);
        if !alone {
            items.iter_mut().for_each(require_alone);
        }

        if as_single_cd_target(&seg.text).is_none() {
            let strips = items
                .iter()
                .any(|it| matches!(it, Item::Gh(p) if p.repo_source == grammar::RepoSource::Flag));
            if strips {
                result
                    .scan_subject
                    .push_str(&strip_repo_override_tokens(&seg.text));
            } else {
                result.scan_subject.push_str(&seg.text);
            }
            result.scan_subject.push('\n');
        }
        // コードとして実行されうる本文は照合の対象に含める。引用符付きでシェルに
        // 流れない本文はデータで、ここでは公開されない(公開する地点で照合する)。
        for h in &seg.heredocs {
            if stdin_shell || !h.quoted {
                result.scan_subject.push_str(&h.body);
            }
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
        assert!(segs[1].text.trim_start().starts_with("gh pr create"));
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
        // クォートが閉じない gh の投稿は、解釈できないものを素通りにしない。
        let ps = posts("gh issue create -R a/b --title 'unclosed --body-file /tmp/b.md");
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].noncanonical, Some("unparsable"));
        // gh の投稿に見えなければ、解釈できなくても何も出さない。
        assert!(posts("echo it's fine").is_empty());
        // ヒアドキュメントで本文を流し込む形は、構文として解釈できる。本文の
        // アポストロフィでクォートは崩れず、標準入力の本文は文法の外。
        let ps = posts("gh issue create -R a/b --body-file /dev/stdin <<EOF\nit's here\nEOF");
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].noncanonical, Some("body-stdin"));
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

    /// 末尾の空のセグメント(最後の改行や、本文を読み終えた後)を除く。
    fn split_nonempty(cmd: &str) -> Vec<Segment> {
        split_command_segments(cmd)
            .into_iter()
            .filter(|s| !s.text.trim().is_empty())
            .collect()
    }

    fn close_repos(cmd: &str) -> Vec<String> {
        posts(cmd).into_iter().filter_map(|p| p.repo).collect()
    }

    #[test]
    fn heredoc_body_is_attached_to_its_segment_not_split() {
        let segs = split_nonempty("cat > f <<'EOF'\nline1\nline2\nEOF\necho x");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text.trim(), "cat > f <<'EOF'");
        assert_eq!(segs[0].heredocs.len(), 1);
        assert!(segs[0].heredocs[0].quoted);
        assert_eq!(segs[0].heredocs[0].body, "line1\nline2\n");
        assert_eq!(segs[1].text, "echo x");

        // 引用符なし、`<<-`(先頭のタブを無視)、複数、パイプの手前。
        let segs = split_nonempty("cat <<EOF | tee f\nbody\nEOF");
        assert_eq!(segs.len(), 2);
        assert!(!segs[0].heredocs[0].quoted);
        assert!(segs[1].heredocs.is_empty());
        let segs = split_nonempty("cat <<-X\n\tbody\n\tX\n");
        assert_eq!(segs[0].heredocs[0].body, "\tbody\n");
        let segs = split_nonempty("cat <<A; cat <<\"B\"\na\nA\nb\nB");
        assert_eq!(segs.len(), 2);
        assert!(!segs[0].heredocs[0].quoted && segs[1].heredocs[0].quoted);
        assert_eq!(segs[1].heredocs[0].body, "b\n");
        // バックスラッシュも引用符。
        assert!(split_nonempty("cat <<\\EOF\nx\nEOF")[0].heredocs[0].quoted);
    }

    #[test]
    fn data_heredoc_bodies_are_not_scanned_for_posts() {
        // 引用符付きでシェルに流れない本文は、何を書いてあってもデータ。
        assert!(
            posts("cat > f <<'EOF'\n`gh issue close 5 -R a/b`\nit's: gh pr create\nEOF").is_empty()
        );
        // `bash -c` があれば標準入力はコードではない。
        assert!(posts("bash -c true <<'EOF'\ngh issue close 5 -R a/b\nEOF").is_empty());
        // 単一引用符の中の置換、エスケープされた置換、コメントはデータ。
        assert!(posts("echo '$(gh issue close 5 -R a/b)'").is_empty());
        assert!(posts("echo \\$(gh issue close 5 -R a/b)").is_empty());
        assert!(posts("echo hi # gh issue close 5 -R a/b").is_empty());
    }

    #[test]
    fn code_heredoc_bodies_are_scanned_for_posts() {
        let want = vec!["a/b".to_string()];
        // 標準入力をシェルが読む形(直接、パイプ越し、`-s`、`. /dev/stdin`)。
        for cmd in [
            "bash <<'EOF'\ngh issue close 5 -R a/b\nEOF",
            "sh -s <<EOF\ngh issue close 5 -R a/b\nEOF",
            "env X=1 bash -o pipefail <<'EOF'\ngh issue close 5 -R a/b\nEOF",
            "cat <<'EOF' | bash\ngh issue close 5 -R a/b\nEOF",
            ". /dev/stdin <<'EOF'\ngh issue close 5 -R a/b\nEOF",
            // 引用符なしの区切りは、本文の置換がコード(本文の `'` は文字)。
            "cat <<EOF\nit's $(gh issue close 5 -R a/b)\nEOF",
            "cat <<EOF\n`gh issue close 5 -R a/b`\nEOF",
        ] {
            assert_eq!(close_repos(cmd), want, "{cmd}");
        }
    }

    #[test]
    fn heredoc_misreadings_never_hide_a_command() {
        // 本物のヒアドキュメントでないものを本文と取り違えると、続く行のコマンドを
        // データとして見落とす。曖昧なときはヒアドキュメントと見なさない。
        let want = vec!["a/b".to_string()];
        for cmd in [
            // 算術式の中の `<<`、here-string、コメントの中の `<<`、終端の行が無い
            "echo $((1<<2))\ngh issue close 5 -R a/b\n2))",
            "(( x = 1 << 2 ))\ngh issue close 5 -R a/b\n2",
            "cat <<<x\ngh issue close 5 -R a/b\nx",
            "echo hi # <<X\ngh issue close 5 -R a/b\nX",
            "cat <<EOF\ngh issue close 5 -R a/b",
            // 引用符の取り違え: `$'…\\''` は 1 語で、後ろの置換は展開される
            "echo $'\\'' $(gh issue close 5 -R a/b)",
            "echo 'a' $(gh issue close 5 -R a/b)",
            "echo \"it's\" $(gh issue close 5 -R a/b)",
        ] {
            assert_eq!(close_repos(cmd), want, "{cmd}");
        }
    }

    #[test]
    fn scan_subject_includes_code_bodies_only() {
        let r = analyze("cat > f <<'EOF'\nsecret-data-body\nEOF\ngh issue close 5 -R a/b");
        assert!(!r.scan_subject.contains("secret-data-body"));
        let r = analyze("bash <<'EOF'\nsecret-code-body\nEOF");
        assert!(r.scan_subject.contains("secret-code-body"));
        let r = analyze("cat > f <<EOF\nsecret-expanding-body\nEOF");
        assert!(r.scan_subject.contains("secret-expanding-body"));
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
