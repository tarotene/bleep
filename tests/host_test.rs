//! host モード(--host=claude|codex|copilot)の統合テスト。3つの Bash
//! adapter の selftest が使っていたのと同じ tests/fixtures/ を読む(D4 —
//! Bash と Rust が同一 fixture を共有する)。

use assert_cmd::Command;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixtures_dir() -> PathBuf {
    repo_root().join("tests/fixtures")
}

struct Env {
    tmp: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("config")).unwrap();
        std::fs::create_dir_all(tmp.path().join("state")).unwrap();
        std::fs::write(tmp.path().join("config/orgs.txt"), "acme\n").unwrap();
        std::fs::write(tmp.path().join("config/repos.txt"), "acme/secret-project\n").unwrap();
        let gh_stub = tmp.path().join("bin-gh");
        std::fs::copy(fixtures_dir().join("gh-stub.sh"), &gh_stub).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&gh_stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        Env { tmp }
    }

    fn pg_command(&self) -> Command {
        let mut cmd = Command::cargo_bin("bleep-hook").unwrap();
        cmd.env("BLEEP_BIN", repo_root().join("bleep"))
            // bleep(bash)の cmd_scan_bash_command 自身も字句解析の
            // ために bleep-hook を呼ぶ(D1 — 継ぎ目は2箇所ある)。
            // CARGO_BIN_EXE_<name> はテスト対象と同じビルド済みバイナリを
            // 指す cargo 提供の compile-time env var。
            .env("BLEEP_LEX_BIN", env!("CARGO_BIN_EXE_bleep-hook"))
            .env("BLEEP_CONFIG_DIR", self.tmp.path().join("config"))
            .env("BLEEP_STATE_DIR", self.tmp.path().join("state"))
            .env("BLEEP_ORGS_FILE", self.tmp.path().join("config/orgs.txt"))
            .env("BLEEP_REPOS_FILE", self.tmp.path().join("config/repos.txt"))
            .env("BLEEP_OWNER", "test-owner")
            .env("BLEEP_GH_BIN", self.tmp.path().join("bin-gh"))
            // 判定レッジャーをテスト用 tmp に隔離する — これが無いと
            // テスト実行のたびに実マシンの ~/.local/state/agent-verdicts/
            // を汚してしまう。
            .env("BLEEP_LEDGER_DIR", self.ledger_dir());
        cmd
    }

    fn ledger_dir(&self) -> PathBuf {
        self.tmp.path().join("agent-verdicts")
    }

    /// tests/fixtures/push-cwd-repo.sh の build_cwd_push_repo を bash 経由で
    /// 呼び、--cwd 検証用リポジトリを作る(D4 — セットアップ手順は単一正本)。
    fn build_cwd_push_repo(&self) -> String {
        let script = fixtures_dir().join("push-cwd-repo.sh");
        let out = std::process::Command::new("bash")
            .arg("-c")
            .arg(format!(
                "source {:?} && build_cwd_push_repo {:?} && printf '%s' \"$CWD_PUSH_REPO\"",
                script,
                self.tmp.path()
            ))
            .output()
            .expect("failed to spawn bash for push-cwd-repo.sh");
        assert!(
            out.status.success(),
            "build_cwd_push_repo failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }
}

fn read_fixture(rel: &str) -> Vec<u8> {
    std::fs::read(fixtures_dir().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn decision_wrapped(stdout: &[u8]) -> Option<String> {
    if stdout.is_empty() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(stdout).expect("valid JSON output");
    v["hookSpecificOutput"]["permissionDecision"]
        .as_str()
        .map(str::to_string)
}

fn decision_flat(stdout: &[u8]) -> Option<String> {
    if stdout.is_empty() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(stdout).expect("valid JSON output");
    v["permissionDecision"].as_str().map(str::to_string)
}

#[test]
fn claude_and_codex_fixture_corpus() {
    let env = Env::new();
    let cases: &[(&str, Option<&str>)] = &[
        ("claude-codex/deny_bash.json", Some("deny")),
        ("claude-codex/ask_bash.json", Some("ask")),
        ("claude-codex/pass_bash.json", None),
        ("claude-codex/deny_mcp.json", Some("deny")),
        ("claude-codex/deny_mcp_newline.json", Some("deny")), // tostring 回帰の固定
        ("claude-codex/pass_mcp.json", None),
        ("claude-codex/no_tool_name.json", None),
    ];
    for host in ["claude", "codex"] {
        for (fixture, expected) in cases {
            let input = read_fixture(fixture);
            let assert = env
                .pg_command()
                .arg(format!("--host={host}"))
                .write_stdin(input)
                .assert()
                .success();
            let out = assert.get_output().stdout.clone();
            assert_eq!(
                decision_wrapped(&out),
                expected.map(str::to_string),
                "host={host} fixture={fixture}"
            );
        }
    }
}

#[test]
fn copilot_fixture_corpus() {
    let env = Env::new();
    let cases: &[(&str, Option<&str>)] = &[
        ("copilot/deny_bash.json", Some("deny")),
        ("copilot/ask_bash.json", Some("ask")),
        ("copilot/pass_bash.json", None),
        ("copilot/deny_other.json", Some("deny")),
        ("copilot/pass_other.json", None),
        ("copilot/no_tool_name.json", None),
    ];
    for (fixture, expected) in cases {
        let input = read_fixture(fixture);
        let assert = env
            .pg_command()
            .arg("--host=copilot")
            .write_stdin(input)
            .assert()
            .success();
        let out = assert.get_output().stdout.clone();
        assert_eq!(
            decision_flat(&out),
            expected.map(str::to_string),
            "fixture={fixture}"
        );
    }
}

#[test]
fn cwd_passthrough_claude() {
    let env = Env::new();
    let repo = env.build_cwd_push_repo();
    let deny_cwd_push = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {"command": "git push origin main"},
        "cwd": repo,
    });
    let pass_bad_cwd = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {"command": "echo hello"},
        "cwd": "/does-not-exist",
    });

    let assert = env
        .pg_command()
        .arg("--host=claude")
        .write_stdin(deny_cwd_push.to_string())
        .assert()
        .success();
    assert_eq!(
        decision_wrapped(&assert.get_output().stdout),
        Some("deny".to_string())
    );

    let assert = env
        .pg_command()
        .arg("--host=claude")
        .write_stdin(pass_bad_cwd.to_string())
        .assert()
        .success();
    assert_eq!(decision_wrapped(&assert.get_output().stdout), None);
}

