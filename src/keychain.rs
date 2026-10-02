//! Account storage in the macOS login Keychain (generic password items).
//!
//! Each account is one item: service = [`SERVICE`], account = the user's name for it,
//! data = the canonical otpauth URI. The display order is one more item, [`ORDER_ACCOUNT`],
//! holding account names only. Nothing is ever written to disk outside the Keychain.

use std::cell::RefCell;
use std::fmt;

use security_framework::item::{ItemClass, ItemSearchOptions, ItemUpdateOptions, Limit, update_item};
use security_framework::passwords::{
    PasswordOptions, delete_generic_password, generic_password, set_generic_password,
};

use crate::otpauth::{Account, ParseError, wipe};

const SERVICE: &str = "com.phumiphat.akey";
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
const ERR_SEC_DUPLICATE_ITEM: i32 = -25299;
/// The item that stores the user's ordering as newline-separated names. Account names may not
/// start with "." (see [`validate_name`]), so it can never collide with an account.
const ORDER_ACCOUNT: &str = ".order";

#[derive(Debug)]
pub enum Error {
    NotFound,
    AlreadyExists,
    /// OSStatus from Security.framework. Only the code is kept so no item data can leak.
    Keychain(i32),
    Corrupt(ParseError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound => f.write_str("no such account"),
            Error::AlreadyExists => f.write_str("an account with that name already exists"),
            Error::Keychain(code) => write!(f, "Keychain error (OSStatus {code})"),
            Error::Corrupt(e) => write!(f, "stored item is invalid: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<security_framework::base::Error> for Error {
    fn from(e: security_framework::base::Error) -> Self {
        match e.code() {
            ERR_SEC_ITEM_NOT_FOUND => Error::NotFound,
            ERR_SEC_DUPLICATE_ITEM => Error::AlreadyExists,
            code => Error::Keychain(code),
        }
    }
}

/// Development aid: `AKEY_DEMO=1` swaps the Keychain for fixed in-memory accounts, so UI tests
/// and screenshots never read (or trigger access prompts for) real items. Writes are ignored.
fn demo() -> bool {
    static DEMO: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DEMO.get_or_init(|| std::env::var_os("AKEY_DEMO").is_some())
}

const DEMO_ACCOUNTS: &[(&str, &str)] = &[
    ("Discord", "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP"),
    (
        "GitHub",
        "otpauth://totp/x?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
    ),
    (
        "Heroku (60 s)",
        "otpauth://totp/x?secret=KRSXG5CTMVRXEZLUKN2XAZLSKNSWG4TFOQ&period=60",
    ),
    (
        "Zoho — a rather long account name",
        "otpauth://totp/x?secret=MFRGGZDFMZTWQ2LK&digits=8",
    ),
];

thread_local! {
    /// The ordering in demo mode, which never touches the Keychain.
    static DEMO_ORDER: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Account names in the user's order. Accounts missing from the stored order (e.g. added by an
/// older version) follow alphabetically. Never reads secret data.
pub fn list() -> Result<Vec<String>, Error> {
    Ok(apply_order(account_names()?, &read_order()))
}

/// Stores the display order (names only).
pub fn set_order(names: &[String]) -> Result<(), Error> {
    if demo() {
        DEMO_ORDER.with(|order| *order.borrow_mut() = names.to_vec());
        return Ok(());
    }
    Ok(set_generic_password(
        SERVICE,
        ORDER_ACCOUNT,
        names.join("\n").as_bytes(),
    )?)
}

/// Moves `name` to `position` (0-based, clamped) in the display order.
pub fn move_to(name: &str, position: usize) -> Result<(), Error> {
    let mut names = list()?;
    let from = names.iter().position(|n| n == name).ok_or(Error::NotFound)?;
    let to = position.min(names.len() - 1);
    move_item(&mut names, from, to);
    set_order(&names)
}

/// Moves the element at `from` so that it ends up at index `to`.
pub fn move_item<T>(items: &mut Vec<T>, from: usize, to: usize) {
    let item = items.remove(from);
    items.insert(to, item);
}

/// Orders `names` by `order`; names not in `order` follow alphabetically, and names in
/// `order` that no longer exist are dropped.
pub fn apply_order(mut names: Vec<String>, order: &[String]) -> Vec<String> {
    names.sort_by_key(|n| n.to_lowercase());
    let mut ordered: Vec<String> = Vec::with_capacity(names.len());
    for name in order.iter().chain(names.iter()) {
        if names.contains(name) && !ordered.contains(name) {
            ordered.push(name.clone());
        }
    }
    ordered
}

/// The stored order, or empty if there is none (or it cannot be read).
fn read_order() -> Vec<String> {
    if demo() {
        return DEMO_ORDER.with(|order| order.borrow().clone());
    }
    generic_password(PasswordOptions::new_generic_password(SERVICE, ORDER_ACCOUNT))
        .ok()
        .and_then(|data| String::from_utf8(data).ok())
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// Every account name, unordered. Reads attributes only.
fn account_names() -> Result<Vec<String>, Error> {
    if demo() {
        return Ok(DEMO_ACCOUNTS.iter().map(|(name, _)| name.to_string()).collect());
    }
    let results = match ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(SERVICE)
        .load_attributes(true)
        .limit(Limit::All)
        .search()
    {
        Ok(results) => results,
        Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    Ok(results
        .iter()
        .filter_map(|r| r.simplify_dict()?.remove("acct"))
        .filter(|name| !name.starts_with('.'))
        .collect())
}

pub fn exists(name: &str) -> Result<bool, Error> {
    if demo() {
        return Ok(DEMO_ACCOUNTS.iter().any(|(n, _)| *n == name));
    }
    match ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(SERVICE)
        .account(name)
        .load_attributes(true)
        .limit(1)
        .search()
    {
        Ok(results) => Ok(!results.is_empty()),
        Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(false),
        Err(e) => Err(e.into()),
    }
}

pub fn get(name: &str) -> Result<Account, Error> {
    if demo() {
        let (_, uri) = DEMO_ACCOUNTS
            .iter()
            .find(|(n, _)| *n == name)
            .ok_or(Error::NotFound)?;
        return Account::parse(uri).map_err(Error::Corrupt);
    }
    let mut data = generic_password(PasswordOptions::new_generic_password(SERVICE, name))?;
    let account = std::str::from_utf8(&data)
        .map_err(|_| Error::Corrupt(ParseError::InvalidUri))
        .and_then(|uri| Account::parse(uri).map_err(Error::Corrupt));
    wipe(&mut data);
    account
}

/// Creates or replaces an account. A new account goes to the end of the display order.
pub fn set(name: &str, account: &Account) -> Result<(), Error> {
    if demo() {
        return Ok(());
    }
    let order = if exists(name)? { None } else { Some(list()?) };
    let mut uri = account.to_uri().into_bytes();
    let result = set_generic_password(SERVICE, name, &uri);
    wipe(&mut uri);
    result?;
    if let Some(mut order) = order {
        order.push(name.to_string());
        set_order(&order)?;
    }
    Ok(())
}

/// Renames in place (`SecItemUpdate`), so the item keeps its access control list.
pub fn rename(old: &str, new: &str) -> Result<(), Error> {
    if old == new || demo() {
        return Ok(());
    }
    if exists(new)? {
        return Err(Error::AlreadyExists);
    }
    let mut search = ItemSearchOptions::new();
    search
        .class(ItemClass::generic_password())
        .service(SERVICE)
        .account(old);
    let mut update = ItemUpdateOptions::new();
    update.set_account_name(new);
    update_item(&search, &update)?;

    let mut order = read_order();
    if let Some(slot) = order.iter_mut().find(|n| *n == old) {
        *slot = new.to_string();
        set_order(&order)?;
    }
    Ok(())
}

/// Shared by the CLI and the app's dialogs.
pub fn validate_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.trim() != name || name.chars().any(char::is_control) {
        return Err("Account name must not be empty or start/end with spaces.");
    }
    if name.starts_with('.') {
        return Err("Account name must not start with “.”.");
    }
    Ok(())
}

pub fn remove(name: &str) -> Result<(), Error> {
    if demo() {
        return Ok(());
    }
    delete_generic_password(SERVICE, name)?;

    let mut order = read_order();
    if order.iter().any(|n| n == name) {
        order.retain(|n| n != name);
        set_order(&order)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{apply_order, move_item, validate_name};

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn applies_stored_order() {
        let names = strings(&["aws", "GitHub", "beam", "Zoho"]);
        // Stored order first, then the rest alphabetically; vanished names are dropped.
        assert_eq!(
            apply_order(names.clone(), &strings(&["Zoho", "gone", "aws"])),
            strings(&["Zoho", "aws", "beam", "GitHub"])
        );
        assert_eq!(
            apply_order(names, &[]),
            strings(&["aws", "beam", "GitHub", "Zoho"])
        );
        assert_eq!(
            apply_order(strings(&["a"]), &strings(&["a", "a"])),
            strings(&["a"])
        );
    }

    #[test]
    fn moves_items() {
        let mut v = vec![0, 1, 2, 3];
        move_item(&mut v, 0, 2);
        assert_eq!(v, [1, 2, 0, 3]);
        move_item(&mut v, 3, 0);
        assert_eq!(v, [3, 1, 2, 0]);
        move_item(&mut v, 1, 1);
        assert_eq!(v, [3, 1, 2, 0]);
    }

    #[test]
    fn validates_names() {
        assert!(validate_name("GitHub").is_ok());
        assert!(validate_name("ACME Co: alice@example.com").is_ok());
        assert!(validate_name("บัญชีไทย").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name(" GitHub").is_err());
        assert!(validate_name("GitHub ").is_err());
        assert!(validate_name("Git\nHub").is_err());
        assert!(validate_name(".order").is_err());
    }
}
