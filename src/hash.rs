//! `bleep-hook hash --key-file F -- TERM` — HMAC-SHA256(key, TERM) の hex を
//! 出力する。判定レッジャー(bash 本体 `bleep` の `ledger_write`)がマッチ語を
//! 平文のまま書かないための唯一のハッシュ経路(dotfiles 側 ADR
//! 「先行例との対比」D9)。
//!
//! 鍵ファイルが無ければこのプロセスが 0600 で新規生成する。複数プロセスが
//! 同時に初回呼び出しをしても、`O_EXCL` で衝突を検出し、負けた側は
//! 生成済みの鍵を読み直す(鍵の作成に排他ロックは要らない — 読み直しで
//! 収束する)。

use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

type HmacSha256 = Hmac<Sha256>;

const KEY_LEN: usize = 32;

fn read_or_create_key(path: &Path) -> io::Result<Vec<u8>> {
    if let Ok(mut f) = fs::File::open(path) {
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        if !buf.is_empty() {
            return Ok(buf);
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }
    let mut key = vec![0u8; KEY_LEN];
    // 依存を増やさないため、乱数源を /dev/urandom から直接読む(unix 限定 —
    // README は Linux/macOS を前提にしている。bleep 本体の bash 実装も
    // 同様に POSIX 環境を前提にしている)。
    {
        let mut urandom = fs::File::open("/dev/urandom")?;
        urandom.read_exact(&mut key)?;
    }

    let mut open_opts = OpenOptions::new();
    open_opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open_opts.mode(0o600);
    }

    // O_EXCL(create_new)で衝突を検出する: 複数プロセスが同時に初回呼び出し
    // をしても、負けた側は AlreadyExists を受けて鍵を読み直すだけで済む
    // (排他ロックは要らない)。
    match open_opts.open(path) {
        Ok(mut f) => {
            f.write_all(&key)?;
            Ok(key)
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let mut f = fs::File::open(path)?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            Ok(buf)
        }
        Err(e) => Err(e),
    }
}

/// $1=key_file $2=term -> exit code(0=成功、1=失敗)。stdout に hex64 を印字。
pub fn run(key_file: &str, term: &str) -> i32 {
    let key = match read_or_create_key(Path::new(key_file)) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("bleep-hook hash: 鍵の読み書きに失敗しました: {e}");
            return 1;
        }
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(&key) else {
        eprintln!("bleep-hook hash: HMAC の初期化に失敗しました。");
        return 1;
    };
    mac.update(term.as_bytes());
    let result = mac.finalize().into_bytes();
    let hex: String = result.iter().map(|b| format!("{b:02x}")).collect();
    println!("{hex}");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_for_same_key_and_term() {
        let dir = tempfile::tempdir().unwrap();
        let key_file = dir.path().join("hmac-key");
        let key = read_or_create_key(&key_file).unwrap();
        assert_eq!(key.len(), KEY_LEN);

        let mut mac1 = HmacSha256::new_from_slice(&key).unwrap();
        mac1.update(b"acme/secret-project");
        let h1 = mac1.finalize().into_bytes();

        // 同じ鍵ファイルを読み直しても同じ鍵が返る(生成は初回のみ)。
        let key2 = read_or_create_key(&key_file).unwrap();
        assert_eq!(key, key2);
        let mut mac2 = HmacSha256::new_from_slice(&key2).unwrap();
        mac2.update(b"acme/secret-project");
        let h2 = mac2.finalize().into_bytes();
        assert_eq!(h1, h2);
    }

    #[test]
    fn different_terms_hash_differently() {
        let dir = tempfile::tempdir().unwrap();
        let key_file = dir.path().join("hmac-key");
        let key = read_or_create_key(&key_file).unwrap();
        let mut mac_a = HmacSha256::new_from_slice(&key).unwrap();
        mac_a.update(b"acme/secret-project");
        let a = mac_a.finalize().into_bytes();
        let mut mac_b = HmacSha256::new_from_slice(&key).unwrap();
        mac_b.update(b"other-repo");
        let b = mac_b.finalize().into_bytes();
        assert_ne!(a, b);
    }

    #[cfg(unix)]
    #[test]
    fn key_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let key_file = dir.path().join("hmac-key");
        read_or_create_key(&key_file).unwrap();
        let mode = fs::metadata(&key_file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
