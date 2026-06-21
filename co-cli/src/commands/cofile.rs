//! CO-86 — `co encode` / `co decode` / `co inspect` for the `.co` wire format.
//!
//! Markdown stays the authoring surface; `.co` is the transport-optimized,
//! self-describing protobuf envelope. These commands wrap the
//! [`co::co_format`] library.

use std::path::Path;
use std::time::Instant;

use colored::Colorize;

use co::co_format::{self, codec};

/// `co encode <input.md> [-o out.co] [--compress]`
pub fn run_encode(input: &str, output: Option<&str>, compress: bool) {
    let md = read_to_string_or_exit(input);

    let mut co = co_format::from_markdown(&md);

    if compress && let Err(e) = codec::compress_body(&mut co, codec::DEFAULT_LEVEL) {
        fail(&format!("compression failed: {e}"));
    }

    // Populate telemetry with a measured encode duration.
    let start = Instant::now();
    let _ = co_format::to_bytes(&co);
    let encode_ns = start.elapsed().as_nanos() as i64;
    co_format::telemetry::populate(&mut co, encode_ns);

    let bytes = co_format::to_bytes(&co);

    let out_path = output
        .map(|s| s.to_string())
        .unwrap_or_else(|| default_extension(input, "co"));

    if let Err(e) = std::fs::write(&out_path, &bytes) {
        fail(&format!("failed to write {out_path}: {e}"));
    }

    let info = co_format::inspect(&co, bytes.len());
    println!(
        "{} {} → {} ({} → {} bytes, codec {})",
        "encoded".green().bold(),
        input,
        out_path,
        info.size_uncompressed,
        info.size_on_wire,
        co.telemetry
            .as_ref()
            .map(|t| t.codec.as_str())
            .unwrap_or("raw"),
    );
}

/// `co decode <in.co> [-o out.md]`
pub fn run_decode(input: &str, output: Option<&str>) {
    let bytes = read_bytes_or_exit(input);

    let co = match co_format::from_bytes(&bytes) {
        Ok(c) => c,
        Err(e) => fail(&format!("failed to decode {input}: {e}")),
    };

    let md = match co_format::to_markdown(&co) {
        Ok(m) => m,
        Err(co::co_format::CoError::BodyEncrypted) => {
            fail("body is encrypted; decryption keys are not yet wired into the CLI")
        }
        Err(e) => fail(&format!("failed to render markdown: {e}")),
    };

    match output {
        Some(path) => {
            if let Err(e) = std::fs::write(path, md.as_bytes()) {
                fail(&format!("failed to write {path}: {e}"));
            }
            println!("{} {} → {}", "decoded".green().bold(), input, path);
        }
        None => print!("{md}"),
    }
}

/// `co inspect <in.co>` — print envelope metadata.
pub fn run_inspect(input: &str) {
    let bytes = read_bytes_or_exit(input);

    let co = match co_format::from_bytes(&bytes) {
        Ok(c) => c,
        Err(e) => fail(&format!("failed to decode {input}: {e}")),
    };

    let info = co_format::inspect(&co, bytes.len());
    let yn = |b: bool| if b { "yes" } else { "no" };

    println!("{}", input.bold());
    println!("  version          {}", info.version);
    println!("  content_type     {}", info.content_type);
    println!("  size_uncompressed {} bytes", info.size_uncompressed);
    println!("  size_on_wire     {} bytes", info.size_on_wire);
    println!("  signed           {}", yn(info.signed));
    println!("  encrypted        {}", yn(info.encrypted));
    println!("  attachment_count {}", info.attachment_count);
}

// --- helpers ---------------------------------------------------------------

fn read_to_string_or_exit(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| fail(&format!("failed to read {path}: {e}")))
}

fn read_bytes_or_exit(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| fail(&format!("failed to read {path}: {e}")))
}

/// Replace (or add) the extension of `input` with `ext`.
fn default_extension(input: &str, ext: &str) -> String {
    Path::new(input)
        .with_extension(ext)
        .to_string_lossy()
        .into_owned()
}

fn fail(msg: &str) -> ! {
    eprintln!("{} {}", "error:".red().bold(), msg);
    std::process::exit(1);
}
