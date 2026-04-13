#![allow(clippy::type_complexity)]

//! Migration script: quilombo-blog SQLite → co-web SQLite.
//!
//! Reads from quilombo-prod.db (quilombo-blog format, INTEGER user IDs)
//! and writes to co.db (co-web format, TEXT UUID user IDs).
//!
//! Usage:
//!   cargo run --bin migrate_quilombo -- --source /tmp/quilombo-prod.db --target /tmp/co-migrated.db
//!
//! The script:
//! 1. Opens source DB read-only
//! 2. Creates/opens target DB, runs co-web migrations (v3-v5)
//! 3. Maps old INTEGER user IDs → new UUIDs
//! 4. Migrates all tables preserving timestamps and relationships
//! 5. Reports row counts for validation

use std::collections::HashMap;
use std::path::PathBuf;

use clap::Parser;
use rusqlite::{Connection, OpenFlags, params};

#[derive(Parser)]
#[command(
    name = "migrate_quilombo",
    about = "Migrate quilombo-blog DB to co-web format"
)]
struct Args {
    /// Path to quilombo-blog source database (read-only)
    #[arg(long)]
    source: PathBuf,

    /// Path to co-web target database (will be created/updated)
    #[arg(long)]
    target: PathBuf,

    /// Dry run — report what would be migrated without writing
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}

fn main() {
    let args = Args::parse();

    println!("=== Quilombo Data Migration ===");
    println!("Source: {}", args.source.display());
    println!("Target: {}", args.target.display());
    if args.dry_run {
        println!("Mode: DRY RUN (no writes)");
    }
    println!();

    // Open source read-only
    let src = Connection::open_with_flags(&args.source, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("Failed to open source database");

    // Open/create target
    let dst = Connection::open(&args.target).expect("Failed to open target database");
    dst.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
        .ok();

    // Ensure co-web base schema exists (schema_version table)
    dst.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY
        );",
    )
    .ok();

    // Run co-web quilombo migrations (v3-v5)
    println!("Running co-web migrations...");
    run_quilombo_migrations(&dst);
    println!("  Migrations complete (v3-v5).\n");

    if args.dry_run {
        report_source_counts(&src);
        return;
    }

    // Step 1: Migrate usuarios (INTEGER id → TEXT UUID)
    let user_map = migrate_usuarios(&src, &dst);
    println!();

    // Step 2: Migrate eventos
    migrate_eventos(&src, &dst, &user_map);

    // Step 3: Migrate missoes
    migrate_missoes(&src, &dst, &user_map);

    // Step 4: Migrate participacoes
    migrate_participacoes(&src, &dst, &user_map);

    // Step 5: Migrate comentarios
    migrate_comentarios(&src, &dst, &user_map);

    // Step 6: Migrate mensagens
    migrate_mensagens(&src, &dst, &user_map);

    // Step 7: Migrate atividades
    migrate_atividades(&src, &dst, &user_map);

    // Step 8: Migrate visitantes + visitas → quilombo_telemetria (denormalize)
    migrate_telemetria(&src, &dst);

    println!("\n=== Migration Complete ===");
    report_target_counts(&dst);
}

