//! bleep 本体(Bash)の `split_command_segments` / `tokenize_segment` /
//! `classify_segment` / `resolve_effective_dir` の移植(D1)。純粋関数のみ —
//! I/O・gh・キャッシュは一切触らない。`match_verdict` / `build_patterns` /
//! `resolve_repo_nwo` は正本が Bash 実装のまま(判断を運ばない継ぎ目)。

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
/// resolve_effective_dir と scan_subject の cd 除外の両方で使う共通判定
/// (Bash 版では同じ正規表現が2箇所にコピーされている — ここでは共有する)。
fn as_single_cd_target(seg: &str) -> Option<&str> {
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

/// resolve_effective_dir の移植: `segments[..upto]` を先頭から歩き、厳密に
/// `cd <単一トークン>` と一致するセグメントだけを反映した実効ディレクトリを
/// 返す。絶対パスはそのまま、相対パスは直前の実効ディレクトリに連結する。
/// `~` 展開や変数展開はしない(静的解析の範囲外)。
pub fn resolve_effective_dir(start_dir: &str, segments: &[String], upto: usize) -> String {
    let mut dir = start_dir.to_string();
    for seg in segments.iter().take(upto) {
        if let Some(tgt) = as_single_cd_target(seg) {
            if tgt.starts_with('/') {
                dir = tgt.to_string();
            } else {
                dir = format!("{dir}/{tgt}");
            }
        }
    }
    dir
}

#[derive(Debug, Default)]
pub struct SegClassification {
    pub kind_is_push: bool,
    pub kind_is_gh_publish: bool,
    pub dir_override: Option<String>,
    pub repo_override: Option<String>,
}

/// tokenize_segment の代替。POSIX のクォート/エスケープ規則で単語分割する
/// (shell-words クレート、#22 の Bash 実装と同一の規則を検証済み)。
/// 閉じていないクォートは shell-words が Err を返す — Bash 版は不正な
/// 部分トークンのまま処理を続けるが、ここでは安全側に倒してトークン0件
/// (=classify_segment は Other のまま何も分類しない)として扱う。これは
/// Bash 版より緩く誤検出することはなく、fail-loud の格を落とさない。
fn tokenize_segment(seg: &str) -> Vec<String> {
    shell_words::split(seg).unwrap_or_default()
}

/// classify_segment の移植: セグメントの種別(push/gh_publish/other)と、
/// `git -C`/`gh --repo`/`-R` の明示的な override を判定する。global option
/// をトークン位置で読み飛ばしてからサブコマンド位置を確定するため、
/// `git -C <dir> push` や `gh --repo x pr create` も正しく分類できる。
/// 未知の global option(`-` 始まり)は安全側(読み飛ばし)に倒す。
pub fn classify_segment(seg: &str) -> SegClassification {
    let t = tokenize_segment(seg);
    let n = t.len();
    let mut result = SegClassification::default();
    if n == 0 {
        return result;
    }

    match t[0].as_str() {
        "git" => {
            let mut idx = 1usize;
            while idx < n {
                match t[idx].as_str() {
                    "-C" => {
                        result.dir_override = t.get(idx + 1).cloned();
                        idx += 2;
                        continue;
                    }
                    "-c" | "--git-dir" | "--work-tree" | "--namespace" => {
                        idx += 2; // 値を1個消費
                        continue;
                    }
                    s if s.starts_with("--git-dir=")
                        || s.starts_with("--work-tree=")
                        || s.starts_with("--namespace=")
                        || s.starts_with("-c=") =>
                    {
                        idx += 1;
                        continue;
                    }
                    "--no-pager"
                    | "--paginate"
                    | "-p"
                    | "--bare"
                    | "--literal-pathspecs"
                    | "--no-optional-locks" => {
                        idx += 1;
                        continue;
                    }
                    s if s.starts_with('-') => {
                        idx += 1; // 未知の global option。読み飛ばす(安全側)。
                        continue;
                    }
                    _ => break,
                }
            }
            if idx < n && t[idx] == "push" {
                result.kind_is_push = true;
            }
        }
        "gh" => {
            let mut idx = 1usize;
            while idx < n {
                match t[idx].as_str() {
                    "--repo" | "-R" => {
                        result.repo_override = t.get(idx + 1).cloned();
                        idx += 2;
                        continue;
                    }
                    s if s.starts_with("--repo=") => {
                        result.repo_override = Some(s.trim_start_matches("--repo=").to_string());
                        idx += 1;
                        continue;
                    }
                    "--hostname" => {
                        idx += 2;
                        continue;
                    }
                    s if s.starts_with('-') => {
                        idx += 1;
                        continue;
                    }
                    _ => break,
                }
            }
            if idx < n {
                let sub = t[idx].as_str();
                let action = t.get(idx + 1).map(|s| s.as_str()).unwrap_or("");
                if (sub == "pr" || sub == "issue")
                    && (action == "create" || action == "edit" || action == "comment")
                {
                    result.kind_is_gh_publish = true;
                } else if sub == "release" && (action == "create" || action == "edit") {
                    result.kind_is_gh_publish = true; // gh release create|edit(#8 由来)
                } else if sub == "repo" && action == "edit" {
                    result.kind_is_gh_publish = true; // gh repo edit --description 等
                } else if sub == "gist" && action == "create" {
                    result.kind_is_gh_publish = true; // gh gist create
                } else if sub == "api" {
                    // gh api は --repo を取らない。書き込みメソッドの有無だけ見る
                    // (Bash 版の for ループと同じ意味論 — スキップせず全トークンを
                    // 舐めて、最後に一致した値を採用する)。
                    let mut method = String::new();
                    let rest = &t[idx + 1..];
                    for j in 0..rest.len() {
                        match rest[j].as_str() {
                            "-X" | "--method" => {
                                if let Some(v) = rest.get(j + 1) {
                                    method = v.clone();
                                }
                            }
                            s if s.starts_with("--method=") => {
                                method = s.trim_start_matches("--method=").to_string();
                            }
                            _ => {}
                        }
                    }
                    if matches!(method.to_uppercase().as_str(), "POST" | "PUT" | "PATCH") {
                        result.kind_is_gh_publish = true;
                    }
                }
            }
        }
        _ => {}
    }
    result
}

/// cmd_scan_bash_command のうち、I/O を伴わない部分(セグメント分割・
/// scan_subject の構築・push/gh セグメントの検出と実効ディレクトリの解決)
/// をまとめた結果。
#[derive(Debug, Default)]
pub struct AnalyzeResult {
    pub scan_subject: String,
    pub found_push: bool,
    pub push_dir: String,
    pub found_gh: bool,
    pub gh_repo_override: String,
    /// found_gh かつ gh_repo_override が空のときだけ意味を持つ — 呼び出し側
    /// (Bash)がこのディレクトリを基準に resolve_repo_nwo(git remote 参照、
    /// I/O)を実行する。
    pub gh_effective_dir: String,
}

pub fn analyze(cmd: &str, start_dir: &str) -> AnalyzeResult {
    let segments = split_command_segments(cmd);

    let mut scan_subject = String::new();
    for seg in &segments {
        if as_single_cd_target(seg).is_some() {
            continue;
        }
        scan_subject.push_str(seg);
        scan_subject.push('\n');
    }

    let mut result = AnalyzeResult {
        scan_subject,
        ..Default::default()
    };

    for (i, seg) in segments.iter().enumerate() {
        let c = classify_segment(seg);
        if c.kind_is_push && !result.found_push {
            result.found_push = true;
            result.push_dir = c
                .dir_override
                .unwrap_or_else(|| resolve_effective_dir(start_dir, &segments, i));
        }
        if c.kind_is_gh_publish && !result.found_gh {
            result.found_gh = true;
            match c.repo_override {
                Some(r) => result.gh_repo_override = r,
                None => {
                    result.gh_effective_dir = resolve_effective_dir(start_dir, &segments, i);
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backslash_escaped_quote_does_not_close_segment_early() {
        // #22 の回帰1と同型: \" が && の区切りを壊さないこと。
        let segs = split_command_segments(r#"echo \" && gh pr create --title t"#);
        assert_eq!(segs.len(), 2);
        assert!(segs[1].trim_start().starts_with("gh pr create"));
    }

    #[test]
    fn repo_override_with_escaped_quote_extracts_correctly() {
        // #22 の回帰2と同型: --repo の値に \" があっても正しく抽出できる。
        let c = classify_segment(r#"gh --repo "x\"y" pr create --title t"#);
        assert!(c.kind_is_gh_publish);
        assert_eq!(c.repo_override.as_deref(), Some("x\"y"));
    }

    #[test]
    fn repo_after_subcommand_is_not_detected() {
        // #23 の既知の未対応(サブコマンド後の --repo は検出しない、
        // Bash 版と同じ挙動を保つことを固定 — 直す場合は #23 で対応する)。
        let c = classify_segment(r#"gh pr create --repo acme/other --title t"#);
        assert!(c.kind_is_gh_publish);
        assert_eq!(c.repo_override, None);
    }

    #[test]
    fn cd_segment_excluded_from_scan_subject() {
        let r = analyze(r#"cd /x/secret-repo-work && gh pr create --title t"#, ".");
        assert!(!r.scan_subject.contains("secret-repo-work"));
    }

    #[test]
    fn multiple_cd_accumulate_in_order() {
        let r = analyze(r#"cd /a && cd b && gh pr create --title t --body "x""#, ".");
        assert!(r.found_gh);
        assert_eq!(r.gh_effective_dir, "/a/b");
    }

    #[test]
    fn git_dash_c_overrides_cd_history() {
        let r = analyze(r#"cd /a && git -C /b push origin main"#, ".");
        assert!(r.found_push);
        assert_eq!(r.push_dir, "/b");
    }

    #[test]
    fn gh_api_write_method_is_publish() {
        let c = classify_segment(r#"gh api repos/acme/secret -X POST -f name=x"#);
        assert!(c.kind_is_gh_publish);
        let c = classify_segment(r#"gh api repos/acme/secret --method=GET"#);
        assert!(!c.kind_is_gh_publish);
    }

    #[test]
    fn gh_release_repo_edit_gist_are_publish() {
        assert!(classify_segment("gh release create v1 --notes x").kind_is_gh_publish);
        assert!(classify_segment("gh repo edit --description x").kind_is_gh_publish);
        assert!(classify_segment("gh gist create file.txt").kind_is_gh_publish);
        assert!(!classify_segment("gh repo view").kind_is_gh_publish);
    }

    #[test]
    fn unterminated_quote_segment_is_not_classified() {
        // split_command_segments はクォート閉じ忘れをコマンド全体1セグメント
        // にする。tokenize_segment(shell-words)は unterminated で Err を
        // 返すため、安全側で SegKind::Other 扱い(誤って push/gh_publish に
        // 分類しない)。
        let c = classify_segment(r#"git push "unterminated"#);
        assert!(!c.kind_is_push);
        assert!(!c.kind_is_gh_publish);
    }
}
