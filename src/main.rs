mod accounts;
mod app;
mod cli;
mod keychain;
mod otpauth;
mod totp;
mod ui;

use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn unix_time() -> u64 {
    unix_time_f64() as u64
}

pub fn unix_time_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    let result = match args.as_slice() {
        // Older macOS versions pass -psn_X_Y when launched from Finder.
        [] => {
            app::run();
            Ok(())
        }
        [psn] if psn.starts_with("-psn_") => {
            app::run();
            Ok(())
        }
        ["add", name] => cli::add(name),
        ["list"] => cli::list(),
        ["rename", old, new] => cli::rename(old, new),
        ["move", name, position] => cli::move_to(name, position),
        ["remove", name] => cli::remove(name),
        ["-h" | "--help" | "help"] => {
            println!("{}", cli::USAGE);
            Ok(())
        }
        ["-V" | "--version"] => {
            println!("akey {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => {
            eprintln!("{}", cli::USAGE);
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("akey: {e}");
            ExitCode::FAILURE
        }
    }
}
