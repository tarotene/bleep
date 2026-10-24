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
fn collect_string_leaves(v: &Value, path: &str, out: &mut Vec<Leaf>) {
    match v {
        Value::String(s) => out.push(Leaf {
            path: path.to_string(),
            text: s.clone(),
        }),
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                collect_string_leaves(item, &format!("{path}[{i}]"), out);
            }
        }
        Value::Object(map) => {
            for (k, val) in map {
                collect_string_leaves(val, &format!("{path}.{}", safe_segment(k)), out);
            }
        }
        _ => {}
    }
}

/// 文字列リーフ 1 つ分: JSON の経路(`tool_input.blocks[0].text` の形)と本文。
struct Leaf {
    path: String,
    text: String,
}

/// 欄の名前として拒否文に出せるキー。キーは呼び出し側(エージェント)が書く値なので、
/// 英数字・`_`・`-` だけの短い名前以外は `?` に置き換える(改行や長い文字列を、欄の
/// 一覧に持ち込まない)。
fn safe_segment(k: &str) -> String {
    let ok = !k.is_empty()
        && k.len() <= 40
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok {
        k.to_string()
    } else {
        "?".to_string()
    }
}

/// `BLEEP_FIELD_MAP` の上限。環境変数に載せる大きさには限りがあるので、超えるときは
/// 欄の一覧を渡さない(bleep は「入力の第 N 行」とだけ示す)。
const FIELD_MAP_MAX_BYTES: usize = 32 * 1024;

/// 文字列リーフを `\n` で結合した本文と、欄の一覧(`<開始行>\t<経路>` の行の列)。
/// 開始行は結合後の本文での 1 始まりの行番号で、bleep が一致した行番号から欄とその
/// 欄の中の行番号を引くのに使う。
fn join_string_leaves(root: &str, v: Option<&Value>) -> (String, Option<String>) {
    let mut leaves = Vec::new();
    if let Some(v) = v {
        collect_string_leaves(v, root, &mut leaves);
    }
    let mut map = String::new();
    let mut line = 1usize;
    for leaf in &leaves {
        map.push_str(&format!("{line}\t{}\n", leaf.path));
        line += leaf.text.matches('\n').count() + 1;
    }
    let text = leaves
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let map = (!map.is_empty() && map.len() <= FIELD_MAP_MAX_BYTES).then_some(map);
    (text, map)
}

/// 拒否文に出す欄の名前の根。ホストが入力を渡すキーの名前そのもの(エージェントが
/// 自分の呼び出しで見ている名前)にする。
fn payload_root(host: Host) -> &'static str {
    match host {
        Host::Claude | Host::Codex => "tool_input",
        Host::Copilot => "toolArgs",
    }
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
/// bleep が守ろうとしている「private リポ名」ではない)。`field_map` は
/// MCP 入力の欄の一覧(`join_string_leaves`)で、拒否文に一致した欄を示すために
/// `BLEEP_FIELD_MAP` で渡す。欄の名前は入力のキーで、検査対象の値ではない。
/// 1 回の呼び出しの文脈。子プロセスの env に載せる値(`run_pg` を参照)。
#[derive(Clone, Copy)]
struct Call<'a> {
    host: Host,
    session_id: Option<&'a str>,
    tool_name: &'a str,
    field_map: Option<&'a str>,
}

fn run_pg(
    pg_bin: &str,
    cwd: Option<&str>,
    args: &[&str],
    stdin_text: Option<&str>,
    ctx: &Call,
) -> Verdict {
    let Call {
        host,
        session_id,
        tool_name,
        field_map,
    } = *ctx;
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
    if let Some(map) = field_map {
        cmd.env("BLEEP_FIELD_MAP", map);
    }
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
                &Call {
                    host,
                    session_id: d.session_id,
                    tool_name,
                    field_map: None,
                },
            ),
            _ => Verdict::Pass, // Bash だが command が空 = 何もしない(既存 Bash 版と同じ)
        }
    } else {
        let (text, field_map) = join_string_leaves(payload_root(host), d.tool_payload);
        run_pg(
            pg_bin,
            d.cwd,
            &["scan", "-"],
            Some(&text),
            &Call {
                host,
                session_id: d.session_id,
                tool_name,
                field_map: field_map.as_deref(),
            },
        )
    };

    emit(host, verdict);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_map_lists_each_leaf_with_its_start_line() {
        let v = serde_json::json!({
            "title": "t",
            "blocks": [{"text": "a\nb"}, {"text": "c"}],
            "odd key\n": "z",
            "n": 1,
        });
        let (text, map) = join_string_leaves("tool_input", Some(&v));
        // serde_json の Map は既定ではキーの辞書順(preserve_order なし)。
        assert_eq!(text, "a\nb\nc\nz\nt");
        assert_eq!(
            map.as_deref(),
            Some(
                "1\ttool_input.blocks[0].text\n3\ttool_input.blocks[1].text\n4\ttool_input.?\n5\ttool_input.title\n"
            )
        );
    }

    #[test]
    fn no_leaves_means_no_field_map() {
        assert_eq!(
            join_string_leaves("tool_input", None),
            (String::new(), None)
        );
        let v = serde_json::json!({"n": 1});
        assert_eq!(join_string_leaves("tool_input", Some(&v)).1, None);
    }

    #[test]
    fn an_oversized_field_map_is_dropped() {
        let leaves: Vec<_> = (0..4000).map(|i| format!("v{i}")).collect();
        let v = serde_json::json!({ "items": leaves });
        let (_, map) = join_string_leaves("tool_input", Some(&v));
        assert_eq!(map, None);
    }

    #[test]
    fn only_short_plain_keys_are_shown() {
        assert_eq!(safe_segment("body-text_2"), "body-text_2");
        assert_eq!(safe_segment("has space"), "?");
        assert_eq!(safe_segment(""), "?");
        assert_eq!(safe_segment(&"a".repeat(41)), "?");
    }
}
