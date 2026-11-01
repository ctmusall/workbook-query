//! File fingerprints, so an export shows exactly which inputs it was made from.

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

/// SHA-256 of the file's bytes as lowercase hex, or None when the file can't be read.
pub fn file_sha256(path: impl AsRef<Path>) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