#[test]
fn session_id_and_host_reach_ledger() {
    // host.rs が受け取った session_id/host/tool_name が BLEEP_SESSION_ID/
    // BLEEP_HOST/BLEEP_TOOL_NAME として bleep(bash)に渡り、判定レッジャーに
    // 記録されること。マッチした平文の語(acme/secret-project)はレッジャーの
    // どのフィールドにも現れないこと。
    let env = Env::new();
    // MCP 系ツール(tool_name != "Bash")は cmd_scan(素の text scan)を通る
    // ため、字句解析(lex)を挟まず match_verdict まで確実に到達する —
    // Bash コマンドだと "git push"/"gh ... create" の形でないと
    // fall-through pass になり、この検証には向かない。
    let input = serde_json::json!({
        "tool_name": "mcp__github__create_issue",
        "tool_input": {"title": "t", "body": "acme/secret-project"},
        "cwd": ".",
        "session_id": "sess-xyz",
    });
    let assert = env
        .pg_command()
        .arg("--host=claude")
        .write_stdin(input.to_string())
        .assert()
        .success();
    assert_eq!(
        decision_wrapped(&assert.get_output().stdout),
        Some("deny".to_string())
    );

    let ledger_file = env.ledger_dir().join("bleep.jsonl");
    let contents = std::fs::read_to_string(&ledger_file)
        .unwrap_or_else(|e| panic!("ledger not written at {ledger_file:?}: {e}"));
    let line = contents.lines().next().expect("at least one ledger line");
    let record: serde_json::Value = serde_json::from_str(line).expect("ledger line is valid JSON");
    assert_eq!(record["tool"], "bleep");
    assert_eq!(record["repo"], "tarotene/bleep");
    assert_eq!(record["host"], "claude");
    assert_eq!(record["session_id"], "sess-xyz");
    assert_eq!(record["tool_name"], "mcp__github__create_issue");
    assert_eq!(record["verdict"], "deny");
    assert_eq!(record["reason_id"], "repo-ref");
    assert!(
        !line.contains("acme/secret-project"),
        "ledger line must not contain the plaintext match: {line}"
    );
}

#[test]
fn allow_bypass_still_works() {
    let env = Env::new();
    let input = read_fixture("claude-codex/deny_bash.json");
    let assert = env
        .pg_command()
        .env("BLEEP_ALLOW", "1")
        .arg("--host=claude")
        .write_stdin(input)
        .assert()
        .success();
    assert!(assert.get_output().stdout.is_empty());
}
