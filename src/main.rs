mod host;
mod lex;

use std::env;
use std::process::ExitCode;

fn usage() -> String {
    "\
usage: publish-guard-hook --host=<claude|codex|copilot>
       publish-guard-hook lex [--cwd DIR] -- CMD

--host=<name>   PreToolUse hook adapter モード。stdin から host 固有の JSON
                を読み、PUBLISH_GUARD_BIN(既定 \"publish-guard\")を呼び出して
                判定結果を host 固有の出力 JSON に翻訳する。
lex             publish-guard(Bash)本体の cmd_scan_bash_command から呼ばれる
                純粋な字句解析モード。I/O・gh は一切行わない。出力は5行の
                ヘッダ(found_push/push_dir/found_gh/gh_repo_override/
                gh_effective_dir)に続けて scan_subject を書く固定書式(jq 非
                依存 — 呼び出し側の publish-guard 本体を jq フリーに保つ)。
"
    .to_string()
}

fn escape_line(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn cmd_lex(cwd: &str, cmd: &str) {
    let r = lex::analyze(cmd, cwd);
    println!("{}", if r.found_push { "1" } else { "0" });
    println!("{}", escape_line(&r.push_dir));
    println!("{}", if r.found_gh { "1" } else { "0" });
    println!("{}", escape_line(&r.gh_repo_override));
    println!("{}", escape_line(&r.gh_effective_dir));
    print!("{}", r.scan_subject); // 既に各セグメント末尾に \n が付いている
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    if let Some(first) = args.first() {
        if let Some(name) = first.strip_prefix("--host=") {
            return match host::Host::parse(name) {
                Some(h) => {
                    let pg_bin = env::var("PUBLISH_GUARD_BIN")
                        .unwrap_or_else(|_| "publish-guard".to_string());
                    host::run(h, &pg_bin);
                    ExitCode::SUCCESS
                }
                None => {
                    eprintln!("unknown --host value: {name}\n\n{}", usage());
                    ExitCode::from(2)
                }
            };
        }
        if first == "--host" {
            let Some(name) = args.get(1) else {
                eprintln!("--host requires a value\n\n{}", usage());
                return ExitCode::from(2);
            };
            return match host::Host::parse(name) {
                Some(h) => {
                    let pg_bin = env::var("PUBLISH_GUARD_BIN")
                        .unwrap_or_else(|_| "publish-guard".to_string());
                    host::run(h, &pg_bin);
                    ExitCode::SUCCESS
                }
                None => {
                    eprintln!("unknown --host value: {name}\n\n{}", usage());
                    ExitCode::from(2)
                }
            };
        }
        if first == "lex" {
            let mut cwd = ".".to_string();
            let mut rest = &args[1..];
            if let Some(v) = rest.first() {
                if v == "--cwd" {
                    let Some(dir) = rest.get(1) else {
                        eprintln!("--cwd requires a value\n\n{}", usage());
                        return ExitCode::from(2);
                    };
                    cwd = dir.clone();
                    rest = &rest[2..];
                }
            }
            let rest = if rest.first().map(String::as_str) == Some("--") {
                &rest[1..]
            } else {
                rest
            };
            let Some(cmd) = rest.first() else {
                eprintln!("lex: missing CMD\n\n{}", usage());
                return ExitCode::from(2);
            };
            cmd_lex(&cwd, cmd);
            return ExitCode::SUCCESS;
        }
        if first == "--help" || first == "-h" {
            print!("{}", usage());
            return ExitCode::SUCCESS;
        }
    }

    eprint!("{}", usage());
    ExitCode::from(2)
}
