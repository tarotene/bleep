#!/usr/bin/env bash
# claude-adapter.sh — Claude Code の PreToolUse hook から stdin JSON で呼ばれる
# 薄い adapter。判定ロジックは一切持たない: tool_name/tool_input を
# publish-guard の入口(scan-bash-command / scan)に渡し、その exit code
# (0=pass/1=ask/2=deny)を hookSpecificOutput の JSON に翻訳するだけ。
#
# matcher は hooks.json 側で "Bash|mcp__.*" の複合1本にしてある。存在判定が
# command 文字列の完全一致しかできない登録系(dotfiles 側の registerHooks)
# と挙動を揃えるため、Bash と MCP を2つの hook エントリに分けない —
# 分けると片方が無検査で素通りする経路が生まれる(詳細は README)。
#
# 使い方: 通常は Claude Code から stdin JSON で呼ばれる(引数無し)。
#   claude-adapter.sh --selftest   回帰テスト(スタブ gh)。
set -uo pipefail # publish-guard の非0 exit を意図的に扱うため -e は使わない

SELF="$(realpath "$0")"
CLAUDE_PLUGIN_ROOT="${CLAUDE_PLUGIN_ROOT:-$(dirname "$(dirname "$SELF")")}"
PG="$CLAUDE_PLUGIN_ROOT/publish-guard"
FIXTURES_DIR="$CLAUDE_PLUGIN_ROOT/tests/fixtures" # selftest 専用(D4 — 3 adapter 共通)
TEST_TMP='' # selftest の trap から参照するグローバル(local 変数だと関数
# 返り後の EXIT trap 発火時に「unbound variable」になる — publish-guard
# 本体と同じ idiom)

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
    # jq 不在時の固定形フォールバック。JSON の形は固定(フィールド数が
    # 変わらない)ので jq は要らない — reason 中の \ と " だけ最低限
    # エスケープする。
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
    # jq が無いと tool_name/tool_input を安全に取り出せない。無言 pass では
    # なく ask にエスカレートする(fail-loud — 判定不能を検査省略の理由に
    # しない)。
    emit_json ask "jq が見つからないため PreToolUse 入力を解析できませんでした(publish-guard adapter)。"
    exit 0
  fi

  local tool_name
  tool_name="$(jq -r '.tool_name // empty' <<< "$input" 2> /dev/null)" || tool_name=""
  [[ -n $tool_name ]] || exit 0 # tool_name が無い入力はこの hook の対象外

  # PreToolUse payload の cwd を publish-guard に明示的に渡す(#10/#14 —
  # フックプロセス自身の cwd ではなく、Claude が実際にいるディレクトリを
  # 対象にする。Anthropic "Hooks reference" が cwd フィールドの存在を明記)。
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
    # mcp__<server>__<tool> 等。tool_input の文字列リーフだけを再帰収集し、
    # デコード済みの実改行で連結してから渡す。tostring は使わない — JSON
    # エスケープが残るため改行が "\n"(バックスラッシュ+n)になり、直前の
    # 文字が単語文字扱いになって publish-guard の grep -Fw が改行直後の
    # 裸のリポ名・org 名を見逃す(この selftest で回帰を固定している)。
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
    *) : ;; # 0(pass)— 何も出力しない(hook 未介入 = allow)
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

  # fixture JSON は3 adapter(claude/codex/copilot)共通の tests/fixtures/
  # 以下を参照する(D4)。claude/codex は入出力の形が同一のため
  # claude-codex/ を共有する。
  local fx="$FIXTURES_DIR/claude-codex"

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
  assert_decision() { # $1=fixture(絶対パスまたは $tmp 相対) $2=expected(deny/ask)
    out="$("$SELF" < "$1")"
    decision="$(jq -r '.hookSpecificOutput.permissionDecision // empty' <<< "$out" 2> /dev/null)"
    if [[ $decision != "$2" ]]; then
      echo "FAIL($1): 期待=$2 実際=$decision out=$out" >&2
      fails=$((fails + 1))
    fi
  }
  assert_empty() { # $1=fixture(絶対パスまたは $tmp 相対)
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
  assert_decision "$fx/deny_mcp_newline.json" deny # tostring 回帰の固定
  assert_empty "$fx/pass_mcp.json"
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
