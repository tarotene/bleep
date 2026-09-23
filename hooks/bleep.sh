#!/usr/bin/env bash
# bleep.sh — bleep-hook(Rust)への単一 shim。Claude/Codex/Copilot
# の hook 登録はすべてこのスクリプトを指す(--host=<name> で分岐)。判定
# ロジックは一切持たない(旧 claude-adapter.sh/codex-adapter.sh/
# copilot-adapter.sh を統合した bleep-hook を起動するだけ)。
#
# バイナリ探索: BLEEP_HOOK_BIN → PATH 上の bleep-hook →
# ~/.cargo/bin/bleep-hook。見つからなければ、host 固有の固定文
# (補間なしの定数、手書きエスケープの問題は生じない)の ask JSON を印字
# して exit 0(D2 — fail-loud)。
#
# なぜこの shim が要るか: プラグイン導入済みでバイナリ未設置という状態が
# 静かに pass する穴を塞ぐため。Claude Code の PreToolUse hook は、
# command パスが存在しない場合 non-blocking(exit 127)扱いになり tool
# call をそのまま素通りさせる
# (https://code.claude.com/docs/en/hooks.md、2026-09-24 取得、
# "Failed with non-blocking status code" の項)。見つかれば
# BLEEP_BIN(このリポジトリ内の bash 本体 bleep への
# パス)を設定して exec する。
#
# 使い方: 通常は host から stdin JSON + 引数 --host=<name> で呼ばれる。
#   bleep.sh --selftest   回帰テスト(バイナリ発見/未発見の両経路)。
set -uo pipefail

SELF="$(realpath "$0")"
BLEEP_ROOT="$(dirname "$(dirname "$SELF")")"
TEST_TMP='' # selftest の trap から参照するグローバル(local 変数だと関数
# 返り後の EXIT trap 発火時に「unbound variable」になる — bleep
# 本体・旧 adapter と同じ idiom)

find_hook_bin() {
  if [[ -n "${BLEEP_HOOK_BIN:-}" && -x "${BLEEP_HOOK_BIN:-}" ]]; then
    printf '%s' "$BLEEP_HOOK_BIN"
    return 0
  fi
  local found
  found="$(command -v bleep-hook 2> /dev/null)" || found=""
  if [[ -n $found ]]; then
    printf '%s' "$found"
    return 0
  fi
  if [[ -x "${HOME:-/nonexistent}/.cargo/bin/bleep-hook" ]]; then
    printf '%s' "$HOME/.cargo/bin/bleep-hook"
    return 0
  fi
  return 1
}

fallback_ask() { # $1=host
  if [[ "$1" == "copilot" ]]; then
    printf '{"permissionDecision":"ask","permissionDecisionReason":"bleep-hook バイナリが見つかりません(cargo install bleep-hook でインストールしてください)。"}\n'
  else
    printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"ask","permissionDecisionReason":"bleep-hook バイナリが見つかりません(cargo install bleep-hook でインストールしてください)。"}}\n'
  fi
}

main() {
  local host="${1#--host=}"
  local bin
  if bin="$(find_hook_bin)"; then
    export BLEEP_BIN="$BLEEP_ROOT/bleep"
    exec "$bin" "$@"
  fi
  fallback_ask "$host"
  exit 0
}