/// Maps old INTEGER user ID → new UUID string.
fn migrate_usuarios(src: &Connection, dst: &Connection) -> HashMap<i64, String> {
    println!("Migrating usuarios...");
    let mut map = HashMap::new();

    let mut stmt = src
        .prepare(
            "SELECT id, usuario, nome, senha_hash, papel, bio, foto_url, criado_em, atualizado_em
             FROM usuarios ORDER BY id",
        )
        .expect("Failed to prepare usuarios query");

    let rows: Vec<(
        i64,
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
        String,
    )> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ))
        })
        .expect("Failed to query usuarios")
        .filter_map(|r| r.ok())
        .collect();

    for (old_id, usuario, nome, senha_hash, papel, bio, foto_url, criado_em, atualizado_em) in &rows
    {
        let new_id = uuid::Uuid::new_v4().to_string();
        map.insert(*old_id, new_id.clone());

        // Check if user already exists (by usuario)
        let exists: bool = dst
            .query_row(
                "SELECT COUNT(*) > 0 FROM quilombo_usuarios WHERE usuario = ?1",
                params![usuario],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if exists {
            println!("  SKIP: usuario '{}' already exists in target", usuario);
            // Update map to use existing ID
            if let Ok(existing_id) = dst.query_row(
                "SELECT id FROM quilombo_usuarios WHERE usuario = ?1",
                params![usuario],
                |row| row.get::<_, String>(0),
            ) {
                map.insert(*old_id, existing_id);
            }
            continue;
        }

        dst.execute(
            "INSERT INTO quilombo_usuarios (id, usuario, nome, senha_hash, papel, bio, foto_url, criado_em, atualizado_em)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![new_id, usuario, nome, senha_hash, papel, bio, foto_url, criado_em, atualizado_em],
        )
        .unwrap_or_else(|e| panic!("Failed to insert usuario '{}': {}", usuario, e));

        println!("  {} (#{}) → {}", usuario, old_id, new_id);
    }

    println!("  Migrated {} usuarios", rows.len());
    map
}

fn migrate_eventos(src: &Connection, dst: &Connection, user_map: &HashMap<i64, String>) {
    println!("Migrating eventos...");

    let mut stmt = src
        .prepare(
            "SELECT id, slug, titulo, data, hora, local, descricao_md, criado_por, criado_em
             FROM eventos ORDER BY id",
        )
        .expect("Failed to prepare eventos query");

    let rows: Vec<(
        i64,
        String,
        String,
        String,
        String,
        String,
        String,
        Option<i64>,
        String,
    )> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ))
        })
        .expect("Failed to query eventos")
        .filter_map(|r| r.ok())
        .collect();

    let mut count = 0;
    for (_, slug, titulo, data, hora, local, descricao_md, criado_por, criado_em) in &rows {
        let new_criado_por = criado_por.and_then(|id| user_map.get(&id).cloned());

        // Check if event with this slug already exists
        let exists: bool = dst
            .query_row(
                "SELECT COUNT(*) > 0 FROM quilombo_eventos WHERE slug = ?1",
                params![slug],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if exists {
            continue;
        }

        dst.execute(
            "INSERT INTO quilombo_eventos (titulo, data, hora, local, descricao_md, criado_por, criado_em, slug, tags)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '[]')",
            params![titulo, data, hora, local, descricao_md, new_criado_por, criado_em, slug],
        )
        .unwrap_or_else(|e| panic!("Failed to insert evento '{}': {}", titulo, e));
        count += 1;
    }

    println!(
        "  Migrated {} eventos (skipped {} existing)",
        count,
        rows.len() - count
    );
}

fn migrate_missoes(src: &Connection, dst: &Connection, user_map: &HashMap<i64, String>) {
    println!("Migrating missoes...");

    // We need to map old mission IDs to new ones for participacoes
    let mut stmt = src
        .prepare(
            "SELECT id, titulo, descricao, objetivo, status, criado_por, criado_em, atualizado_em
             FROM missoes ORDER BY id",
        )
        .expect("Failed to prepare missoes query");

    let rows: Vec<(
        i64,
        String,
        String,
        String,
        String,
        Option<i64>,
        String,
        String,
    )> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        })
        .expect("Failed to query missoes")
        .filter_map(|r| r.ok())
        .collect();

    let mut count = 0;
    for (_, titulo, descricao, objetivo, status, criado_por, criado_em, atualizado_em) in &rows {
        let new_criado_por = criado_por.and_then(|id| user_map.get(&id).cloned());

        // Check if mission with this title already exists
        let exists: bool = dst
            .query_row(
                "SELECT COUNT(*) > 0 FROM quilombo_missoes WHERE titulo = ?1 AND criado_em = ?2",
                params![titulo, criado_em],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if exists {
            continue;
        }

        dst.execute(
            "INSERT INTO quilombo_missoes (titulo, descricao, objetivo, status, criado_por, criado_em, atualizado_em)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![titulo, descricao, objetivo, status, new_criado_por, criado_em, atualizado_em],
        )
        .unwrap_or_else(|e| panic!("Failed to insert missao '{}': {}", titulo, e));
        count += 1;
    }

    println!("  Migrated {} missoes", count);
}

