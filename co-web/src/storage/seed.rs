use chrono::Utc;
use rusqlite::params;
use serde_json::json;

use crate::entry_index::make_entry;

use super::Storage;
use super::schema::{seed_page_body, seed_page_frontmatter, upsert_entry_row};

use super::{
    SEED_CO_PLATAFORMA_MD, SEED_DADOS_RASTREADOS_MD, SEED_GUIA_MD, SEED_INFRA_CO_MD,
    SEED_INFRA_MD, SEED_INFRA_QUILOMBO_MD, SEED_INFRA_RFQ_MD, SEED_INFRA_YGGDRASIL_MD,
    SEED_LICENSA_MD, SEED_LINHAS_DO_TEMPO_MD, SEED_PRIVACIDADE_MD, SEED_RENDERERS_MD,
    SEED_SEGURANCA_CENARIOS_MD, SEED_SEGURANCA_DEPS_MD, SEED_SEGURANCA_MD,
    SEED_SEGURANCA_VAPID_MD, SEED_SOBRE_MD, SEED_TEMPLATE_INDEX_MD, SEED_TERMOS_MD,
};

impl Storage {
    pub fn seed_template_universe(&mut self) {
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        // Template universe with Modern theme (default) + conteudo layout
        // (content-first: README on entry, kanban one click away).
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_template, is_public, \
              visibility, theme_preset, layout) \
             VALUES ('template', 'Co', \
             'Aprenda a usar o Co — arraste, crie e explore', \
             'system', ?1, 1, 1, 'template', 'modern', 'conteudo')",
            params![now_str],
        );
        // Idempotent flip for installs predating the conteudo-as-default
        // change: existing template rows were created with layout='board'.
        let _ = self.conn.execute(
            "UPDATE universes SET layout = 'conteudo' \
             WHERE key = 'template' AND is_template = 1 AND layout = 'board'",
            [],
        );
        // Ensure form config YAML is written for the template.
        if let Some(config) = self.get_universe_form_config("template") {
            let _ = self.write_universo_yaml("template", &config);
        }

        // Check if project entry already exists (query per-universe DB — CO-77).
        let proj_path = "projects/CO/_project.md";
        let template_uc = self.universe_pool.get_or_open("template");
        let already_seeded: bool = {
            let uc_guard = template_uc.lock().expect("template universe conn lock");
            uc_guard
                .query_row(
                    "SELECT COUNT(*) FROM entries WHERE universe_key = 'template' AND path = ?1",
                    params![proj_path],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0
        };

        if already_seeded {
            return;
        }

        // Create project entry
        let proj_fm = json!({
            "type": "project",
            "key": "CO",
            "title": "Bem-vindo ao Co",
            "status": "active",
            "next_id": 10,
            "created": now_str,
            "modified": now_str,
            "archived": false,
            "tags": ["onboarding"]
        });
        let proj_entry = make_entry(
            proj_path,
            proj_fm,
            "Cada cartão é uma ideia. Arraste, crie, explore.",
        );
        let universe_root = self.universe_root("template");
        let _ = co::entry::write_entry(&universe_root, &proj_entry);
        {
            let uc_guard = template_uc.lock().expect("template universe conn lock");
            let _ = upsert_entry_row(&uc_guard, "template", &proj_entry);
        }
        // Register in project_universe_index so get_project() works
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO project_universe_index (project_key, universe_key) VALUES ('CO', 'template')",
            [],
        );

        // Onboarding tasks — game tutorial tone, curiosity-driven
        // Content always in Portuguese. UI labels translate via i18n.
        struct SeedTask {
            id: i64,
            title: &'static str,
            description: &'static str,
            status: &'static str,
            priority: &'static str,
            labels: Vec<&'static str>,
            due_days: Option<i64>,
            parent: Option<i64>,
        }

        let tasks = [
            // --- Act 1: First contact ---
            SeedTask {
                id: 1,
                title: "Mova este cartão para Concluído",
                description: "Você acabou de chegar. Que tal começar com algo simples?\n\nArraste este cartão direto para a coluna **Concluído**.\n\nPronto — você já terminou sua primeira tarefa no Co. Cada coluna representa um estado. Mova os cartões conforme avança.",
                status: "todo",
                priority: "high",
                labels: vec!["inicio"],
                due_days: None,
                parent: None,
            },
            // --- Act 2: Make it yours ---
            SeedTask {
                id: 2,
                title: "Crie algo seu",
                description: "Clique em **+ Nova Tarefa** e escreva o que vier à mente.\n\nPode ser uma ideia, um lembrete, um projeto. A descrição aceita **Markdown** — negrito, listas, links, código.\n\nCada tarefa vira um arquivo `.md` que você pode abrir no Obsidian, editar no VS Code, ou versionar no Git.",
                status: "todo",
                priority: "high",
                labels: vec!["inicio"],
                due_days: None,
                parent: None,
            },
            SeedTask {
                id: 3,
                title: "Quebre em partes menores",
                description: "Toda grande ideia começa com um passo pequeno.\n\nAbra uma tarefa e escolha um **pai** no campo \"Tarefa Pai\". A subtarefa aparece aninhada no Kanban — clique no triângulo para expandir.\n\nVocê pode criar quantos níveis quiser.",
                status: "todo",
                priority: "medium",
                labels: vec!["inicio"],
                due_days: None,
                parent: Some(2),
            },
            // --- Act 3: Discover ---
            SeedTask {
                id: 4,
                title: "Escolha um visual",
                description: "Cada universo tem sua identidade. Use o seletor de tema no cabeçalho para experimentar:\n\n- **Scholarly** — editorial acadêmico, tons de cobre\n- **Relic** — cinema escuro, rosa e ouro\n- **Cyberpunk** — neon sobre noite\n- **Garden** — verde orgânico\n- **Matrix** — fósforo sobre preto\n- **Terminal** — minimalismo absoluto\n\nSão 12 temas. Cada um transforma completamente a interface.",
                status: "todo",
                priority: "medium",
                labels: vec!["explorar"],
                due_days: None,
                parent: None,
            },
            SeedTask {
                id: 5,
                title: "Veja de outro ângulo",
                description: "Os mesmos dados, apresentados de formas diferentes. Alterne entre as abas:\n\n- **Kanban** — visão espacial, arraste entre colunas\n- **Tabela** — lista ordenável, filtros rápidos\n- **Painel** — visão geral, métricas\n- **Conteúdo** — seus textos como artigos\n\nO Conteúdo é especial: cada descrição de tarefa é um texto Markdown completo. Escreva documentação, notas, artigos — tudo organizado no mesmo lugar.",
                status: "todo",
                priority: "medium",
                labels: vec!["explorar"],
                due_days: None,
                parent: None,
            },
            // --- Act 4: Understand the system ---
            SeedTask {
                id: 6,
                title: "Entenda o que é Conteúdo",
                description: "No CO, **tudo é conteúdo**. Uma tarefa é um arquivo Markdown com metadados (título, status, prioridade) no cabeçalho.\n\n```yaml\n---\ntype: task\ntitle: Minha tarefa\nstatus: todo\ntags: [projeto, ideia]\n---\n```\n\nIsso significa que seu quadro de tarefas é também um banco de dados de textos. Abra a aba **Conteúdo** para ver seus cartões como artigos.\n\nVocê pode sincronizar com o **Obsidian**, editar no seu editor favorito, ou acessar via API. O conteúdo é seu — em Markdown, sempre portátil.",
                status: "todo",
                priority: "low",
                labels: vec!["explorar", "conteudo"],
                due_days: None,
                parent: None,
            },
            SeedTask {
                id: 7,
                title: "Troque o idioma da interface",
                description: "A interface do CO funciona em **Português** e **English**.\n\nClique no botão de idioma no cabeçalho. Os rótulos da interface mudam, mas o conteúdo (seus textos, descrições, títulos) permanece como você escreveu.\n\nIsso porque conteúdo é seu — a interface é só a moldura.",
                status: "todo",
                priority: "low",
                labels: vec!["explorar"],
                due_days: None,
                parent: None,
            },
            // --- Act 5: Join ---
            SeedTask {
                id: 8,
                title: "Faça parte",
                description: "Tudo o que você fez até agora está salvo neste navegador.\n\nQuando criar uma conta, seu universo ganha um endereço permanente que você pode compartilhar. Outras pessoas podem colaborar em tempo real — com cursores visíveis e edição simultânea.\n\nSeu conteúdo continua sendo Markdown. Seu universo continua sendo seu.\n\n**Crie uma conta gratuita** para salvar, compartilhar e colaborar.",
                status: "todo",
                priority: "critical",
                labels: vec!["acao"],
                due_days: None,
                parent: None,
            },
            // --- Bonus: hidden depth ---
            SeedTask {
                id: 9,
                title: "Conecte com o Obsidian",
                description: "Se você usa Obsidian, pode sincronizar este universo como um vault.\n\nCada tarefa vira uma nota `.md` com frontmatter YAML. Subtarefas viram `[[wikilinks]]`. Tags viram #tags.\n\nInstale o plugin **CO Universe Sync** no Obsidian e conecte com sua conta. Seus dados fluem entre o CO e o Obsidian sem atrito.\n\nDataview queries funcionam nativamente:\n\n```dataview\nTABLE status, priority\nFROM \"projects\"\nWHERE type = \"task\" AND status != \"done\"\nSORT priority DESC\n```",
                status: "todo",
                priority: "low",
                labels: vec!["avancado", "obsidian"],
                due_days: None,
                parent: None,
            },
        ];

        for t in &tasks {
            let created_at = (now - chrono::Duration::days(30 - t.id * 3)).to_rfc3339();
            let updated_at = (now - chrono::Duration::days(5)).to_rfc3339();
            let due_date: Option<String> = t.due_days.map(|d| {
                (now + chrono::Duration::days(d))
                    .format("%Y-%m-%d")
                    .to_string()
            });
            let task_path = format!("projects/CO/{}.md", t.id);
            let labels: Vec<serde_json::Value> = t.labels.iter().map(|l| json!(l)).collect();
            let task_fm = json!({
                "type": "task",
                "id": t.id,
                "title": t.title,
                "status": t.status,
                "priority": t.priority,
                "due": due_date,
                "parent": t.parent,
                "tags": labels,
                "created": created_at,
                "modified": updated_at,
                "archived": false,
                "assignee": null,
                "project": "CO"
            });
            let task_entry = make_entry(&task_path, task_fm, t.description);
            let _ = co::entry::write_entry(&universe_root, &task_entry);
            {
                let uc_guard = template_uc.lock().expect("template universe conn lock");
                let _ = upsert_entry_row(&uc_guard, "template", &task_entry);
            }
        }

        // Seed/refresh the template's content pages (intro + legal).
        // Extracted so it can also run unconditionally on each startup.
        self.reseed_template_content_pages();
    }

    /// Force the template universe's `theme_preset` to a known value.
    ///
    /// Earlier migrations defaulted `theme_preset` to `'scholarly-light'` and
    /// updated the existing template row to match — even though the seed code
    /// today uses `'modern'`. Because the row is then `INSERT OR IGNORE`d on
    /// every boot, the migration value is sticky. This setter overrides it on
    /// every startup so the template page is consistently rendered with the
    /// product's intended default look.
    pub fn ensure_template_theme_preset(&self, preset: &str) {
        let _ = self.conn.execute(
            "UPDATE universes SET theme_preset = ?1 WHERE key = 'template'",
            params![preset],
        );
    }

    /// Always-overwrite seed of the template universe's content pages from the
    /// embedded `seed/template/*.md` files.
    ///
    /// Called both from `seed_template_universe()` (first-boot path) and on
    /// every server startup, so the binary's bundled legal/intro content is
    /// the source of truth — even when the database already exists from a
    /// prior version. `upsert_entry_row` does an `INSERT OR REPLACE`, so this
    /// is safe to call repeatedly.
    pub fn reseed_template_content_pages(&mut self) {
        if !self.template_exists() {
            return; // template universe not seeded yet — first-boot path will handle it
        }
        let now_str = Utc::now().to_rfc3339();
        let universe_root = self.universe_root("template");

        for (path, md) in [
            ("index.md", SEED_TEMPLATE_INDEX_MD),
            ("content/sobre.md", SEED_SOBRE_MD),
            ("content/termos.md", SEED_TERMOS_MD),
            ("content/privacidade.md", SEED_PRIVACIDADE_MD),
            ("content/dados-rastreados.md", SEED_DADOS_RASTREADOS_MD),
            ("content/linhas-do-tempo.md", SEED_LINHAS_DO_TEMPO_MD),
            ("content/co-plataforma.md", SEED_CO_PLATAFORMA_MD),
            ("content/guia.md", SEED_GUIA_MD),
            // 2026-05-13: transparency pages anchored in the template
            // so anon users see the security model + dependencies +
            // license + renderer options as readable content.
            ("content/seguranca.md", SEED_SEGURANCA_MD),
            ("content/seguranca-dependencias.md", SEED_SEGURANCA_DEPS_MD),
            ("content/seguranca-cenarios.md", SEED_SEGURANCA_CENARIOS_MD),
            ("content/seguranca-vapid.md", SEED_SEGURANCA_VAPID_MD),
            ("content/licensa.md", SEED_LICENSA_MD),
            ("content/renderers.md", SEED_RENDERERS_MD),
            // 2026-05-13: infra catalog (compute portfolio) mirrored
            // from /projects/infra/ so the template is the canonical
            // public-readable description of how ArteLonga deploys.
            ("content/infra.md", SEED_INFRA_MD),
            ("content/infra-co.md", SEED_INFRA_CO_MD),
            ("content/infra-yggdrasil.md", SEED_INFRA_YGGDRASIL_MD),
            ("content/infra-quilomboaraucaria.md", SEED_INFRA_QUILOMBO_MD),
            ("content/infra-rfq-gateway.md", SEED_INFRA_RFQ_MD),
        ] {
            let entry = make_entry(
                path,
                seed_page_frontmatter(md, &now_str),
                seed_page_body(md),
            );
            if let Err(e) = co::entry::write_entry(&universe_root, &entry) {
                tracing::warn!("Failed to write {path} file: {e}");
            }
            let template_uc = self.universe_pool.get_or_open("template");
            let uc_guard = template_uc.lock().expect("template universe conn lock");
            if let Err(e) = upsert_entry_row(&uc_guard, "template", &entry) {
                tracing::warn!("Failed to upsert {path} page: {e}");
            }
        }
    }

    // --- Quilombo Araucária universe ---

    /// Returns true if the quilomboaraucaria universe already exists.
    pub fn quilombo_universe_exists(&self) -> bool {
        self.get_universe("quilomboaraucaria").is_some()
    }

    /// Seed the `quilomboaraucaria` public universe (CO-41).
    ///
    /// Creates the universe with is_public=1, visibility='public-subscribable',
    /// owner_id='system', and the quilombo theme preset.
    /// Safe to call multiple times — idempotent via INSERT OR IGNORE.
    pub fn seed_quilombo_universe(&mut self) {
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        // Custom tokens for the quilombo-blog design system
        let custom_tokens = serde_json::json!({
            "--bg": "#f5f0e8",
            "--card-bg": "#faf6ef",
            "--border": "#c8b48e",
            "--accent": "#2d4a22",
            "--text-muted": "#8b6914"
        });
        let tokens_str = custom_tokens.to_string();

        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_template, is_public, \
              visibility, theme_preset, layout, font_headline, font_body, custom_tokens, content_count) \
             VALUES ('quilomboaraucaria', 'Quilombo Araucária', \
             'Comunidade quilombola do Paraná — publicações, eventos e missões', \
             'system', ?1, 0, 1, 'public-subscribable', 'quilombo', 'board', \
             'Playfair Display', 'Inter', ?2, 0)",
            params![now_str, tokens_str],
        );

        // Sync .universo.yaml
        if let Some(config) = self.get_universe_form_config("quilomboaraucaria") {
            let _ = self.write_universo_yaml("quilomboaraucaria", &config);
        }
    }

    /// Import markdown files from the quilomboaraucaria content repo into the universe.
    #[allow(dead_code)]
    fn import_quilombo_content(&mut self) {
        // Look for the repo in common locations
        let candidates = [
            std::path::PathBuf::from("/app/seed-co/quilomboaraucaria"),
            std::path::PathBuf::from("quilomboaraucaria"),
            dirs::home_dir()
                .unwrap_or_default()
                .join("projects/quilomboaraucaria"),
        ];
        let repo = candidates.iter().find(|p| p.join("schema.yaml").exists());
        let repo = match repo {
            Some(r) => r.clone(),
            None => {
                tracing::info!("quilomboaraucaria content repo not found, skipping import");
                return;
            }
        };
        tracing::info!(
            "Importing quilomboaraucaria content from {}",
            repo.display()
        );

        let now = Utc::now().to_rfc3339();
        let universe_root = self.universe_root("quilomboaraucaria");

        // Map folder → entry type
        let folder_types = [
            ("eventos", "event"),
            ("relatos", "post"),
            ("jardim", "page"),
            ("membros", "member"),
            ("quadro", "task"),
        ];

        let mut count: i64 = 0;
        for (folder, entry_type) in &folder_types {
            let dir = repo.join(folder);
            if !dir.is_dir() {
                continue;
            }
            let entries = std::fs::read_dir(&dir);
            if entries.is_err() {
                continue;
            }

            for entry in entries.unwrap().flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }

                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                // Parse frontmatter
                let parsed = co::entry::Entry::parse_frontmatter(&content);
                let (mut fm, body) = match parsed {
                    Some((f, b)) => (f, b),
                    None => {
                        // No frontmatter — treat entire content as body
                        let fm = json!({"type": entry_type});
                        (fm, content.clone())
                    }
                };

                // Ensure type field
                if let Some(obj) = fm.as_object_mut() {
                    if !obj.contains_key("type") {
                        obj.insert("type".to_string(), json!(entry_type));
                    }
                    // Map Portuguese fields to standard
                    if let Some(titulo) = obj.remove("titulo") {
                        obj.entry("title".to_string()).or_insert(titulo);
                    }
                    if !obj.contains_key("created") {
                        obj.insert("created".to_string(), json!(now));
                    }
                    if !obj.contains_key("modified") {
                        obj.insert("modified".to_string(), json!(now));
                    }
                }

                let title = fm
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let filename = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");
                let entry_path = format!("content/{}/{}.md", folder, filename);

                let entry = make_entry(&entry_path, fm, &body);
                let _ = co::entry::write_entry(&universe_root, &entry);
                let _ = upsert_entry_row(&self.conn, "quilomboaraucaria", &entry);
                count += 1;

                if !title.is_empty() {
                    tracing::debug!("  imported: {}/{} — {}", folder, filename, title);
                }
            }
        }

        // Update content count
        let _ = self.conn.execute(
            "UPDATE universes SET content_count = ?1 WHERE key = 'quilomboaraucaria'",
            params![count],
        );

        tracing::info!("quilomboaraucaria: imported {} entries", count);
    }

    /// Stats for the quilomboaraucaria universe.
    ///
    /// Entry counts come from the SQLite index; totalUsuarios, versaoApp and
    /// ultimaSync are read from the `meta/stats.md` metadata entry written by
    /// the importer on each run. Returns a zeroed struct if the universe has no
    /// content yet.
    pub fn quilombo_stats(&self) -> crate::models::QuilomboStats {
        let count_by_type = |entry_type: &str| -> i64 {
            self.conn
                .query_row(
                    "SELECT COUNT(*) FROM entries \
                     WHERE universe_key = 'quilomboaraucaria' AND entry_type = ?1",
                    params![entry_type],
                    |row| row.get(0),
                )
                .unwrap_or(0)
        };

        let total_publicacoes = count_by_type("post");
        let total_eventos = count_by_type("event");
        let total_missoes = count_by_type("mission");

        // Read metadata from meta/stats.md entry
        let meta: Option<(i64, String, String)> = self
            .conn
            .query_row(
                "SELECT frontmatter_json FROM entries \
                 WHERE universe_key = 'quilomboaraucaria' AND path = 'meta/stats.md'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|json_str| serde_json::from_str::<serde_json::Value>(&json_str).ok())
            .and_then(|fm| {
                let usuarios = fm.get("total_usuarios")?.as_i64()?;
                let versao = fm.get("versao_app")?.as_str()?.to_string();
                let sync = fm.get("ultima_sync")?.as_str()?.to_string();
                Some((usuarios, versao, sync))
            });

        let (total_usuarios, versao_app, ultima_sync) = match meta {
            Some((u, v, s)) => (u, v, Some(s)),
            None => (0, "—".to_string(), None),
        };

        crate::models::QuilomboStats {
            total_usuarios,
            total_publicacoes,
            total_eventos,
            total_missoes,
            versao_app,
            ultima_sync,
        }
    }

    // --- Yggdrasil universe (CO-38) ---

    /// Returns true if the yggdrasil universe already exists.
    pub fn yggdrasil_universe_exists(&self) -> bool {
        self.get_universe("yggdrasil").is_some()
    }

    /// Seed the `yggdrasil` special universe — the minigames hub (CO-38).
    ///
    /// 1.46.0: `public-subscribable` (anonymous gets metadata, authed gets
    /// full read) + `default_for_new_users=1` so every new signup auto-
    /// subscribes. Owner='system', Relic Dark theme, layout='gaming'.
    /// Idempotent via INSERT OR IGNORE; the migration v29 already flipped
    /// existing rows from `requires_login` to this shape.
    pub fn seed_yggdrasil_universe(&mut self) {
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_template, is_public, \
              requires_login, visibility, default_for_new_users, theme_preset, layout, \
              font_headline, font_body, content_count) \
             VALUES ('yggdrasil', 'Yggdrasil', \
             'Hub de minijogos — perfis de jogadores e rankings globais', \
             'system', ?1, 0, 1, 0, 'public-subscribable', 1, 'relic', 'gaming', \
             'Newsreader', 'Manrope', 0)",
            params![now_str],
        );
        tracing::info!("Yggdrasil universe seeded (public-subscribable, default-for-new-users)");
    }

    // --- CO Dev universe (CO-53 / CO-140) ---

    /// Seed the `co-dev` private universe — the CO platform development board.
    ///
    /// Owned by 'system', private, scholarly-dark, board layout.
    /// `ensure_admin_universe_memberships` makes Yuri a member so it appears
    /// in his sidebar. Idempotent via INSERT OR IGNORE.
    /// Ensure admin-owned content universes exist — idempotent, runs every boot.
    ///
    /// Creates artelonga, rfq, and co universes owned by any admin-tier user so
    /// they appear in the sidebar without manual API calls.  Content is pushed
    /// separately via the Vault API or `co push`; this only guarantees the DB row.
    pub fn seed_admin_content_universes(&mut self) {
        let now = Utc::now().to_rfc3339();

        // (key, name, description, visibility, parent_key)
        for (key, name, desc, vis, parent) in [
            (
                "co",
                "CO",
                "CO platform — development board, tasks CO-1…, docs",
                "public-subscribable",
                None,
            ),
            (
                "artelonga",
                "ArteLonga",
                "Arte Longa — conteúdo público, portfólio e presença digital",
                "public-subscribable",
                None,
            ),
            (
                "rfq",
                "RFQ Gateway",
                "Plataforma de cotações e registro de negociações",
                "private",
                None,
            ),
            // Language parent — groups mbya + topologia
            (
                "language",
                "Language",
                "Parent group for language universes",
                "public-subscribable",
                None,
            ),
            (
                "mbya",
                "Mbya Guarani",
                "Arandu — Mbyá Guarani lexicon and learning content",
                "public-subscribable",
                Some("language"),
            ),
            (
                "topologia",
                "Topologia da Linguagem",
                "Cross-language meaning topology — concepts, terms, relations",
                "public-subscribable",
                Some("language"),
            ),
            (
                "time",
                "Time",
                "Time-stamped events: astronomical, earth-time milestones, system events",
                // 1.53.0: was hardcoded `private` and the boot-reconcile UPDATE
                // below stomped any user-set visibility on every deploy.
                // co-universes.yaml declares this as public-subscribable —
                // align the seed with the registry.
                "public-subscribable",
                None,
            ),
        ] {
            // 1.54.0: INSERT OR IGNORE only — no boot-reconcile UPDATE. The
            // pre-1.54 reconcile stomped user-set name/description/visibility
            // /parent_key on every deploy, contradicting the 1.45.0 single-
            // tier "any authed user can edit any universe" model. Seed values
            // are initial defaults only. Corrections to the declared intent
            // (e.g., renaming a universe in this list) require an explicit
            // migration that targets the specific row.
            let is_public_bit: i64 = if vis == "private" { 0 } else { 1 };
            let _ = self.conn.execute(
                "INSERT OR IGNORE INTO universes \
                 (key, name, description, owner_id, created_at, is_template, is_public, \
                  visibility, theme_preset, layout, content_count, parent_key) \
                 VALUES (?1, ?2, ?3, 'system', ?4, 0, ?5, ?6, 'scholarly-light', 'board', 0, ?7)",
                rusqlite::params![key, name, desc, now, is_public_bit, vis, parent],
            );
        }
    }

    pub fn seed_co_dev_universe(&mut self) {
        let now = Utc::now().to_rfc3339();
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_template, is_public, \
              requires_login, visibility, theme_preset, layout, content_count) \
             VALUES ('co-dev', 'CO Dev', \
             'CO platform development board — all tickets, sprints, and architecture', \
             'system', ?1, 0, 0, 1, 'requires_login', 'scholarly-dark', 'board', 0)",
            params![now],
        );
        // co-dev membership is handled by ensure_admin_universe_memberships at startup.
    }

    /// Ingest CO-*.md ticket files from `/app/seed-co/` (or any source dir) into
    /// the `co` universe's entries table at path `tasks/<filename>`.
    ///
    /// Mirrors `reseed_template_content_pages` shape: idempotent upsert, runs
    /// on every boot. Closes the "co has 0 entries" gap user-reported on
    /// 2026-05-02 — Phase E of CO-142 populated `/data/co/` for the dev_board
    /// admin scan, but the SPA's `/co/co` board reads from the per-universe
    /// `entries` table. This function bridges that.
    ///
    /// **1.52.0:** No longer wipes the entry table before re-seeding. The
    /// pre-1.52 wipe killed user-generated content (states/branches/proposals
    /// from the CO-native versioning roadmap) on every deploy. Now purely
    /// additive: the seed only upserts paths it knows about (CO-N.md files
    /// from `seed-co/`); paths the seed never wrote stay untouched. If you
    /// remove a file from `seed-co/`, the corresponding entry will linger
    /// on prod until explicitly deleted via `DELETE /universes/:slug/...` —
    /// but that's the right tradeoff for a system whose canonical source
    /// is auto-sync from local, not the baked-in seed.
    pub fn seed_co_universe_tasks(&mut self, source_dir: &std::path::Path) {
        if !source_dir.exists() {
            tracing::warn!(
                "seed_co_universe_tasks: source dir {} does not exist — skipped",
                source_dir.display()
            );
            return;
        }
        if self.get_universe("co").is_none() {
            tracing::warn!("seed_co_universe_tasks: 'co' universe row missing — skipped");
            return;
        }

        let universe_root = self.universe_root("co");
        let now_str = Utc::now().to_rfc3339();
        let mut upserted = 0usize;
        let mut skipped = 0usize;

        // Recursively walk source_dir. Path layout in the `co` universe:
        //   - top-level *.md  → tasks/<filename>   (legacy compat with 1.34.3)
        //   - subdir/<f>.md   → <subdir>/<f>       (CO-144: e.g. processos/)
        //
        // Files >2 levels deep are flattened to <leaf-subdir>/<filename> for
        // safety against pathological nesting; rare in practice.
        fn walk(
            dir: &std::path::Path,
            base: &std::path::Path,
        ) -> Vec<(std::path::PathBuf, String)> {
            let mut out = Vec::new();
            let Ok(read) = std::fs::read_dir(dir) else {
                return out;
            };
            for entry in read.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    out.extend(walk(&p, base));
                    continue;
                }
                if p.extension().and_then(|s| s.to_str()) != Some("md") {
                    continue;
                }
                let rel = match p.strip_prefix(base) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                // Entry path = path relative to source dir, forward slashes.
                // No tasks/ prefix — CO-*.md live at root of the universe.
                let entry_path = rel.display().to_string().replace('\\', "/");
                out.push((p, entry_path));
            }
            out
        }

        let candidates = walk(source_dir, source_dir);

        for (path, entry_path) in candidates {
            let filename = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => {
                    skipped += 1;
                    continue;
                }
            };
            let raw = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };

            let entry = make_entry(
                &entry_path,
                seed_page_frontmatter(&raw, &now_str),
                seed_page_body(&raw),
            );
            if let Err(e) = co::entry::write_entry(&universe_root, &entry) {
                tracing::warn!("seed_co_universe_tasks: write {filename}: {e}");
                skipped += 1;
                continue;
            }
            let co_uc = self.universe_pool.get_or_open("co");
            let uc_guard = co_uc.lock().expect("co universe conn lock");
            if let Err(e) = upsert_entry_row(&uc_guard, "co", &entry) {
                tracing::warn!("seed_co_universe_tasks: upsert {filename}: {e}");
                skipped += 1;
                continue;
            }
            upserted += 1;
        }

        // 1.52.0: count actual rows in the per-universe entry index, not
        // just `upserted` (which only sees seed-co files). Otherwise user-
        // generated states/branches/proposals get under-counted.
        let actual_count: i64 = {
            let co_uc = self.universe_pool.get_or_open("co");
            co_uc
                .lock()
                .ok()
                .and_then(|g| {
                    g.query_row(
                        "SELECT COUNT(*) FROM entries WHERE universe_key = 'co'",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .ok()
                })
                .unwrap_or(upserted as i64)
        };
        let _ = self.conn.execute(
            "UPDATE universes SET content_count = ?1 WHERE key = 'co'",
            rusqlite::params![actual_count],
        );

        tracing::info!(
            "seed_co_universe_tasks: seeded {upserted} from {} (skipped {skipped}); co now has {actual_count} entries total",
            source_dir.display()
        );
    }
}
