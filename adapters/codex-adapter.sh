#!/usr/bin/env bash
# codex-adapter.sh — OpenAI Codex CLI の PreToolUse hook から stdin JSON で
# 呼ばれる薄い adapter。判定ロジックは一切持たない: hooks/claude-adapter.sh
# と実質同型で、tool_name/tool_input の場所も出力 JSON の形
# (hookSpecificOutput.permissionDecision/permissionDecisionReason)も同じ
# ことを実機(`codex exec --dangerously-bypass-hook-trust`、2026-09-10)で
# 実測して確認した:
#   実測 PreToolUse 入力(Bash): {"tool_name":"Bash","tool_input":
#     {"command":"..."},"hook_event_name":"PreToolUse","permission_mode":
#     "bypassPermissions", ...}
#   deny を返す hookSpecificOutput JSON を返すと、実際にコマンドがブロック
#   されることも実測済み。
#
# claude-adapter.sh との唯一の違い: Codex には ${CLAUDE_PLUGIN_ROOT} 相当の
# 変数が無い(plugin マーケットプレイス機構自体が無い)ため、常に自分の
# パスから publish-guard を解決する。
#
# hooks.json の登録例(matcher は "Bash" のみ実機確認済み。MCP tool の
# 命名規則は Codex 側で未確認 — README の「未検証の前提」参照):
#   {
#     "hooks": {
#       "PreToolUse": [
#         {
#           "matcher": "Bash|mcp__.*",
#           "hooks": [
#             {"type": "command", "command": "/path/to/codex-adapter.sh", "timeout": 20}
#           ]
#         }
#       ]
#     }
#   }
#
# 使い方: 通常は Codex CLI から stdin JSON で呼ばれる(引数無し)。
#   codex-adapter.sh --selftest   回帰テスト(スタブ gh)。
set -uo pipefail # publish-guard の非0 exit を意図的に扱うため -e は使わない

SELF="$(realpath "$0")"
ADAPTER_ROOT="$(dirname "$(dirname "$SELF")")"
PG="$ADAPTER_ROOT/publish-guard"
FIXTURES_DIR="$ADAPTER_ROOT/tests/fixtures" # selftest 専用(D4 — 3 adapter 共通)
TEST_TMP='' # selftest の trap から参照するグローバル(local だと関数返り後の
# EXIT trap 発火時に「unbound variable」になる)

emit_json() { # $1=decision(ask|deny) $2=reason
  if command -v jq > /dev/null 2>&1; then
    jq -n --arg decision "$1" --arg reason "$2" '{
      hookSpecificOutput: {
        hookEventName: "PreToolUse",
        permissionDecision: $decision,
        permissionDecisionReason: $reason
      }
    }'
  else
    local esc="${2//\\/\\\\}"
    esc="${esc//\"/\\\"}"
    printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"%s","permissionDecisionReason":"%s"}}\n' \
      "$1" "$esc"
  fi
}

run_hook() {
  local input
  input="$(cat)" || {
    emit_json ask "stdin を読み取れませんでした(publish-guard adapter)。"
    exit 0
  }

  if ! command -v jq > /dev/null 2>&1; then
    emit_json ask "jq が見つからないため PreToolUse 入力を解析できませんでした(publish-guard adapter)。"
    exit 0
  fi

  local tool_name
  tool_name="$(jq -r '.tool_name // empty' <<< "$input" 2> /dev/null)" || tool_name=""
  [[ -n $tool_name ]] || exit 0

  # PreToolUse payload の cwd を publish-guard に明示的に渡す(#10/#14 —
  # フックプロセス自身の cwd ではなく、セッションが実際にいるディレクトリを
  # 対象にする。OpenAI "Hooks" が cwd フィールドの存在を明記)。
  local hook_cwd
  hook_cwd="$(jq -r '.cwd // empty' <<< "$input" 2> /dev/null)" || hook_cwd=""
  local -a cwd_args=()
  [[ -n $hook_cwd ]] && cwd_args=(--cwd "$hook_cwd")

  local reason="" rc=0 cmd text
  if [[ $tool_name == "Bash" ]]; then
    cmd="$(jq -r '.tool_input.command // empty' <<< "$input" 2> /dev/null)" || cmd=""
    if [[ -n $cmd ]]; then
      if reason="$("$PG" "${cwd_args[@]}" scan-bash-command "$cmd")"; then
        rc=0
      else
        rc=$?
      fi
    fi
  else
    text="$(jq -r '[.tool_input | .. | strings] | join("\n")' <<< "$input" 2> /dev/null)" || text=""
    if reason="$(printf '%s' "$text" | "$PG" "${cwd_args[@]}" scan -)"; then
      rc=0
    else
      rc=$?
    fi
  fi

  case "$rc" in
    1) emit_json ask "$reason" ;;
    2) emit_json deny "$reason" ;;
    *) : ;;
  esac
  exit 0
}

