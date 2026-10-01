//! `lex` の出力形式の版(bleep 本体の LEX_PROTOCOL と揃える)。版を上げたら
//! bleep の LEX_PROTOCOL と、この期待値を一緒に変える。

use assert_cmd::Command;

const PROTOCOL: &str = "4";

#[test]
fn protocol_flag_prints_the_version() {
    Command::cargo_bin("bleep-hook")
        .unwrap()
        .arg("--protocol")
        .assert()
        .success()
        .stdout(format!("{PROTOCOL}\n"));
}

#[test]
fn lex_output_starts_with_the_version_header() {
    let out = Command::cargo_bin("bleep-hook")
        .unwrap()
        .args(["lex", "--cwd", "/work", "--", "echo hi"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(
        text.lines().next(),
        Some(format!("#lex {PROTOCOL}").as_str())
    );
}

#[test]
fn bleep_body_and_binary_agree_on_the_version() {
    let script = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/bleep")).unwrap();
    assert!(
        script
            .lines()
            .any(|l| l == format!("LEX_PROTOCOL={PROTOCOL}")),
        "bleep の LEX_PROTOCOL が {PROTOCOL} と一致しない"
    );
}