fn migrate_participacoes(src: &Connection, dst: &Connection, user_map: &HashMap<i64, String>) {
    println!("Migrating participacoes...");

    let mut stmt = src
        .prepare(
            "SELECT missao_id, usuario_id, status, criado_em
             FROM participacoes ORDER BY id",
        )
        .expect("Failed to prepare participacoes query");

    let rows: Vec<(i64, i64, String, String)> = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("Failed to query participacoes")
        .filter_map(|r| r.ok())
        .collect();

    let mut count = 0;
    for (old_missao_id, old_usuario_id, status, criado_em) in &rows {
        let new_usuario_id = match user_map.get(old_usuario_id) {
            Some(id) => id,
            None => {
                println!(
                    "  WARN: usuario_id {} not found in map, skipping",
                    old_usuario_id
                );
                continue;
            }
        };

        // Mission IDs are AUTOINCREMENT — in a fresh DB they'll match if migrated in order.
        // But to be safe, find mission by position.
        let new_missao_id: Option<i64> = dst
            .query_row(
                "SELECT id FROM quilombo_missoes ORDER BY id LIMIT 1 OFFSET ?1",
                params![old_missao_id - 1],
                |row| row.get(0),
            )
            .ok();

        let missao_id = match new_missao_id {
            Some(id) => id,
            None => {
                println!(
                    "  WARN: missao_id {} not found in target, skipping",
                    old_missao_id
                );
                continue;
            }
        };

        dst.execute(
            "INSERT OR IGNORE INTO quilombo_participacoes (missao_id, usuario_id, status, criado_em)
             VALUES (?1, ?2, ?3, ?4)",
            params![missao_id, new_usuario_id, status, criado_em],
        )
        .ok();
        count += 1;
    }

    println!("  Migrated {} participacoes", count);
}

fn migrate_comentarios(src: &Connection, dst: &Connection, user_map: &HashMap<i64, String>) {
    println!("Migrating comentarios...");

    // In quilombo-blog: publicacao_id (INTEGER FK → publicacoes) or evento_id (INTEGER FK → eventos)
    // In co-web: relato_slug (TEXT) or evento_id (INTEGER)
    // We need to resolve publicacao_id → slug via conteudo table

    let mut stmt = src
        .prepare(
            "SELECT c.id, c.publicacao_id, c.evento_id, c.autor_id, c.nome_exibicao,
                    c.conteudo, c.anonimo, c.criado_em,
                    p.slug as pub_slug,
                    e.slug as evt_slug
             FROM comentarios c
             LEFT JOIN publicacoes p ON c.publicacao_id = p.id
             LEFT JOIN conteudo ct ON c.publicacao_id = ct.id AND ct.tipo = 'post'
             LEFT JOIN eventos e ON c.evento_id = e.id
             ORDER BY c.id",
        )
        .expect("Failed to prepare comentarios query");

    let rows: Vec<(
        i64,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<String>,
        String,
        bool,
        String,
        Option<String>,
        Option<String>,
    )> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
            ))
        })
        .expect("Failed to query comentarios")
        .filter_map(|r| r.ok())
        .collect();

    // Also try to resolve publicacao_id via conteudo table (since publicacoes may be empty)
    let slug_map: HashMap<i64, String> = {
        let mut stmt = src
            .prepare("SELECT id, slug FROM conteudo WHERE tipo = 'post'")
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    };

    let mut count = 0;
    for (
        _,
        publicacao_id,
        evento_id,
        autor_id,
        nome_exibicao,
        conteudo,
        anonimo,
        criado_em,
        pub_slug,
        _evt_slug,
    ) in &rows
    {
        let new_autor_id = autor_id.and_then(|id| user_map.get(&id).cloned());

        // Resolve relato slug
        let relato_slug = pub_slug
            .clone()
            .or_else(|| publicacao_id.and_then(|pid| slug_map.get(&pid).cloned()));

        // Resolve new evento_id in target (by slug lookup)
        let new_evento_id: Option<i64> = evento_id.and_then(|eid| {
            // Get slug from source evento
            let slug: Option<String> = src
                .query_row(
                    "SELECT slug FROM eventos WHERE id = ?1",
                    params![eid],
                    |row| row.get(0),
                )
                .ok();
            slug.and_then(|s| {
                dst.query_row(
                    "SELECT id FROM quilombo_eventos WHERE slug = ?1",
                    params![s],
                    |row| row.get(0),
                )
                .ok()
            })
        });

        dst.execute(
            "INSERT INTO quilombo_comentarios (relato_slug, publicacao_slug, evento_id, autor_id, nome_exibicao, conteudo, anonimo, criado_em)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                relato_slug,
                relato_slug,
                new_evento_id,
                new_autor_id,
                nome_exibicao,
                conteudo,
                anonimo,
                criado_em,
            ],
        )
        .unwrap_or_else(|e| panic!("Failed to insert comentario: {}", e));
        count += 1;
    }

    println!("  Migrated {} comentarios", count);
}

