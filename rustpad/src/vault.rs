//! Hidden encrypted documents.
//!
//! A protected file is an ordinary text file: a *decoy* text anyone can open and edit, followed by an
//! encrypted payload written as invisible zero-width characters. RustPad shows only the decoy until the
//! right password is given; then it swaps in the real text, and every save re-encrypts it. Locking, closing
//! or restarting brings the decoy back.
//!
//! Layout of the payload (the "blob", no magic bytes so it looks like noise):
//! `salt (16) | nonce (24) | XChaCha20-Poly1305(len u32 LE | text | zero padding to a multiple of 512)`.
//! The key is Argon2id(password, salt). Each byte of the blob becomes four zero-width characters (2 bits
//! each), so the file grows about 12x the size of the hidden text. The payload is the maximal run of
//! those characters at the very end of the file, which is why they are stripped from the end of the decoy.

use std::rc::Rc;

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use crate::doc::Doc;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
/// Smallest possible blob: salt + nonce + a tag over an empty message.
const MIN_BLOB: usize = SALT_LEN + NONCE_LEN + TAG_LEN;
/// The hidden text is padded to a multiple of this, so the file does not leak its exact length.
const PAD: usize = 512;
/// 2 bits per character, high bits first.
const ZW: [char; 4] = ['\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}'];

/// Argon2id cost: 64 MiB, 3 passes, one lane (about a third of a second on a laptop).
const KDF_MEM_KIB: u32 = 64 * 1024;
const KDF_PASSES: u32 = 3;

#[derive(Clone)]
pub struct Key(Zeroizing<[u8; 32]>);

/// A document that is currently showing its real text.
#[derive(Clone)]
pub struct Unlocked {
    pub key: Key,
    pub salt: [u8; SALT_LEN],
    /// What the file shows to everyone else; written back in front of the payload on each save.
    pub decoy: String,
}

#[derive(Default, Clone)]
pub enum Vault {
    /// An ordinary document.
    #[default]
    None,
    /// The tab shows the decoy; this is the encrypted payload that follows it in the file.
    Locked(Vec<u8>),
    /// The tab shows the real text.
    Unlocked(Box<Unlocked>),
}

impl Vault {
    pub fn is_none(&self) -> bool {
        matches!(self, Vault::None)
    }
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("operating system random number generator");
    b
}

pub fn derive(password: &str, salt: &[u8; SALT_LEN]) -> Key {
    let params = Params::new(KDF_MEM_KIB, KDF_PASSES, 1, Some(32)).expect("valid Argon2 parameters");
    let mut out = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, out.as_mut())
        .expect("Argon2 with a fixed salt length");
    Key(out)
}

/// A fresh salt and the key for a new password.
pub fn new_key(password: &str) -> (Key, [u8; SALT_LEN]) {
    let salt = random::<SALT_LEN>();
    (derive(password, &salt), salt)
}

/// Encrypt `text` under an already derived key (a fresh nonce each time).
pub fn seal(key: &Key, salt: &[u8; SALT_LEN], text: &str) -> Vec<u8> {
    let mut plain = Zeroizing::new(Vec::with_capacity(4 + text.len() + PAD));
    plain.extend_from_slice(&(text.len() as u32).to_le_bytes());
    plain.extend_from_slice(text.as_bytes());
    let padded = plain.len().div_ceil(PAD) * PAD;
    plain.resize(padded, 0);
    let nonce = random::<NONCE_LEN>();
    let cipher = XChaCha20Poly1305::new((&*key.0).into());
    let ct = cipher.encrypt(&XNonce::from(nonce), plain.as_slice()).expect("encryption cannot fail");
    let mut blob = Vec::with_capacity(SALT_LEN + NONCE_LEN + ct.len());
    blob.extend_from_slice(salt);
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ct);
    blob
}

/// Decrypt a blob. `None` for a wrong password or a damaged blob (deliberately not told apart).
pub fn open(password: &str, blob: &[u8]) -> Option<(String, Key, [u8; SALT_LEN])> {
    if blob.len() < MIN_BLOB {
        return None;
    }
    let salt: [u8; SALT_LEN] = blob[..SALT_LEN].try_into().ok()?;
    let nonce = XNonce::try_from(&blob[SALT_LEN..SALT_LEN + NONCE_LEN]).ok()?;
    let key = derive(password, &salt);
    let cipher = XChaCha20Poly1305::new((&*key.0).into());
    let plain = Zeroizing::new(cipher.decrypt(&nonce, &blob[SALT_LEN + NONCE_LEN..]).ok()?);
    let len = u32::from_le_bytes(plain.get(..4)?.try_into().ok()?) as usize;
    let text = std::str::from_utf8(plain.get(4..4usize.checked_add(len)?)?).ok()?.to_string();
    Some((text, key, salt))
}

fn is_zw(c: char) -> bool {
    ZW.contains(&c)
}

