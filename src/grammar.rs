//! gh の「投稿」の正準形の認識器(docs/adr/0003-constructive-grammar.md)。
//!
//! 任意の bash から投稿の中身を復元するのをやめ、受理する形を小さな文法に
//! 刈り取る。ここは純粋関数で、I/O・gh は一切行わない。`gh` コマンドの
//! トークン列(先頭が `gh`)を受け取り、
//!
//! - 自由記述を公開面へ運ぶ既知のコマンド(表に載ったもの)でなければ `None`
//!   (読み取りや、本文を持たない操作。従来どおり検査しない)
//! - 表に載っていれば `GhPost`。正準形に入っていれば `noncanonical` が `None`、
//!   外れていれば最初の違反の閉じたコードを持つ
//!
//! を返す。正準形(`gh pr|issue|release|repo|gist|api` の投稿):
//!
//! - 宛先: `-R|--repo OWNER/REPO` がリテラルで必須(`gh pr|issue` は PR/Issue の
//!   URL、`gh repo edit` は位置引数の OWNER/REPO でも可。`gh gist` は不要)
//! - 本文: ファイルの絶対パスのリテラルだけ(`--body-file`・`--notes-file`・
//!   `-F`、gist は位置引数のファイル)。`--body`・`-b`・`--notes`・`-n`・
//!   `close|reopen` の `--comment`・stdin(`-`)は文法の外
//! - そのほかの値(タイトル・ラベルなど)はリテラルだけ — コマンド置換・変数
//!   展開を含まない
//! - `gh api` の書き込み: パスと値がリテラル。`repos/<owner>/<repo>/…` なら
//!   宛先はそのパス、プレースホルダ(`{owner}`)は cwd 依存なので文法の外
//!
//! 文法の外の形は、素通りでも ask でもなく deny にする(呼び出し側)。

/// 宛先をどこから取れたか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoSource {
    /// `-R` / `--repo`
    Flag,
    /// PR / Issue の URL(位置引数)
    Url,
    /// `gh repo edit` の位置引数
    Positional,
    /// `gh api` の API パス(`repos/<owner>/<repo>/…`)
    ApiPath,
    /// 取れなかった(`gh api graphql` など、宛先を静的に決められない形を含む)
    None,
}

impl RepoSource {
    pub fn as_str(self) -> &'static str {
        match self {
            RepoSource::Flag => "flag",
            RepoSource::Url => "url",
            RepoSource::Positional => "positional",
            RepoSource::ApiPath => "api-path",
            RepoSource::None => "none",
        }
    }
}

#[derive(Debug, Clone)]
pub struct GhPost {
    /// `pr` / `issue` / `release` / `repo` / `gist` / `api`
    pub surface: String,
    /// `create` / `edit` / `comment` / `review` / `close` / `merge` / `reopen`、
    /// `gh api` は小文字のメソッド
    pub action: String,
    pub repo_source: RepoSource,
    /// リテラルの `owner/repo`(取れたときだけ)
    pub repo: Option<String>,
    /// 本文を読む入力元の絶対パス(重複なし。stdin は含めない)
    pub body_sources: Vec<String>,
    /// 最初の違反の閉じたコード(`[a-z-]`)。正準形なら `None`。
    pub noncanonical: Option<&'static str>,
}

/// 違反コードの閉じた語彙。判定レッジャーの `detail` と、理由文の分岐
/// (bleep 本体)に使う。テストが、`recognize` の返すコードがここに収まる
/// ことを確かめる。
#[cfg(test)]
pub const NONCANONICAL_CODES: [&str; 8] = [
    "no-repo",
    "bad-repo",
    "inline-body",
    "inline-comment",
    "body-stdin",
    "body-path",
    "dynamic-value",
    "unparsable",
];

/// 値がコマンド置換・変数展開を含み、コマンド文字列だけでは内容が定まらないか。
/// shell-words で引用符を外した後の値で見るので、単一引用符の中のリテラルな
/// `$X` も同じに扱う(過剰側 — 書き直せば通る)。
pub fn is_dynamic(v: &str) -> bool {
    if v.contains('`') {
        return true;
    }
    let b = v.as_bytes();
    for i in 0..b.len() {
        if b[i] == b'$' {
            if let Some(&n) = b.get(i + 1) {
                if n == b'(' || n == b'{' || n == b'_' || n.is_ascii_alphanumeric() {
                    return true;
                }
                if matches!(n, b'@' | b'*' | b'?' | b'!' | b'#' | b'-' | b'$') {
                    return true;
                }
            }
        }
    }
    false
}

