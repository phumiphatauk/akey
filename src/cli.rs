//! `akey add|list|rename|move|remove` subcommands.

use std::error::Error;
use std::io::{BufRead, IsTerminal};

use crate::keychain;
use crate::otpauth::{Account, wipe};

pub const USAGE: &str = "\
Usage:
  akey                 run the menu bar app
  akey add <name>      add an account; the secret (Base32 or otpauth:// URI)
                       is read from stdin with echo disabled
  akey list            list account names
  akey rename <old> <new>
                       rename an account
  akey move <name> <position>
                       move an account to a position in the list (1 = top)
  akey remove <name>   delete an account
  akey --version       print the version

Accounts can also be added, edited and removed from the menu bar app.";

pub fn add(name: &str) -> Result<(), Box<dyn Error>> {
    keychain::validate_name(name)?;
    if keychain::exists(name)? {
        return Err(format!("account '{name}' already exists; remove it first").into());
    }

    let account = {
        let input = if std::io::stdin().is_terminal() {
            rpassword::prompt_password("Secret or otpauth:// URI (input hidden): ")?
        } else {
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line)?;
            line
        };
        let mut bytes = input.into_bytes();
        let parsed = std::str::from_utf8(&bytes)
            .map_err(|_| "input is not valid UTF-8".to_string())
            .and_then(|s| Account::parse(s).map_err(|e| e.to_string()));
        wipe(&mut bytes);
        parsed?
    };

    keychain::set(name, &account)?;

    let now = crate::unix_time();
    let remaining = account.period - now % account.period;
    println!(
        "Added '{name}'. Current code: {} ({remaining}s left) — compare it with the website.",
        account.formatted_code_at(now)
    );
    Ok(())
}

pub fn list() -> Result<(), Box<dyn Error>> {
    let names = keychain::list()?;
    if names.is_empty() {
        eprintln!("No accounts. Add one with: akey add <name>");
    }
    for name in names {
        println!("{name}");
    }
    Ok(())
}

pub fn rename(old: &str, new: &str) -> Result<(), Box<dyn Error>> {
    keychain::validate_name(new)?;
    match keychain::rename(old, new) {
        Ok(()) => {
            println!("Renamed '{old}' to '{new}'.");
            Ok(())
        }
        Err(keychain::Error::NotFound) => Err(format!("no account named '{old}'").into()),
        Err(keychain::Error::AlreadyExists) => Err(format!("account '{new}' already exists").into()),
        Err(e) => Err(e.into()),
    }
}

pub fn move_to(name: &str, position: &str) -> Result<(), Box<dyn Error>> {
    let position: usize = position
        .parse()
        .ok()
        .filter(|&p| p >= 1)
        .ok_or("position must be a number starting at 1")?;
    match keychain::move_to(name, position - 1) {
        Ok(()) => Ok(()),
        Err(keychain::Error::NotFound) => Err(format!("no account named '{name}'").into()),
        Err(e) => Err(e.into()),
    }
}

pub fn remove(name: &str) -> Result<(), Box<dyn Error>> {
    match keychain::remove(name) {
        Ok(()) => {
            println!("Removed '{name}'.");
            Ok(())
        }
        Err(keychain::Error::NotFound) => Err(format!("no account named '{name}'").into()),
        Err(e) => Err(e.into()),
    }
}
