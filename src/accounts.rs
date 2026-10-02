//! Add/edit logic shared by the popover forms. Messages are user-facing sentences and never
//! contain the secret.

use crate::keychain;
use crate::otpauth::{self, Account, wipe};

/// What the user typed into a form. The secret copy is wiped on drop.
pub struct FormInput {
    pub name: String,
    pub secret: String,
}

impl Drop for FormInput {
    fn drop(&mut self) {
        // SAFETY: zero bytes are valid UTF-8.
        wipe(unsafe { self.secret.as_bytes_mut() });
    }
}

/// Adds a new account. An empty name is filled from the otpauth:// issuer or label.
pub fn add(input: &FormInput) -> Result<(), String> {
    let account = Account::parse(&input.secret).map_err(|e| sentence(&e.to_string()))?;
    let name = match input.name.trim() {
        "" => otpauth::suggested_name(&input.secret).ok_or("Enter a name for the account.")?,
        name => name.to_string(),
    };
    keychain::validate_name(&name)?;
    if keychain::exists(&name).map_err(|e| sentence(&e.to_string()))? {
        return Err(format!("An account named “{name}” already exists."));
    }
    keychain::set(&name, &account).map_err(|e| sentence(&e.to_string()))
}

/// Renames and/or replaces the secret. Everything is validated before anything is written,
/// so a bad secret never leaves a half-applied rename.
pub fn edit(old: &str, input: &FormInput) -> Result<(), String> {
    let new = input.name.trim();
    keychain::validate_name(new)?;
    let account = match input.secret.trim() {
        "" => None,
        secret => Some(Account::parse(secret).map_err(|e| sentence(&e.to_string()))?),
    };
    if new != old && keychain::exists(new).map_err(|e| sentence(&e.to_string()))? {
        return Err(format!("An account named “{new}” already exists."));
    }
    keychain::rename(old, new).map_err(|e| sentence(&e.to_string()))?;
    if let Some(account) = account {
        keychain::set(new, &account).map_err(|e| sentence(&e.to_string()))?;
    }
    Ok(())
}

/// "secret is not valid Base32" → "Secret is not valid Base32."
fn sentence(message: &str) -> String {
    let mut chars = message.chars();
    match chars.next() {
        Some(first) => {
            let s: String = first.to_uppercase().chain(chars).collect();
            format!("{}.", s.trim_end_matches('.'))
        }
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sentences() {
        assert_eq!(
            sentence("secret is not valid Base32"),
            "Secret is not valid Base32."
        );
        assert_eq!(
            sentence("Enter a name for the account."),
            "Enter a name for the account."
        );
        assert_eq!(sentence(""), "");
    }

    // Validation that fails before touching the Keychain.
    #[test]
    fn rejects_bad_input_without_keychain() {
        let input = |name: &str, secret: &str| FormInput {
            name: name.into(),
            secret: secret.into(),
        };
        assert_eq!(
            add(&input("x", "not base32!")),
            Err("Secret is not valid Base32.".into())
        );
        assert_eq!(
            add(&input("", "MZXW6")),
            Err("Enter a name for the account.".into())
        );
        assert!(edit("x", &input("", "")).is_err());
        assert_eq!(
            edit("x", &input("x", "otpauth://hotp/x?secret=MZXW6")),
            Err("Only otpauth://totp/ URIs are supported.".into())
        );
    }
}