fn is_nwo(s: &str) -> bool {
    let ok = |p: &str| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    };
    match s.split_once('/') {
        Some((o, r)) => ok(o) && ok(r),
        None => false,
    }
}

/// `https://github.com/<owner>/<repo>/(pull|issues)/<n>` から owner/repo。
fn nwo_from_url(s: &str) -> Option<String> {
    let rest = s
        .strip_prefix("https://github.com/")
        .or_else(|| s.strip_prefix("http://github.com/"))?;
    let mut it = rest.split('/');
    let owner = it.next()?;
    let repo = it.next()?;
    let kind = it.next()?;
    if !matches!(kind, "pull" | "issues") {
        return None;
    }
    let nwo = format!("{owner}/{repo}");
    is_nwo(&nwo).then_some(nwo)
}

/// 本文の入力元として使えるパス(絶対・リテラル)か。
fn is_literal_abs_path(p: &str) -> bool {
    p.starts_with('/') && !is_dynamic(p) && !p.contains(['\\', '\n', '\0'])
}

/// 違反を 1 つだけ(最初のもの)覚える。
#[derive(Default)]
struct Violation(Option<&'static str>);

impl Violation {
    fn set(&mut self, code: &'static str) {
        if self.0.is_none() {
            self.0 = Some(code);
        }
    }
}

/// 本文の入力元ファイルの値を検査して `body_sources` に積む。
fn take_body_file(v: &str, sources: &mut Vec<String>, viol: &mut Violation) {
    if v == "-" {
        viol.set("body-stdin");
    } else if is_literal_abs_path(v) {
        if !sources.iter().any(|s| s == v) {
            sources.push(v.to_string());
        }
    } else {
        viol.set("body-path");
    }
}

/// `--flag value` / `--flag=value` / `-X value` / `-Xvalue` / `-X=value` の
/// どれかとして `tokens[i]` が `names`(長い名前と短い名前)のフラグなら、値と
/// 消費したトークン数を返す。短い名前は 1 文字(`-b`)。
fn flag_value<'a>(tokens: &'a [String], i: usize, names: &[&str]) -> Option<(&'a str, usize)> {
    let t = tokens[i].as_str();
    for n in names {
        if n.len() == 1 {
            let short = format!("-{n}");
            if t == short {
                return Some((tokens.get(i + 1).map(String::as_str).unwrap_or(""), 2));
            }
            if let Some(rest) = t.strip_prefix(&short) {
                if !rest.starts_with('-') && !rest.is_empty() {
                    return Some((rest.strip_prefix('=').unwrap_or(rest), 1));
                }
            }
        } else {
            let long = format!("--{n}");
            if t == long {
                return Some((tokens.get(i + 1).map(String::as_str).unwrap_or(""), 2));
            }
            if let Some(v) = t.strip_prefix(&format!("{long}=")) {
                return Some((v, 1));
            }
        }
    }
    None
}

/// `-R` / `--repo` を位置に依らず全トークンから探す(gh の persistent flag。
/// 複数回指定は最後を採用)。値があっても NWO として読めなければ Err。
fn find_repo_flag(tokens: &[String]) -> Option<&str> {
    let mut found: Option<&str> = None;
    let mut i = 1usize;
    while i < tokens.len() {
        if let Some((v, n)) = flag_value(tokens, i, &["R", "repo"]) {
            found = Some(v);
            i += n;
        } else {
            i += 1;
        }
    }
    found
}

/// gh のグローバルフラグを読み飛ばして、サブコマンドの位置を返す。
fn subcommand_index(t: &[String]) -> usize {
    let mut idx = 1usize;
    while idx < t.len() {
        match t[idx].as_str() {
            "--repo" | "-R" | "--hostname" => idx += 2,
            s if s.starts_with('-') => idx += 1,
            _ => break,
        }
    }
    idx
}