fn migrate_mensagens(src: &Connection, dst: &Connection, user_map: &HashMap<i64, String>) {
    println!("Migrating mensagens...");

    let mut stmt = src
        .prepare(
            "SELECT remetente_id, destinatario_id, missao_id, assunto, conteudo,
                    nome_remetente, email_remetente, lida, criado_em
             FROM mensagens ORDER BY id",
        )
        .expect("Failed to prepare mensagens query");

    let rows: Vec<(
        Option<i64>,
        Option<i64>,
        Option<i64>,
        String,
        String,
        Option<String>,
        Option<String>,
        bool,
        String,
    )> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ))
        })
        .expect("Failed to query mensagens")
        .filter_map(|r| r.ok())
        .collect();

    let mut count = 0;
    for (rem_id, dest_id, missao_id, assunto, conteudo, nome_rem, email_rem, lida, criado_em) in
        &rows
    {
        let new_rem = rem_id.and_then(|id| user_map.get(&id).cloned());
        let new_dest = dest_id.and_then(|id| user_map.get(&id).cloned());

        // Map missao_id (same offset approach)
        let new_missao_id: Option<i64> = missao_id.and_then(|mid| {
            dst.query_row(
                "SELECT id FROM quilombo_missoes ORDER BY id LIMIT 1 OFFSET ?1",
                params![mid - 1],
                |row| row.get(0),
            )
            .ok()
        });

        dst.execute(
            "INSERT INTO quilombo_mensagens (remetente_id, destinatario_id, missao_id, assunto, conteudo, nome_remetente, email_remetente, lida, criado_em)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![new_rem, new_dest, new_missao_id, assunto, conteudo, nome_rem, email_rem, lida, criado_em],
        )
        .unwrap_or_else(|e| panic!("Failed to insert mensagem: {}", e));
        count += 1;
    }

    println!("  Migrated {} mensagens", count);
}

fn migrate_atividades(src: &Connection, dst: &Connection, user_map: &HashMap<i64, String>) {
    println!("Migrating atividades...");

    let mut stmt = src
        .prepare(
            "SELECT acao, entidade, entidade_id, usuario_id, conteudo, criado_em
             FROM atividades ORDER BY id",
        )
        .expect("Failed to prepare atividades query");

    let rows: Vec<(
        String,
        String,
        Option<String>,
        Option<i64>,
        Option<String>,
        String,
    )> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .expect("Failed to query atividades")
        .filter_map(|r| r.ok())
        .collect();

    let mut count = 0;
    for (acao, entidade, entidade_id, usuario_id, conteudo, criado_em) in &rows {
        let new_usuario_id = usuario_id.and_then(|id| user_map.get(&id).cloned());

        dst.execute(
            "INSERT INTO quilombo_atividades (acao, entidade, entidade_id, usuario_id, conteudo, criado_em)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![acao, entidade, entidade_id, new_usuario_id, conteudo, criado_em],
        )
        .unwrap_or_else(|e| panic!("Failed to insert atividade: {}", e));
        count += 1;
    }

    println!("  Migrated {} atividades", count);
}

