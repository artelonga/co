use rusqlite::{Connection, params};

use crate::quilombo_models::*;

/// Sanitize HTML entities to prevent XSS.
pub fn sanitize(text: &str) -> String {
    text.replace('<', "&lt;")
        .replace('>', "&gt;")
        .trim()
        .to_string()
}

/// Run quilombo-specific migrations (v3+).
pub fn run_quilombo_migrations(conn: &Connection) {
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
            -- Profile overrides (user-editable, merges with membros/*.md)
            CREATE TABLE IF NOT EXISTS quilombo_perfis (
                usuario_id TEXT PRIMARY KEY REFERENCES quilombo_usuarios(id),
                nome_exibicao TEXT,
                bio TEXT,
                foto_url TEXT,
                atualizado_em TEXT NOT NULL DEFAULT (datetime('now'))
            );

            -- Telemetry / visit tracking
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

            -- Rename publicacao_slug → relato_slug (SQLite can't rename columns < 3.25, so add new + copy)
            ALTER TABLE quilombo_comentarios ADD COLUMN relato_slug TEXT;

            INSERT INTO schema_version (version) VALUES (4);
            ",
        )
        .expect("Failed to run quilombo migration v4");

        // Copy existing publicacao_slug values to relato_slug
        conn.execute(
            "UPDATE quilombo_comentarios SET relato_slug = publicacao_slug WHERE publicacao_slug IS NOT NULL",
            [],
        )
        .ok();
    }

    if current_version < 5 {
        // Add slug and tags to eventos table
        conn.execute_batch(
            "
            ALTER TABLE quilombo_eventos ADD COLUMN slug TEXT;
            ALTER TABLE quilombo_eventos ADD COLUMN tags TEXT DEFAULT '[]';
            ",
        )
        .expect("Failed to add slug/tags columns to quilombo_eventos");

        // Generate slugs for existing events: slugify(titulo-data)
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

/// Convert a string to a URL-safe slug.
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

fn parse_datetime(s: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").map(|ndt| ndt.and_utc())
        })
        .unwrap_or_else(|_| chrono::Utc::now())
}

// --- Usuarios ---

pub fn criar_usuario(
    conn: &Connection,
    id: &str,
    usuario: &str,
    nome: &str,
    senha_hash: &str,
) -> Result<Usuario, String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO quilombo_usuarios (id, usuario, nome, senha_hash, criado_em, atualizado_em) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![id, usuario, nome, senha_hash, now],
    )
    .map_err(|e| format!("Failed to create user: {e}"))?;

    obter_usuario_por_id(conn, id).ok_or_else(|| "User not found after insert".to_string())
}

pub fn obter_usuario_por_id(conn: &Connection, id: &str) -> Option<Usuario> {
    conn.query_row(
        "SELECT id, usuario, nome, papel, bio, foto_url, criado_em, atualizado_em FROM quilombo_usuarios WHERE id = ?1",
        params![id],
        |row| {
            Ok(Usuario {
                id: row.get(0)?,
                usuario: row.get(1)?,
                nome: row.get(2)?,
                papel: row.get::<_, String>(3)?.parse().unwrap_or(Papel::Membro),
                bio: row.get(4)?,
                foto_url: row.get(5)?,
                criado_em: parse_datetime(&row.get::<_, String>(6)?),
                atualizado_em: parse_datetime(&row.get::<_, String>(7)?),
            })
        },
    )
    .ok()
}

pub fn obter_usuario_por_nome(conn: &Connection, usuario: &str) -> Option<(Usuario, String)> {
    conn.query_row(
        "SELECT id, usuario, nome, papel, bio, foto_url, criado_em, atualizado_em, senha_hash FROM quilombo_usuarios WHERE usuario = ?1",
        params![usuario],
        |row| {
            Ok((
                Usuario {
                    id: row.get(0)?,
                    usuario: row.get(1)?,
                    nome: row.get(2)?,
                    papel: row.get::<_, String>(3)?.parse().unwrap_or(Papel::Membro),
                    bio: row.get(4)?,
                    foto_url: row.get(5)?,
                    criado_em: parse_datetime(&row.get::<_, String>(6)?),
                    atualizado_em: parse_datetime(&row.get::<_, String>(7)?),
                },
                row.get::<_, String>(8)?,
            ))
        },
    )
    .ok()
}

pub fn listar_membros(conn: &Connection) -> Vec<Membro> {
    let mut stmt = conn
        .prepare("SELECT id, usuario, nome, bio, foto_url, criado_em FROM quilombo_usuarios ORDER BY criado_em DESC")
        .expect("Failed to prepare listar_membros");

    stmt.query_map([], |row| {
        Ok(Membro {
            id: row.get(0)?,
            usuario: row.get(1)?,
            nome: row.get(2)?,
            bio: row.get(3)?,
            foto_url: row.get(4)?,
            criado_em: parse_datetime(&row.get::<_, String>(5)?),
        })
    })
    .expect("Failed to list members")
    .filter_map(|r| r.ok())
    .collect()
}

pub fn atualizar_perfil(
    conn: &Connection,
    id: &str,
    update: &AtualizarPerfil,
) -> Result<Usuario, String> {
    let now = chrono::Utc::now().to_rfc3339();
    let current = obter_usuario_por_id(conn, id).ok_or("User not found")?;

    let nome = update.nome.as_deref().unwrap_or(&current.nome);
    let bio = update.bio.as_deref().or(current.bio.as_deref());
    let foto_url = update.foto_url.as_deref().or(current.foto_url.as_deref());

    conn.execute(
        "UPDATE quilombo_usuarios SET nome = ?1, bio = ?2, foto_url = ?3, atualizado_em = ?4 WHERE id = ?5",
        params![nome, bio, foto_url, now, id],
    )
    .map_err(|e| format!("Failed to update profile: {e}"))?;

    obter_usuario_por_id(conn, id).ok_or_else(|| "User not found after update".to_string())
}

// --- Eventos ---

pub fn listar_eventos(conn: &Connection) -> Vec<Evento> {
    let mut stmt = conn
        .prepare("SELECT id, titulo, data, hora, local, descricao_md, criado_por, criado_em FROM quilombo_eventos ORDER BY data DESC, hora DESC")
        .expect("Failed to prepare listar_eventos");

    stmt.query_map([], |row| {
        Ok(Evento {
            id: row.get(0)?,
            titulo: row.get(1)?,
            data: row.get(2)?,
            hora: row.get(3)?,
            local: row.get(4)?,
            descricao_md: row.get(5)?,
            criado_por: row.get(6)?,
            criado_em: parse_datetime(&row.get::<_, String>(7)?),
        })
    })
    .expect("Failed to list events")
    .filter_map(|r| r.ok())
    .collect()
}

pub fn obter_evento(conn: &Connection, id: i64) -> Option<Evento> {
    conn.query_row(
        "SELECT id, titulo, data, hora, local, descricao_md, criado_por, criado_em FROM quilombo_eventos WHERE id = ?1",
        params![id],
        |row| {
            Ok(Evento {
                id: row.get(0)?,
                titulo: row.get(1)?,
                data: row.get(2)?,
                hora: row.get(3)?,
                local: row.get(4)?,
                descricao_md: row.get(5)?,
                criado_por: row.get(6)?,
                criado_em: parse_datetime(&row.get::<_, String>(7)?),
            })
        },
    )
    .ok()
}

pub fn obter_evento_por_slug(conn: &Connection, slug: &str) -> Option<Evento> {
    conn.query_row(
        "SELECT id, titulo, data, hora, local, descricao_md, criado_por, criado_em FROM quilombo_eventos WHERE slug = ?1",
        params![slug],
        |row| {
            Ok(Evento {
                id: row.get(0)?,
                titulo: row.get(1)?,
                data: row.get(2)?,
                hora: row.get(3)?,
                local: row.get(4)?,
                descricao_md: row.get(5)?,
                criado_por: row.get(6)?,
                criado_em: parse_datetime(&row.get::<_, String>(7)?),
            })
        },
    )
    .ok()
}

pub fn criar_evento(
    conn: &Connection,
    evento: &CriarEvento,
    criado_por: &str,
) -> Result<Evento, String> {
    let slug = slugify(&format!("{}-{}", evento.titulo, evento.data));
    conn.execute(
        "INSERT INTO quilombo_eventos (titulo, data, hora, local, descricao_md, criado_por, slug) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            sanitize(&evento.titulo),
            evento.data,
            evento.hora,
            sanitize(&evento.local),
            sanitize(&evento.descricao_md),
            criado_por,
            slug,
        ],
    )
    .map_err(|e| format!("Failed to create event: {e}"))?;

    let id = conn.last_insert_rowid();
    obter_evento(conn, id).ok_or_else(|| "Event not found after insert".to_string())
}

pub fn atualizar_evento(
    conn: &Connection,
    id: i64,
    update: &AtualizarEvento,
) -> Result<Evento, String> {
    let current = obter_evento(conn, id).ok_or("Event not found")?;

    let titulo = update
        .titulo
        .as_deref()
        .map(sanitize)
        .unwrap_or(current.titulo);
    let data = update.data.as_deref().unwrap_or(&current.data).to_string();
    let hora = update.hora.as_deref().unwrap_or(&current.hora).to_string();
    let local = update
        .local
        .as_deref()
        .map(sanitize)
        .unwrap_or(current.local);
    let descricao_md = update
        .descricao_md
        .as_deref()
        .map(sanitize)
        .unwrap_or(current.descricao_md);

    conn.execute(
        "UPDATE quilombo_eventos SET titulo = ?1, data = ?2, hora = ?3, local = ?4, descricao_md = ?5 WHERE id = ?6",
        params![titulo, data, hora, local, descricao_md, id],
    )
    .map_err(|e| format!("Failed to update event: {e}"))?;

    obter_evento(conn, id).ok_or_else(|| "Event not found after update".to_string())
}

pub fn excluir_evento(conn: &Connection, id: i64) -> Result<(), String> {
    let changes = conn
        .execute("DELETE FROM quilombo_eventos WHERE id = ?1", params![id])
        .map_err(|e| format!("Failed to delete event: {e}"))?;
    if changes == 0 {
        return Err("Event not found".to_string());
    }
    Ok(())
}

// --- Missoes ---

pub fn listar_missoes(conn: &Connection) -> Vec<Missao> {
    let mut stmt = conn
        .prepare(
            "SELECT m.id, m.titulo, m.descricao, m.objetivo, m.status, m.criado_por, m.criado_em, m.atualizado_em,
                    (SELECT COUNT(*) FROM quilombo_participacoes p WHERE p.missao_id = m.id AND p.status = 'aprovado') as participantes
             FROM quilombo_missoes m ORDER BY m.criado_em DESC"
        )
        .expect("Failed to prepare listar_missoes");

    stmt.query_map([], |row| {
        Ok(Missao {
            id: row.get(0)?,
            titulo: row.get(1)?,
            descricao: row.get(2)?,
            objetivo: row.get(3)?,
            status: row
                .get::<_, String>(4)?
                .parse()
                .unwrap_or(StatusMissao::Aberta),
            criado_por: row.get(5)?,
            criado_em: parse_datetime(&row.get::<_, String>(6)?),
            atualizado_em: parse_datetime(&row.get::<_, String>(7)?),
            participantes: row.get(8)?,
        })
    })
    .expect("Failed to list missions")
    .filter_map(|r| r.ok())
    .collect()
}

pub fn obter_missao(conn: &Connection, id: i64) -> Option<Missao> {
    conn.query_row(
        "SELECT m.id, m.titulo, m.descricao, m.objetivo, m.status, m.criado_por, m.criado_em, m.atualizado_em,
                (SELECT COUNT(*) FROM quilombo_participacoes p WHERE p.missao_id = m.id AND p.status = 'aprovado') as participantes
         FROM quilombo_missoes m WHERE m.id = ?1",
        params![id],
        |row| {
            Ok(Missao {
                id: row.get(0)?,
                titulo: row.get(1)?,
                descricao: row.get(2)?,
                objetivo: row.get(3)?,
                status: row.get::<_, String>(4)?.parse().unwrap_or(StatusMissao::Aberta),
                criado_por: row.get(5)?,
                criado_em: parse_datetime(&row.get::<_, String>(6)?),
                atualizado_em: parse_datetime(&row.get::<_, String>(7)?),
                participantes: row.get(8)?,
            })
        },
    )
    .ok()
}

pub fn criar_missao(
    conn: &Connection,
    missao: &CriarMissao,
    criado_por: &str,
) -> Result<Missao, String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO quilombo_missoes (titulo, descricao, objetivo, criado_por, criado_em, atualizado_em) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![
            sanitize(&missao.titulo),
            sanitize(&missao.descricao),
            sanitize(&missao.objetivo),
            criado_por,
            now,
        ],
    )
    .map_err(|e| format!("Failed to create mission: {e}"))?;

    let id = conn.last_insert_rowid();

    // Creator auto-joins as approved
    conn.execute(
        "INSERT INTO quilombo_participacoes (missao_id, usuario_id, status, criado_em) VALUES (?1, ?2, 'aprovado', ?3)",
        params![id, criado_por, now],
    )
    .ok();

    obter_missao(conn, id).ok_or_else(|| "Mission not found after insert".to_string())
}

pub fn atualizar_missao(
    conn: &Connection,
    id: i64,
    update: &AtualizarMissao,
) -> Result<Missao, String> {
    let current = obter_missao(conn, id).ok_or("Mission not found")?;
    let now = chrono::Utc::now().to_rfc3339();

    let titulo = update
        .titulo
        .as_deref()
        .map(sanitize)
        .unwrap_or(current.titulo);
    let descricao = update
        .descricao
        .as_deref()
        .map(sanitize)
        .unwrap_or(current.descricao);
    let objetivo = update
        .objetivo
        .as_deref()
        .map(sanitize)
        .unwrap_or(current.objetivo);
    let status = update
        .status
        .as_ref()
        .unwrap_or(&current.status)
        .to_string();

    conn.execute(
        "UPDATE quilombo_missoes SET titulo = ?1, descricao = ?2, objetivo = ?3, status = ?4, atualizado_em = ?5 WHERE id = ?6",
        params![titulo, descricao, objetivo, status, now, id],
    )
    .map_err(|e| format!("Failed to update mission: {e}"))?;

    obter_missao(conn, id).ok_or_else(|| "Mission not found after update".to_string())
}

// --- Participacoes ---

pub fn participar_missao(
    conn: &Connection,
    missao_id: i64,
    usuario_id: &str,
) -> Result<Participacao, String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO quilombo_participacoes (missao_id, usuario_id, status, criado_em) VALUES (?1, ?2, 'pendente', ?3)",
        params![missao_id, usuario_id, now],
    )
    .map_err(|e| format!("Failed to join mission: {e}"))?;

    let id = conn.last_insert_rowid();
    conn.query_row(
        "SELECT id, missao_id, usuario_id, status, criado_em FROM quilombo_participacoes WHERE id = ?1",
        params![id],
        |row| {
            Ok(Participacao {
                id: row.get(0)?,
                missao_id: row.get(1)?,
                usuario_id: row.get(2)?,
                status: row.get::<_, String>(3)?.parse().unwrap_or(StatusParticipacao::Pendente),
                criado_em: parse_datetime(&row.get::<_, String>(4)?),
            })
        },
    )
    .map_err(|e| format!("Participation not found: {e}"))
}

pub fn listar_participacoes(conn: &Connection, missao_id: i64) -> Vec<Participacao> {
    let mut stmt = conn
        .prepare("SELECT id, missao_id, usuario_id, status, criado_em FROM quilombo_participacoes WHERE missao_id = ?1 ORDER BY criado_em")
        .expect("Failed to prepare listar_participacoes");

    stmt.query_map(params![missao_id], |row| {
        Ok(Participacao {
            id: row.get(0)?,
            missao_id: row.get(1)?,
            usuario_id: row.get(2)?,
            status: row
                .get::<_, String>(3)?
                .parse()
                .unwrap_or(StatusParticipacao::Pendente),
            criado_em: parse_datetime(&row.get::<_, String>(4)?),
        })
    })
    .expect("Failed to list participations")
    .filter_map(|r| r.ok())
    .collect()
}

pub fn atualizar_participacao(
    conn: &Connection,
    missao_id: i64,
    usuario_id: &str,
    status: &StatusParticipacao,
) -> Result<(), String> {
    let changes = conn
        .execute(
            "UPDATE quilombo_participacoes SET status = ?1 WHERE missao_id = ?2 AND usuario_id = ?3",
            params![status.to_string(), missao_id, usuario_id],
        )
        .map_err(|e| format!("Failed to update participation: {e}"))?;
    if changes == 0 {
        return Err("Participation not found".to_string());
    }
    Ok(())
}

// --- Comentarios ---

pub fn listar_comentarios(conn: &Connection, query: &ComentarioQuery) -> Vec<Comentario> {
    let (sql, param_values): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(slug) =
        &query.relato
    {
        (
            "SELECT id, publicacao_slug, evento_id, autor_id, nome_exibicao, conteudo, anonimo, criado_em FROM quilombo_comentarios WHERE publicacao_slug = ?1 ORDER BY criado_em",
            vec![Box::new(slug.clone())],
        )
    } else if let Some(eid) = query.evento {
        (
            "SELECT id, publicacao_slug, evento_id, autor_id, nome_exibicao, conteudo, anonimo, criado_em FROM quilombo_comentarios WHERE evento_id = ?1 ORDER BY criado_em",
            vec![Box::new(eid)],
        )
    } else {
        (
            "SELECT id, publicacao_slug, evento_id, autor_id, nome_exibicao, conteudo, anonimo, criado_em FROM quilombo_comentarios ORDER BY criado_em DESC LIMIT 50",
            vec![],
        )
    };

    let mut stmt = conn
        .prepare(sql)
        .expect("Failed to prepare listar_comentarios");
    let params: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();

    stmt.query_map(params.as_slice(), |row| {
        Ok(Comentario {
            id: row.get(0)?,
            publicacao_slug: row.get(1)?,
            evento_id: row.get(2)?,
            autor_id: row.get(3)?,
            nome_exibicao: row.get(4)?,
            conteudo: row.get(5)?,
            anonimo: row.get::<_, i64>(6)? != 0,
            criado_em: parse_datetime(&row.get::<_, String>(7)?),
        })
    })
    .expect("Failed to list comments")
    .filter_map(|r| r.ok())
    .collect()
}

pub fn criar_comentario(
    conn: &Connection,
    comment: &CriarComentario,
    autor_id: Option<&str>,
) -> Result<Comentario, String> {
    let conteudo = sanitize(&comment.conteudo);
    if conteudo.is_empty() || conteudo.len() > 2000 {
        return Err("Comment must be 1-2000 characters".to_string());
    }

    let nome = comment.nome_exibicao.as_deref().map(sanitize);

    conn.execute(
        "INSERT INTO quilombo_comentarios (publicacao_slug, evento_id, autor_id, nome_exibicao, conteudo, anonimo) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            comment.publicacao_slug,
            comment.evento_id,
            autor_id,
            nome,
            conteudo,
            comment.anonimo as i64,
        ],
    )
    .map_err(|e| format!("Failed to create comment: {e}"))?;

    let id = conn.last_insert_rowid();
    conn.query_row(
        "SELECT id, publicacao_slug, evento_id, autor_id, nome_exibicao, conteudo, anonimo, criado_em FROM quilombo_comentarios WHERE id = ?1",
        params![id],
        |row| {
            Ok(Comentario {
                id: row.get(0)?,
                publicacao_slug: row.get(1)?,
                evento_id: row.get(2)?,
                autor_id: row.get(3)?,
                nome_exibicao: row.get(4)?,
                conteudo: row.get(5)?,
                anonimo: row.get::<_, i64>(6)? != 0,
                criado_em: parse_datetime(&row.get::<_, String>(7)?),
            })
        },
    )
    .map_err(|e| format!("Comment not found: {e}"))
}

// --- Mensagens ---

pub fn listar_mensagens(conn: &Connection, usuario_id: &str) -> Vec<Mensagem> {
    let mut stmt = conn
        .prepare(
            "SELECT id, remetente_id, destinatario_id, missao_id, assunto, conteudo, nome_remetente, email_remetente, lida, criado_em
             FROM quilombo_mensagens
             WHERE destinatario_id = ?1 OR remetente_id = ?1
             ORDER BY criado_em DESC LIMIT 100"
        )
        .expect("Failed to prepare listar_mensagens");

    stmt.query_map(params![usuario_id], |row| {
        Ok(Mensagem {
            id: row.get(0)?,
            remetente_id: row.get(1)?,
            destinatario_id: row.get(2)?,
            missao_id: row.get(3)?,
            assunto: row.get(4)?,
            conteudo: row.get(5)?,
            nome_remetente: row.get(6)?,
            email_remetente: row.get(7)?,
            lida: row.get::<_, i64>(8)? != 0,
            criado_em: parse_datetime(&row.get::<_, String>(9)?),
        })
    })
    .expect("Failed to list messages")
    .filter_map(|r| r.ok())
    .collect()
}

pub fn criar_mensagem(
    conn: &Connection,
    msg: &CriarMensagem,
    remetente_id: Option<&str>,
) -> Result<Mensagem, String> {
    let conteudo = sanitize(&msg.conteudo);
    if conteudo.is_empty() || conteudo.len() > 5000 {
        return Err("Message must be 1-5000 characters".to_string());
    }
    let assunto = sanitize(&msg.assunto);
    let nome = msg.nome_remetente.as_deref().map(sanitize);

    conn.execute(
        "INSERT INTO quilombo_mensagens (remetente_id, destinatario_id, missao_id, assunto, conteudo, nome_remetente, email_remetente) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            remetente_id,
            msg.destinatario_id,
            msg.missao_id,
            assunto,
            conteudo,
            nome,
            msg.email_remetente,
        ],
    )
    .map_err(|e| format!("Failed to create message: {e}"))?;

    let id = conn.last_insert_rowid();
    conn.query_row(
        "SELECT id, remetente_id, destinatario_id, missao_id, assunto, conteudo, nome_remetente, email_remetente, lida, criado_em FROM quilombo_mensagens WHERE id = ?1",
        params![id],
        |row| {
            Ok(Mensagem {
                id: row.get(0)?,
                remetente_id: row.get(1)?,
                destinatario_id: row.get(2)?,
                missao_id: row.get(3)?,
                assunto: row.get(4)?,
                conteudo: row.get(5)?,
                nome_remetente: row.get(6)?,
                email_remetente: row.get(7)?,
                lida: row.get::<_, i64>(8)? != 0,
                criado_em: parse_datetime(&row.get::<_, String>(9)?),
            })
        },
    )
    .map_err(|e| format!("Message not found: {e}"))
}

// --- Atividades ---

pub fn registrar_atividade(
    conn: &Connection,
    acao: &str,
    entidade: &str,
    entidade_id: Option<&str>,
    usuario_id: Option<&str>,
    conteudo: Option<&str>,
) {
    conn.execute(
        "INSERT INTO quilombo_atividades (acao, entidade, entidade_id, usuario_id, conteudo) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![acao, entidade, entidade_id, usuario_id, conteudo],
    )
    .ok();
}

pub fn listar_atividades_usuario(conn: &Connection, usuario_id: &str) -> Vec<Atividade> {
    let mut stmt = conn
        .prepare("SELECT id, acao, entidade, entidade_id, usuario_id, conteudo, criado_em FROM quilombo_atividades WHERE usuario_id = ?1 ORDER BY criado_em DESC LIMIT 200")
        .expect("Failed to prepare listar_atividades_usuario");

    stmt.query_map(params![usuario_id], |row| {
        Ok(Atividade {
            id: row.get(0)?,
            acao: row.get(1)?,
            entidade: row.get(2)?,
            entidade_id: row.get(3)?,
            usuario_id: row.get(4)?,
            conteudo: row.get(5)?,
            criado_em: parse_datetime(&row.get::<_, String>(6)?),
        })
    })
    .expect("Failed to list user activities")
    .filter_map(|r| r.ok())
    .collect()
}

pub fn listar_atividades(conn: &Connection, limit: u64) -> Vec<Atividade> {
    let mut stmt = conn
        .prepare("SELECT id, acao, entidade, entidade_id, usuario_id, conteudo, criado_em FROM quilombo_atividades ORDER BY criado_em DESC LIMIT ?1")
        .expect("Failed to prepare listar_atividades");

    stmt.query_map(params![limit], |row| {
        Ok(Atividade {
            id: row.get(0)?,
            acao: row.get(1)?,
            entidade: row.get(2)?,
            entidade_id: row.get(3)?,
            usuario_id: row.get(4)?,
            conteudo: row.get(5)?,
            criado_em: parse_datetime(&row.get::<_, String>(6)?),
        })
    })
    .expect("Failed to list activities")
    .filter_map(|r| r.ok())
    .collect()
}

// --- Perfis (user-editable overrides) ---

pub fn obter_perfil(conn: &Connection, usuario_id: &str) -> Option<Perfil> {
    conn.query_row(
        "SELECT usuario_id, nome_exibicao, bio, foto_url, atualizado_em FROM quilombo_perfis WHERE usuario_id = ?1",
        params![usuario_id],
        |row| {
            Ok(Perfil {
                usuario_id: row.get(0)?,
                nome_exibicao: row.get(1)?,
                bio: row.get(2)?,
                foto_url: row.get(3)?,
                atualizado_em: parse_datetime(&row.get::<_, String>(4)?),
            })
        },
    )
    .ok()
}

pub fn salvar_perfil(
    conn: &Connection,
    usuario_id: &str,
    update: &AtualizarPerfilUnificado,
) -> Result<Perfil, String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO quilombo_perfis (usuario_id, nome_exibicao, bio, foto_url, atualizado_em)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(usuario_id) DO UPDATE SET
           nome_exibicao = COALESCE(?2, quilombo_perfis.nome_exibicao),
           bio = COALESCE(?3, quilombo_perfis.bio),
           foto_url = COALESCE(?4, quilombo_perfis.foto_url),
           atualizado_em = ?5",
        params![
            usuario_id,
            update.nome_exibicao.as_deref().map(sanitize),
            update.bio.as_deref().map(sanitize),
            update.foto_url,
            now,
        ],
    )
    .map_err(|e| format!("Failed to save profile: {e}"))?;

    obter_perfil(conn, usuario_id).ok_or_else(|| "Profile not found after save".to_string())
}

// --- Telemetria ---

#[allow(clippy::too_many_arguments)]
pub fn registrar_visita(
    conn: &Connection,
    visitante_token: Option<&str>,
    usuario_id: Option<&str>,
    path: &str,
    referrer: Option<&str>,
    user_agent: Option<&str>,
    ip_hash: Option<&str>,
    duracao_ms: Option<i64>,
) {
    conn.execute(
        "INSERT INTO quilombo_telemetria (visitante_token, usuario_id, path, referrer, user_agent, ip_hash, duracao_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![visitante_token, usuario_id, path, referrer, user_agent, ip_hash, duracao_ms],
    )
    .ok();
}

pub fn listar_telemetria_usuario(conn: &Connection, usuario_id: &str) -> Vec<Telemetria> {
    let mut stmt = conn
        .prepare(
            "SELECT id, visitante_token, usuario_id, path, referrer, duracao_ms, criado_em
             FROM quilombo_telemetria
             WHERE usuario_id = ?1
             ORDER BY criado_em DESC LIMIT 500",
        )
        .expect("Failed to prepare listar_telemetria_usuario");

    stmt.query_map(params![usuario_id], |row| {
        Ok(Telemetria {
            id: row.get(0)?,
            visitante_token: row.get(1)?,
            usuario_id: row.get(2)?,
            path: row.get(3)?,
            referrer: row.get(4)?,
            duracao_ms: row.get(5)?,
            criado_em: parse_datetime(&row.get::<_, String>(6)?),
        })
    })
    .expect("Failed to list telemetry")
    .filter_map(|r| r.ok())
    .collect()
}

/// Link anonymous telemetry to a user account (called on registration).
pub fn vincular_telemetria(conn: &Connection, visitante_token: &str, usuario_id: &str) {
    conn.execute(
        "UPDATE quilombo_telemetria SET usuario_id = ?1 WHERE visitante_token = ?2 AND usuario_id IS NULL",
        params![usuario_id, visitante_token],
    )
    .ok();
}

pub fn listar_comentarios_usuario(conn: &Connection, usuario_id: &str) -> Vec<Comentario> {
    let mut stmt = conn
        .prepare(
            "SELECT id, publicacao_slug, evento_id, autor_id, nome_exibicao, conteudo, anonimo, criado_em
             FROM quilombo_comentarios WHERE autor_id = ?1 ORDER BY criado_em DESC",
        )
        .expect("Failed to prepare listar_comentarios_usuario");

    stmt.query_map(params![usuario_id], |row| {
        Ok(Comentario {
            id: row.get(0)?,
            publicacao_slug: row.get(1)?,
            evento_id: row.get(2)?,
            autor_id: row.get(3)?,
            nome_exibicao: row.get(4)?,
            conteudo: row.get(5)?,
            anonimo: row.get::<_, i64>(6)? != 0,
            criado_em: parse_datetime(&row.get::<_, String>(7)?),
        })
    })
    .expect("Failed to list user comments")
    .filter_map(|r| r.ok())
    .collect()
}

// --- Admin queries ---

pub fn listar_usuarios(conn: &Connection) -> Vec<Usuario> {
    let mut stmt = conn
        .prepare("SELECT id, usuario, nome, papel, bio, foto_url, criado_em, atualizado_em FROM quilombo_usuarios ORDER BY criado_em")
        .expect("Failed to prepare listar_usuarios");

    stmt.query_map([], |row| {
        Ok(Usuario {
            id: row.get(0)?,
            usuario: row.get(1)?,
            nome: row.get(2)?,
            papel: row.get::<_, String>(3)?.parse().unwrap_or(Papel::Membro),
            bio: row.get(4)?,
            foto_url: row.get(5)?,
            criado_em: parse_datetime(&row.get::<_, String>(6)?),
            atualizado_em: parse_datetime(&row.get::<_, String>(7)?),
        })
    })
    .expect("Failed to list users")
    .filter_map(|r| r.ok())
    .collect()
}

pub fn atualizar_papel(conn: &Connection, id: &str, papel: &Papel) -> Result<Usuario, String> {
    let now = chrono::Utc::now().to_rfc3339();
    let changes = conn
        .execute(
            "UPDATE quilombo_usuarios SET papel = ?1, atualizado_em = ?2 WHERE id = ?3",
            params![papel.to_string(), now, id],
        )
        .map_err(|e| format!("Failed to update role: {e}"))?;
    if changes == 0 {
        return Err("User not found".to_string());
    }
    obter_usuario_por_id(conn, id).ok_or_else(|| "User not found after update".to_string())
}

/// Admin dashboard summary.
pub fn resumo_admin(conn: &Connection, days: i64) -> serde_json::Value {
    let since = format!("datetime('now', '-{} days')", days.clamp(1, 365));

    let total_visitas: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM quilombo_telemetria WHERE criado_em >= {since}"),
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let visitantes_unicos: i64 = conn
        .query_row(
            &format!("SELECT COUNT(DISTINCT visitante_token) FROM quilombo_telemetria WHERE criado_em >= {since} AND visitante_token IS NOT NULL"),
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let total_usuarios: i64 = conn
        .query_row("SELECT COUNT(*) FROM quilombo_usuarios", [], |row| {
            row.get(0)
        })
        .unwrap_or(0);

    let total_eventos: i64 = conn
        .query_row("SELECT COUNT(*) FROM quilombo_eventos", [], |row| {
            row.get(0)
        })
        .unwrap_or(0);

    let total_missoes: i64 = conn
        .query_row("SELECT COUNT(*) FROM quilombo_missoes", [], |row| {
            row.get(0)
        })
        .unwrap_or(0);

    let total_comentarios: i64 = conn
        .query_row("SELECT COUNT(*) FROM quilombo_comentarios", [], |row| {
            row.get(0)
        })
        .unwrap_or(0);

    // Popular pages
    let mut stmt = conn
        .prepare(&format!(
            "SELECT path, COUNT(*) as c FROM quilombo_telemetria WHERE criado_em >= {since} GROUP BY path ORDER BY c DESC LIMIT 10"
        ))
        .expect("Failed to prepare popular pages");
    let paginas_populares: Vec<serde_json::Value> = stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "path": row.get::<_, String>(0)?,
                "visitas": row.get::<_, i64>(1)?
            }))
        })
        .expect("Failed to query popular pages")
        .filter_map(|r| r.ok())
        .collect();

    serde_json::json!({
        "periodo_dias": days,
        "visitas": total_visitas,
        "visitantes_unicos": visitantes_unicos,
        "usuarios": total_usuarios,
        "eventos": total_eventos,
        "missoes": total_missoes,
        "comentarios": total_comentarios,
        "paginas_populares": paginas_populares,
    })
}

/// Query telemetry data for admin dashboard.
pub fn listar_telemetria_admin(conn: &Connection, days: i64) -> Vec<Telemetria> {
    let since = format!("datetime('now', '-{} days')", days.clamp(1, 365));
    let mut stmt = conn
        .prepare(&format!(
            "SELECT id, visitante_token, usuario_id, path, referrer, duracao_ms, criado_em
             FROM quilombo_telemetria
             WHERE criado_em >= {since}
             ORDER BY criado_em DESC LIMIT 1000"
        ))
        .expect("Failed to prepare listar_telemetria_admin");

    stmt.query_map([], |row| {
        Ok(Telemetria {
            id: row.get(0)?,
            visitante_token: row.get(1)?,
            usuario_id: row.get(2)?,
            path: row.get(3)?,
            referrer: row.get(4)?,
            duracao_ms: row.get(5)?,
            criado_em: parse_datetime(&row.get::<_, String>(6)?),
        })
    })
    .expect("Failed to list telemetry")
    .filter_map(|r| r.ok())
    .collect()
}

/// Delete all user data (GDPR compliance).
pub fn excluir_conta(conn: &Connection, usuario_id: &str) -> Result<(), String> {
    // Anonymize comments (keep content, remove author)
    conn.execute(
        "UPDATE quilombo_comentarios SET autor_id = NULL, nome_exibicao = NULL WHERE autor_id = ?1",
        params![usuario_id],
    )
    .ok();

    // Delete messages
    conn.execute(
        "DELETE FROM quilombo_mensagens WHERE remetente_id = ?1 OR destinatario_id = ?1",
        params![usuario_id],
    )
    .ok();

    // Delete telemetry
    conn.execute(
        "DELETE FROM quilombo_telemetria WHERE usuario_id = ?1",
        params![usuario_id],
    )
    .ok();

    // Delete profile overrides
    conn.execute(
        "DELETE FROM quilombo_perfis WHERE usuario_id = ?1",
        params![usuario_id],
    )
    .ok();

    // Delete participations
    conn.execute(
        "DELETE FROM quilombo_participacoes WHERE usuario_id = ?1",
        params![usuario_id],
    )
    .ok();

    // Log deletion
    registrar_atividade(
        conn,
        "excluir",
        "conta",
        Some(usuario_id),
        Some(usuario_id),
        None,
    );

    // Delete auth record last
    conn.execute(
        "DELETE FROM quilombo_usuarios WHERE id = ?1",
        params![usuario_id],
    )
    .map_err(|e| format!("Failed to delete account: {e}"))?;

    Ok(())
}