/// 値を 1 個取るフラグ(位置引数と取り違えないための表)。`gh repo edit` と
/// `gh gist create` の位置引数を拾うときだけ使う。
const REPO_EDIT_VALUE_FLAGS: [&str; 12] = [
    "-d",
    "--description",
    "-h",
    "--homepage",
    "--default-branch",
    "--add-topic",
    "--remove-topic",
    "--visibility",
    "--squash-merge-commit-message",
    "-R",
    "--repo",
    "--hostname",
];
const GIST_VALUE_FLAGS: [&str; 6] = ["-d", "--desc", "-f", "--filename", "-R", "--hostname"];

/// 位置引数(フラグでも、フラグの値でもないトークン)を集める。
fn positionals<'a>(args: &'a [String], value_flags: &[&str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut j = 0usize;
    while j < args.len() {
        let a = args[j].as_str();
        if value_flags.contains(&a) {
            j += 2;
        } else if a.starts_with('-') && a != "-" {
            j += 1;
        } else {
            out.push(a);
            j += 1;
        }
    }
    out
}

pub fn recognize(t: &[String]) -> Option<GhPost> {
    if t.first().map(String::as_str) != Some("gh") {
        return None;
    }
    let idx = subcommand_index(t);
    let sub = t.get(idx)?.as_str();
    let action = t.get(idx + 1).map(String::as_str).unwrap_or("");
    let known = match sub {
        "pr" => matches!(
            action,
            "create" | "edit" | "comment" | "review" | "close" | "merge" | "reopen"
        ),
        "issue" => matches!(action, "create" | "edit" | "comment" | "close" | "reopen"),
        "release" => matches!(action, "create" | "edit"),
        "repo" => action == "edit",
        "gist" => action == "create",
        "api" => true,
        _ => false,
    };
    if !known {
        return None;
    }
    if sub == "api" {
        return recognize_api(t, idx);
    }
    Some(recognize_cli(t, idx, sub, action))
}

fn recognize_cli(t: &[String], idx: usize, sub: &str, action: &str) -> GhPost {
    let args = &t[idx + 2..];
    let mut viol = Violation::default();
    let mut body_sources: Vec<String> = Vec::new();

    // ---- 宛先 ----
    let mut repo: Option<String> = None;
    let mut repo_source = RepoSource::None;
    match find_repo_flag(t) {
        Some(v) if is_nwo(v) && !is_dynamic(v) => {
            repo = Some(v.to_string());
            repo_source = RepoSource::Flag;
        }
        Some(_) => viol.set("bad-repo"),
        None => {
            if matches!(sub, "pr" | "issue") {
                if let Some(nwo) = positionals(args, &[]).iter().find_map(|p| nwo_from_url(p)) {
                    repo = Some(nwo);
                    repo_source = RepoSource::Url;
                }
            } else if sub == "repo" {
                if let Some(p) = positionals(args, &REPO_EDIT_VALUE_FLAGS)
                    .into_iter()
                    .find(|p| is_nwo(p))
                {
                    repo = Some(p.to_string());
                    repo_source = RepoSource::Positional;
                }
            }
            // gist は宛先を持たない(自分のアカウント)。
            if repo.is_none() && sub != "gist" {
                viol.set("no-repo");
            }
        }
    }

    // ---- 本文・自由記述 ----
    let body_flags_apply = matches!(sub, "pr" | "issue" | "release");
    let comment_flag_applies =
        matches!(sub, "pr" | "issue") && matches!(action, "close" | "reopen");
    let mut j = 0usize;
    while j < args.len() {
        if let Some((v, n)) = flag_value(args, j, &["body-file", "notes-file", "F"]) {
            if sub != "gist" && sub != "repo" {
                take_body_file(v, &mut body_sources, &mut viol);
                j += n;
                continue;
            }
        }
        if body_flags_apply {
            let names: &[&str] = if sub == "release" {
                &["notes", "n"]
            } else {
                &["body", "b"]
            };
            if flag_value(args, j, names).is_some() {
                viol.set("inline-body");
                j += flag_value(args, j, names).map(|(_, n)| n).unwrap_or(1);
                continue;
            }
        }
        if comment_flag_applies {
            if let Some((_, n)) = flag_value(args, j, &["comment", "c"]) {
                viol.set("inline-comment");
                j += n;
                continue;
            }
        }
        j += 1;
    }

    // ---- gist の位置引数のファイル ----
    if sub == "gist" {
        for p in positionals(args, &GIST_VALUE_FLAGS) {
            take_body_file(p, &mut body_sources, &mut viol);
        }
    }

    // ---- そのほかの値はリテラルだけ(本文ファイルのパスは上で検査済み) ----
    // 違反の優先順位は、宛先 → 本文 → そのほかの値(最初に立った 1 つを残す)。
    if t.iter().skip(1).any(|tok| is_dynamic(tok)) {
        viol.set("dynamic-value");
    }

    GhPost {
        surface: sub.to_string(),
        action: action.to_string(),
        repo_source,
        repo,
        body_sources,
        noncanonical: viol.0,
    }
}

