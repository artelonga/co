//! One-shot importer: quilombo-araucaria SQLite → Co universe `quilomboaraucaria`.
//!
//! Reads from the quilombo-blog production DB (obtain with
//! `flyctl ssh sftp get -a quilombo-araucaria /app/data/quilombo.db`)
//! and writes entries to the `quilomboaraucaria` Co universe.
//!
//! Usage:
//!   cargo run --bin import_quilombo -- --src /tmp/quilombo.db --data /data
//!
//! What it does:
//! 1. Opens source DB read-only
//! 2. Opens (or creates) co.db in --data, runs all migrations
//! 3. Ensures the `quilomboaraucaria` universe exists
//! 4. Maps DB tables → Co entries (idempotent upsert by path):
//!    - publicacoes → type:post   (path: posts/{slug}.md)
//!    - eventos     → type:event  (path: events/{slug}.md)
//!    - missoes     → type:mission (path: missions/{id}.md)
//! 5. Decompresses zlib-compressed `corpo` field if needed
//! 6. Writes meta/stats.md with totalUsuarios, versaoApp, ultimaSync
//! 7. Reports row counts

use std::io::Read as IoRead;
use std::path::{Path, PathBuf};

use chrono::Utc;
use clap::Parser;
use flate2::read::ZlibDecoder;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::json;

/// Slugify a string: lowercase, spaces/special chars → hyphens.
fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_hyphen = true;
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || c == '-' {
            out.push(c);
            last_hyphen = c == '-';
        } else if !last_hyphen {
            out.push('-');
            last_hyphen = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Try to decompress a blob as zlib. Returns the raw blob interpreted as UTF-8 on failure.
fn decompress_body(raw: &[u8]) -> String {
    // First try: interpret as-is UTF-8 (most common in newer quilombo-blog versions)
    if let Ok(s) = std::str::from_utf8(raw) {
        return s.to_string();
    }
    // Fallback: zlib decompress
    let mut decoder = ZlibDecoder::new(raw);
    let mut buf = String::new();
    if decoder.read_to_string(&mut buf).is_ok() && !buf.is_empty() {
        return buf;
    }
    // Last resort: lossy UTF-8
    String::from_utf8_lossy(raw).into_owned()
}

/// Compute a short xxhash3 of a body string (hex, 8 chars).
fn body_hash(body: &str) -> String {
    use xxhash_rust::xxh3::xxh3_64;
    format!("{:016x}", xxh3_64(body.as_bytes()))
}

struct EntryArgs<'a> {
    path: &'a str,
    universe_key: &'a str,
    entry_type: &'a str,
    title: &'a str,
    frontmatter: &'a serde_json::Value,
    body: &'a str,
    created_at: &'a str,
    updated_at: &'a str,
}

/// Upsert a row into the SQLite `entries` table (INSERT OR REPLACE).
fn upsert_entry(conn: &Connection, args: EntryArgs<'_>) {
    let path = args.path;
    let universe_key = args.universe_key;
    let entry_type = args.entry_type;
    let title = args.title;
    let frontmatter = args.frontmatter;
    let body = args.body;
    let created_at = args.created_at;
    let updated_at = args.updated_at;
    let fm_str = frontmatter.to_string();
    let hash = body_hash(body);
    conn.execute(
        "INSERT OR REPLACE INTO entries \
         (path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
          created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            path,
            universe_key,
            entry_type,
            title,
            fm_str,
            body,
            hash,
            created_at,
            updated_at
        ],
    )
    .unwrap_or_else(|e| panic!("Failed to upsert entry '{}': {}", path, e));
}

/// Write a markdown file with YAML frontmatter to the universe directory.
fn write_md_file(root: &Path, rel_path: &str, frontmatter: &serde_json::Value, body: &str) {
    let file_path = root.join(rel_path);
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("Failed to create dir {:?}: {}", parent, e));
    }
    let mut content = String::from("---\n");
    if let Some(obj) = frontmatter.as_object() {
        for (k, v) in obj {
            let val_str = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Array(arr) => {
                    let items: Vec<String> = arr
                        .iter()
                        .filter_map(|i| i.as_str().map(String::from))
                        .collect();
                    format!("[{}]", items.join(", "))
                }
                other => other.to_string(),
            };
            content.push_str(&format!("{}: {}\n", k, val_str));
        }
    }
    content.push_str("---\n");
    if !body.is_empty() {
        content.push('\n');
        content.push_str(body);
        if !body.ends_with('\n') {
            content.push('\n');
        }
    }
    std::fs::write(&file_path, &content)
        .unwrap_or_else(|e| panic!("Failed to write {:?}: {}", file_path, e));
}

