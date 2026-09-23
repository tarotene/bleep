#!/usr/bin/env bash
# gh-stub.sh — publish-guard 本体 / adapter 3本の selftest が共通で使う
# スタブ gh。repo list は private/internal どちらも 'my-private-tool' を、
# repo view は常に PUBLIC を、api user は 'test-owner' を返す。実在の
# org/repo 名は使わない(CONTRIBUTING.md の fixture 規約)。
#
# 使い方: selftest が $tmp/bin-gh にコピーして chmod +x し、
# PUBLISH_GUARD_GH_BIN として渡す。
case "$1 $2" in
  "repo list") printf 'my-private-tool\n' ;;
  "repo view") printf 'PUBLIC' ;;
  "api user") printf 'test-owner' ;;
  *) exit 1 ;;
esac
