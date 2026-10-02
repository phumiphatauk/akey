//! Parsing of Base32 secrets and `otpauth://totp/...` URIs, plus the canonical
//! serialization stored in the Keychain.
//!
//! Error messages never include the input, so secrets cannot leak into a terminal or log.

use std::fmt;

use crate::totp::{self, Algorithm};

pub struct Account {
    pub secret: Vec<u8>,
    pub algorithm: Algorithm,
    pub digits: u32,
    pub period: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    InvalidBase32,
    EmptySecret,
    MissingSecret,
    NotTotp,
    InvalidUri,
    UnsupportedAlgorithm,
    InvalidDigits,
    InvalidPeriod,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ParseError::InvalidBase32 => "secret is not valid Base32",
            ParseError::EmptySecret => "secret is empty",
            ParseError::MissingSecret => "otpauth URI has no secret parameter",
            ParseError::NotTotp => "only otpauth://totp/ URIs are supported",
            ParseError::InvalidUri => "malformed otpauth URI",
            ParseError::UnsupportedAlgorithm => "algorithm must be SHA1, SHA256 or SHA512",
            ParseError::InvalidDigits => "digits must be 6, 7 or 8",
            ParseError::InvalidPeriod => "period must be between 1 and 86400 seconds",
        })
    }
}

impl std::error::Error for ParseError {}

impl fmt::Debug for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Account")
            .field("secret", &"<redacted>")
            .field("algorithm", &self.algorithm)
            .field("digits", &self.digits)
            .field("period", &self.period)
            .finish()
    }
}

impl Drop for Account {
    fn drop(&mut self) {
        wipe(&mut self.secret);
    }
}

/// Best-effort zeroing of secret material before it is freed.
pub fn wipe(bytes: &mut [u8]) {
    bytes.fill(0);
    std::hint::black_box(bytes);
}

impl Account {
    /// Accepts either a bare Base32 secret (SHA1 / 6 digits / 30 s) or an `otpauth://totp/` URI.
    pub fn parse(input: &str) -> Result<Account, ParseError> {
        let input = input.trim();
        if let Some(rest) = strip_scheme(input) {
            parse_uri(rest)
        } else {
            Ok(Account {
                secret: decode_secret(input)?,
                algorithm: Algorithm::Sha1,
                digits: 6,
                period: 30,
            })
        }
    }

    /// Canonical form stored in the Keychain.
    pub fn to_uri(&self) -> String {
        format!(
            "otpauth://totp/akey?secret={}&algorithm={}&digits={}&period={}",
            base32_encode(&self.secret),
            self.algorithm.name(),
            self.digits,
            self.period
        )
    }

    pub fn code_at(&self, unix_time: u64) -> u32 {
        totp::totp(self.algorithm, &self.secret, self.digits, self.period, unix_time)
    }

    pub fn formatted_code_at(&self, unix_time: u64) -> String {
        totp::format_code(self.code_at(unix_time), self.digits)
    }
}

/// A display name for an otpauth URI: its `issuer` parameter, else the issuer prefix of the
/// label (`Issuer:account`), else the whole label. `None` for bare secrets.
pub fn suggested_name(input: &str) -> Option<String> {
    let (_, rest) = strip_scheme(input.trim())?.split_once('/')?;
    let (label, query) = rest.split_once('?').unwrap_or((rest, ""));
    let issuer = query
        .split('#')
        .next()
        .unwrap_or("")
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| key.eq_ignore_ascii_case("issuer"))
        .and_then(|(_, value)| percent_decode(value).ok());
    let from_label = || {
        let label = percent_decode(label).ok()?;
        Some(label.split(':').next()?.to_string())
    };
    [issuer, from_label()]
        .into_iter()
        .flatten()
        .map(|name| name.trim().to_string())
        .find(|name| !name.is_empty())
}

fn strip_scheme(input: &str) -> Option<&str> {
    let scheme = input.get(..10)?;
    scheme.eq_ignore_ascii_case("otpauth://").then(|| &input[10..])
}

