use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();

    // Protobuf code generation
    prost_build::compile_protos(&["1cafa6d8/6ca5cab7.proto"], &["1cafa6d8/"])
        .expect("Failed to compile protobuf");
    println!("cargo:rerun-if-changed=1cafa6d8/6ca5cab7.proto");

    // Read the Lua plugin source
    let lua_src =
        fs::read_to_string("5e689e2b/6ca5cab7.lua").expect("Failed to read plugin source");

    // Generate random 32-byte key and 24-byte nonce for AEAD encryption
    use chacha20poly1305::{
        aead::{Aead, KeyInit, OsRng},
        AeadCore, XChaCha20Poly1305,
    };

    let key = XChaCha20Poly1305::generate_key(&mut OsRng);
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let cipher = XChaCha20Poly1305::new(&key);

    let ciphertext = cipher
        .encrypt(&nonce, lua_src.as_bytes())
        .expect("Lua AEAD encryption failed");

    // Write as Rust source with byte arrays
    let mut content = String::new();

    // Encrypted content
    content.push_str("const ENCRYPTED_LUA: &[u8] = &[\n");
    for (i, byte) in ciphertext.iter().enumerate() {
        if i % 16 == 0 {
            content.push_str("    ");
        }
        content.push_str(&format!("{:#04x},", byte));
        if i % 16 == 15 {
            content.push('\n');
        }
    }
    content.push_str("\n];\n\n");

    // 32-byte AEAD key
    content.push_str("const LUA_KEY: [u8; 32] = [\n    ");
    for (i, &k) in key.iter().enumerate() {
        content.push_str(&format!("{:#04x},", k));
        if i < key.len() - 1 {
            content.push(' ');
        }
    }
    content.push_str("\n];\n\n");

    // 24-byte nonce
    content.push_str("const LUA_NONCE: [u8; 24] = [\n    ");
    for (i, &n) in nonce.iter().enumerate() {
        content.push_str(&format!("{:#04x},", n));
        if i < nonce.len() - 1 {
            content.push(' ');
        }
    }
    content.push_str("\n];\n");

    let dest = Path::new(&out_dir).join("encrypted_lua.rs");
    fs::write(dest, content).unwrap();

    println!("cargo:rerun-if-changed=5e689e2b/6ca5cab7.lua");
    println!("cargo:rerun-if-changed=build.rs");
}
