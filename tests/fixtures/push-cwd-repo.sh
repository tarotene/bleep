#!/usr/bin/env bash
# push-cwd-repo.sh — adapter 3本(claude/codex/copilot)の selftest が共通で
# 使う --cwd 検証用 git リポジトリのセットアップ(#10/#14 の --cwd 回帰を
# 固定する fixture)。#16 の diff がこの手順(25行)を3本へ逐語コピーして
# いたのを一本化した。
#
# 使い方: source した上で build_cwd_push_repo "$tmp" を呼ぶ。呼び出し後、
# CWD_PUSH_REPO にリポジトリのパスが入る — deny_cwd_push.json の cwd に使う
# (acme/secret-project 混入コミットを origin/main に対して push する形)。
# pass_bad_cwd.json 側は cwd 解決失敗を固定するためのもので、実在しない
# パスをそのまま使うのでこの fixture は不要(呼び出し側で直接組み立てる)。
CWD_PUSH_REPO=''
build_cwd_push_repo() {
  local tmp="$1" repo
  repo="$tmp/repo"
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
  CWD_PUSH_REPO="$repo"
}
