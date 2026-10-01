mod grammar;
mod hash;
mod host;
mod intent;
mod lex;

use std::env;
use std::process::ExitCode;

/// `lex` の出力形式の版。出力形式を変えたら上げる。bleep(Bash)本体の
/// `LEX_PROTOCOL` と一致しなければ、本体は ask にする(古いバイナリと新しい
/// 本体、またはその逆の組み合わせが、行のずれで黙って誤判定するのを防ぐ、
/// docs/adr/0002-gh-intent-and-layers.md)。版 1 = 6 行ヘッダのみ、
/// 版 2 = 先頭に `#lex <版>` 行、push_spec・本文の入力元(#54/#55)を追加、
/// 版 3 = push の対象解決(push_dir・push_spec)を外し、pre-push を迂回する形
/// (push_bypass)に置き換え(docs/adr/0003-constructive-grammar.md)、
/// 版 4 = gh の投稿を正準形の認識器(grammar.rs)の結果に置き換え。投稿ごとに
/// 宛先・違反コード・本文の入力元を返し、cd 追跡・unresolved_var・
/// gh_effective_dir を廃止。
const LEX_PROTOCOL: u32 = 4;

fn usage() -> String {
    "\
usage: bleep-hook --host=<claude|codex|copilot>
       bleep-hook lex [--cwd DIR] -- CMD
       bleep-hook intent [--cwd DIR] -- CMD
       bleep-hook --protocol
       bleep-hook hash --key-file FILE -- TERM

--host=<name>   PreToolUse hook adapter モード。stdin から host 固有の JSON
                を読み、BLEEP_BIN(既定 \"bleep\")を呼び出して
                判定結果を host 固有の出力 JSON に翻訳する。
--protocol      lex の出力形式の版(整数)を1行印字する。`bleep doctor` が
                bash 本体との整合の検査に使う。
lex             bleep(Bash)本体の cmd_scan_bash_command から呼ばれる
                純粋な字句解析モード。I/O・gh は一切行わない。出力は
                先頭の `#lex <版>` 行(--protocol と同じ版)に続けて、
                found_push、push_bypass(`git push` が pre-push を無効にする
                `--no-verify` / `-c core.hooksPath=` の形か)、gh の投稿の件数
                N、投稿ごとに(違反コード・宛先の owner/repo・本文の入力元の
                件数 M・M 行の絶対パス)、に続けて scan_subject を書く固定書式
                (jq 非依存 — 呼び出し側の bleep 本体を jq フリーに保つ)。
                違反コードは正準形を外れた最初の理由(空なら正準形)、宛先は
                リテラルで取れたときだけ(空なら静的に決められない)。
                `--cwd` は受け取るが使わない(cd の追跡をしないため)。
intent          gh の投稿コマンドから「投稿の意図」(面・操作・宛先・本文の
                入力元パス・正準形かどうかと違反コード)を取り出し、投稿
                ごとの JSON 配列を stdout に書く(#56)。lex と同じく I/O・gh は
                一切行わない。書式と適合 fixture は src/intent.rs と
                tests/fixtures/intent/。
hash            bleep(Bash)本体の判定レッジャー(ledger_write)から呼ばれる。
                FILE に保存された鍵(無ければ新規生成)で TERM を
                HMAC-SHA256 し、hex を1行 stdout に印字する。
"
    .to_string()
}

fn escape_line(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn cmd_intent(cmd: &str) {
    println!("{}", intent::intents(cmd));
}

/// `lex` / `intent` 共通の引数解析: `[--cwd DIR] [--] CMD`。`--cwd` は読み飛ばす
/// だけ(cd の追跡をやめたので、実効ディレクトリを使わない。bleep 本体が付けて
/// 呼ぶ形を変えないために受け取る)。
fn parse_cwd_and_cmd(name: &str, args: &[String]) -> Result<String, ExitCode> {
    let mut rest = args;
    if let Some(v) = rest.first() {
        if v == "--cwd" {
            if rest.get(1).is_none() {
                eprintln!("--cwd requires a value\n\n{}", usage());
                return Err(ExitCode::from(2));
            }
            rest = &rest[2..];
        }
    }
    let rest = if rest.first().map(String::as_str) == Some("--") {
        &rest[1..]
    } else {
        rest
    };
    let Some(cmd) = rest.first() else {
        eprintln!("{name}: missing CMD\n\n{}", usage());
        return Err(ExitCode::from(2));
    };
    Ok(cmd.clone())
}

fn cmd_lex(cmd: &str) {
    let r = lex::analyze(cmd);
    println!("#lex {LEX_PROTOCOL}");
    println!("{}", if r.found_push { "1" } else { "0" });
    println!("{}", if r.push_bypass { "1" } else { "0" });
    println!("{}", r.posts.len());
    for p in &r.posts {
        println!("{}", p.noncanonical.unwrap_or(""));
        println!("{}", escape_line(p.repo.as_deref().unwrap_or("")));
        println!("{}", p.body_sources.len());
        for path in &p.body_sources {
            println!("{}", escape_line(path));
        }
    }
    print!("{}", r.scan_subject); // 既に各セグメント末尾に \n が付いている
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    if let Some(first) = args.first() {
        if let Some(name) = first.strip_prefix("--host=") {
            return match host::Host::parse(name) {
                Some(h) => {
                    let pg_bin = env::var("BLEEP_BIN").unwrap_or_else(|_| "bleep".to_string());
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
                    let pg_bin = env::var("BLEEP_BIN").unwrap_or_else(|_| "bleep".to_string());
                    host::run(h, &pg_bin);
                    ExitCode::SUCCESS
                }
                None => {
                    eprintln!("unknown --host value: {name}\n\n{}", usage());
                    ExitCode::from(2)
                }
            };
        }
        if first == "lex" || first == "intent" {
            let cmd = match parse_cwd_and_cmd(first, &args[1..]) {
                Ok(v) => v,
                Err(code) => return code,
            };
            if first == "lex" {
                cmd_lex(&cmd);
            } else {
                cmd_intent(&cmd);
            }
            return ExitCode::SUCCESS;
        }
        if first == "hash" {
            let mut rest = &args[1..];
            let Some(flag) = rest.first() else {
                eprintln!("hash: missing --key-file\n\n{}", usage());
                return ExitCode::from(2);
            };
            if flag != "--key-file" {
                eprintln!("hash: expected --key-file\n\n{}", usage());
                return ExitCode::from(2);
            }
            let Some(key_file) = rest.get(1) else {
                eprintln!("--key-file requires a value\n\n{}", usage());
                return ExitCode::from(2);
            };
            rest = &rest[2..];
            let rest = if rest.first().map(String::as_str) == Some("--") {
                &rest[1..]
            } else {
                rest
            };
            let Some(term) = rest.first() else {
                eprintln!("hash: missing TERM\n\n{}", usage());
                return ExitCode::from(2);
            };
            return ExitCode::from(hash::run(key_file, term) as u8);
        }
        if first == "--protocol" {
            println!("{LEX_PROTOCOL}");
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