fn parse_uri(rest: &str) -> Result<Account, ParseError> {
    let (kind, rest) = rest.split_once('/').ok_or(ParseError::InvalidUri)?;
    if !kind.eq_ignore_ascii_case("totp") {
        return Err(ParseError::NotTotp);
    }
    let query = rest.split_once('?').map(|(_, q)| q).unwrap_or("");
    let query = query.split('#').next().unwrap_or("");

    let mut secret = None;
    let mut algorithm = Algorithm::Sha1;
    let mut digits = 6;
    let mut period = 30;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decode(value)?;
        match key.to_ascii_lowercase().as_str() {
            "secret" => secret = Some(decode_secret(&value)?),
            "algorithm" => {
                algorithm = Algorithm::from_name(&value).ok_or(ParseError::UnsupportedAlgorithm)?
            }
            "digits" => {
                digits = value.parse().map_err(|_| ParseError::InvalidDigits)?;
                if !(6..=8).contains(&digits) {
                    return Err(ParseError::InvalidDigits);
                }
            }
            "period" => {
                period = value.parse().map_err(|_| ParseError::InvalidPeriod)?;
                if !(1..=86400).contains(&period) {
                    return Err(ParseError::InvalidPeriod);
                }
            }
            _ => {} // issuer, image, etc. are irrelevant here
        }
    }

    Ok(Account {
        secret: secret.ok_or(ParseError::MissingSecret)?,
        algorithm,
        digits,
        period,
    })
}

fn decode_secret(input: &str) -> Result<Vec<u8>, ParseError> {
    let secret = base32_decode(input)?;
    if secret.is_empty() {
        return Err(ParseError::EmptySecret);
    }
    Ok(secret)
}

/// RFC 4648 Base32. Case-insensitive; ignores spaces, dashes and `=` padding,
/// which authenticator sites commonly add for readability.
pub fn base32_decode(input: &str) -> Result<Vec<u8>, ParseError> {
    let mut out = Vec::with_capacity(input.len() * 5 / 8);
    let mut buffer: u64 = 0;
    let mut bits = 0;
    for c in input.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a',
            b'2'..=b'7' => c - b'2' + 26,
            b' ' | b'-' | b'=' | b'\t' => continue,
            _ => return Err(ParseError::InvalidBase32),
        };
        buffer = (buffer << 5) | u64::from(value);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Ok(out)
}

pub fn base32_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity(data.len().div_ceil(5) * 8);
    let mut buffer: u64 = 0;
    let mut bits = 0;
    for &byte in data {
        buffer = (buffer << 8) | u64::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// Decodes `%XX` escapes and `+` (space) in a URI query value.
fn percent_decode(input: &str) -> Result<String, ParseError> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = bytes.get(i + 1..i + 3).ok_or(ParseError::InvalidUri)?;
                let hex = std::str::from_utf8(hex).map_err(|_| ParseError::InvalidUri)?;
                out.push(u8::from_str_radix(hex, 16).map_err(|_| ParseError::InvalidUri)?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| ParseError::InvalidUri)
}

#[cfg(test)]
mod tests {
    use super::*;

    // "12345678901234567890" in Base32 (the RFC 6238 SHA1 seed).
    const RFC_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

    #[test]
    fn base32_roundtrip_and_leniency() {
        assert_eq!(base32_decode(RFC_SECRET).unwrap(), b"12345678901234567890");
        assert_eq!(base32_decode("gezd gnbv-gy3t qojq").unwrap(), b"1234567890");
        assert_eq!(base32_decode("MZXW6===").unwrap(), b"foo");
        assert_eq!(base32_encode(b"foo"), "MZXW6");
        assert_eq!(base32_encode(b"12345678901234567890"), RFC_SECRET);
        assert_eq!(base32_decode("MZXW1"), Err(ParseError::InvalidBase32));
    }

    #[test]
    fn bare_secret_uses_defaults() {
        let a = Account::parse(&format!("  {RFC_SECRET}\n")).unwrap();
        assert_eq!(a.secret, b"12345678901234567890");
        assert_eq!((a.algorithm, a.digits, a.period), (Algorithm::Sha1, 6, 30));
    }

    #[test]
    fn uri_with_defaults() {
        let a = Account::parse(&format!(
            "otpauth://totp/ACME%20Co:john.doe@email.com?secret={RFC_SECRET}&issuer=ACME%20Co"
        ))
        .unwrap();
        assert_eq!(a.secret, b"12345678901234567890");
        assert_eq!((a.algorithm, a.digits, a.period), (Algorithm::Sha1, 6, 30));
    }

