use chrono::Utc;
use rusqlite::params;
use serde_json::json;

use crate::entry_index::make_entry;

use super::Storage;
use super::schema::{seed_page_body, seed_page_frontmatter, split_frontmatter, upsert_entry_row};

use super::{
    SEED_CO_INDEX_MD, SEED_CO_PLATAFORMA_MD, SEED_CONTA_MD, SEED_DADOS_RASTREADOS_MD, SEED_GUIA_MD,
    SEED_INFRA_CO_MD, SEED_INFRA_MD, SEED_LICENSA_MD, SEED_LINHAS_DO_TEMPO_MD, SEED_PRIVACIDADE_MD,
    SEED_RENDERERS_MD, SEED_SEGURANCA_CENARIOS_MD, SEED_SEGURANCA_CRIPTO_MD,
    SEED_SEGURANCA_DEPS_MD, SEED_SEGURANCA_MD, SEED_SEGURANCA_VAPID_MD, SEED_SOBRE_MD,
    SEED_TEMPLATE_INDEX_MD, SEED_TERMOS_MD, SEED_TX_LOG_MD,
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
        // Hierarchy: template is the public-facing subuniverse of `co`.
        // The `co` universe owns the dev board; `template` is its public
        // window — the surface anon visitors see. Setting parent_key
        // here makes the relationship explicit + idempotent (only
        // updates when parent_key is currently NULL).
        let _ = self.conn.execute(
            "UPDATE universes SET parent_key = 'co' \
             WHERE key = 'template' AND parent_key IS NULL",
            [],
        );
        // Ensure form config YAML is written for the template.
        if let Some(config) = self.get_universe_form_config("template") {
            let _ = self.write_universo_yaml("template", &config);
        }

        // Check if project entry already exists (query per-universe DB — CO-77).
        let proj_path = "projects/TUTORIAL/_project.md";
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
            "key": "TUTORIAL",
            "title": "Tutorial — comece por aqui",
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
            "INSERT OR IGNORE INTO project_universe_index (project_key, universe_key) VALUES ('TUTORIAL', 'template')",
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
            let task_path = format!("projects/TUTORIAL/{}.md", t.id);
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
                "project": "TUTORIAL"
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
            // Welcome / onboarding pages stay in template (the anon
            // landing universe). The transparency cluster moved to
            // `co::public/*` in 2.7.20 — see `reseed_co_public_pages`.
            ("content/sobre.md", SEED_SOBRE_MD),
            ("content/termos.md", SEED_TERMOS_MD),
            ("content/privacidade.md", SEED_PRIVACIDADE_MD),
            ("content/dados-rastreados.md", SEED_DADOS_RASTREADOS_MD),
            ("content/linhas-do-tempo.md", SEED_LINHAS_DO_TEMPO_MD),
            ("content/co-plataforma.md", SEED_CO_PLATAFORMA_MD),
            ("content/guia.md", SEED_GUIA_MD),
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

    /// Reseed the transparency content cluster (security, license,
    /// infra catalog, renderers) into `co::public/*`. Anon visitors
    /// only see entries under `public/` in the `co` universe; logged-
    /// in users see everything they have access to.
    ///
    /// Idempotent: writes via `upsert_entry_row`. Runs on every boot.
    pub fn reseed_co_public_pages(&mut self) {
        if self.get_universe("co").is_none() {
            return; // co universe not seeded yet — admin seeder will handle it
        }
        let now_str = Utc::now().to_rfc3339();
        let universe_root = self.universe_root("co");

        for (path, md) in [
            ("index.md", SEED_CO_INDEX_MD),
            ("public/seguranca.md", SEED_SEGURANCA_MD),
            ("public/seguranca-dependencias.md", SEED_SEGURANCA_DEPS_MD),
            ("public/seguranca-cenarios.md", SEED_SEGURANCA_CENARIOS_MD),
            ("public/seguranca-vapid.md", SEED_SEGURANCA_VAPID_MD),
            ("public/seguranca-criptografia.md", SEED_SEGURANCA_CRIPTO_MD),
            ("public/licensa.md", SEED_LICENSA_MD),
            ("public/renderers.md", SEED_RENDERERS_MD),
            ("public/infra.md", SEED_INFRA_MD),
            ("public/infra-co.md", SEED_INFRA_CO_MD),
            // Cross-repo infra pages removed — those belong in their native universe.
            ("public/transaction-log.md", SEED_TX_LOG_MD),
            ("public/conta-e-mensagens.md", SEED_CONTA_MD),
        ] {
            let entry = make_entry(
                path,
                seed_page_frontmatter(md, &now_str),
                seed_page_body(md),
            );
            if let Err(e) = co::entry::write_entry(&universe_root, &entry) {
                tracing::warn!("Failed to write co/{path}: {e}");
            }
            let co_uc = self.universe_pool.get_or_open("co");
            let uc_guard = co_uc.lock().expect("co universe conn lock");
            if let Err(e) = upsert_entry_row(&uc_guard, "co", &entry) {
                tracing::warn!("Failed to upsert co/{path}: {e}");
            }
        }
    }

    /// One-time cleanup: the transparency cluster moved from
    /// `template::content/*` to `co::public/*` in 2.7.20. Remove the
    /// stale template entries (file + DB row) so the listing stays
    /// tidy. Idempotent — no-op once the entries are gone.
    pub fn cleanup_template_moved_pages(&mut self) {
        if !self.template_exists() {
            return;
        }
        const MOVED: &[&str] = &[
            "content/seguranca.md",
            "content/seguranca-dependencias.md",
            "content/seguranca-cenarios.md",
            "content/seguranca-vapid.md",
            "content/licensa.md",
            "content/renderers.md",
            "content/infra.md",
            "content/infra-co.md",
            "content/infra-yggdrasil.md",
            "content/infra-quilomboaraucaria.md",
            "content/infra-rfq-gateway.md",
        ];
        let universe_root = self.universe_root("template");
        let template_uc = self.universe_pool.get_or_open("template");
        let uc_guard = template_uc.lock().expect("template universe conn lock");
        for path in MOVED {
            let file = universe_root.join(path);
            if file.exists() {
                let _ = std::fs::remove_file(&file);
            }
            let _ = uc_guard.execute(
                "DELETE FROM entries WHERE universe_key = 'template' AND path = ?1",
                params![path],
            );
        }
    }

    /// CO-254: rename the tutorial project from `CO` to `TUTORIAL` on existing
    /// installs. Template is ephemeral (CO-49) so we simply drop the old entries
    /// and let `seed_template_universe` re-seed fresh with the new key.
    ///
    /// Idempotent: no-op when old `projects/CO/_project.md` is already absent.
    pub fn migrate_template_project_rename(&mut self) {
        if !self.template_exists() {
            return;
        }
        let template_uc = self.universe_pool.get_or_open("template");
        let paths_to_delete: Vec<String> = {
            let uc_guard = template_uc.lock().expect("template universe conn lock");
            let old_exists: bool = uc_guard
                .query_row(
                    "SELECT COUNT(*) FROM entries \
                     WHERE universe_key = 'template' AND path = 'projects/CO/_project.md'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;
            if !old_exists {
                return;
            }
            let mut stmt = match uc_guard.prepare(
                "SELECT path FROM entries \
                 WHERE universe_key = 'template' AND path LIKE 'projects/CO/%'",
            ) {
                Ok(s) => s,
                Err(_) => return,
            };
            stmt.query_map([], |row| row.get::<_, String>(0))
                .into_iter()
                .flatten()
                .filter_map(|r| r.ok())
                .collect()
        };

        if paths_to_delete.is_empty() {
            return;
        }

        let universe_root = self.universe_root("template");
        let uc_guard = template_uc.lock().expect("template universe conn lock");
        for path in &paths_to_delete {
            let file = universe_root.join(path);
            if file.exists() {
                let _ = std::fs::remove_file(&file);
            }
            let _ = uc_guard.execute(
                "DELETE FROM entries WHERE universe_key = 'template' AND path = ?1",
                params![path],
            );
        }
        tracing::info!(
            "CO-254: dropped {} stale CO tutorial entries from template; \
             will re-seed as TUTORIAL on next seed_template_universe call",
            paths_to_delete.len()
        );
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

    /// Idempotent reseed of the yggdrasil universe's content pages.
    /// Mirrors `reseed_template_content_pages` shape: runs on every boot
    /// so content updates land for existing installs without a manual
    /// migration.
    pub fn reseed_yggdrasil_content_pages(&mut self) {
        if self.get_universe("yggdrasil").is_none() {
            return;
        }
        let now_str = Utc::now().to_rfc3339();
        let yggdrasil_root = self.universe_root("yggdrasil");

        {
            let (path, md) = ("index.md", super::SEED_YGGDRASIL_INDEX_MD);
            let entry = make_entry(
                path,
                seed_page_frontmatter(md, &now_str),
                seed_page_body(md),
            );
            if let Err(e) = co::entry::write_entry(&yggdrasil_root, &entry) {
                tracing::warn!("Failed to write yggdrasil/{path}: {e}");
            }
            let yggdrasil_uc = self.universe_pool.get_or_open("yggdrasil");
            let uc_guard = yggdrasil_uc.lock().expect("yggdrasil universe conn lock");
            if let Err(e) = upsert_entry_row(&uc_guard, "yggdrasil", &entry) {
                tracing::warn!("Failed to upsert yggdrasil/{path}: {e}");
            }
        }
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
    /// the `co` universe's entries table as board-compatible tasks under a
    /// "CO Development Board" project.
    ///
    /// CO-261: creates a `projects/CO/_project.md` entry so the kanban sidebar
    /// shows the CO project, then seeds each `{PREFIX}-{N}.md` file (e.g.
    /// CO-261.md) as a `task` entry with `project: CO` so it appears in the
    /// kanban columns grouped by `status`. Documentation files (CLAUDE.md,
    /// ROADMAP.md, etc.) are skipped.
    ///
    /// Idempotent: runs on every boot via `run_co142_refresh`. Purely additive —
    /// user-created entries in the `co` universe are never deleted.
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

        // CO-261: ensure the CO Development Board project entry exists so the
        // kanban sidebar shows the project. Upsert is safe on every boot.
        {
            let proj_path = "projects/CO/_project.md";
            let proj_fm = json!({
                "type": "project",
                "key": "CO",
                "title": "CO Development Board",
                "status": "active",
                "next_id": 1000,
                "created": now_str,
                "modified": now_str,
                "archived": false,
                "tags": ["dev", "platform"],
            });
            let proj_entry = make_entry(
                proj_path,
                proj_fm,
                "CO platform — development tasks CO-1..N, sourced from work/co/ at build time.",
            );
            if let Err(e) = co::entry::write_entry(&universe_root, &proj_entry) {
                tracing::warn!("seed_co_universe_tasks: write CO project: {e}");
            }
            let co_uc = self.universe_pool.get_or_open("co");
            let uc_guard = co_uc.lock().expect("co universe conn lock");
            if let Err(e) = upsert_entry_row(&uc_guard, "co", &proj_entry) {
                tracing::warn!("seed_co_universe_tasks: upsert CO project: {e}");
            }
        }
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO project_universe_index (project_key, universe_key) \
             VALUES ('CO', 'co')",
            [],
        );

        // Recursively walk source_dir. Entry paths are relative to the source dir.
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
                // CO-262: prefix with public/ so anon visitors see these entries
                // via the entries API (filter_public_for_anon requires public/* for co).
                let entry_path = format!("public/{}", rel.display().to_string().replace('\\', "/"));
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

            // CO-261: only seed {PREFIX}-{DIGITS}.md task specs.
            // Documentation files (CLAUDE.md, ROADMAP.md, etc.) are skipped.
            if !is_task_filename(&filename) {
                skipped += 1;
                continue;
            }

            let raw = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };

            let entry = make_entry(
                &entry_path,
                work_task_frontmatter(&raw, &now_str, "CO"),
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

        // Count actual rows — includes user-created entries beyond the seed set.
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
            "seed_co_universe_tasks: seeded {upserted} CO tasks from {} (skipped {skipped}); \
             co now has {actual_count} entries total",
            source_dir.display()
        );
    }

    /// CO-264: seed top-of-repo well-known files (CHANGELOG.md, README.md, LICENSE.md)
    /// into the `co` universe as `page` entries.
    ///
    /// `root_dir` is the directory containing these files — the repo root in
    /// local dev, or `/app/` in Docker (when `COPY CHANGELOG.md README.md /app/`
    /// is present in the Dockerfile). Falls through silently when no file is found.
    ///
    /// Idempotent: uses `upsert_entry_row`. Called on every boot from
    /// `run_co142_refresh` so the bundled docs stay in sync with the binary.
    pub fn reseed_co_root_files(&mut self, root_dir: &std::path::Path) {
        if self.get_universe("co").is_none() {
            return;
        }
        let now_str = Utc::now().to_rfc3339();
        let universe_root = self.universe_root("co");

        // (candidate filenames, canonical entry path)
        let candidates: &[(&[&str], &str)] = &[
            (
                &["CHANGELOG.md", "changelog.md", "Changelog.md"],
                "CHANGELOG.md",
            ),
            (&["README.md", "readme.md", "Readme.md"], "README.md"),
            (
                &["LICENSE.md", "LICENSE", "license.md", "License.md"],
                "LICENSE.md",
            ),
        ];

        for (file_candidates, entry_path) in candidates {
            let content = file_candidates
                .iter()
                .find_map(|fname| std::fs::read_to_string(root_dir.join(fname)).ok());
            let content = match content {
                Some(c) => c,
                None => continue,
            };
            // Infer title from first H1 heading or use the filename stem.
            let title = content
                .lines()
                .find(|l| l.starts_with("# "))
                .map(|l| l.trim_start_matches('#').trim().to_string())
                .unwrap_or_else(|| entry_path.trim_end_matches(".md").to_string());
            let fm = json!({
                "type": "page",
                "title": title,
                "created": now_str,
                "modified": now_str,
            });
            let entry = make_entry(entry_path, fm, &content);
            if let Err(e) = co::entry::write_entry(&universe_root, &entry) {
                tracing::warn!("CO-264: failed to write co/{entry_path}: {e}");
                continue;
            }
            let co_uc = self.universe_pool.get_or_open("co");
            let uc_guard = co_uc.lock().expect("co universe conn lock");
            if let Err(e) = upsert_entry_row(&uc_guard, "co", &entry) {
                tracing::warn!("CO-264: failed to upsert co/{entry_path}: {e}");
            } else {
                tracing::info!("CO-264: seeded co/{entry_path} from {}", root_dir.display());
            }
        }
    }

    /// CO-267 (replaces CO-261 Wave B stubs): sister-repo tasks are now pushed
    /// via `co-sync push` from each repo's CI on every merge to main. The
    /// Vault API marks those entries `entry_origin = 'synced'` so subsequent
    /// boots don't overwrite them. No stub seeding needed.
    pub fn reseed_sister_repo_stubs(&mut self) {
        // no-op — stubs replaced by CI-driven co-sync push (CO-267)
    }
}

