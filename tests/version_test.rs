//! `--version` / `-V`: crates.io から入れたバイナリが、自分の版を言えること。
//! bleep(Bash)本体・プラグインと同じ版かを、人が確かめる入口になる。

use assert_cmd::Command;

fn expected() -> String {
    format!("bleep-hook {}\n", env!("CARGO_PKG_VERSION"))
}

#[test]
fn long_flag_prints_the_crate_version() {
    Command::cargo_bin("bleep-hook")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(expected());
}

#[test]
fn short_flag_prints_the_crate_version() {
    Command::cargo_bin("bleep-hook")
        .unwrap()
        .arg("-V")
        .assert()
        .success()
        .stdout(expected());
}