/// Count rows in a source table (returns 0 if table doesn't exist).
fn count_table(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {}", table), [], |row| {
        row.get(0)
    })
    .unwrap_or(0)
}

// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "import_quilombo",
    about = "Import quilombo-araucaria content into the Co quilomboaraucaria universe"
)]
struct Args {
    /// Path to quilombo-blog source database (read-only)
    #[arg(long)]
    src: PathBuf,

    /// Path to co-web data directory (contains co.db, universes/)
    #[arg(long, default_value = "/data")]
    data: PathBuf,

    /// quilombo-blog application version string (stored in meta/stats.md)
    #[arg(long, default_value = "quilombo-blog")]
    versao_app: String,

    /// Dry run — print what would be imported without writing
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}

fn main() {
    let args = Args::parse();

    println!("=== Quilombo Araucária Importer ===");
    println!("Source:  {}", args.src.display());
    println!("Data:    {}", args.data.display());
    if args.dry_run {
        println!("Mode:    DRY RUN (no writes)");
    }
    println!();

    // Open source DB read-only
    let src = Connection::open_with_flags(&args.src, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap_or_else(|e| panic!("Failed to open source DB {:?}: {}", args.src, e));

    if args.dry_run {
        println!("Source DB row counts:");
        for table in &["publicacoes", "eventos", "missoes", "usuarios"] {
            println!("  {}: {}", table, count_table(&src, table));
        }
        return;
    }

    // Open (or create) co.db with migrations
    let db_path = args.data.join("co.db");
    let co = co_web::storage::Storage::new(&args.data);
    // Drop storage to release the lock; we'll work with a raw connection for upserts.
    drop(co);
    let dst = Connection::open(&db_path)
        .unwrap_or_else(|e| panic!("Failed to open co.db {:?}: {}", db_path, e));
    dst.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
        .ok();

    // Ensure the quilomboaraucaria universe exists
    let now_str = Utc::now().to_rfc3339();
    let custom_tokens = json!({
        "--bg": "#f5f0e8",
        "--card-bg": "#faf6ef",
        "--border": "#c8b48e",
        "--accent": "#2d4a22",
        "--text-muted": "#8b6914"
    })
    .to_string();
    dst.execute(
        "INSERT OR IGNORE INTO universes \
         (key, name, description, owner_id, created_at, is_template, is_public, \
          theme_preset, layout, font_headline, font_body, custom_tokens, content_count) \
         VALUES ('quilomboaraucaria', 'Quilombo Araucária', \
         'Comunidade quilombola do Paraná — publicações, eventos e missões', \
         'system', ?1, 0, 1, 'quilombo', 'board', \
         'Playfair Display', 'Inter', ?2, 0)",
        params![now_str, custom_tokens],
    )
    .ok();

    let universe_root = args.data.join("universes").join("quilomboaraucaria");
    std::fs::create_dir_all(&universe_root).expect("Failed to create universe directory");

    // ── Import publicacoes → type:post ────────────────────────────────────────
    let total_publicacoes = import_publicacoes(&src, &dst, &universe_root);

    // ── Import eventos → type:event ───────────────────────────────────────────
    let total_eventos = import_eventos(&src, &dst, &universe_root);

    // ── Import missoes → type:mission ─────────────────────────────────────────
    let total_missoes = import_missoes(&src, &dst, &universe_root);

    // ── usuarios — count only (profiles not imported) ─────────────────────────
    let total_usuarios = count_table(&src, "usuarios");

    // ── Write meta/stats.md ───────────────────────────────────────────────────
    let sync_at = Utc::now().to_rfc3339();
    let stats_fm = json!({
        "type": "meta",
        "total_usuarios": total_usuarios,
        "total_publicacoes": total_publicacoes,
        "total_eventos": total_eventos,
        "total_missoes": total_missoes,
        "versao_app": args.versao_app,
        "ultima_sync": sync_at,
        "created": sync_at,
        "modified": sync_at
    });
    write_md_file(&universe_root, "meta/stats.md", &stats_fm, "");
    upsert_entry(
        &dst,
        EntryArgs {
            path: "meta/stats.md",
            universe_key: "quilomboaraucaria",
            entry_type: "meta",
            title: "Quilombo Stats",
            frontmatter: &stats_fm,
            body: "",
            created_at: &sync_at,
            updated_at: &sync_at,
        },
    );

    // Update content_count
    let total_entries: i64 = total_publicacoes + total_eventos + total_missoes;
    dst.execute(
        "UPDATE universes SET content_count = ?1 WHERE key = 'quilomboaraucaria'",
        params![total_entries],
    )
    .ok();

    println!("\n=== Import Complete ===");
    println!("  publicacoes (posts):  {}", total_publicacoes);
    println!("  eventos (events):     {}", total_eventos);
    println!("  missoes (missions):   {}", total_missoes);
    println!("  usuarios (count):     {}", total_usuarios);
    println!("  ultima_sync:          {}", sync_at);
}

/// Import publicacoes → Co entries of type `post`.
///
/// Expected source schema (quilombo-blog):
///   publicacoes (id, slug, titulo, corpo BLOB, publicado, autor_id, criado_em, tags)
///
/// The `corpo` field may be zlib-compressed; we detect and decompress automatically.
fn import_publicacoes(src: &Connection, dst: &Connection, root: &Path) -> i64 {
    println!("Importing publicacoes → posts...");

    // Probe available columns (quilombo-blog versions differ)
    let has_tags = src
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('publicacoes') WHERE name='tags'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    let query = if has_tags {
        "SELECT id, slug, titulo, corpo, publicado, criado_em, tags \
         FROM publicacoes ORDER BY id"
    } else {
        "SELECT id, slug, titulo, corpo, publicado, criado_em, '[]' \
         FROM publicacoes ORDER BY id"
    };

    let mut stmt = match src.prepare(query) {
        Ok(s) => s,
        Err(e) => {
            println!("  SKIP: publicacoes table not found ({})", e);
            return 0;
        }
    };

    struct Row {
        slug: String,
        titulo: String,
        corpo: Vec<u8>,
        publicado: bool,
        criado_em: String,
        tags: String,
    }

    let rows: Vec<Row> = stmt
        .query_map([], |row| {
            Ok(Row {
                slug: row.get(1)?,
                titulo: row.get(2)?,
                corpo: row.get(3)?,
                publicado: row.get::<_, i64>(4).unwrap_or(0) != 0,
                criado_em: row.get(5)?,
                tags: row.get(6).unwrap_or_else(|_| "[]".to_string()),
            })
        })
        .expect("Failed to query publicacoes")
        .filter_map(|r| r.ok())
        .collect();

    let mut count: i64 = 0;
    for row in &rows {
        let slug = if row.slug.is_empty() {
            slugify(&row.titulo)
        } else {
            row.slug.clone()
        };
        if slug.is_empty() {
            continue;
        }

        let body = decompress_body(&row.corpo);
        let path = format!("posts/{}.md", slug);

        // Parse tags from JSON array string
        let tags: Vec<String> = serde_json::from_str::<Vec<String>>(&row.tags).unwrap_or_default();

        let fm = json!({
            "type": "post",
            "title": row.titulo,
            "slug": slug,
            "publicado": row.publicado,
            "tags": tags,
            "data": row.criado_em,
            "created": row.criado_em,
            "modified": row.criado_em
        });

        write_md_file(root, &path, &fm, &body);
        upsert_entry(
            dst,
            EntryArgs {
                path: &path,
                universe_key: "quilomboaraucaria",
                entry_type: "post",
                title: &row.titulo,
                frontmatter: &fm,
                body: &body,
                created_at: &row.criado_em,
                updated_at: &row.criado_em,
            },
        );
        count += 1;
    }

    println!("  {} publicacoes imported", count);
    count
}

/// Import eventos → Co entries of type `event`.
fn import_eventos(src: &Connection, dst: &Connection, root: &Path) -> i64 {
    println!("Importing eventos → events...");

    let has_slug = src
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('eventos') WHERE name='slug'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    let has_tags = src
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('eventos') WHERE name='tags'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    let query = match (has_slug, has_tags) {
        (true, true) => {
            "SELECT id, slug, titulo, data, hora, local, descricao_md, criado_em, tags FROM eventos ORDER BY id"
        }
        (true, false) => {
            "SELECT id, slug, titulo, data, hora, local, descricao_md, criado_em, '[]' FROM eventos ORDER BY id"
        }
        (false, _) => {
            "SELECT id, '' as slug, titulo, data, hora, local, descricao_md, criado_em, '[]' FROM eventos ORDER BY id"
        }
    };

    let mut stmt = match src.prepare(query) {
        Ok(s) => s,
        Err(e) => {
            println!("  SKIP: eventos table not found ({})", e);
            return 0;
        }
    };

    struct Row {
        id: i64,
        slug: String,
        titulo: String,
        data: String,
        hora: String,
        local: String,
        descricao_md: String,
        criado_em: String,
        tags: String,
    }

    let rows: Vec<Row> = stmt
        .query_map([], |row| {
            Ok(Row {
                id: row.get(0)?,
                slug: row.get(1).unwrap_or_default(),
                titulo: row.get(2)?,
                data: row.get(3)?,
                hora: row.get(4).unwrap_or_default(),
                local: row.get(5).unwrap_or_default(),
                descricao_md: row.get(6).unwrap_or_default(),
                criado_em: row.get(7)?,
                tags: row.get(8).unwrap_or_else(|_| "[]".to_string()),
            })
        })
        .expect("Failed to query eventos")
        .filter_map(|r| r.ok())
        .collect();

    let mut count: i64 = 0;
    for row in &rows {
        let slug = if row.slug.is_empty() {
            format!("{}-{}", row.data, slugify(&row.titulo))
        } else {
            row.slug.clone()
        };

        let path = format!("events/{}.md", slug);
        let tags: Vec<String> = serde_json::from_str::<Vec<String>>(&row.tags).unwrap_or_default();

        let fm = json!({
            "type": "event",
            "title": row.titulo,
            "slug": slug,
            "data": row.data,
            "hora": row.hora,
            "local": row.local,
            "tags": tags,
            "created": row.criado_em,
            "modified": row.criado_em,
            "_src_id": row.id
        });

        write_md_file(root, &path, &fm, &row.descricao_md);
        upsert_entry(
            dst,
            EntryArgs {
                path: &path,
                universe_key: "quilomboaraucaria",
                entry_type: "event",
                title: &row.titulo,
                frontmatter: &fm,
                body: &row.descricao_md,
                created_at: &row.criado_em,
                updated_at: &row.criado_em,
            },
        );
        count += 1;
    }

    println!("  {} eventos imported", count);
    count
}

/// Import missoes → Co entries of type `mission`.
fn import_missoes(src: &Connection, dst: &Connection, root: &Path) -> i64 {
    println!("Importing missoes → missions...");

    let query = "SELECT id, titulo, descricao, objetivo, status, criado_em \
                 FROM missoes ORDER BY id";

    let mut stmt = match src.prepare(query) {
        Ok(s) => s,
        Err(e) => {
            println!("  SKIP: missoes table not found ({})", e);
            return 0;
        }
    };

    struct Row {
        id: i64,
        titulo: String,
        descricao: String,
        objetivo: String,
        status: String,
        criado_em: String,
    }

    let rows: Vec<Row> = stmt
        .query_map([], |row| {
            Ok(Row {
                id: row.get(0)?,
                titulo: row.get(1)?,
                descricao: row.get(2).unwrap_or_default(),
                objetivo: row.get(3).unwrap_or_default(),
                status: row.get(4).unwrap_or_else(|_| "Aberta".to_string()),
                criado_em: row.get(5)?,
            })
        })
        .expect("Failed to query missoes")
        .filter_map(|r| r.ok())
        .collect();

    let mut count: i64 = 0;
    for row in &rows {
        let path = format!("missions/{}.md", row.id);
        let body = if row.objetivo.is_empty() {
            row.descricao.clone()
        } else {
            format!("{}\n\n## Objetivo\n\n{}", row.descricao, row.objetivo)
        };

        let fm = json!({
            "type": "mission",
            "title": row.titulo,
            "status": row.status,
            "objetivo": row.objetivo,
            "created": row.criado_em,
            "modified": row.criado_em,
            "_src_id": row.id
        });

        write_md_file(root, &path, &fm, &body);
        upsert_entry(
            dst,
            EntryArgs {
                path: &path,
                universe_key: "quilomboaraucaria",
                entry_type: "mission",
                title: &row.titulo,
                frontmatter: &fm,
                body: &body,
                created_at: &row.criado_em,
                updated_at: &row.criado_em,
            },
        );
        count += 1;
    }

    println!("  {} missoes imported", count);
    count
}
