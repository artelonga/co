//! co-pwhash — Argon2id hash generator for the `CO_SEED_ADMIN_PASSWORD_HASH`
//! Fly secret. Same Argon2 crate the server uses to verify, so the parameters
//! match by construction.
//!
//! Usage:
//!   co-pwhash 'YOUR_PASSWORD'
//!   echo 'YOUR_PASSWORD' | co-pwhash         # reads from stdin (trims newline)
//!
//! Output: a single line `$argon2id$v=19$...` ready to paste into
//!   `flyctl secrets set CO_SEED_ADMIN_PASSWORD_HASH=...`

use std::io::{self, Read, Write};

use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
};

fn main() {
    let password = match std::env::args().nth(1) {
        Some(arg) => arg,
        None => {
            // Read from stdin
            let mut buf = String::new();
            if let Err(e) = io::stdin().read_to_string(&mut buf) {
                eprintln!("error reading stdin: {e}");
                std::process::exit(1);
            }
            buf.trim_end_matches(['\n', '\r']).to_string()
        }
    };

    if password.is_empty() {
        eprintln!("error: empty password");
        eprintln!("usage: co-pwhash 'YOUR_PASSWORD'");
        eprintln!("       echo 'YOUR_PASSWORD' | co-pwhash");
        std::process::exit(1);
    }

    let salt = SaltString::generate(&mut OsRng);
    match Argon2::default().hash_password(password.as_bytes(), &salt) {
        Ok(hash) => {
            let _ = writeln!(io::stdout(), "{hash}");
        }
        Err(e) => {
            eprintln!("error hashing password: {e}");
            std::process::exit(1);
        }
    }
}
