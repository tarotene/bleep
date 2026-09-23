#!/usr/bin/env bash
# copilot-adapter.sh — GitHub Copilot CLI の preToolUse hook から stdin JSON
# で呼ばれる薄い adapter。判定ロジックは一切持たない。
#
# Copilot の preToolUse は Claude/Codex と入出力の形が違う(2026-09-10 に
# 実機(`copilot -p ... --allow-all-tools`、隔離 HOME + 実 auth コピー)で
# deny/pass を実測して確認した):
#   実測入力: {"sessionId":"...","timestamp":..,"cwd":"...",
#              "toolName":"bash","toolArgs":{"command":"echo ...","description":"..."}}
#   出力は hookSpecificOutput でラップしない直下の JSON:
#     {"permissionDecision":"deny","permissionDecisionReason":"..."}
#   を返すと実際にブロックされることを実測済み(pass は無出力で確認)。
#   toolName は小文字 "bash"(Bash/Shell ではない)。
#   Copilot の preToolUse には matcher が無い(全 tool call で無条件発火)
#   ので、tool 種別の絞り込みはこの adapter 内部で行う — hooks.json 側の
#   登録に matcher フィールドは書けない(書いても無視される)。
#
# hooks.json(~/.copilot/settings.json の "hooks" キー)の登録例:
#   {
#     "hooks": {
#       "preToolUse": [
#         {"type": "command", "bash": "/path/to/copilot-adapter.sh", "timeoutSec": 20}
#       ]
#     }
#   }
#
# 使い方: 通常は Copilot CLI から stdin JSON で呼ばれる(引数無し)。
#   copilot-adapter.sh --selftest   回帰テスト(スタブ gh)。
set -uo pipefail # publish-guard の非0 exit を意図的に扱うため -e は使わない

SELF="$(realpath "$0")"
ADAPTER_ROOT="$(dirname "$(dirname "$SELF")")"
PG="$ADAPTER_ROOT/publish-guard"
TEST_TMP=''

emit_json() { # $1=decision(ask|deny) $2=reason
  # Copilot は hookSpecificOutput でラップしない直下の JSON を読む(Claude/
  # Codex とはここだけ違う — 実機で確認済み)。
  if command -v jq > /dev/null 2>&1; then
    jq -n --arg decision "$1" --arg reason "$2" '{
      permissionDecision: $decision,
      permissionDecisionReason: $reason
    }'
  else
    local esc="${2//\\/\\\\}"
    esc="${esc//\"/\\\"}"
    printf '{"permissionDecision":"%s","permissionDecisionReason":"%s"}\n' "$1" "$esc"
  fi
}

run_hook() {
  local input
  input="$(cat)" || {
    emit_json ask "stdin を読み取れませんでした(publish-guard adapter)。"
    exit 0
  }

  if ! command -v jq > /dev/null 2>&1; then
    emit_json ask "jq が見つからないため preToolUse 入力を解析できませんでした(publish-guard adapter)。"
    exit 0
  fi

  local tool_name
  tool_name="$(jq -r '.toolName // empty' <<< "$input" 2> /dev/null)" || tool_name=""
  [[ -n $tool_name ]] || exit 0

  # preToolUse payload のトップレベル cwd を publish-guard に明示的に渡す
  # (#10/#14 — 実機実測済み: adapters/copilot-adapter.sh:8 参照)。
  local hook_cwd
  hook_cwd="$(jq -r '.cwd // empty' <<< "$input" 2> /dev/null)" || hook_cwd=""
  local -a cwd_args=()
  [[ -n $hook_cwd ]] && cwd_args=(--cwd "$hook_cwd")

  local reason="" rc=0 cmd text
  if [[ $tool_name == "bash" ]]; then
    cmd="$(jq -r '.toolArgs.command // empty' <<< "$input" 2> /dev/null)" || cmd=""
    if [[ -n $cmd ]]; then
      if reason="$("$PG" "${cwd_args[@]}" scan-bash-command "$cmd")"; then
        rc=0
      else
        rc=$?
      fi
    fi
  else
    # bash 以外の全 tool(matcher が無いので preToolUse は無条件で発火する)。
    # toolArgs の文字列リーフを再帰収集して実改行で連結する(tostring は
    # 使わない — claude-adapter.sh と同じ理由、改行直後の単語境界一致が壊れる)。
    text="$(jq -r '[.toolArgs | .. | strings] | join("\n")' <<< "$input" 2> /dev/null)" || text=""
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
  cat > "$tmp/bin-gh" <<'STUB'
#!/usr/bin/env bash
case "$1 $2" in
  "repo list") printf 'my-private-tool\n' ;;
  "repo view") printf 'PUBLIC' ;;
  "api user") printf 'test-owner' ;;
  *) exit 1 ;;
