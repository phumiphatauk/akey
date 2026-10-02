//! HOTP (RFC 4226) and TOTP (RFC 6238).

use hmac::{Hmac, KeyInit, Mac};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

impl Algorithm {
    pub fn name(self) -> &'static str {
        match self {
            Algorithm::Sha1 => "SHA1",
            Algorithm::Sha256 => "SHA256",
            Algorithm::Sha512 => "SHA512",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_uppercase().as_str() {
            "SHA1" => Some(Algorithm::Sha1),
            "SHA256" => Some(Algorithm::Sha256),
            "SHA512" => Some(Algorithm::Sha512),
            _ => None,
        }
    }
}

/// Runs `$body` with `$mac` bound to an HMAC instance of the chosen hash.
macro_rules! with_hmac {
    ($alg:expr, $key:expr, |$mac:ident| $body:expr) => {
        match $alg {
            Algorithm::Sha1 => {
                let mut $mac = Hmac::<sha1::Sha1>::new_from_slice($key).expect("HMAC accepts any key length");
                $body
            }
            Algorithm::Sha256 => {
                let mut $mac =
                    Hmac::<sha2::Sha256>::new_from_slice($key).expect("HMAC accepts any key length");
                $body
            }
            Algorithm::Sha512 => {
                let mut $mac =
                    Hmac::<sha2::Sha512>::new_from_slice($key).expect("HMAC accepts any key length");
                $body
            }
        }
    };
}

/// HOTP value for `counter`, truncated to `digits` decimal digits (RFC 4226 §5.3).
pub fn hotp(algorithm: Algorithm, key: &[u8], counter: u64, digits: u32) -> u32 {
    let mut digest = [0u8; 64];
    let len = with_hmac!(algorithm, key, |mac| {
        mac.update(&counter.to_be_bytes());
        let out = mac.finalize().into_bytes();
        digest[..out.len()].copy_from_slice(&out);
        out.len()
    });
    let hash = &digest[..len];

    let offset = (hash[len - 1] & 0x0f) as usize;
    let binary = u32::from_be_bytes([hash[offset], hash[offset + 1], hash[offset + 2], hash[offset + 3]])
        & 0x7fff_ffff;
    digest.fill(0);
    binary % 10u32.pow(digits)
}

/// TOTP value at `unix_time` seconds (RFC 6238 §4.2).
pub fn totp(algorithm: Algorithm, key: &[u8], digits: u32, period: u64, unix_time: u64) -> u32 {
    hotp(algorithm, key, unix_time / period, digits)
}

/// Zero-pads `code` and splits it in two groups for readability, e.g. "012 345".
pub fn format_code(code: u32, digits: u32) -> String {
    let s = format!("{code:0width$}", width = digits as usize);
    let (a, b) = s.split_at(s.len() / 2);
    format!("{a} {b}")
}

/// Zero-pads `code` and groups it in pairs, MiniKey style ("07 66 84"). An odd digit count
/// ends with a group of three ("12 34 567").
pub fn format_code_pairs(code: u32, digits: u32) -> String {
    let s = format!("{code:0width$}", width = digits as usize);
    let mut out = String::with_capacity(s.len() * 3 / 2);
    let mut rest = s.as_str();
    while !rest.is_empty() {
        let take = if rest.len() == 3 { 3 } else { rest.len().min(2) };
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&rest[..take]);
        rest = &rest[take..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 6238 Appendix B seeds: the ASCII string "1234567890" repeated to the hash's block length.
    const SEED_SHA1: &[u8] = b"12345678901234567890";
    const SEED_SHA256: &[u8] = b"12345678901234567890123456789012";
    const SEED_SHA512: &[u8] = b"1234567890123456789012345678901234567890123456789012345678901234";

    #[test]
    fn rfc6238_test_vectors() {
        let vectors: &[(u64, u32, u32, u32)] = &[
            // (time, SHA1, SHA256, SHA512)
            (59, 94287082, 46119246, 90693936),
            (1111111109, 7081804, 68084774, 25091201),
            (1111111111, 14050471, 67062674, 99943326),
            (1234567890, 89005924, 91819424, 93441116),
            (2000000000, 69279037, 90698825, 38618901),
            (20000000000, 65353130, 77737706, 47863826),
        ];
        for &(t, sha1, sha256, sha512) in vectors {
            assert_eq!(totp(Algorithm::Sha1, SEED_SHA1, 8, 30, t), sha1, "SHA1 at {t}");
            assert_eq!(
                totp(Algorithm::Sha256, SEED_SHA256, 8, 30, t),
                sha256,
                "SHA256 at {t}"
            );
            assert_eq!(
                totp(Algorithm::Sha512, SEED_SHA512, 8, 30, t),
                sha512,
                "SHA512 at {t}"
            );
        }
    }

    #[test]
    fn rfc4226_test_vectors() {
        let expected = [
            755224, 287082, 359152, 969429, 338314, 254676, 287922, 162583, 399871, 520489,
        ];
        for (counter, &code) in expected.iter().enumerate() {
            assert_eq!(hotp(Algorithm::Sha1, SEED_SHA1, counter as u64, 6), code);
        }
    }

    #[test]
    fn period_changes_counter() {
        // With a 60s period, t=59 and t=0 share counter 0; t=60 is counter 1.
        let a = totp(Algorithm::Sha1, SEED_SHA1, 6, 60, 0);
        assert_eq!(totp(Algorithm::Sha1, SEED_SHA1, 6, 60, 59), a);
        assert_eq!(
            totp(Algorithm::Sha1, SEED_SHA1, 6, 60, 60),
            hotp(Algorithm::Sha1, SEED_SHA1, 1, 6)
        );
    }

    #[test]
    fn formats_codes() {
        assert_eq!(format_code(123456, 6), "123 456");
        assert_eq!(format_code(7081804, 8), "0708 1804");
        assert_eq!(format_code(42, 6), "000 042");
        assert_eq!(format_code(1234567, 7), "123 4567");
        assert_eq!(format_code_pairs(76684, 6), "07 66 84");
        assert_eq!(format_code_pairs(1234567, 7), "12 34 567");
        assert_eq!(format_code_pairs(7081804, 8), "07 08 18 04");
    }

    #[test]
    fn algorithm_names() {
        assert_eq!(Algorithm::from_name("sha256"), Some(Algorithm::Sha256));
        assert_eq!(Algorithm::from_name("MD5"), None);
        assert_eq!(Algorithm::Sha512.name(), "SHA512");
    }
}