    #[test]
    fn uri_with_all_parameters() {
        let a = Account::parse(&format!(
            "OTPAUTH://TOTP/Example:alice?algorithm=sha512&digits=8&period=60&secret={}",
            RFC_SECRET.to_lowercase()
        ))
        .unwrap();
        assert_eq!((a.algorithm, a.digits, a.period), (Algorithm::Sha512, 8, 60));

        let a = Account::parse(&format!("otpauth://totp/x?secret={RFC_SECRET}&algorithm=SHA256")).unwrap();
        assert_eq!(a.algorithm, Algorithm::Sha256);
    }

    #[test]
    fn uri_percent_encoded_secret() {
        let a = Account::parse("otpauth://totp/x?secret=MZXW6%3D%3D%3D").unwrap();
        assert_eq!(a.secret, b"foo");
    }

    #[test]
    fn uri_errors() {
        let err = |s: &str| Account::parse(s).unwrap_err();
        assert_eq!(
            err(&format!("otpauth://hotp/x?secret={RFC_SECRET}&counter=1")),
            ParseError::NotTotp
        );
        assert_eq!(err("otpauth://totp/x?issuer=Foo"), ParseError::MissingSecret);
        assert_eq!(
            err(&format!("otpauth://totp/x?secret={RFC_SECRET}&algorithm=MD5")),
            ParseError::UnsupportedAlgorithm
        );
        assert_eq!(
            err(&format!("otpauth://totp/x?secret={RFC_SECRET}&digits=4")),
            ParseError::InvalidDigits
        );
        assert_eq!(
            err(&format!("otpauth://totp/x?secret={RFC_SECRET}&digits=abc")),
            ParseError::InvalidDigits
        );
        assert_eq!(
            err(&format!("otpauth://totp/x?secret={RFC_SECRET}&period=0")),
            ParseError::InvalidPeriod
        );
        assert_eq!(err("otpauth://totp/x?secret=%ZZ"), ParseError::InvalidUri);
        assert_eq!(err("otpauth://totp"), ParseError::InvalidUri);
        assert_eq!(err("otpauth://totp/x?secret="), ParseError::EmptySecret);
        assert_eq!(err(""), ParseError::EmptySecret);
        assert_eq!(err("not base32!"), ParseError::InvalidBase32);
    }

    #[test]
    fn canonical_uri_roundtrip() {
        let a = Account::parse(&format!(
            "otpauth://totp/x?secret={RFC_SECRET}&algorithm=SHA256&digits=8&period=45"
        ))
        .unwrap();
        let b = Account::parse(&a.to_uri()).unwrap();
        assert_eq!(a.secret, b.secret);
        assert_eq!(
            (a.algorithm, a.digits, a.period),
            (b.algorithm, b.digits, b.period)
        );
    }

    #[test]
    fn debug_redacts_secret() {
        let a = Account::parse(RFC_SECRET).unwrap();
        let dbg = format!("{a:?}");
        assert!(dbg.contains("<redacted>"));
        assert!(!dbg.contains("49")); // no raw bytes ('1' = 0x31 = 49)
    }

    #[test]
    fn errors_do_not_echo_input() {
        let msg = Account::parse("SECRETVALUE1").unwrap_err().to_string();
        assert!(!msg.contains("SECRETVALUE"));
    }

    #[test]
    fn suggests_names() {
        let s = |uri: &str| suggested_name(uri);
        assert_eq!(
            s("otpauth://totp/ACME%20Co:john@example.com?secret=MZXW6&issuer=ACME+Inc").as_deref(),
            Some("ACME Inc")
        );
        assert_eq!(
            s("otpauth://totp/GitHub:alice?secret=MZXW6").as_deref(),
            Some("GitHub")
        );
        assert_eq!(
            s("OTPAUTH://TOTP/alice%40example.com?secret=MZXW6").as_deref(),
            Some("alice@example.com")
        );
        assert_eq!(s("otpauth://totp/?secret=MZXW6&issuer=").as_deref(), None);
        assert_eq!(s("  otpauth://totp/%20Bank%20:bob  ").as_deref(), Some("Bank"));
        assert_eq!(s("MZXW6"), None);
        assert_eq!(s("otpauth://totp"), None);
    }

    #[test]
    fn code_from_rfc_secret() {
        let a = Account::parse(&format!("otpauth://totp/x?secret={RFC_SECRET}&digits=8")).unwrap();
        assert_eq!(a.formatted_code_at(59), "9428 7082");
    }
}