selftest() {
  local tmp
  tmp="$(mktemp -d)"
  TEST_TMP="$tmp"
  trap 'rm -rf "$TEST_TMP"' EXIT

  export PUBLISH_GUARD_CONFIG_DIR="$tmp/config"
  export PUBLISH_GUARD_STATE_DIR="$tmp/state"
  export PUBLISH_GUARD_ORGS_FILE="$tmp/config/orgs.txt"
  export PUBLISH_GUARD_REPOS_FILE="$tmp/config/repos.txt"
  export PUBLISH_GUARD_OWNER="test-owner"
  export PUBLISH_GUARD_GH_BIN="$tmp/bin-gh"
  mkdir -p "$tmp/config" "$tmp/state"
  printf 'acme\n' > "$PUBLISH_GUARD_ORGS_FILE"
  printf 'acme/secret-project\n' > "$PUBLISH_GUARD_REPOS_FILE"
  cp "$FIXTURES_DIR/gh-stub.sh" "$tmp/bin-gh"
  chmod +x "$tmp/bin-gh"

  # 実機確認済みの Codex PreToolUse 入力の形(tool_name/tool_input.command)は
  # claude-adapter.sh と同一のため、fixture は tests/fixtures/claude-codex/
  # を共有する(D4)。
  local fx="$ADAPTER_ROOT/tests/fixtures/claude-codex"

  # --cwd: payload の cwd を経由して、adapter 自身の cwd ではなく対象リポジトリの
  # push 差分を検査できること(#10/#14)。cwd が無効でも無関係コマンドは pass。
  # git セットアップは tests/fixtures/push-cwd-repo.sh 共通(#16 の3重コピー
  # を一本化)。
  # shellcheck source=/dev/null
  source "$FIXTURES_DIR/push-cwd-repo.sh"
  build_cwd_push_repo "$tmp"
  jq -n --arg cwd "$CWD_PUSH_REPO" \
    '{tool_name:"Bash", tool_input:{command:"git push origin main"}, cwd:$cwd}' \
    > "$tmp/deny_cwd_push.json"
  jq -n '{tool_name:"Bash", tool_input:{command:"echo hello"}, cwd:"/does-not-exist"}' \
    > "$tmp/pass_bad_cwd.json"

  local fails=0 out decision
  assert_decision() {
    out="$("$SELF" < "$1")"
    decision="$(jq -r '.hookSpecificOutput.permissionDecision // empty' <<< "$out" 2> /dev/null)"
    if [[ $decision != "$2" ]]; then
      echo "FAIL($1): 期待=$2 実際=$decision out=$out" >&2
      fails=$((fails + 1))
    fi
  }
  assert_empty() {
    out="$("$SELF" < "$1")"
    if [[ -n $out ]]; then
      echo "FAIL($1): 出力があった(期待=無出力): $out" >&2
      fails=$((fails + 1))
    fi
  }

  assert_decision "$fx/deny_bash.json" deny
  assert_decision "$fx/ask_bash.json" ask
  assert_empty "$fx/pass_bash.json"
  assert_decision "$fx/deny_mcp.json" deny
  assert_empty "$fx/no_tool_name.json"
  assert_decision "$tmp/deny_cwd_push.json" deny # --cwd 経由で対象リポジトリの push を検査
  assert_empty "$tmp/pass_bad_cwd.json" # --cwd 解決失敗でも無関係コマンドは無言 pass

  out="$(PUBLISH_GUARD_ALLOW=1 "$SELF" < "$fx/deny_bash.json")"
  [[ -z $out ]] || { echo "FAIL(ALLOW=1): 出力があった: $out" >&2; fails=$((fails + 1)); }

  if [[ $fails -gt 0 ]]; then
    echo "selftest: ${fails} 件失敗" >&2
    exit 1
  fi
  echo "selftest: OK"
}

case "${1-}" in
  --selftest) selftest ;;
  *) run_hook ;;
esac
