//! 本物の bash を正解にした、コード位置の認識の差分テスト
//! (docs/adr/0005-code-position-closure.md)。
//!
//! PATH の先頭に `gh` のスタブ(引数を記録して終了するだけ)を置き、無害な
//! コマンドと `gh` だけから成るコーパスを `bash -c` で実行して、実際に呼ばれた
//! `gh` の宛先(`-R`)の集合を取る。`bleep-hook intent` が出した投稿の宛先と比べる。
//!
//! - 健全性(全件): 実際に呼ばれた `gh` は、すべて認識される(偽陰性ゼロ)
//! - 精密性(`exact`): 認識された `gh` は、すべて実際に呼ばれる(偽陽性ゼロ)。
//!   `||` の短絡や終端の無いヒアドキュメントのように、実行されなくても認識して
//!   よい形(過剰側の近似)は `exact: false` にする
//! - 既知の限界(`GAPS`): 文法の外に残ると ADR に書いた不透明な実行。認識されない
//!   ことを固定する(境界が変わったら、ADR と一緒に直す)
//!
//! 宛先は `pub/…` の架空の名前だけを使う(CONTRIBUTING.md)。

use assert_cmd::Command;
use serde_json::Value;
use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::process::Stdio;

struct Case {
    cmd: &'static str,
    /// 認識された gh がすべて実行される(偽陽性ゼロ)ことまで確かめるか。
    exact: bool,
}

const fn case(cmd: &'static str, exact: bool) -> Case {
    Case { cmd, exact }
}

const CORPUS: &[Case] = &[
    // 透過的な構文
    case("gh issue close 5 -R pub/a", true),
    case(
        "echo hi && gh issue close 5 -R pub/a ; gh pr close 2 -R pub/b",
        true,
    ),
    case("true || gh issue close 5 -R pub/a", false),
    case("env X=1 gh issue close 5 -R pub/a", true),
    case("sh -c 'gh issue close 5 -R pub/a'", true),
    case(
        "sh -c \"gh issue close 5 -R pub/a; sh -c 'gh pr close 2 -R pub/b'\"",
        true,
    ),
    case("eval gh issue close 5 -R pub/a", true),
    case("echo $(gh issue close 5 -R pub/a)", true),
    case("x=`gh issue close 5 -R pub/a`", true),
    case("echo \"$(echo 'x')\" $(gh issue close 5 -R pub/a)", true),
    // ヒアドキュメント: 本文がコードになる形
    case("bash <<'EOF'\ngh issue close 5 -R pub/a\nEOF", true),
    case("cat <<'EOF' | bash\ngh issue close 5 -R pub/a\nEOF", true),
    case(
        "cat <<EOF >/dev/null\n$(gh issue close 5 -R pub/a)\nEOF",
        true,
    ),
    case(
        "cat <<EOF >/dev/null\nit's $(gh issue close 5 -R pub/a)\nEOF",
        true,
    ),
    case(
        "eval \"$(cat <<'EOF'\ngh issue close 5 -R pub/a\nEOF\n)\"",
        true,
    ),
    // ヒアドキュメント: 本文がデータの形(偽陽性にしない)
    case(
        "cat > f <<'EOF'\n`gh issue close 5 -R pub/a`\nit's: gh pr close 2 -R pub/b\nEOF",
        true,
    ),
    case("bash -c true <<'EOF'\ngh issue close 5 -R pub/a\nEOF", true),
    case(
        "cat <<'EOF' >/dev/null\n$(gh issue close 5 -R pub/a)\nEOF\ngh pr close 2 -R pub/b",
        true,
    ),
    // 引用符・コメントの中のデータ
    case("echo '$(gh issue close 5 -R pub/a)'", true),
    case("echo \"it's\" '$(gh pr close 2 -R pub/b)'", true),
    case("echo \\$(gh issue close 5 -R pub/a)", true),
    case("echo hi # gh issue close 5 -R pub/a", true),
    // ヒアドキュメントの取り違えを狙った形(続く行を本文と誤認しない)
    case("echo $((1<<2))\ngh issue close 5 -R pub/a\n2))", false),
    case("cat <<<x\ngh issue close 5 -R pub/a\nx", true),
    case("echo hi # <<X\ngh issue close 5 -R pub/a\nX", true),
    case("cat <<EOF\ngh issue close 5 -R pub/a", false),
    // 引用符の取り違えを狙った形
    case("echo $'\\'' $(gh issue close 5 -R pub/a)", true),
    case("echo $'a\\'b' $(gh issue close 5 -R pub/a)", true),
];

/// 文法の外に残る不透明な実行(ADR-0005)。実行はされるが、認識されない。
const GAPS: &[&str] = &[
    "f=gh; $f issue close 5 -R pub/gap",
    "echo 'gh issue close 5 -R pub/gap' | bash",
];

