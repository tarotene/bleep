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

/// `~`/`~/…`/`$HOME`/`$HOME/…`/`${HOME}`/`${HOME}/…` の先頭一致だけを、呼び
/// 出し元(main.rs)が読んだ hook プロセス自身の `HOME` に展開する
/// (#39)。閉じた集合以外(`~user`、`$HOMEBREW/…`、
/// `${HOME_X}` 等)は展開しない — 変数一般の静的解決は依然しない設計を維持
/// する(#34 とは別問題のまま切り分ける)。`home` が `None`(HOME 未設定)
/// なら常に未加工で返す。
fn expand_home(tok: &str, home: Option<&str>) -> String {
    let Some(home) = home else {
        return tok.to_string();
    };
    for prefix in ["~", "$HOME", "${HOME}"] {
        if tok == prefix {
            return home.to_string();
        }
        if let Some(rest) = tok.strip_prefix(prefix) {
            if rest.starts_with('/') {
                return format!("{home}{rest}");
            }
        }
    }
    tok.to_string()
}

/// resolve_effective_dir の移植: `segments[..upto]` を先頭から歩き、厳密に
/// `cd <単一トークン>` と一致するセグメントだけを反映した実効ディレクトリを
/// 返す。絶対パスはそのまま、相対パスは直前の実効ディレクトリに連結する。
/// ターゲットには `expand_home` を通す(#39) — それ以外の変数展開はしない
/// (静的解析の範囲外)。
pub fn resolve_effective_dir(
    start_dir: &str,
    segments: &[String],
    upto: usize,
    home: Option<&str>,
) -> String {
    let mut dir = start_dir.to_string();
    for seg in segments.iter().take(upto) {
        if let Some(tgt) = as_single_cd_target(seg) {
            let tgt = expand_home(tgt, home);
            if tgt.starts_with('/') {
                dir = tgt;
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
            // サブコマンドの位置を確定するためだけの読み飛ばしループ。
            // --repo/-R/--hostname の値も消費するが、repo_override の捕捉は
            // ここでは行わない(#23 — 以前はこのループの中でしか --repo を
            // 見ておらず、サブコマンドで break した後ろにある --repo を
            // 一切検出できなかった)。
            let mut idx = 1usize;
            while idx < n {
                match t[idx].as_str() {
                    "--repo" | "-R" | "--hostname" => {
                        idx += 2;
                        continue;
                    }
                    s if s.starts_with("--repo=") => {
                        idx += 1;
                        continue;
                    }
                    s if s.starts_with('-') => {
                        idx += 1;
                        continue;
                    }
                    _ => break,
                }
            }

            // --repo/-R/--repo= は gh の persistent flag(gh CLI manual
            // "Options inherited from parent commands")で、サブコマンドの
            // 前後どちらに置いても解釈される。位置に依存せず全トークンを
            // 走査して検出する(#23、#43)。複数回指定された場合は最後の
            // 一致を採用する(gh api の書き込みメソッド判定と同じ、舐めて
            // 最後を採用する意味論)。
            let mut j = 1usize;
            while j < n {
                match t[j].as_str() {
                    "--repo" | "-R" => {
                        result.repo_override = t.get(j + 1).cloned();
                        j += 2;
                        continue;
                    }
                    s if s.starts_with("--repo=") => {
                        result.repo_override = Some(s.trim_start_matches("--repo=").to_string());
                        j += 1;
                        continue;
                    }
                    _ => {
                        j += 1;
                    }
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
    /// found_push/found_gh の対象を実際に解決するのに使った値(cd 追跡、
    /// `-C`/`--repo`/`-R` の override)が、未展開の `$` 参照(閉集合外の
    /// 変数)を含むために静的に解決できなかったかどうか(#34)。呼び出し側
    /// はこの場合、値を無理に使わず ask にエスカレートする。found_push も
    /// found_gh も立っていない(push/gh セグメントが無い)ときは常に false。
    pub unresolved_var: bool,
}

/// gh セグメントの生テキストから `--repo`/`-R`(値を伴う2トークン形)と
/// `--repo=値`(1トークン形)を取り除き、残りのトークンを
/// `shell_words::join`(`split` の逆演算、必要最小限の再クォート)で
/// 再結合したテキストを返す(#43、D2)。宛先そのものを指すトークンは
/// 「漏洩ではない」という #9(cd セグメント全体除外)と同じ考え方を、
/// セグメント単位からトークン単位に広げたもの。複数回指定されていても
/// 全て取り除く(scan_subject からは常に除外してよい — どの occurrence が
/// 実際に採用されるかに関わらず、どれも宛先参照であることに変わりないため)。
fn strip_repo_override_tokens(seg: &str) -> String {
    let t = tokenize_segment(seg);
    let mut kept: Vec<String> = Vec::with_capacity(t.len());
    let mut j = 0usize;
    while j < t.len() {
        match t[j].as_str() {
            "--repo" | "-R" => {
                j += 2; // フラグ+値の2トークンを両方取り除く
                continue;
            }
            s if s.starts_with("--repo=") => {
                j += 1;
                continue;
            }
            _ => {
                kept.push(t[j].clone());
                j += 1;
            }
        }
    }
    shell_words::join(kept)
}

/// `segments[..upto]` の中で、`cd <単一トークン>` として実際に消費される
/// (= resolve_effective_dir が読む)ターゲットのいずれかが、`expand_home`
/// 適用後も未展開の `$` 参照(`~`/`$HOME`/`${HOME}` の閉集合外)を含むかを
/// 返す(#34)。resolve_effective_dir 自身の絶対/相対分岐を変えず、同じ
/// 対象を辿って `$` の有無だけを見る副関数として持つ(判断を運ばない継ぎ目
/// — 実際の実効ディレクトリの計算は resolve_effective_dir の正本のまま)。
fn cd_chain_has_unresolved_var(segments: &[String], upto: usize, home: Option<&str>) -> bool {
    segments.iter().take(upto).any(|seg| {
        as_single_cd_target(seg)
            .map(|tgt| expand_home(tgt, home).contains('$'))
            .unwrap_or(false)
    })
}

pub fn analyze(cmd: &str, start_dir: &str, home: Option<&str>) -> AnalyzeResult {
    let segments = split_command_segments(cmd);
    let mut result = AnalyzeResult::default();
    // found_push/found_gh の対象を実際に解決するのに使った値が未展開の `$`
    // 参照を含むかどうか(#34)。gh > push の優先順位(下記ループ後の
    // 確定処理)は cmd_scan_bash_command(Bash 本体)の分岐順序と揃える。
    let mut push_unresolved_var = false;
    let mut gh_unresolved_var = false;

    for (i, seg) in segments.iter().enumerate() {
        let c = classify_segment(seg);

        if as_single_cd_target(seg).is_none() {
            if c.kind_is_gh_publish && c.repo_override.is_some() {
                result
                    .scan_subject
                    .push_str(&strip_repo_override_tokens(seg));
            } else {
                result.scan_subject.push_str(seg);
            }
            result.scan_subject.push('\n');
        }

        if c.kind_is_push && !result.found_push {
            result.found_push = true;
            result.push_dir = match c.dir_override {
                // git -C <dir> の <dir> にも同じ展開を適用する(#39) —
                // resolve_effective_dir を経由しない唯一の経路なので、
                // ここで expand_home を直接通す。展開後も絶対パスでなく
                // (先頭 `/` 無し)、かつ未展開の `$` 参照(`$HOMEBREW/x` の
                // ような閉集合外の変数)を含まないときだけ、cd 追跡結果に
                // 連結する(#41 — 相対パスは直前の実効ディレクトリからの
                // 相対、という git -C 自体の意味論に合わせる)。`$` が残る
                // ケースは静的に解決できないため、以前と同じく無加工で返す
                // (#34 の対象— 検出して ask にエスカレートするかどうかは
                // 別の継ぎ目で判断する)。
                Some(d) => {
                    let expanded = expand_home(&d, home);
                    if expanded.contains('$') {
                        push_unresolved_var = true;
                        expanded
                    } else if expanded.starts_with('/') {
                        expanded
                    } else {
                        format!(
                            "{}/{}",
                            resolve_effective_dir(start_dir, &segments, i, home),
                            expanded
                        )
                    }
                }
                None => {
                    push_unresolved_var = cd_chain_has_unresolved_var(&segments, i, home);
                    resolve_effective_dir(start_dir, &segments, i, home)
                }
            };
        }
        if c.kind_is_gh_publish && !result.found_gh {
            result.found_gh = true;
            match c.repo_override {
                Some(r) => {
                    // gh の --repo/-R の値自体は cd/-C と異なりパスではない
                    // ため expand_home は適用しない。未展開の `$` が残って
                    // いれば、その値をそのまま静的な解決不能として扱う。
                    gh_unresolved_var = r.contains('$');
                    result.gh_repo_override = r;
                }
                None => {
                    gh_unresolved_var = cd_chain_has_unresolved_var(&segments, i, home);
                    result.gh_effective_dir = resolve_effective_dir(start_dir, &segments, i, home);
                }
            }
        }
    }

    // gh > push の優先順位(cmd_scan_bash_command は found_gh を先に見て、
    // 見つかれば push 側を一切参照しない)に合わせて、実際に使われる方の
    // 未解決フラグだけを採用する。
    result.unresolved_var = if result.found_gh {
        gh_unresolved_var
    } else if result.found_push {
        push_unresolved_var
    } else {
        false
    };

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
    fn repo_after_subcommand_is_detected() {
        // #23: --repo/-R は gh の persistent flag で位置に依存しない。
        // 以前はサブコマンド確定ループで break した後ろを一切見ておらず、
        // ここは None(未対応)を固定するテストだった。
        let c = classify_segment(r#"gh pr create --repo acme/other --title t"#);
        assert!(c.kind_is_gh_publish);
        assert_eq!(c.repo_override.as_deref(), Some("acme/other"));
    }

    #[test]
    fn dash_r_after_subcommand_is_detected() {
        // #23: -R(短縮形)もサブコマンド後で検出できる。
        let c = classify_segment(r#"gh issue create -R acme/other --title t"#);
        assert!(c.kind_is_gh_publish);
        assert_eq!(c.repo_override.as_deref(), Some("acme/other"));
    }

    #[test]
    fn repo_equals_after_subcommand_is_detected() {
        // #23: --repo=値 形もサブコマンド後で検出できる。
        let c = classify_segment(r#"gh pr create --repo=acme/other --title t"#);
        assert!(c.kind_is_gh_publish);
        assert_eq!(c.repo_override.as_deref(), Some("acme/other"));
    }

    #[test]
    fn repo_override_token_excluded_from_scan_subject() {
        // #43(D2): 明示された --repo の宛先トークン自体は scan_subject から
        // 除外される(#9 の cd 除外と同じ考え方)。body の内容は残る。
        let r = analyze(
            r#"gh pr create --repo acme/public-oss --title t --body "unrelated""#,
            ".",
            None,
        );
        assert!(r.found_gh);
        assert!(!r.scan_subject.contains("acme/public-oss"));
        assert!(r.scan_subject.contains("unrelated"));
    }

    #[test]
    fn cd_segment_excluded_from_scan_subject() {
        let r = analyze(
            r#"cd /x/secret-repo-work && gh pr create --title t"#,
            ".",
            None,
        );
        assert!(!r.scan_subject.contains("secret-repo-work"));
    }

    #[test]
    fn multiple_cd_accumulate_in_order() {
        let r = analyze(
            r#"cd /a && cd b && gh pr create --title t --body "x""#,
            ".",
            None,
        );
        assert!(r.found_gh);
        assert_eq!(r.gh_effective_dir, "/a/b");
    }

    #[test]
    fn git_dash_c_overrides_cd_history() {
        let r = analyze(r#"cd /a && git -C /b push origin main"#, ".", None);
        assert!(r.found_push);
        assert_eq!(r.push_dir, "/b");
    }

    #[test]
    fn git_dash_c_relative_path_joins_cd_history() {
        // #41: 相対 -C は resolve_effective_dir と同じ規則(直前の実効
        // ディレクトリに連結)で解決する。以前は cd 履歴を無視して
        // 素通しの "b" になっていた。
        let r = analyze(r#"cd /a && git -C b push origin main"#, ".", None);
        assert!(r.found_push);
        assert_eq!(r.push_dir, "/a/b");
    }

    #[test]
    fn git_dash_c_relative_path_without_cd_joins_start_dir() {
        // #41: cd が無い場合も、相対 -C は呼び出しプロセスの --cwd(ここでは
        // start_dir)からの相対として解決する(以前は "b" のまま素通しだった)。
        let r = analyze(r#"git -C b push origin main"#, "/x", None);
        assert!(r.found_push);
        assert_eq!(r.push_dir, "/x/b");
    }

    // ---- HOME 展開(#39) ----------------------------------------------

    #[test]
    fn cd_tilde_slash_expands_to_home() {
        let r = analyze("cd ~/repo && git push origin main", ".", Some("/home/x"));
        assert!(r.found_push);
        assert_eq!(r.push_dir, "/home/x/repo");
    }

    #[test]
    fn cd_bare_tilde_expands_to_home() {
        let r = analyze("cd ~ && git push origin main", ".", Some("/home/x"));
        assert!(r.found_push);
        assert_eq!(r.push_dir, "/home/x");
    }

    #[test]
    fn cd_dollar_home_expands() {
        let r = analyze(
            "cd $HOME/repo && git push origin main",
            ".",
            Some("/home/x"),
        );
        assert!(r.found_push);
        assert_eq!(r.push_dir, "/home/x/repo");
    }

    #[test]
    fn git_dash_c_tilde_expands() {
        let r = analyze("git -C ~/repo push origin main", ".", Some("/home/x"));
        assert!(r.found_push);
        assert_eq!(r.push_dir, "/home/x/repo");
    }

    #[test]
    fn git_dash_c_quoted_dollar_home_brace_expands() {
        // -C の値は shell_words 経由(classify_segment)なのでクォートが
        // 外れた後の "${HOME}/repo" に対して expand_home が働く。
        let r = analyze(
            r#"git -C "${HOME}/repo" push origin main"#,
            ".",
            Some("/home/x"),
        );
        assert!(r.found_push);
        assert_eq!(r.push_dir, "/home/x/repo");
    }

    #[test]
    fn tilde_user_form_is_not_expanded() {
        // ~user は passwd 引きが要る形で、閉集合の対象外(還元性、#39 とは
        // 切り分ける)。展開されず素通しのまま resolve_effective_dir に渡る。
        let r = analyze("cd ~someone && git push origin main", ".", Some("/home/x"));
        assert!(r.found_push);
        assert_eq!(r.push_dir, "./~someone");
    }

    #[test]
    fn dollar_home_prefixed_other_var_is_not_expanded() {
        // $HOMEBREW は $HOME の前方一致だけでは弾けない罠 — 次の文字が '/'
        // でも文字列末でもないので展開しない。
        let r = analyze("git -C $HOMEBREW/x push origin main", ".", Some("/home/x"));
        assert!(r.found_push);
        assert_eq!(r.push_dir, "$HOMEBREW/x");
    }

    #[test]
    fn braced_home_with_suffix_is_not_expanded() {
        let r = analyze(
            "cd ${HOME_X}/y && git push origin main",
            ".",
            Some("/home/x"),
        );
        assert!(r.found_push);
        assert_eq!(r.push_dir, "./${HOME_X}/y");
    }

    #[test]
    fn home_none_leaves_tilde_unexpanded() {
        let r = analyze("cd ~/repo && git push origin main", ".", None);
        assert!(r.found_push);
        assert_eq!(r.push_dir, "./~/repo");
    }

    // ---- 未展開のシェル変数参照の検出(#34) -----------------------------

    #[test]
    fn cd_with_shell_variable_sets_unresolved_var_for_gh() {
        // --repo が無く、gh_effective_dir の解決が cd 履歴だけに頼る場合、
        // cd の対象が未展開の $ を含めば unresolved_var が立つ。
        let r = analyze(r#"cd "$D" && gh pr create --title t"#, ".", None);
        assert!(r.found_gh);
        assert!(r.unresolved_var);
    }

    #[test]
    fn cd_with_shell_variable_sets_unresolved_var_for_push() {
        let r = analyze(r#"cd "$D" && git push origin main"#, ".", None);
        assert!(r.found_push);
        assert!(r.unresolved_var);
    }

    #[test]
    fn git_dash_c_with_shell_variable_sets_unresolved_var() {
        let r = analyze(r#"git -C "$D" push origin main"#, ".", None);
        assert!(r.found_push);
        assert!(r.unresolved_var);
    }

    #[test]
    fn gh_repo_with_shell_variable_sets_unresolved_var() {
        let r = analyze(r#"gh --repo "$X" pr create --title t"#, ".", None);
        assert!(r.found_gh);
        assert!(r.unresolved_var);
    }

    #[test]
    fn literal_only_commands_do_not_set_unresolved_var() {
        let r = analyze(r#"cd /a && git -C b push origin main"#, ".", None);
        assert!(r.found_push);
        assert!(!r.unresolved_var);

        let r = analyze(r#"gh pr create --repo acme/other --title t"#, ".", None);
        assert!(r.found_gh);
        assert!(!r.unresolved_var);
    }

    #[test]
    fn explicit_repo_override_wins_over_unresolved_cd_for_gh() {
        // #23 の修正後、--repo が明示されていれば gh_effective_dir(cd
        // 追跡)は使われない。cd 側に未展開の $ が残っていても、実際に
        // 使われるのは --repo のリテラル値なので unresolved_var は立たない
        // (#34 の対象は「実際の解決に使った値」に限る)。
        let r = analyze(
            r#"cd "$D" && gh pr create --repo acme/other --title t"#,
            ".",
            None,
        );
        assert!(r.found_gh);
        assert_eq!(r.gh_repo_override, "acme/other");
        assert!(!r.unresolved_var);
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