/// `gh api` の API パス(最初の位置引数)から宛先を取り出す。
enum ApiDest {
    Repo(String),
    /// `{owner}`/`{repo}` プレースホルダ — gh が cwd の origin で展開する。
    Placeholder,
    /// `repos/` 以外・graphql など、宛先を静的に決められない。
    Unknown,
}

fn api_path_dest(path: &str) -> ApiDest {
    let p = path.trim_start_matches('/');
    let Some(rest) = p.strip_prefix("repos/") else {
        return ApiDest::Unknown;
    };
    let mut it = rest.split('/');
    let owner = it.next().unwrap_or("");
    let repo = it.next().and_then(|s| s.split('?').next()).unwrap_or("");
    if owner.is_empty() || repo.is_empty() {
        return ApiDest::Unknown;
    }
    if owner.contains('{') || repo.contains('{') {
        return ApiDest::Placeholder;
    }
    let nwo = format!("{owner}/{repo}");
    if is_nwo(&nwo) {
        ApiDest::Repo(nwo)
    } else {
        ApiDest::Unknown
    }
}

fn recognize_api(t: &[String], idx: usize) -> Option<GhPost> {
    let rest = &t[idx + 1..];
    let mut method = String::new();
    let mut has_field = false;
    let mut path: Option<&str> = None;
    let mut viol = Violation::default();
    let mut body_sources: Vec<String> = Vec::new();

    let mut j = 0usize;
    while j < rest.len() {
        let a = rest[j].as_str();
        if let Some((v, n)) = flag_value(rest, j, &["method", "X"]) {
            method = v.to_string();
            j += n;
        } else if let Some((v, n)) = flag_value(rest, j, &["input"]) {
            has_field = true;
            take_body_file(v, &mut body_sources, &mut viol);
            j += n;
        } else if let Some((v, n)) = flag_value(rest, j, &["field", "F"]) {
            has_field = true;
            if let Some((_, val)) = v.split_once('=') {
                if let Some(p) = val.strip_prefix('@') {
                    take_body_file(p, &mut body_sources, &mut viol);
                }
            }
            j += n;
        } else if let Some((_, n)) = flag_value(rest, j, &["raw-field", "f"]) {
            has_field = true;
            j += n;
        } else if matches!(
            a,
            "-H" | "--header"
                | "-q"
                | "--jq"
                | "-t"
                | "--template"
                | "--cache"
                | "-p"
                | "--preview"
                | "--hostname"
                | "-R"
                | "--repo"
        ) {
            j += 2;
        } else if a.starts_with('-') {
            j += 1;
        } else {
            if path.is_none() {
                path = Some(a);
            }
            j += 1;
        }
    }

    let effective = if method.is_empty() {
        if has_field {
            "POST"
        } else {
            "GET"
        }
    } else {
        method.as_str()
    };
    if !matches!(effective.to_uppercase().as_str(), "POST" | "PUT" | "PATCH") {
        return None;
    }

    let mut repo: Option<String> = None;
    let mut repo_source = RepoSource::None;
    match find_repo_flag(t) {
        Some(v) if is_nwo(v) && !is_dynamic(v) => {
            repo = Some(v.to_string());
            repo_source = RepoSource::Flag;
        }
        Some(_) => viol.set("bad-repo"),
        None => match path.map(api_path_dest) {
            Some(ApiDest::Repo(nwo)) => {
                repo = Some(nwo);
                repo_source = RepoSource::ApiPath;
            }
            Some(ApiDest::Placeholder) => viol.set("bad-repo"),
            Some(ApiDest::Unknown) | None => {}
        },
    }

    if t.iter().skip(1).any(|tok| is_dynamic(tok)) {
        viol.set("dynamic-value");
    }

    Some(GhPost {
        surface: "api".to_string(),
        action: effective.to_lowercase(),
        repo_source,
        repo,
        body_sources,
        noncanonical: viol.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(cmd: &str) -> Option<GhPost> {
        let t = shell_words::split(cmd).unwrap();
        recognize(&t)
    }

    fn code(cmd: &str) -> Option<&'static str> {
        rec(cmd).expect("post").noncanonical
    }

    #[test]
    fn canonical_forms_are_accepted() {
        for cmd in [
            "gh issue create -R pub/repo --title t --body-file /tmp/b.md",
            "gh pr create --repo pub/repo --base main --head topic --title 'a b' --body-file /tmp/b.md --draft",
            "gh pr create --repo=pub/repo --body-file=/tmp/b.md",
            "gh -R pub/repo pr comment 5 --body-file /tmp/b.md",
            "gh issue comment 5 -R pub/repo -F /tmp/b.md",
            "gh issue edit 5 -R pub/repo --add-label x --title t",
            "gh pr merge 5 -R pub/repo --squash --subject 'msg' --body-file /tmp/b.md",
            "gh pr review 5 -R pub/repo --approve --body-file /tmp/b.md",
            "gh pr close 5 -R pub/repo",
            "gh issue close 5 -R pub/repo --reason completed",
            "gh release create v1 -R pub/repo --notes-file /tmp/n.md --title v1",
            "gh repo edit pub/repo --description 'd'",
            "gh repo edit -R pub/repo --homepage https://example.invalid",
            "gh gist create /tmp/a.txt --desc d",
            "gh pr comment https://github.com/pub/repo/pull/5 --body-file /tmp/b.md",
            "gh api -X POST repos/pub/repo/issues -f title=t -F body=@/tmp/b.md",
            "gh api repos/pub/repo/issues --input /tmp/in.json",
        ] {
            assert_eq!(code(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn non_posts_are_not_recognized() {
        for cmd in [
            "gh pr view 5 -R pub/repo",
            "gh pr list",
            "gh pr checks 5",
            "gh issue view 5",
            "gh auth status",
            "gh run watch 123",
            "gh pr ready 5",
            "gh api repos/pub/repo/issues",
            "gh api -X GET repos/pub/repo/issues -f q=x",
            "gh repo view pub/repo",
            "git push",
        ] {
            assert!(rec(cmd).is_none(), "{cmd}");
        }
    }

    #[test]
    fn violations_have_closed_codes() {
        for (cmd, want) in [
            ("gh issue create --title t --body-file /tmp/b.md", "no-repo"),
            ("gh issue create -R pub --body-file /tmp/b.md", "bad-repo"),
            ("gh issue create -R $X --body-file /tmp/b.md", "bad-repo"),
            ("gh issue create -R pub/repo --body hello", "inline-body"),
            ("gh issue create -R pub/repo -b hello", "inline-body"),
            ("gh issue create -R pub/repo --body=hello", "inline-body"),
            ("gh issue create -R pub/repo -bhello", "inline-body"),
            ("gh release create v1 -R pub/repo --notes hi", "inline-body"),
            (
                "gh issue close 5 -R pub/repo --comment hi",
                "inline-comment",
            ),
            ("gh pr close 5 -R pub/repo -c hi", "inline-comment"),
            ("gh pr reopen 5 -R pub/repo --comment=hi", "inline-comment"),
            ("gh issue create -R pub/repo --body-file -", "body-stdin"),
            ("gh issue create -R pub/repo --body-file b.md", "body-path"),
            (
                "gh issue create -R pub/repo --body-file ~/b.md",
                "body-path",
            ),
            (
                "gh issue create -R pub/repo --body-file /tmp/$X.md",
                "body-path",
            ),
            (
                "gh issue create -R pub/repo --title \"$(date)\" --body-file /tmp/b.md",
                "dynamic-value",
            ),
            (
                "gh issue create -R pub/repo --label `x` --body-file /tmp/b.md",
                "dynamic-value",
            ),
            ("gh gist create a.txt", "body-path"),
            (
                "gh api -X POST repos/{owner}/{repo}/issues -f a=b",
                "bad-repo",
            ),
            (
                "gh api -X POST repos/pub/repo/issues -f body=\"$(cat x)\"",
                "dynamic-value",
            ),
            (
                "gh api -X POST repos/pub/repo/issues -F body=@b.md",
                "body-path",
            ),
            ("gh api repos/pub/repo/issues --input -", "body-stdin"),
        ] {
            assert_eq!(code(cmd), Some(want), "{cmd}");
            assert!(NONCANONICAL_CODES.contains(&want));
        }
    }

    #[test]
    fn first_violation_wins_in_a_stable_order() {
        // 宛先の違反が本文の違反より先(宛先 → 本文 → そのほかの値)。
        assert_eq!(code("gh issue create --body hi"), Some("no-repo"));
        assert_eq!(
            code("gh issue create -R pub/repo --body hi --title \"$X\""),
            Some("inline-body")
        );
    }

    #[test]
    fn destination_sources() {
        let p = rec("gh issue create -R pub/repo --body-file /tmp/b.md").unwrap();
        assert_eq!(
            (p.repo.as_deref(), p.repo_source),
            (Some("pub/repo"), RepoSource::Flag)
        );
        let p =
            rec("gh pr comment https://github.com/pub/repo/pull/5 --body-file /tmp/b.md").unwrap();
        assert_eq!(
            (p.repo.as_deref(), p.repo_source),
            (Some("pub/repo"), RepoSource::Url)
        );
        let p = rec("gh repo edit pub/repo --description d").unwrap();
        assert_eq!(p.repo_source, RepoSource::Positional);
        let p = rec("gh api -X POST repos/pub/repo/issues -f a=b").unwrap();
        assert_eq!(
            (p.repo.as_deref(), p.repo_source),
            (Some("pub/repo"), RepoSource::ApiPath)
        );
        // 宛先を静的に決められない api(graphql、repos/ 以外)は違反ではなく、
        // 宛先なしのまま公開宛てとして検査される。
        for cmd in [
            "gh api graphql -f query=x",
            "gh api -X POST user/repos -f name=x",
        ] {
            let p = rec(cmd).unwrap();
            assert_eq!(
                (p.repo, p.repo_source, p.noncanonical),
                (None, RepoSource::None, None),
                "{cmd}"
            );
        }
        // -R/--repo は位置に依らず、最後を採用する。
        let p = rec("gh issue create --body-file /tmp/b.md -R a/b -R c/d").unwrap();
        assert_eq!(p.repo.as_deref(), Some("c/d"));
        // gist は宛先を持たない。
        let p = rec("gh gist create /tmp/a.txt").unwrap();
        assert_eq!((p.repo, p.noncanonical), (None, None));
    }

    #[test]
    fn body_sources_are_collected_without_duplicates() {
        let p = rec("gh issue create -R a/b --body-file /tmp/x.md -F /tmp/x.md").unwrap();
        assert_eq!(p.body_sources, ["/tmp/x.md"]);
        let p = rec("gh gist create /tmp/a.txt /tmp/b.txt -d desc").unwrap();
        assert_eq!(p.body_sources, ["/tmp/a.txt", "/tmp/b.txt"]);
        let p = rec("gh api -X PATCH repos/a/b/issues/1 -F body=@/tmp/b.md -F n=1").unwrap();
        assert_eq!(p.body_sources, ["/tmp/b.md"]);
        // api の -f は文字通り(@ でもファイルを読まない)。
        let p = rec("gh api -X POST repos/a/b/issues -f body=@/tmp/b.md").unwrap();
        assert!(p.body_sources.is_empty());
    }

    #[test]
    fn api_implicit_post_and_methods() {
        assert!(rec("gh api repos/a/b/issues -f body=x").is_some());
        assert!(rec("gh api repos/a/b/issues --input /tmp/f.json").is_some());
        assert!(rec("gh api -X PUT repos/a/b/contents/f -f m=x").is_some());
        assert!(rec("gh api -X DELETE repos/a/b/issues/1").is_none());
        assert!(rec("gh api --method=GET repos/a/b").is_none());
        // フラグの値(-H の値)を位置引数と取り違えない。
        let p = rec("gh api -H 'Accept: x' -X POST repos/pub/repo/issues -f a=b").unwrap();
        assert_eq!(p.repo.as_deref(), Some("pub/repo"));
    }

    #[test]
    fn dynamic_detection() {
        for v in ["$X", "${X}", "$(x)", "`x`", "a$1", "$@", "$$"] {
            assert!(is_dynamic(v), "{v}");
        }
        for v in ["cost $", "$ 5", "plain", "a.b/c", "#12", "(a)"] {
            assert!(!is_dynamic(v), "{v}");
        }
    }
}
