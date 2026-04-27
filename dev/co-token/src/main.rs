//! co-token — encrypted secret storage via the OS keychain.
//!
//! Tokens (and other small secrets) are stored encrypted at rest by the
//! operating system: macOS Keychain, Linux Secret Service (gnome-keyring /
//! KWallet), Windows Credential Manager. The plaintext is never written to
//! disk by this tool; it lives in OS-managed encrypted storage and is only
//! decrypted on-demand for the current process when `get` is called.
//!
//! Usage:
//!   co-token set <name>            # reads token from stdin (no echo), stores in keychain
//!   co-token get <name>            # prints the stored token to stdout
//!   co-token rm  <name>            # removes the entry
//!   co-token list                  # NOT supported by all backends; prints a help message
//!
//! Naming: <name> is a free-form label. Common: "prod", "uat", "uat-mirror".
//! Internally stored under service="co" account="<name>".
//!
//! Examples:
//!   echo 'co_VxJK...' | co-token set prod
//!   TOKEN=$(co-token get prod)
//!   curl -H "Authorization: Bearer $TOKEN" https://co-artelonga.fly.dev/api/v1/...
//!
//! Security:
//! - The keychain is unlocked when the user is logged in (macOS); first access
//!   per session may prompt for the user's login password.
//! - Each `get` may be audited by the OS (macOS Console.app shows access).
//! - The plaintext is not written to disk by this tool; piping it elsewhere
//!   is the caller's responsibility.

use std::io::{self, Read, Write};
use std::process::ExitCode;

const SERVICE: &str = "co";

fn usage() -> ExitCode {
    eprintln!("co-token — OS-encrypted secret storage");
    eprintln!();
    eprintln!("usage:");
    eprintln!("  co-token set <name>     # reads from stdin, stores in OS keychain");
    eprintln!("  co-token get <name>     # prints the secret to stdout");
    eprintln!("  co-token rm  <name>     # removes the entry");
    eprintln!();
    eprintln!("examples:");
    eprintln!("  echo 'co_xyz' | co-token set prod");
    eprintln!("  TOKEN=$(co-token get prod)");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let cmd = match args.next() {
        Some(c) => c,
        None => return usage(),
    };
    let name = match args.next() {
        Some(n) => n,
        None => return usage(),
    };

    let entry = match keyring::Entry::new(SERVICE, &name) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("co-token: keychain backend error: {e}");
            return ExitCode::from(1);
        }
    };

    match cmd.as_str() {
        "set" => {
            let mut buf = String::new();
            if let Err(e) = io::stdin().read_to_string(&mut buf) {
                eprintln!("co-token: read stdin: {e}");
                return ExitCode::from(1);
            }
            let value = buf.trim_end_matches(['\n', '\r']).to_string();
            if value.is_empty() {
                eprintln!("co-token: empty value (nothing on stdin)");
                return ExitCode::from(1);
            }
            match entry.set_password(&value) {
                Ok(()) => {
                    eprintln!("co-token: stored '{name}' (service='{SERVICE}', {} bytes)", value.len());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("co-token: set failed: {e}");
                    ExitCode::from(1)
                }
            }
        }
        "get" => match entry.get_password() {
            Ok(value) => {
                let _ = writeln!(io::stdout(), "{value}");
                ExitCode::SUCCESS
            }
            Err(keyring::Error::NoEntry) => {
                eprintln!("co-token: '{name}' not found");
                ExitCode::from(1)
            }
            Err(e) => {
                eprintln!("co-token: get failed: {e}");
                ExitCode::from(1)
            }
        },
        "rm" | "delete" | "remove" => match entry.delete_credential() {
            Ok(()) => {
                eprintln!("co-token: removed '{name}'");
                ExitCode::SUCCESS
            }
            Err(keyring::Error::NoEntry) => {
                eprintln!("co-token: '{name}' not found");
                ExitCode::from(1)
            }
            Err(e) => {
                eprintln!("co-token: rm failed: {e}");
                ExitCode::from(1)
            }
        },
        _ => usage(),
    }
}
