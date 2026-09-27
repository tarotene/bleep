//! 3つの Bash adapter(claude-adapter.sh / codex-adapter.sh /
//! copilot-adapter.sh)の統合移植(D1)。判定ロジックは一切持たない —
//! tool_name/tool_input(または toolName/toolArgs)を bleep の入口
//! (scan-bash-command / scan)に渡し、その exit code(0=pass/1=ask/2=deny)を
//! ホストごとの出力 JSON に翻訳するだけ。

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    Claude,
    Codex,
    Copilot,
}

impl Host {
    pub fn parse(name: &str) -> Option<Host> {
        match name {
            "claude" => Some(Host::Claude),
            "codex" => Some(Host::Codex),
            "copilot" => Some(Host::Copilot),
            _ => None,
        }
    }
}

/// 判定レッジャーの `host` フィールド(bleep 側 BLEEP_HOST env)に使う名前。
fn host_env_name(host: Host) -> &'static str {
    match host {
        Host::Claude => "claude",
        Host::Codex => "codex",
        Host::Copilot => "copilot",
    }
}

/// tool_input/toolArgs の文字列リーフだけを出現順に再帰収集する
/// (`[.. | strings] | join("\n")` の移植)。tostring は使わない — JSON
/// エスケープが残ると改行が "\n"(バックスラッシュ+n)になり、
/// bleep 側の grep -Fw が改行直後の裸の単語一致を見逃す
/// (claude-adapter.sh の既存回帰と同じ理由、tests/fixtures/claude-codex/
/// deny_mcp_newline.json で固定している)。
fn collect_string_leaves(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(items) => {
            for item in items {
                collect_string_leaves(item, out);
            }
        }
        Value::Object(map) => {
            for val in map.values() {
                collect_string_leaves(val, out);
            }
        }
        _ => {}
    }
}

fn join_string_leaves(v: Option<&Value>) -> String {
    let mut leaves = Vec::new();
    if let Some(v) = v {
        collect_string_leaves(v, &mut leaves);
    }
    leaves.join("\n")
}

struct Decoded<'a> {
    tool_name: Option<&'a str>,
    is_bash: bool,
    bash_command: Option<&'a str>,
    tool_payload: Option<&'a Value>,
    cwd: Option<&'a str>,
    session_id: Option<&'a str>,
}

fn decode(host: Host, v: &Value) -> Decoded<'_> {
    let (tool_name, payload, bash_marker, session_id) = match host {
        Host::Claude | Host::Codex => (
            v.get("tool_name").and_then(Value::as_str),
            v.get("tool_input"),
            "Bash",
            v.get("session_id").and_then(Value::as_str),
        ),
        Host::Copilot => (
            v.get("toolName").and_then(Value::as_str),
            v.get("toolArgs"),
            "bash",
            v.get("sessionId").and_then(Value::as_str),
        ),
    };
    let cwd = v.get("cwd").and_then(Value::as_str);
    let is_bash = tool_name == Some(bash_marker);
    let bash_command = if is_bash {
        payload
            .and_then(|p| p.get("command"))
            .and_then(Value::as_str)
    } else {
        None
    };
    Decoded {
        tool_name,
        is_bash,
        bash_command,
        tool_payload: payload,
        cwd,
        session_id,
    }
}

enum Verdict {
    Pass,
    Ask(String),
    Deny(String),
}

/// host/session_id/tool_name を子プロセスの env に渡す。bleep(Bash)側の
/// 判定レッジャー(`ledger_write`)がこれらを読んで記録する — 値そのものに
/// 秘匿情報は含まない(host 名・セッション ID・呼ばれたツール名はどれも
/// bleep が守ろうとしている「private リポ名」ではない)。
fn run_pg(
    pg_bin: &str,
    cwd: Option<&str>,
    args: &[&str],
    stdin_text: Option<&str>,
    host: Host,
    session_id: Option<&str>,
    tool_name: &str,
) -> Verdict {
    let mut cmd = Command::new(pg_bin);
    if let Some(cwd) = cwd {
        if !cwd.is_empty() {
            cmd.arg("--cwd").arg(cwd);
        }
    }
    cmd.args(args);
    cmd.env("BLEEP_HOST", host_env_name(host));
    if let Some(sid) = session_id {
        if !sid.is_empty() {
            cmd.env("BLEEP_SESSION_ID", sid);
        }
    }
    cmd.env("BLEEP_TOOL_NAME", tool_name);
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());
    if stdin_text.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return Verdict::Ask("bleep を起動できませんでした(bleep-hook)。".to_string()),
    };
    if let Some(text) = stdin_text {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
    }
    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => return Verdict::Ask("bleep の終了を待てませんでした(bleep-hook)。".to_string()),
    };
    let reason = String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string();
    match output.status.code() {
        Some(1) => Verdict::Ask(reason),
        Some(2) => Verdict::Deny(reason),
        _ => Verdict::Pass, // 0(pass)、または想定外の exit code は安全側で pass 扱い
    }
}

fn emit(host: Host, verdict: Verdict) {
    let (decision, reason) = match verdict {
        Verdict::Pass => return, // 何も出力しない(hook 未介入 = allow)
        Verdict::Ask(r) => ("ask", r),
        Verdict::Deny(r) => ("deny", r),
    };
    let body = match host {
        Host::Claude | Host::Codex => json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": decision,
                "permissionDecisionReason": reason,
            }
        }),
        Host::Copilot => json!({
            "permissionDecision": decision,
            "permissionDecisionReason": reason,
        }),
    };
    println!("{body}");
}

/// run_hook の移植。pg_bin は `$BLEEP_BIN`(既定 "bleep"、
/// PATH 解決)。
pub fn run(host: Host, pg_bin: &str) {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        emit(
            host,
            Verdict::Ask("stdin を読み取れませんでした(bleep-hook)。".to_string()),
        );
        return;
    }

    // 不正な JSON は tool_name 不在と同じ扱いにする(既存 Bash+jq 版と同じ
    // 挙動 — jq のパース失敗は `// empty` で空文字列にフォールバックし、
    // 結果として無出力 pass になる。新たに ask へ強めることはしない —
    // Stage 3 検証の「Bash 版と Rust 版の出力完全一致」比較が壊れるため)。
    let v: Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(_) => return,
    };

    let d = decode(host, &v);
    let Some(tool_name) = d.tool_name else {
        return; // tool_name が無い入力はこの hook の対象外
    };

    let verdict = if d.is_bash {
        match d.bash_command {
            Some(cmd) if !cmd.is_empty() => run_pg(
                pg_bin,
                d.cwd,
                &["scan-bash-command", cmd],
                None,
                host,
                d.session_id,
                tool_name,
            ),
            _ => Verdict::Pass, // Bash だが command が空 = 何もしない(既存 Bash 版と同じ)
        }
    } else {
        let text = join_string_leaves(d.tool_payload);
        run_pg(
            pg_bin,
            d.cwd,
            &["scan", "-"],
            Some(&text),
            host,
            d.session_id,
            tool_name,
        )
    };

    emit(host, verdict);
}