/// Split file text into the visible decoy and the payload hidden after it, if there is one.
pub fn split(text: &str) -> (&str, Option<Vec<u8>>) {
    let tail_chars = text.chars().rev().take_while(|c| is_zw(*c)).count();
    if tail_chars % 4 != 0 || tail_chars / 4 < MIN_BLOB {
        return (text, None);
    }
    // every zero-width character in the alphabet is 3 bytes in UTF-8
    let (decoy, tail) = text.split_at(text.len() - tail_chars * 3);
    let mut blob = Vec::with_capacity(tail_chars / 4);
    let mut cur = 0u8;
    for (i, c) in tail.chars().enumerate() {
        let bits = ZW.iter().position(|z| *z == c).unwrap_or(0) as u8;
        cur = cur << 2 | bits;
        if i % 4 == 3 {
            blob.push(cur);
            cur = 0;
        }
    }
    (decoy, Some(blob))
}

/// The file text for a decoy and its payload.
pub fn embed(decoy: &str, blob: &[u8]) -> String {
    let decoy = decoy.trim_end_matches(is_zw);
    let mut out = String::with_capacity(decoy.len() + blob.len() * 12);
    out.push_str(decoy);
    for b in blob {
        for shift in [6, 4, 2, 0] {
            out.push(ZW[(b >> shift & 3) as usize]);
        }
    }
    out
}

impl Doc {
    /// The text to write to disk (or to a session backup): for protected documents the decoy plus the
    /// encrypted payload, never the real text.
    pub fn file_text(&self) -> Rc<String> {
        match &self.vault {
            Vault::None => self.text_string(),
            Vault::Locked(blob) => Rc::new(embed(&self.text_string(), blob)),
            Vault::Unlocked(u) => Rc::new(embed(&u.decoy, &seal(&u.key, &u.salt, &self.text_string()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_padding() {
        let (key, salt) = new_key("hunter2");
        for text in ["", "secret", "multi\r\nline ünïcode 🔥\n", &"x".repeat(5000)] {
            let blob = seal(&key, &salt, text);
            assert_eq!((blob.len() - SALT_LEN - NONCE_LEN - TAG_LEN) % PAD, 0);
            let (back, _, s2) = open("hunter2", &blob).expect("right password opens");
            assert_eq!(back, text);
            assert_eq!(s2, salt);
        }
    }

    #[test]
    fn wrong_password_and_tampering_fail() {
        let (key, salt) = new_key("right");
        let mut blob = seal(&key, &salt, "secret");
        assert!(open("wrong", &blob).is_none());
        assert!(open("right", &blob).is_some());
        let last = blob.len() - 1;
        blob[last] ^= 1;
        assert!(open("right", &blob).is_none());
        assert!(open("right", &blob[..10]).is_none());
    }

    #[test]
    fn fresh_nonce_each_seal() {
        let (key, salt) = new_key("pw");
        assert_ne!(seal(&key, &salt, "same"), seal(&key, &salt, "same"));
    }

    #[test]
    fn embed_split_roundtrip() {
        let blob: Vec<u8> = (0..=255u8).cycle().take(200).collect();
        let file = embed("Shopping list:\n- milk\n", &blob);
        let (decoy, back) = split(&file);
        assert_eq!(decoy, "Shopping list:\n- milk\n");
        assert_eq!(back.as_deref(), Some(blob.as_slice()));
        // the payload is invisible: nothing but the decoy shows up once zero-width characters are removed
        assert_eq!(file.chars().filter(|c| !is_zw(*c)).collect::<String>(), decoy);
    }

    #[test]
    fn decoy_ending_in_zero_width_chars_is_not_confused_with_payload() {
        let blob = vec![0xAB; MIN_BLOB];
        let file = embed("hello\u{200B}\u{200C}\u{200D}", &blob);
        let (decoy, back) = split(&file);
        assert_eq!(decoy, "hello");
        assert_eq!(back.unwrap(), blob);
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        assert_eq!(split("plain text\n"), ("plain text\n", None));
        // a short or misaligned run of zero-width characters is just text
        let few = format!("a{}", "\u{200B}".repeat(8));
        assert_eq!(split(&few), (few.as_str(), None));
        let odd = format!("a{}", "\u{200B}".repeat(MIN_BLOB * 4 + 1));
        assert_eq!(split(&odd), (odd.as_str(), None));
    }

    #[test]
    fn full_file_cycle_through_a_doc() {
        use crate::doc::Eol;
        use crate::encoding::Enc;
        let (key, salt) = new_key("pw");
        let mut d = Doc::from_text("the real text", Enc::Utf8, Eol::Lf);
        d.vault = Vault::Unlocked(Box::new(Unlocked { key, salt, decoy: "just a note\n".into() }));
        let file = d.file_text();
        assert!(file.starts_with("just a note\n"));
        assert!(!file.contains("real"));
        let (decoy, blob) = split(&file);
        assert_eq!(decoy, "just a note\n");
        let (real, ..) = open("pw", &blob.unwrap()).unwrap();
        assert_eq!(real, "the real text");
    }
}
