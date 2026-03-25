use game_core::storage::database::Storage;
use game_core::storage::schema;
use game_core::storage::telemetry;
use game_core::storage::SessionManager;
use game_core::{GameError, Result};
use obfstr::obfstr;
use std::fs;
use std::io::Write;
use std::process::Command;

// Encrypted Lua plugin — decrypted at runtime via XChaCha20-Poly1305
include!(concat!(env!("OUT_DIR"), "/encrypted_lua.rs"));

fn decrypt_lua() -> Result<String> {
    use chacha20poly1305::{
        aead::{Aead, KeyInit},
        XChaCha20Poly1305, XNonce,
    };

    let cipher = XChaCha20Poly1305::new((&LUA_KEY).into());
    let nonce = XNonce::from_slice(&LUA_NONCE);

    let plaintext = match cipher.decrypt(nonce, ENCRYPTED_LUA) {
        Ok(pt) => pt,
        Err(_) => {
            return Err(GameError::Script(
                obfstr!("Plugin integrity check failed").into(),
            ));
        }
    };

    match String::from_utf8(plaintext) {
        Ok(s) => Ok(s),
        Err(_) => Err(GameError::Script(obfstr!("Plugin decode failed").into())),
    }
}

pub fn run_vim(
    storage: &Storage,
    user: &schema::UserProfile,
    session: &mut SessionManager<'_>,
) -> Result<()> {
    // Record entering vim universe
    session.enter_universe("vim");
    let _ = telemetry::record_game_enter(storage, &user.user_id, session.session_id(), "vim");

    // Create a temp file with the game plugin code
    let mut temp_path = std::env::temp_dir();
    temp_path.push(obfstr!("game_nvim_plugin.lua"));

    // Decrypt and write the plugin code to temp file
    let plugin_source = decrypt_lua()?;
    let mut file = fs::File::create(&temp_path)?;
    file.write_all(plugin_source.as_bytes())?;
    drop(file);

    // Launch Neovim with the plugin
    let output = Command::new("nvim")
        .arg("-u")
        .arg("NONE")
        .arg("-N")
        .arg("+")
        .arg(format!("source {}", temp_path.display()))
        .status()?;

    // Clean up temp file
    let _ = fs::remove_file(&temp_path);

    // Record exiting vim universe
    session.exit_universe();
    let _ = telemetry::record_game_exit(storage, &user.user_id, session.session_id(), "vim");

    if output.success() {
        Ok(())
    } else {
        Err(GameError::Command(
            obfstr!("Neovim exited with error").into(),
        ))
    }
}