/// 認識された投稿の宛先の集合。宛先を取れない投稿(`unparsable` など)は、どの
/// 実行も覆う deny として `*` で表す(素通りではないので、健全性には足りる)。
fn recognized(cmd: &str) -> BTreeSet<String> {
    let out = Command::cargo_bin("bleep-hook")
        .unwrap()
        .env("HOME", "/home/test")
        .args(["intent", "--cwd", "/work", "--", cmd])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .map(|p| p["repo"].as_str().unwrap_or("*").to_string())
        .collect()
}

/// bash で実行し、スタブが記録した `gh` の宛先(`-R` の値)を返す。
fn executed(cmd: &str, dir: &std::path::Path) -> BTreeSet<String> {
    let stubs = dir.join("stubs");
    let log = dir.join("gh.log");
    let _ = std::fs::remove_file(&log);
    let status = std::process::Command::new("bash")
        .arg("-c")
        .arg(cmd)
        .current_dir(dir)
        .env("PATH", format!("{}:/usr/bin:/bin", stubs.to_string_lossy()))
        .env("GH_LOG", &log)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("bash を起動できる");
    let _ = status; // `2))` のような構文エラーの行は、実行の結果に関係しない
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    let mut set = BTreeSet::new();
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if let Some(i) = words.iter().position(|w| *w == "-R") {
            set.insert(words[i + 1].to_string());
        }
    }
    set
}

fn setup() -> Option<tempfile::TempDir> {
    if std::process::Command::new("bash")
        .arg("-c")
        .arg("true")
        .status()
        .map(|s| !s.success())
        .unwrap_or(true)
    {
        eprintln!("bash が無いので oracle テストを飛ばす");
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    let stubs = dir.path().join("stubs");
    std::fs::create_dir(&stubs).unwrap();
    let gh = stubs.join("gh");
    std::fs::write(&gh, "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$GH_LOG\"\n").unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    Some(dir)
}

#[test]
fn recognizer_agrees_with_what_bash_executes() {
    let Some(dir) = setup() else { return };
    for c in CORPUS {
        let ran = executed(c.cmd, dir.path());
        let seen = recognized(c.cmd);
        let missed: Vec<_> = if seen.contains("*") {
            Vec::new()
        } else {
            ran.difference(&seen).collect()
        };
        assert!(
            missed.is_empty(),
            "偽陰性(実行されたが認識されない) {missed:?}: {:?}",
            c.cmd
        );
        if c.exact {
            let extra: Vec<_> = seen.difference(&ran).collect();
            assert!(
                extra.is_empty(),
                "偽陽性(認識されたが実行されない) {extra:?}: {:?}",
                c.cmd
            );
        }
    }
}

/// ヒアドキュメントの開始に見える形(本物と、本物でないもの)の後ろに、gh の行と
/// 終端らしき行を組み合わせる。bash が実行した gh は、どの組み合わせでも認識される
/// (敵対的な入力が、続く行を本文に見せかけて隠せない)。
#[test]
fn heredoc_lookalikes_never_hide_an_executed_command() {
    const OPENERS: &[&str] = &[
        "cat <<EOF",
        "cat <<'EOF'",
        "cat <<\"EOF\"",
        "cat <<\\EOF",
        "cat <<-EOF",
        "cat <<EOF | cat",
        "cat <<'EOF' >/dev/null",
        "cat <<<EOF",
        "echo $((1<<2))",
        "echo $(( (1+2) << 3 ))",
        "(( 1<<2 ))",
        "let x=1<<2",
        "echo hi # <<EOF",
        "echo \"<<EOF\"",
        "echo '<<EOF'",
        "echo \\<<EOF",
        "echo $'<<\\'EOF'",
        "x=$(cat <<'EOF'",
        "x=`cat <<EOF`",
        "bash <<EOF",
        "bash -c true <<EOF",
    ];
    const TAILS: &[&str] = &["", "\nEOF", "\nX", "\n2", "\n2))", "\n)", "\n\tEOF", "\n'"];
    let Some(dir) = setup() else { return };
    for opener in OPENERS {
        for tail in TAILS {
            let cmd = format!("{opener}\ngh issue close 5 -R pub/a{tail}");
            let ran = executed(&cmd, dir.path());
            let seen = recognized(&cmd);
            assert!(
                seen.contains("*") || ran.is_subset(&seen),
                "偽陰性(実行されたが認識されない): {cmd:?}"
            );
        }
    }
}

#[test]
fn opaque_executions_stay_outside_the_grammar() {
    let Some(dir) = setup() else { return };
    for cmd in GAPS {
        assert!(
            !executed(cmd, dir.path()).is_empty(),
            "スタブが呼ばれない(テストの前提が崩れた): {cmd:?}"
        );
        assert!(
            recognized(cmd).is_empty(),
            "認識されるようになった。ADR-0005 の不透明な実行の範囲と一緒に直す: {cmd:?}"
        );
    }
}
