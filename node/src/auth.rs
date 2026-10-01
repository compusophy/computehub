//! Admission: the pairing token and the Origin allowlist.
//!
//! The node listens on loopback only, so the threats are other local
//! programs and web pages the user has open. Pages are stopped by the Origin
//! allowlist at the handshake; everything else by the 256-bit token, compared
//! in constant time. There is no lockout: anyone may send wrong tokens, so a
//! shared one would let them lock out the user (see `session`).

use crate::proto::TOKEN_LEN;
use std::fmt;

/// The pairing secret: 32 random bytes, shown once as 64 lowercase hex digits
/// in the pairing link. `Debug` never prints it.
pub struct Token([u8; TOKEN_LEN]);

impl Token {
    /// A fresh token from the operating system's random source.
    pub fn generate() -> Result<Token, String> {
        let mut bytes = [0u8; TOKEN_LEN];
        getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
        Ok(Token(bytes))
    }

    /// A token with known bytes (tests, and callers that bring their own).
    pub fn from_bytes(bytes: [u8; TOKEN_LEN]) -> Token {
        Token(bytes)
    }

    /// The 64-digit lowercase hex form used in the pairing link.
    pub fn hex(&self) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut s = String::with_capacity(TOKEN_LEN * 2);
        for b in self.0 {
            s.push(char::from(DIGITS[usize::from(b >> 4)]));
            s.push(char::from(DIGITS[usize::from(b & 0xf)]));
        }
        s
    }

    /// Whether `candidate` is this token, in time independent of where they
    /// differ.
    pub fn matches(&self, candidate: &[u8]) -> bool {
        ct_eq(&self.0, candidate)
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(..)")
    }
}

/// Constant-time equality: every byte is visited whatever the contents. The
/// lengths are not secret, so unequal lengths return early.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    std::hint::black_box(diff) == 0
}

/// The browser origins allowed to open a WebSocket to the node. A missing
/// or unlisted `Origin` header is refused with HTTP 403 at the handshake. It
/// starts empty: nothing is trusted unless added (the node adds its page).
#[derive(Debug, Clone, Default)]
pub struct Origins(Vec<String>);

impl Origins {
    /// Adds `origin` (`scheme://host[:port]`, http or https); a trailing `/`
    /// is dropped and duplicates are ignored.
    pub fn add(&mut self, origin: &str) -> Result<(), String> {
        let o = normalize_origin(origin)
            .ok_or_else(|| format!("not an origin (want scheme://host[:port]): {origin}"))?;
        if !self.0.contains(&o) {
            self.0.push(o);
        }
        Ok(())
    }

    /// Whether a handshake carrying this `Origin` header may proceed.
    pub fn allows(&self, header: Option<&str>) -> bool {
        header.is_some_and(|h| self.0.iter().any(|o| o.eq_ignore_ascii_case(h)))
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }
}

/// `scheme://authority` in lowercase, or `None` if `s` is not an http(s)
/// origin. One trailing `/` is tolerated.
pub fn normalize_origin(s: &str) -> Option<String> {
    let s = s.trim();
    let s = s.strip_suffix('/').unwrap_or(s).to_ascii_lowercase();
    let (scheme, authority) = s.split_once("://")?;
    let bad = |c: char| matches!(c, '/' | '?' | '#' | '@' | '\\') || c.is_whitespace();
    let ok =
        matches!(scheme, "http" | "https") && !authority.is_empty() && !authority.contains(bad);
    ok.then_some(s)
}

/// The origin of an http(s) URL: everything before the path.
pub fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    normalize_origin(&format!("{scheme}://{}", &rest[..end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_hex_is_64_lowercase_digits() {
        let mut bytes = [0u8; TOKEN_LEN];
        bytes[0] = 0xab;
        bytes[31] = 0x0f;
        let hex = Token::from_bytes(bytes).hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.starts_with("ab00"));
        assert!(hex.ends_with("000f"));
        let fresh = Token::generate().expect("random source").hex();
        assert_eq!(fresh.len(), 64);
        assert!(fresh.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
        assert_ne!(fresh, Token::generate().expect("random source").hex());
        assert_eq!(format!("{:?}", Token::from_bytes(bytes)), "Token(..)");
    }

    #[test]
    fn constant_time_compare() {
        let a = [7u8; TOKEN_LEN];
        assert!(ct_eq(&a, &a));
        assert!(ct_eq(&[], &[]));
        for i in [0, 15, 31] {
            let mut b = a;
            b[i] ^= 0x80;
            assert!(!ct_eq(&a, &b), "byte {i}");
        }
        assert!(!ct_eq(&a, &a[..31]));
        assert!(!ct_eq(&a[..31], &a));
        let t = Token::from_bytes(a);
        assert!(t.matches(&a));
        assert!(!t.matches(&[7u8; 33]));
        assert!(!t.matches(&[0u8; TOKEN_LEN]));
    }

    #[test]
    fn origin_allowlist() {
        let mut o = Origins::default();
        assert!(!o.allows(Some("http://localhost:8080")), "trusted before it was added");
        o.add("http://localhost:8080").expect("valid");
        assert!(o.allows(Some("http://localhost:8080")));
        assert!(o.allows(Some("HTTP://LOCALHOST:8080")));
        for bad in [
            "https://evil.example",
            "http://localhost:8081",
            "http://localhost",
            "https://localhost:8080",
            "http://127.0.0.1:8080",
            "http://localhost:8080.evil.example",
            "null",
            "",
        ] {
            assert!(!o.allows(Some(bad)), "{bad}");
        }
        assert!(!o.allows(None));
        o.add("http://localhost:3000/").expect("valid");
        o.add("http://localhost:3000").expect("valid");
        assert!(o.allows(Some("http://localhost:3000")));
        assert_eq!(o.iter().count(), 2);
        for bad in ["localhost:3000", "ftp://x", "http://", "http://a/b", "http://u@h", "*", "null"]
        {
            assert!(o.add(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn origin_of_a_url() {
        let o = |u| origin_of(u);
        assert_eq!(
            o("https://computehub-sigma.vercel.app"),
            Some("https://computehub-sigma.vercel.app".into())
        );
        assert_eq!(o("http://localhost:8080/os/#x"), Some("http://localhost:8080".into()));
        assert_eq!(o("HTTPS://Example.com?q"), Some("https://example.com".into()));
        assert_eq!(o("example.com"), None);
        assert_eq!(o("file:///c/x"), None);
    }
}