selftest() {
  local fails=0 out decision
  local tmp
  tmp="$(mktemp -d)"
  TEST_TMP="$tmp"
  trap 'rm -rf "$TEST_TMP"' EXIT

  # 経路1: バイナリが見つかる場合、exec して正常に判定結果が返ること。
  local real_bin="$BLEEP_ROOT/target/debug/bleep-hook"
  if [[ ! -x $real_bin ]]; then
    real_bin="$BLEEP_ROOT/target/release/bleep-hook"
  fi
  if [[ -x $real_bin ]]; then
    export BLEEP_HOOK_BIN="$real_bin"
    # exec 後の bleep-hook が(host モード経由で)呼ぶ bash 本体
    # bleep の cmd_scan_bash_command 自身も、字句解析のために
    # 同じバイナリを再度必要とする(D1 — 継ぎ目は2箇所ある)。
    export BLEEP_LEX_BIN="$real_bin"
    export BLEEP_CONFIG_DIR="$tmp/config"
    export BLEEP_STATE_DIR="$tmp/state"
    export BLEEP_ORGS_FILE="$tmp/config/orgs.txt"
    export BLEEP_REPOS_FILE="$tmp/config/repos.txt"
    export BLEEP_OWNER="test-owner"
    export BLEEP_GH_BIN="$tmp/bin-gh"
    mkdir -p "$tmp/config" "$tmp/state"
    printf 'acme\n' > "$BLEEP_ORGS_FILE"
    printf 'acme/secret-project\n' > "$BLEEP_REPOS_FILE"
    cp "$BLEEP_ROOT/tests/fixtures/gh-stub.sh" "$tmp/bin-gh"
    chmod +x "$tmp/bin-gh"

    out="$("$SELF" --host=claude < "$BLEEP_ROOT/tests/fixtures/claude-codex/deny_bash.json")"
    decision="$(printf '%s' "$out" | sed -n 's/.*"permissionDecision":"\([a-z]*\)".*/\1/p')"
    if [[ $decision != "deny" ]]; then
      echo "FAIL(exec 経路): 期待=deny 実際=$decision out=$out" >&2
      fails=$((fails + 1))
    fi
    unset BLEEP_HOOK_BIN BLEEP_LEX_BIN BLEEP_CONFIG_DIR BLEEP_STATE_DIR \
      BLEEP_ORGS_FILE BLEEP_REPOS_FILE BLEEP_OWNER BLEEP_GH_BIN
  else
    echo "SKIP(exec 経路): $real_bin が無い — 先に cargo build してください" >&2
  fi

  # 経路2: バイナリが一切見つからない場合、無言 pass ではなく ask
  # にエスカレートすること(D2)。PATH/HOME を隔離して確実に「見つからない」
  # 状態を作る。
  local fake_home="$tmp/home"
  mkdir -p "$fake_home"
  out="$(BLEEP_HOOK_BIN='' HOME="$fake_home" PATH="/usr/bin:/bin" "$SELF" --host=claude < "$BLEEP_ROOT/tests/fixtures/claude-codex/deny_bash.json")"
  decision="$(printf '%s' "$out" | sed -n 's/.*"permissionDecision":"\([a-z]*\)".*/\1/p')"
  if [[ $decision != "ask" ]]; then
    echo "FAIL(未発見 fallback, claude): 期待=ask 実際=$decision out=$out" >&2
    fails=$((fails + 1))
  fi

  out="$(BLEEP_HOOK_BIN='' HOME="$fake_home" PATH="/usr/bin:/bin" "$SELF" --host=copilot < "$BLEEP_ROOT/tests/fixtures/copilot/deny_bash.json")"
  decision="$(printf '%s' "$out" | sed -n 's/.*"permissionDecision":"\([a-z]*\)".*/\1/p')"
  if [[ $decision != "ask" ]]; then
    echo "FAIL(未発見 fallback, copilot): 期待=ask 実際=$decision out=$out" >&2
    fails=$((fails + 1))
  fi
  # copilot の出力は hookSpecificOutput でラップしないこと。
  if printf '%s' "$out" | grep -q hookSpecificOutput; then
    echo "FAIL(未発見 fallback, copilot): hookSpecificOutput でラップされている: $out" >&2
    fails=$((fails + 1))
  fi

  if [[ $fails -gt 0 ]]; then
    echo "selftest: ${fails} 件失敗" >&2
    exit 1
  fi
  echo "selftest: OK"
}

case "${1-}" in
  --selftest) selftest ;;
  *) main "$@" ;;
esac
