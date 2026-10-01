//! `bleep-hook intent` の適合テスト。tests/fixtures/intent/*.json の各ファイル
//! (`command` / `expect`)を実バイナリに通し、出力の JSON が
//! `expect` と一致することを確かめる。dotfiles 側の Rust 移植
//! (tarotene/dotfiles#415)も同じ fixture を共有して緑にする想定
//! (docs/adr/0002-gh-intent-and-layers.md)。fixture には架空の名前
//! (`pub/repo` 等)しか使わない(CONTRIBUTING.md)。

use assert_cmd::Command;
use serde_json::Value;
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/intent")
}

#[test]
fn intent_fixture_corpus() {
    let mut names: Vec<PathBuf> = std::fs::read_dir(fixtures_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    names.sort();
    assert!(names.len() >= 10, "fixture が少なすぎる: {}", names.len());

    for path in names {
        let fx: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let command = fx["command"].as_str().expect("command");
        let cwd = "/work"; // cwd は結果に影響しない(cd を追跡しない)
        let out = Command::cargo_bin("bleep-hook")
            .unwrap()
            .env("HOME", "/home/test")
            .args(["intent", "--cwd", cwd, "--", command])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let got: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(
            got,
            fx["expect"],
            "{}: {command}",
            path.file_name().unwrap().to_string_lossy()
        );
    }
}