fn migrate_telemetria(src: &Connection, dst: &Connection) {
    println!("Migrating telemetria (visitantes + visitas → quilombo_telemetria)...");

    // Denormalize: join visitas with visitantes to get token
    let mut stmt = src
        .prepare(
            "SELECT v.path, v.referrer, v.user_agent, v.ip_hash, v.criado_em, v.duracao_ms,
                    vt.token
             FROM visitas v
             LEFT JOIN visitantes vt ON v.visitante_id = vt.id
             ORDER BY v.id",
        )
        .expect("Failed to prepare telemetria query");

    let mut count = 0;

    // Process in batches to avoid excessive memory usage
    let tx = dst
        .unchecked_transaction()
        .expect("Failed to begin transaction");

    {
        let mut insert = tx
            .prepare(
                "INSERT INTO quilombo_telemetria (visitante_token, path, referrer, user_agent, ip_hash, duracao_ms, criado_em)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .expect("Failed to prepare telemetria insert");

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,         // path
                    row.get::<_, Option<String>>(1)?, // referrer
                    row.get::<_, Option<String>>(2)?, // user_agent
                    row.get::<_, Option<String>>(3)?, // ip_hash
                    row.get::<_, String>(4)?,         // criado_em
                    row.get::<_, Option<i64>>(5)?,    // duracao_ms
                    row.get::<_, Option<String>>(6)?, // token
                ))
            })
            .expect("Failed to query visitas");

        for (path, referrer, user_agent, ip_hash, criado_em, duracao_ms, token) in rows.flatten() {
            insert
                .execute(params![
                    token, path, referrer, user_agent, ip_hash, duracao_ms, criado_em
                ])
                .ok();
            count += 1;
        }
    }

    tx.commit()
        .expect("Failed to commit telemetria transaction");
    println!("  Migrated {} visit records to quilombo_telemetria", count);
}