// ---------------------------------------------------------------------------
// CO-261 helpers — free functions used by seed_co_universe_tasks
// ---------------------------------------------------------------------------

/// Returns true if `filename` matches the `{PREFIX}-{DIGITS}.md` pattern
/// (e.g. `CO-261.md`, `YG-62.md`, `RFQ-27.md`).
/// Used to distinguish task specs from documentation files in `work/<space>/`.
fn is_task_filename(filename: &str) -> bool {
    let stem = match filename.strip_suffix(".md") {
        Some(s) => s,
        None => return false,
    };
    if let Some(pos) = stem.rfind('-') {
        let suffix = &stem[pos + 1..];
        !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
    } else {
        false
    }
}

/// Parse the frontmatter from a `work/<space>/*.md` file and inject the fields
/// required for the kanban board:
///
/// - `type` → overridden to `"task"` (board reads `entry_type` from this)
/// - `project` → set to `project_key` (board filters tasks by this field)
/// - `story_type` → preserves the original `type` value (e.g. `"user-story"`)
/// - `created` / `modified` → mapped from `created_at` / `updated_at` if the
///   standard fields are absent (CO-N.md uses the `_at` suffix convention)
/// - `tags` → mapped from `labels` when `tags` is not already present, so
///   work-item labels appear in the board's label column
fn work_task_frontmatter(raw: &str, now_str: &str, project_key: &str) -> serde_json::Value {
    let (fm_yaml, _) = split_frontmatter(raw);
    let mut fm: serde_json::Value = serde_yaml::from_str::<serde_yaml::Value>(fm_yaml)
        .ok()
        .and_then(|v| serde_json::to_value(v).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(obj) = fm.as_object_mut() {
        if let Some(orig) = obj.get("type").and_then(|v| v.as_str()).map(String::from) {
            obj.entry("story_type".to_string())
                .or_insert_with(|| json!(orig));
        }
        obj.insert("type".to_string(), json!("task"));
        obj.insert("project".to_string(), json!(project_key));
        if !obj.contains_key("created") {
            let ts = obj
                .get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or(now_str)
                .to_string();
            obj.insert("created".to_string(), json!(ts));
        }
        if !obj.contains_key("modified") {
            let ts = obj
                .get("updated_at")
                .and_then(|v| v.as_str())
                .unwrap_or(now_str)
                .to_string();
            obj.insert("modified".to_string(), json!(ts));
        }
        if !obj.contains_key("tags")
            && let Some(labels) = obj.get("labels").cloned()
        {
            obj.insert("tags".to_string(), labels);
        }
        // The board SQL filter is `archived = 0`; json_extract returns NULL for
        // absent fields and NULL = 0 is false, so unarchived tasks would be
        // filtered out. Default to false when the field is absent.
        obj.entry("archived".to_string())
            .or_insert_with(|| json!(false));
    }
    fm
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;

    fn make_co_task_md(n: u64, status: &str) -> String {
        format!(
            "---\nid: {n}\ntitle: Test CO-{n}\ntype: user-story\nstatus: {status}\n\
             priority: high\ncreated_at: 2026-01-01T00:00:00Z\nupdated_at: 2026-02-01T00:00:00Z\n\
             labels:\n  - type:feat\n---\n\nBody of CO-{n}.",
        )
    }

    #[test]
    fn test_is_task_filename_matches_task_pattern() {
        assert!(is_task_filename("CO-261.md"));
        assert!(is_task_filename("YG-62.md"));
        assert!(is_task_filename("RFQ-1.md"));
        assert!(!is_task_filename("CLAUDE.md"));
        assert!(!is_task_filename("ROADMAP.md"));
        assert!(!is_task_filename("_universe.yaml"));
        assert!(!is_task_filename("CO-.md"));
        assert!(!is_task_filename("CO-abc.md"));
    }

    #[test]
    fn test_work_task_frontmatter_injects_board_fields() {
        let raw = "---\nid: 42\ntitle: My Story\ntype: user-story\nstatus: in_progress\n\
                   priority: high\nlabels:\n  - type:feat\ncreated_at: 2026-01-15T00:00:00Z\n\
                   updated_at: 2026-02-01T00:00:00Z\n---\n\nBody.";
        let fm = work_task_frontmatter(raw, "2026-03-01T00:00:00Z", "CO");
        assert_eq!(fm["type"].as_str(), Some("task"), "type should be 'task'");
        assert_eq!(fm["project"].as_str(), Some("CO"), "project should be 'CO'");
        assert_eq!(
            fm["story_type"].as_str(),
            Some("user-story"),
            "original type preserved in story_type"
        );
        assert_eq!(
            fm["created"].as_str(),
            Some("2026-01-15T00:00:00Z"),
            "created mapped from created_at"
        );
        assert_eq!(
            fm["modified"].as_str(),
            Some("2026-02-01T00:00:00Z"),
            "modified mapped from updated_at"
        );
        assert!(fm["tags"].is_array(), "labels should be mapped to tags");
    }

    #[test]
    fn test_seed_co_universe_tasks_creates_project_and_tasks() {
        let data_dir = tempfile::tempdir().unwrap();
        let seed_dir = tempfile::tempdir().unwrap();
        std::fs::write(seed_dir.path().join("CO-1.md"), make_co_task_md(1, "todo")).unwrap();
        std::fs::write(seed_dir.path().join("CO-2.md"), make_co_task_md(2, "done")).unwrap();
        // Documentation file — must not become a task
        std::fs::write(seed_dir.path().join("CLAUDE.md"), "# Docs\nNot a task.").unwrap();

        let mut storage = Storage::new(data_dir.path().to_str().unwrap());
        storage.seed_admin_content_universes();
        storage.seed_co_universe_tasks(seed_dir.path());

        let project = storage.get_project("CO");
        assert!(project.is_some(), "CO project should exist after seeding");
        assert_eq!(project.unwrap().key, "CO");

        let tasks = storage.list_tasks("CO");
        assert_eq!(tasks.len(), 2, "only CO-N.md files should become tasks");
        let t1 = tasks.iter().find(|t| t.id == 1).expect("CO-1 missing");
        assert_eq!(t1.key, "CO-1");
        let t2 = tasks.iter().find(|t| t.id == 2).expect("CO-2 missing");
        assert_eq!(t2.status, crate::models::TaskStatus::Done);
        // CLAUDE.md must not appear as a task
        assert!(
            !tasks
                .iter()
                .any(|t| t.title.to_lowercase().contains("docs")),
            "CLAUDE.md must not be seeded as a task"
        );
    }

    #[test]
    fn test_seed_co_universe_tasks_is_idempotent() {
        let data_dir = tempfile::tempdir().unwrap();
        let seed_dir = tempfile::tempdir().unwrap();
        std::fs::write(seed_dir.path().join("CO-1.md"), make_co_task_md(1, "todo")).unwrap();

        let mut storage = Storage::new(data_dir.path().to_str().unwrap());
        storage.seed_admin_content_universes();
        storage.seed_co_universe_tasks(seed_dir.path());
        storage.seed_co_universe_tasks(seed_dir.path()); // second run

        let tasks = storage.list_tasks("CO");
        assert_eq!(tasks.len(), 1, "second run must not duplicate tasks");
    }

    // CO-264: reseed_co_root_files
    #[test]
    fn test_reseed_co_root_files_seeds_changelog_and_readme() {
        let data_dir = tempfile::tempdir().unwrap();
        let root_dir = tempfile::tempdir().unwrap();

        std::fs::write(
            root_dir.path().join("CHANGELOG.md"),
            "# Changelog\n\n## v1.0.0\n\nFirst release.",
        )
        .unwrap();
        std::fs::write(
            root_dir.path().join("README.md"),
            "# My Project\n\nProject readme.",
        )
        .unwrap();

        let mut storage = Storage::new(data_dir.path().to_str().unwrap());
        storage.seed_admin_content_universes();
        storage.reseed_co_root_files(root_dir.path());

        let co_uc = storage.universe_pool.get_or_open("co");
        let uc_guard = co_uc.lock().unwrap();
        let idx = crate::entry_index::EntryIndex::new(&uc_guard);

        let changelog = idx.get("co", "CHANGELOG.md").unwrap();
        assert!(changelog.is_some(), "CHANGELOG.md must be seeded");
        let changelog = changelog.unwrap();
        assert_eq!(changelog.entry_type, "page");
        assert_eq!(changelog.title.as_deref(), Some("Changelog"));

        let readme = idx.get("co", "README.md").unwrap();
        assert!(readme.is_some(), "README.md must be seeded");
    }

    #[test]
    fn test_reseed_co_root_files_is_idempotent() {
        let data_dir = tempfile::tempdir().unwrap();
        let root_dir = tempfile::tempdir().unwrap();
        std::fs::write(root_dir.path().join("CHANGELOG.md"), "# Changelog\n\nv1.").unwrap();

        let mut storage = Storage::new(data_dir.path().to_str().unwrap());
        storage.seed_admin_content_universes();
        storage.reseed_co_root_files(root_dir.path());
        storage.reseed_co_root_files(root_dir.path()); // second run — must not panic or duplicate

        let co_uc = storage.universe_pool.get_or_open("co");
        let uc_guard = co_uc.lock().unwrap();
        let count: i64 = uc_guard
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE universe_key = 'co' AND path = 'CHANGELOG.md'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "idempotent: exactly one CHANGELOG.md row");
    }

    #[test]
    fn test_reseed_co_root_files_silently_skips_missing_files() {
        let data_dir = tempfile::tempdir().unwrap();
        let empty_dir = tempfile::tempdir().unwrap();

        let mut storage = Storage::new(data_dir.path().to_str().unwrap());
        storage.seed_admin_content_universes();
        // Should not panic when no files are found
        storage.reseed_co_root_files(empty_dir.path());
    }
}
