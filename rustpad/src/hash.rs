//! Message digests for the Tools menu.

use serde::{Deserialize, Serialize};
use sha2::Digest;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum HashKind {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl HashKind {
    pub fn name(&self) -> &'static str {
        match self {
            HashKind::Md5 => "MD5",
            HashKind::Sha1 => "SHA-1",
            HashKind::Sha256 => "SHA-256",
            HashKind::Sha512 => "SHA-512",
        }
    }

    pub fn digest_hex(&self, data: &[u8]) -> String {
        let bytes: Vec<u8> = match self {
            HashKind::Md5 => md5::Md5::digest(data).to_vec(),
            HashKind::Sha1 => sha1::Sha1::digest(data).to_vec(),
            HashKind::Sha256 => sha2::Sha256::digest(data).to_vec(),
            HashKind::Sha512 => sha2::Sha512::digest(data).to_vec(),
        };
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(HashKind::Md5.digest_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(HashKind::Sha1.digest_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(HashKind::Sha256.digest_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(HashKind::Sha512.digest_hex(b"abc").len(), 128);
    }
}