esac
STUB
  chmod +x "$tmp/bin-gh"

  # 実機確認済みの Copilot preToolUse 入力の形(toolName/toolArgs.command)
  # をそのまま fixture にする。
  printf '%s' '{"sessionId":"s","timestamp":1,"cwd":"/x","toolName":"bash","toolArgs":{"command":"gh pr create --title t --body \"acme/secret-project\""}}' > "$tmp/deny_bash.json"
  printf '%s' '{"sessionId":"s","timestamp":1,"cwd":"/x","toolName":"bash","toolArgs":{"command":"gh pr create --title t --body \"acme is our employer\""}}' > "$tmp/ask_bash.json"
  printf '%s' '{"sessionId":"s","timestamp":1,"cwd":"/x","toolName":"bash","toolArgs":{"command":"echo hello"}}' > "$tmp/pass_bash.json"
  # matcher が無いので、bash 以外の tool でも無条件発火する(例: ファイル
  # 編集系ツール)。toolArgs の文字列リーフから deny を拾えることを確認する。
  printf '%s' '{"sessionId":"s","timestamp":1,"cwd":"/x","toolName":"str_replace_editor","toolArgs":{"new_str":"acme/secret-project"}}' > "$tmp/deny_other.json"
  printf '%s' '{"sessionId":"s","timestamp":1,"cwd":"/x","toolName":"view","toolArgs":{"path":"/x/y"}}' > "$tmp/pass_other.json"
  printf '%s' '{"sessionId":"s","timestamp":1,"cwd":"/x"}' > "$tmp/no_tool_name.json"

  # --cwd: payload の cwd を経由して、adapter 自身の cwd ではなく対象リポジトリの
  # push 差分を検査できること(#10/#14)。cwd が無効でも無関係コマンドは pass。
  local repo="$tmp/repo"
  mkdir -p "$repo"
  git -C "$repo" init -qb main
  git -C "$repo" config core.hooksPath /dev/null
  git -C "$repo" config user.email test@example.invalid
  git -C "$repo" config user.name test
  git -C "$repo" config commit.gpgsign false
  echo "clean content" > "$repo/a.txt"
  git -C "$repo" add a.txt
  git -C "$repo" commit -qm initial
  git init -q --bare "$tmp/remote.git"
  git -C "$repo" remote add origin "$tmp/remote.git"
  git -C "$repo" push -q origin main
  git -C "$repo" symbolic-ref refs/remotes/origin/HEAD refs/remotes/origin/main
  echo "acme/secret-project が話題" >> "$repo/a.txt"
  git -C "$repo" add a.txt
  git -C "$repo" commit -qm "mentions acme/secret-project"
  jq -n --arg cwd "$repo" \
    '{sessionId:"s", timestamp:1, cwd:$cwd, toolName:"bash", toolArgs:{command:"git push origin main"}}' \
    > "$tmp/deny_cwd_push.json"
  jq -n \
    '{sessionId:"s", timestamp:1, cwd:"/does-not-exist", toolName:"bash", toolArgs:{command:"echo hello"}}' \
    > "$tmp/pass_bad_cwd.json"

  local fails=0 out decision
  assert_decision() {
    out="$("$SELF" < "$tmp/$1")"
    decision="$(jq -r '.permissionDecision // empty' <<< "$out" 2> /dev/null)"
    if [[ $decision != "$2" ]]; then
      echo "FAIL($1): 期待=$2 実際=$decision out=$out" >&2
      fails=$((fails + 1))
    fi
  }
  assert_empty() {
    out="$("$SELF" < "$tmp/$1")"
    if [[ -n $out ]]; then
      echo "FAIL($1): 出力があった(期待=無出力): $out" >&2
      fails=$((fails + 1))
    fi
  }

  assert_decision deny_bash.json deny
  assert_decision ask_bash.json ask
  assert_empty pass_bash.json
  assert_decision deny_other.json deny
  assert_empty pass_other.json
  assert_empty no_tool_name.json
  assert_decision deny_cwd_push.json deny # --cwd 経由で対象リポジトリの push を検査
  assert_empty pass_bad_cwd.json # --cwd 解決失敗でも無関係コマンドは無言 pass

  out="$(PUBLISH_GUARD_ALLOW=1 "$SELF" < "$tmp/deny_bash.json")"
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