fn report_source_counts(src: &Connection) {
    println!("=== Source Database (quilombo-blog) ===");
    let tables = [
        "usuarios",
        "eventos",
        "missoes",
        "participacoes",
        "comentarios",
        "mensagens",
        "atividades",
        "visitantes",
        "visitas",
        "publicacoes",
        "conteudo",
    ];
    for table in &tables {
        let count: i64 = src
            .query_row(&format!("SELECT COUNT(*) FROM {}", table), [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        println!("  {:<20} {:>6} rows", table, count);
    }
}

fn report_target_counts(dst: &Connection) {
    println!("Target Database (co-web):");
    let tables = [
        "quilombo_usuarios",
        "quilombo_eventos",
        "quilombo_missoes",
        "quilombo_participacoes",
        "quilombo_comentarios",
        "quilombo_mensagens",
        "quilombo_atividades",
        "quilombo_telemetria",
    ];
    for table in &tables {
        let count: i64 = dst
            .query_row(&format!("SELECT COUNT(*) FROM {}", table), [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        println!("  {:<25} {:>6} rows", table, count);
    }
}

// --- Co-web migrations (copied from quilombo_storage.rs to keep binary standalone) ---

fn slugify(input: &str) -> String {
    input
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ã' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'õ' | 'ô' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            ' ' | '_' => '-',
            c if c.is_ascii_alphanumeric() || c == '-' => c,
            _ => '-',
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn run_quilombo_migrations(conn: &Connection) {
    let current_version: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if current_version < 3 {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS quilombo_usuarios (
                id TEXT PRIMARY KEY,
                usuario TEXT UNIQUE NOT NULL,
                nome TEXT NOT NULL,
                senha_hash TEXT NOT NULL,
                papel TEXT NOT NULL DEFAULT 'membro',
                bio TEXT,
                foto_url TEXT,
                criado_em TEXT NOT NULL DEFAULT (datetime('now')),
                atualizado_em TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS quilombo_eventos (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                titulo TEXT NOT NULL,
                data TEXT NOT NULL,
                hora TEXT NOT NULL,
                local TEXT NOT NULL,
                descricao_md TEXT NOT NULL DEFAULT '',
                criado_por TEXT REFERENCES quilombo_usuarios(id),
                criado_em TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS quilombo_missoes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                titulo TEXT NOT NULL,
                descricao TEXT NOT NULL DEFAULT '',
                objetivo TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'aberta',
                criado_por TEXT REFERENCES quilombo_usuarios(id),
                criado_em TEXT NOT NULL DEFAULT (datetime('now')),
                atualizado_em TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS quilombo_participacoes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                missao_id INTEGER NOT NULL REFERENCES quilombo_missoes(id),
                usuario_id TEXT NOT NULL REFERENCES quilombo_usuarios(id),
                status TEXT NOT NULL DEFAULT 'pendente',
                criado_em TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(missao_id, usuario_id)
            );

            CREATE TABLE IF NOT EXISTS quilombo_comentarios (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                publicacao_slug TEXT,
                evento_id INTEGER REFERENCES quilombo_eventos(id),
                autor_id TEXT REFERENCES quilombo_usuarios(id),
                nome_exibicao TEXT,
                conteudo TEXT NOT NULL,
                anonimo INTEGER NOT NULL DEFAULT 1,
                criado_em TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS quilombo_mensagens (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                remetente_id TEXT REFERENCES quilombo_usuarios(id),
                destinatario_id TEXT REFERENCES quilombo_usuarios(id),
                missao_id INTEGER REFERENCES quilombo_missoes(id),
                assunto TEXT NOT NULL DEFAULT '',
                conteudo TEXT NOT NULL,
                nome_remetente TEXT,
                email_remetente TEXT,
                lida INTEGER NOT NULL DEFAULT 0,
                criado_em TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS quilombo_atividades (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                acao TEXT NOT NULL,
                entidade TEXT NOT NULL,
                entidade_id TEXT,
                usuario_id TEXT REFERENCES quilombo_usuarios(id),
                conteudo TEXT,
                criado_em TEXT NOT NULL DEFAULT (datetime('now'))
            );

            INSERT INTO schema_version (version) VALUES (3);
            ",
        )
        .expect("Failed to run quilombo migration v3");
    }

    if current_version < 4 {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS quilombo_perfis (
                usuario_id TEXT PRIMARY KEY REFERENCES quilombo_usuarios(id),
                nome_exibicao TEXT,
                bio TEXT,
                foto_url TEXT,
                atualizado_em TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS quilombo_telemetria (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                visitante_token TEXT,
                usuario_id TEXT,
                path TEXT NOT NULL,
                referrer TEXT,
                user_agent TEXT,
                ip_hash TEXT,
                duracao_ms INTEGER,
                criado_em TEXT NOT NULL DEFAULT (datetime('now'))
            );

            ALTER TABLE quilombo_comentarios ADD COLUMN relato_slug TEXT;

            INSERT INTO schema_version (version) VALUES (4);
            ",
        )
        .expect("Failed to run quilombo migration v4");

        conn.execute(
            "UPDATE quilombo_comentarios SET relato_slug = publicacao_slug WHERE publicacao_slug IS NOT NULL",
            [],
        )
        .ok();
    }

    if current_version < 5 {
        conn.execute_batch(
            "
            ALTER TABLE quilombo_eventos ADD COLUMN slug TEXT;
            ALTER TABLE quilombo_eventos ADD COLUMN tags TEXT DEFAULT '[]';
            ",
        )
        .expect("Failed to add slug/tags columns to quilombo_eventos");

        let mut stmt = conn
            .prepare("SELECT id, titulo, data FROM quilombo_eventos WHERE slug IS NULL")
            .expect("Failed to prepare slug generation");
        let rows: Vec<(i64, String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("Failed to query events")
            .filter_map(|r| r.ok())
            .collect();

        for (id, titulo, data) in rows {
            let slug = slugify(&format!("{titulo}-{data}"));
            conn.execute(
                "UPDATE quilombo_eventos SET slug = ?1 WHERE id = ?2",
                params![slug, id],
            )
            .ok();
        }

        conn.execute("INSERT INTO schema_version (version) VALUES (5)", [])
            .expect("Failed to insert v5 schema version");
    }
}
