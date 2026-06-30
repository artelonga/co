use chrono::Utc;
use rusqlite::params;
use serde_json::json;

use crate::entry_index::make_entry;

use crate::storage::Storage;
use crate::storage::schema::{
    seed_page_body, seed_page_frontmatter, split_frontmatter, upsert_entry_row,
};

use crate::storage::{
    SEED_CO_CHANGELOG_MD, SEED_CO_INDEX_MD, SEED_CO_PLATAFORMA_MD, SEED_CO_PUBLIC_INDEX_MD,
    SEED_CONTA_MD, SEED_DADOS_RASTREADOS_MD, SEED_EN_INDEX_MD, SEED_EN_SOBRE_MD, SEED_GUIA_MD,
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
        // CO-279: the template's tutorial project key is `CO` (matches production
        // data + the cloning contract). A short-lived 2026-05-04 rename to
        // `TUTORIAL` broke 4 template_tests.rs tests and was reverted here.
        let proj_path = "projects/CO/_project.md";
        // CO-406: do NOT panic at boot if the template universe's DB can't be
        // opened (disk full, I/O error). Skip seeding and let the server come
        // up degraded — the template recovers on a later access/reopen once the
        // environment clears, instead of crash-looping the whole site.
        let template_uc = match self.universe_pool.try_get_or_open("template") {
            Ok(uc) => uc,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "CO-406: template universe unavailable at boot — skipping seed, server will start degraded"
                );
                return;
            }
        };
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
        // Register in project_universe_index so get_project() works.
        // In production the `co` universe also seeds a `CO` project; the
        // rebuild_project_universe_index pass (clone_ops.rs) sorts `template`
        // as low-priority so the `co` universe wins the routing for the
        // legacy `/api/projects/CO/tasks` endpoint. Tests only seed the
        // template universe, so this INSERT registers `CO → template`.
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
            // --- Act 5: What's new (v3.2.0 — mirrors the release) ---
            SeedTask {
                id: 10,
                title: "Um conteúdo, muitas lentes",
                description: "Desde a versão **3.2.0**, o CO renderiza universos através de **lentes** — o mesmo conteúdo, formas diferentes de ver.\n\nVocê já conhece Kanban, Tabela e Conteúdo. Experimente também:\n\n- **Calendário** — tarefas com data viram eventos\n- **Timeline** — o tempo como eixo\n- **Grafo** — seus textos como rede de conexões\n\nPor baixo, todas são lentes registradas num único sistema — e os formulários nascem do *schema* de cada tipo de conteúdo. Novas lentes (linha do tempo de projeto, grade temporal) chegam nas próximas versões sem mudar nada do seu conteúdo.",
                status: "todo",
                priority: "medium",
                labels: vec!["novidades"],
                due_days: None,
                parent: None,
            },
            SeedTask {
                id: 11,
                title: "Novidades sem sair do terminal",
                description: "O CO também vive na linha de comando. A partir da **3.2.0**, as notas de release moram no próprio binário:\n\n```bash\nco updates           # a release mais recente\nco updates -n 3      # as três últimas\nco updates --all     # o histórico inteiro, desde a 0.1.0\n```\n\nSem rede, sem navegador — a nota sempre corresponde à versão instalada. Cada release nova vira automaticamente a próxima nota.",
                status: "todo",
                priority: "low",
                labels: vec!["novidades"],
                due_days: None,
                parent: None,
            },
            SeedTask {
                id: 12,
                title: "Leia a história do CO",
                description: "**CO** não é sigla — é o pedaço de língua que significa *junto*: **co**nectar, **co**letar, **co**municar, **co**nstruir, **co**laborar, **co**criar. Empilhadas, soletram **Collective Consciousness**.\n\nA história completa — as abstrações, as cinco eras, os universos vizinhos e o seu primeiro CRUD guiado — está em [WELCOME.md](https://github.com/artelonga/co/blob/main/docs/WELCOME.md).\n\nVocê não é a audiência deste sistema. Você está no \"nós\" — *ñandé*, o nós que inclui quem escuta.",
                status: "todo",
                priority: "low",
                labels: vec!["novidades"],
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

    /// CO-555: ingest the English mirror of the template intro pages as
    /// language-tagged entries (`language: en`, path `en/<slug>`).
    ///
    /// The mirror markdown is produced by the modular translator
    /// (`scripts/i18n/translate.py --to en`) and embedded via `include_str!`.
    /// Ingest goes through the vault + entry repository (NOT raw SQLite) and is
    /// idempotent — a re-run with an unchanged mirror is a no-op. These are NEW
    /// paths (`en/…`), so the pt reseed above never touches them.
    ///
    /// Runs on every boot, mirroring `reseed_template_content_pages`.
    pub fn reseed_template_en_pages(&mut self) {
        if !self.template_exists() {
            return; // first-boot path seeds the template first
        }
        let universe_root = self.universe_root("template");
        let template_uc = self.universe_pool.get_or_open("template");
        let repo = crate::repository::SqliteEntryRepository::new(template_uc);

        // (relative slug path, markdown) — the translator keeps slugs, so the pt
        // `/sobre` page maps to the `en/sobre.md` entry → `/en/sobre`.
        let pages = [
            ("index.md".to_string(), SEED_EN_INDEX_MD.to_string()),
            ("sobre.md".to_string(), SEED_EN_SOBRE_MD.to_string()),
        ];
        let report = crate::translate::ingest::ingest_language_pages(
            &repo,
            &universe_root,
            "template",
            "en",
            &pages,
        );
        if !report.skipped.is_empty() {
            tracing::warn!(
                "CO-555: en mirror ingest skipped {} page(s): {:?}",
                report.skipped.len(),
                report.skipped
            );
        }
        tracing::debug!(
            "CO-555: en mirror ingest — created={} updated={} unchanged={}",
            report.created,
            report.updated,
            report.unchanged
        );
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
            // CO-305: CHANGELOG.md stub — lets /co/changelog resolve to 200 HTML
            // (serve_deep_link checks for CHANGELOG.md in the entry index).
            ("CHANGELOG.md", SEED_CO_CHANGELOG_MD),
            // CO-305: public/index.md — lets /co/public/ (trailing slash) resolve
            // to 200 HTML via the folder-level index.md candidate in the serve allowlist.
            ("public/index.md", SEED_CO_PUBLIC_INDEX_MD),
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

    /// CO-279: clean up stale `projects/TUTORIAL/*` entries from the template
    /// universe. The short-lived CO-254 rename (CO → TUTORIAL, landed
    /// 2026-05-04, reverted by CO-279) left `TUTORIAL` rows in any DB that
    /// booted under the broken code. The canonical project key is `CO` again,
    /// so drop the orphans here and let `seed_template_universe` re-seed `CO`
    /// when missing.
    ///
    /// Idempotent: no-op when `projects/TUTORIAL/_project.md` is already absent.
    /// The function name is preserved for ABI compatibility with the
    /// `seed_orchestrator` call site; effective behavior is now an inverse
    /// cleanup of the never-shipped CO-254 rename.
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
                     WHERE universe_key = 'template' AND path = 'projects/TUTORIAL/_project.md'",
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
                 WHERE universe_key = 'template' AND path LIKE 'projects/TUTORIAL/%'",
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
        // Drop the stale routing index row so the rebuild pass picks up the
        // re-seeded `CO → template` mapping on this same boot.
        let _ = self.conn.execute(
            "DELETE FROM project_universe_index \
             WHERE project_key = 'TUTORIAL' AND universe_key = 'template'",
            [],
        );
        tracing::info!(
            "CO-279: dropped {} stale TUTORIAL entries from template; \
             will re-seed as CO on next seed_template_universe call",
            paths_to_delete.len()
        );
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
            let (path, md) = ("index.md", crate::storage::SEED_YGGDRASIL_INDEX_MD);
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
    /// Local-dev workspace: register every top-level content folder under `dir`
    /// as a universe (key = folder name). "All folders are universes" — a folder
    /// qualifies if it has a `_universe.yaml` OR holds markdown content; build and
    /// system dirs (target, node_modules, backups, dot-dirs, …) are skipped. This
    /// makes the `~/projects` tree a universe workspace — dropping/moving a folder
    /// in (e.g. `~/projects/yuri`) promotes it to a universe with no registry, no
    /// code change and no deploy. DB-driven (idempotent `INSERT OR IGNORE`); the existing
    /// `run_sister_repo_seeds` then ingests each folder's content for localhost.
    ///
    /// Gated by the caller on `CO_LOCAL_REPOS_DIR` being set, so it only ever
    /// runs in local dev — prod (which never sets that env) is untouched.
    /// Returns the number of newly-registered universes.
    pub fn register_universes_from_local_dir(&mut self, dir: &std::path::Path) -> usize {
        let now = Utc::now().to_rfc3339();
        let mut registered = 0usize;
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(dirname) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            // Skip system/build/non-content dirs — never universes.
            const SKIP: &[&str] = &[
                "target",
                "node_modules",
                "dist",
                "build",
                "backups",
                ".git",
                ".worktrees",
            ];
            if dirname.starts_with('.') || SKIP.contains(&dirname) {
                continue;
            }
            // "All folders are universes" — no registry/co-create ceremony. A
            // folder qualifies if it carries a `_universe.yaml` (for metadata
            // like parent/visibility) OR simply holds markdown content
            // (`content/` dir, `index.md`, or any root `*.md`). Dropping a
            // content folder into the workspace is enough to promote it.
            let manifest_path = path.join("_universe.yaml");
            let has_manifest = manifest_path.exists();
            let has_content = path.join("content").is_dir()
                || path.join("index.md").exists()
                || std::fs::read_dir(&path)
                    .map(|rd| {
                        rd.flatten().any(|e| {
                            e.path()
                                .extension()
                                .and_then(|x| x.to_str())
                                .is_some_and(|x| x.eq_ignore_ascii_case("md"))
                        })
                    })
                    .unwrap_or(false);
            // CO-467: skip pure software/infra repos — a code-project manifest at
            // root with NO `_universe.yaml` and NO `content/` dir. Their README
            // markdown tripped the content heuristic and over-registered dormant
            // repos (aws, prediction-market, rfq-gateway, …) as universes.
            let is_code_repo = !has_manifest
                && !path.join("content").is_dir()
                && (path.join("Cargo.toml").exists()
                    || path.join("package.json").exists()
                    || path.join("pyproject.toml").exists()
                    || path.join("go.mod").exists());
            if (!has_manifest && !has_content) || is_code_repo {
                continue;
            }
            // key = sanitized folder name ([a-z0-9-], ≤64).
            let key: String = dirname
                .to_lowercase()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                .collect::<String>()
                .trim_matches('-')
                .chars()
                .take(64)
                .collect();
            if key.is_empty() {
                continue;
            }
            // name + parent + visibility from the manifest when present.
            let (name, parent, visibility) = if has_manifest {
                std::fs::read(&manifest_path)
                    .ok()
                    .and_then(|b| co::manifest::parse(&b).ok())
                    .map(|r| (r.manifest.name, r.manifest.parent, r.manifest.visibility))
                    .unwrap_or_else(|| (dirname.to_string(), None, None))
            } else {
                (dirname.to_string(), None, None)
            };
            // CO-467: honor the manifest's declared visibility (default private).
            // `public-*` → is_public=1 so it surfaces in the public list.
            let vis = visibility.as_deref().unwrap_or("private");
            let is_public: i64 = if vis.starts_with("public") { 1 } else { 0 };
            // Index `content/` if present (the canonical mirror dir), else root.
            let subdirs = if path.join("content").is_dir() {
                r#"["content"]"#
            } else {
                r#"[""]"#
            };
            let local = path.to_string_lossy().to_string();
            let inserted = self
                .conn
                .execute(
                    "INSERT OR IGNORE INTO universes \
                     (key, name, description, owner_id, created_at, is_template, is_public, \
                      visibility, theme_preset, layout, content_count, parent_key, \
                      local_repo_path, content_subdirs) \
                     VALUES (?1, ?2, '', 'system', ?3, 0, ?7, ?8, 'scholarly-light', \
                      'board', 0, ?4, ?5, ?6)",
                    rusqlite::params![key, name, now, parent, local, subdirs, is_public, vis],
                )
                .unwrap_or(0);
            // CO-467: reconcile an EXISTING workspace row with the manifest —
            // `INSERT OR IGNORE` never updates, so visibility/parent declared in
            // `_universe.yaml` drifted from a stale earlier seed (miguel/mse/
            // grcsamazonia were stuck `private`). Only touch workspace-scanned
            // rows (those with a `local_repo_path`), never seeded system rows.
            if inserted == 0 && has_manifest {
                let _ = self.conn.execute(
                    "UPDATE universes SET visibility = ?2, is_public = ?3, parent_key = ?4 \
                     WHERE key = ?1 AND local_repo_path IS NOT NULL AND local_repo_path != ''",
                    rusqlite::params![key, vis, is_public, parent],
                );
            }
            if inserted > 0 {
                self.seed_default_project_if_missing(&key);
                registered += 1;
                let src = if has_manifest {
                    "_universe.yaml"
                } else {
                    "content"
                };
                tracing::info!(
                    "workspace: registered universe '{key}' from {local} ({src}, {vis})"
                );
            }
        }
        registered
    }

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
                // CO-319: was "private" so it never appeared in any user's
                // sidebar. RFQ Gateway is a public sister deployment (rfq.fly.dev)
                // — mark public-subscribable so users can discover + subscribe.
                "public-subscribable",
                None,
            ),
            // CO-319: comunicacao universe — the matching sister repo at
            // ~/projects/comunicacao/ exists and has docs+content; create the
            // universe row so CO-317 sister-repo seeding can populate it.
            (
                "comunicacao",
                "Comunicação",
                "Comunicação — protocolos, infraestrutura e canais entre universos",
                "public-subscribable",
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
            // CO-347: four content universes surfaced on prod via remote sync.
            (
                "yuri",
                "Yuri",
                "Caderno pessoal — yuri.artelonga.com.br",
                "public-subscribable",
                None,
            ),
            (
                "retro-umarizal",
                "Retro Umarizal",
                "Memória do bairro — retro-umarizal.artelonga.com.br",
                "public-subscribable",
                None,
            ),
            (
                "yoruba",
                "Yorùbá",
                "Léxico Yorùbá — termos, divindades, conceitos",
                "public-subscribable",
                Some("comunicacao"),
            ),
            (
                "neuro",
                "Neuro",
                "Cartografia neuro — laudo, recursos",
                "public-subscribable",
                Some("artelonga"),
            ),
            // CO-364: external OSS projects surfaced as study/integration universes.
            (
                "odysseus",
                "Odysseus",
                "Self-hosted AI workspace — open-source reference (pewdiepie-archdaemon/odysseus)",
                "public-subscribable",
                None,
            ),
            // CO-438 (Bug 1): claude-code is intentionally NOT seeded here.
            // It is a *private* importable universe; seeding a private row owned
            // by the sentinel 'system' (with no real owner) made it an orphan —
            // `POST /universes` → 409 "key taken", but `GET /universes/{key}` →
            // 404 because `check_universe_access` admits only the owner_id, and
            // 'system' is no real user. Per the fix, importable private
            // universes are created by `co source add` as the importing user
            // (fully provisioned via `create_universe`: owner + membership +
            // pool). A fresh DB therefore has no claude-code until imported —
            // never "taken but 404". The remote-sync backfill + metadata
            // reconcile below still tidy an *existing* (real-owner) row.
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
            // CO-279: every admin-content universe needs a default project so
            // "/co/<key>" never renders the "no project found" dead-end. The
            // `co` universe is the exception — `seed_co_universe_tasks` runs
            // later and seeds its own `CO` project from /app/seed-co/.
            if key != "co" {
                self.seed_default_project_if_missing(key);
            }
        }
        // CO-319 one-shot migration: rfq was originally seeded as `private`
        // (1.x line) so existing installs have it stuck that way — INSERT OR
        // IGNORE above won't touch existing rows. This UPDATE targets only
        // the rfq row and only when it's still in the wrong state, so it's
        // safe to run on every boot without stomping user-set visibility.
        let _ = self.conn.execute(
            "UPDATE universes SET visibility = 'public-subscribable', is_public = 1 \
             WHERE key = 'rfq' AND visibility = 'private'",
            [],
        );

        // CO-429: claude-code was seeded by CO-364 as public-subscribable with
        // parent_key=NULL. Fix existing installs so the metadata matches intent
        // (private, parent=co). Guards fire once and are no-ops thereafter.
        let _ = self.conn.execute(
            "UPDATE universes SET parent_key = 'co' \
             WHERE key = 'claude-code' AND parent_key IS NULL",
            [],
        );
        // CO-438 (Bug 1): only privatize a claude-code row that has a *real*
        // owner. Flipping a sentinel 'system'-owned public-subscribable row to
        // private is exactly what created the orphan (private + no real owner →
        // GET 404, POST 409). A system-owned legacy row stays public-subscribable
        // (still GET-able) rather than being orphaned; a real-owner import is
        // correctly privatized and remains accessible to its owner.
        let _ = self.conn.execute(
            "UPDATE universes SET visibility = 'private', is_public = 0 \
             WHERE key = 'claude-code' AND visibility = 'public-subscribable' \
               AND owner_id != 'system'",
            [],
        );
        // CO-429: fix subdirs — upstream repo has examples/ and plugins/, no docs/.
        let _ = self.conn.execute(
            "UPDATE universes \
             SET content_subdirs = '[\"examples\",\"plugins\",\"README.md\",\"CHANGELOG.md\"]' \
             WHERE key = 'claude-code' \
               AND content_subdirs = '[\"docs\",\"README.md\",\"CHANGELOG.md\"]'",
            [],
        );

        // CO-347: backfill remote sync config for the four new content universes.
        // WHERE remote_url IS NULL is the idempotency guard — once an operator
        // sets a custom remote_url the boot UPDATE never overwrites it.
        for (key, url, gitref, subdirs, anon_only) in [
            (
                "yuri",
                "https://github.com/artelonga/artelonga",
                "main",
                Some("[\"yuri\"]"),
                true,
            ),
            (
                "retro-umarizal",
                "https://github.com/artelonga/retro-umarizal",
                "main",
                None,
                false,
            ),
            (
                "yoruba",
                "https://github.com/artelonga/comunicacao",
                "main",
                Some("[\"yoruba\"]"),
                false,
            ),
            (
                "neuro",
                "https://github.com/artelonga/artelonga",
                "main",
                Some("[\"neuro\"]"),
                false,
            ),
            // CO-364: OSS reference universes — read-only mirrors for study/integration.
            (
                "odysseus",
                "https://github.com/pewdiepie-archdaemon/odysseus",
                "dev",
                Some("[\"docs\",\"README.md\",\"CHANGELOG.md\"]"),
                false,
            ),
            // CO-429: the upstream repo has examples/ and plugins/ (no docs/).
            (
                "claude-code",
                "https://github.com/anthropics/claude-code",
                "main",
                Some("[\"examples\",\"plugins\",\"README.md\",\"CHANGELOG.md\"]"),
                false,
            ),
        ] {
            let _ = self.conn.execute(
                "UPDATE universes \
                 SET remote_url = ?2, remote_ref = ?3, content_subdirs = ?4, anon_published_only = ?5 \
                 WHERE key = ?1 AND remote_url IS NULL",
                rusqlite::params![key, url, gitref, subdirs, anon_only as i64],
            );
        }
    }

    /// CO-379: seed stable fixture universes for the staging environment.
    ///
    /// These universes are read-only baselines — never mutated by Playwright
    /// test runs. Idempotent: `INSERT OR IGNORE` skips already-present rows.
    ///
    /// Fixture set:
    /// - `recursion-a` / `recursion-a-b` / `recursion-a-b-c` — nested-universe
    ///   fixture (keys match the CO-374 suite preconditions exactly)
    /// - `funnel-fixture` — pre-seeded mixed leads + users for funnel report tests
    /// - `mbya-staging` / `yoruba-staging` — workspace template fixtures
    pub fn seed_staging_fixture_universes(&mut self) {
        let now = Utc::now().to_rfc3339();

        // (key, name, description, visibility, parent_key)
        //
        // CO-401: the recursion chain keys are `recursion-a` → `recursion-a-b`
        // → `recursion-a-b-c` — the exact keys the CO-374 staging suite
        // (`universe-recursion.spec.ts`, `subuniverse-promotion.spec.ts`)
        // preconditions on. (The original CO-379 seeder used `recursion-ab`/
        // `recursion-abc`, which the suite never matched — every recursion test
        // skipped.)
        let fixtures: &[(&str, &str, &str, &str, Option<&str>)] = &[
            (
                "recursion-a",
                "Recursion A",
                "Staging fixture: top-level universe for recursion tests",
                "public",
                None,
            ),
            (
                "recursion-a-b",
                "Recursion A/B",
                "Staging fixture: sub-universe of recursion-a",
                "public",
                Some("recursion-a"),
            ),
            (
                "recursion-a-b-c",
                "Recursion A/B/C",
                "Staging fixture: sub-sub-universe of recursion-a-b",
                "public",
                Some("recursion-a-b"),
            ),
            (
                "funnel-fixture",
                "Funnel Fixture",
                "Staging fixture: pre-seeded leads + users for funnel report tests",
                "private",
                None,
            ),
            (
                "mbya-staging",
                "Mbya Staging",
                "Staging fixture: workspace template fixture (mbya lexicon)",
                "public",
                None,
            ),
            (
                "yoruba-staging",
                "Yoruba Staging",
                "Staging fixture: workspace template fixture (yoruba lexicon)",
                "public",
                None,
            ),
        ];

        for &(key, name, desc, vis, parent) in fixtures {
            let _ = self.conn.execute(
                "INSERT OR IGNORE INTO universes \
                 (key, name, description, owner_id, created_at, visibility) \
                 VALUES (?1, ?2, ?3, 'system', ?4, ?5)",
                rusqlite::params![key, name, desc, now, vis],
            );
            if let Some(p) = parent {
                let _ = self.conn.execute(
                    "UPDATE universes SET parent_key = ?2 \
                     WHERE key = ?1 AND parent_key IS NULL",
                    rusqlite::params![key, p],
                );
            }
        }

        tracing::info!(
            "CO-379: staging fixture universes seeded: recursion-a, recursion-a-b, \
             recursion-a-b-c, funnel-fixture, mbya-staging, yoruba-staging"
        );
    }

    /// CO-401: seed synthetic funnel/lead fixtures for the staging suite.
    ///
    /// Each row is flagged `is_synthetic = 1` (migration v82) so the
    /// acquisition-funnel rollup (`funnel_routes::query_funnel_steps`) excludes
    /// it — the suite can assert lead/funnel behavior without polluting the real
    /// analytics metrics. The set spans the lead lifecycle so funnel steps 4
    /// (Capture) and 5 (Qualify) have data to exercise.
    ///
    /// Idempotent: once any synthetic lead exists this is a no-op, so re-running
    /// on every staging boot never duplicates fixtures.
    pub fn seed_staging_funnel_fixtures(&self) {
        let existing: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM leads WHERE is_synthetic = 1",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if existing > 0 {
            return;
        }

        let now = Utc::now().to_rfc3339();
        // (nome, email, mensagem, status)
        let fixtures: &[(&str, &str, &str, &str)] = &[
            (
                "Fixture Lead — Novo",
                "fixture-new@staging.local",
                "synthetic funnel fixture",
                "new",
            ),
            (
                "Fixture Lead — Triado",
                "fixture-triaged@staging.local",
                "synthetic funnel fixture",
                "triaged",
            ),
            (
                "Fixture Lead — Em progresso",
                "fixture-progress@staging.local",
                "synthetic funnel fixture",
                "in_progress",
            ),
            (
                "Fixture Lead — Ganho",
                "fixture-won@staging.local",
                "synthetic funnel fixture",
                "closed_won",
            ),
            (
                "Fixture Lead — Perdido",
                "fixture-lost@staging.local",
                "synthetic funnel fixture",
                "closed_lost",
            ),
        ];

        for &(nome, email, mensagem, status) in fixtures {
            let _ = self.conn.execute(
                "INSERT INTO leads \
                 (created_at, updated_at, nome, email, mensagem, status, priority, source, is_synthetic) \
                 VALUES (?1, ?1, ?2, ?3, ?4, ?5, 'normal', 'staging-fixture', 1)",
                rusqlite::params![now, nome, email, mensagem, status],
            );
        }

        tracing::info!(
            "CO-401: seeded {} synthetic funnel/lead fixtures (is_synthetic=1, excluded from rollups)",
            fixtures.len()
        );
    }

    /// CO-401: ensure the dedicated `staging-admin` user exists (admin tier),
    /// idempotently. The staging suite's capability-scoped token is owned by
    /// this user. Admin tier is required so the owner could legitimately *mint*
    /// admin-surface capabilities — but the token itself (seeded separately)
    /// carries explicit least-privilege scopes, never the owner's full tier.
    /// Returns the user id.
    pub fn ensure_staging_admin_user(&self) -> anyhow::Result<String> {
        let id = "staging-admin";
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR IGNORE INTO users \
             (id, email, display_name, tier, created_at, status) \
             VALUES (?1, ?2, ?3, 'admin', ?4, 'active')",
            rusqlite::params![
                id,
                "staging-admin@staging.local",
                "Staging Suite Admin",
                now
            ],
        )?;
        // Idempotently keep admin tier even if a prior row drifted lower.
        self.conn.execute(
            "UPDATE users SET tier = 'admin' WHERE id = ?1 AND tier <> 'admin'",
            rusqlite::params![id],
        )?;
        Ok(id.to_string())
    }

    /// CO-379: delete `u-test-*` universe rows (and their directories) that are
    /// older than `max_age_days`, retaining the most recent `keep_count` rows for
    /// forensic inspection. Returns the number of universes deleted.
    pub fn sweep_test_namespaces(&mut self, max_age_days: i64, keep_count: usize) -> usize {
        let cutoff = (chrono::Utc::now()
            - chrono::Duration::try_days(max_age_days).unwrap_or(chrono::Duration::zero()))
        .to_rfc3339();

        // IDs of the most-recent `keep_count` u-test-* universes — these are preserved.
        let keep_keys: Vec<String> = {
            let mut stmt = match self.conn.prepare(
                "SELECT key FROM universes WHERE key LIKE 'u-test-%' \
                 ORDER BY created_at DESC LIMIT ?1",
            ) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("CO-379: sweep_test_namespaces prepare (keep): {e}");
                    return 0;
                }
            };
            match stmt.query_map(rusqlite::params![keep_count as i64], |r| r.get(0)) {
                Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                Err(_) => vec![],
            }
        };

        // Collect universe keys and dirs to delete (older than cutoff, not in keep set).
        let to_delete: Vec<(String, std::path::PathBuf)> = {
            let mut stmt = match self.conn.prepare(
                "SELECT key FROM universes \
                 WHERE key LIKE 'u-test-%' AND created_at < ?1",
            ) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("CO-379: sweep_test_namespaces prepare (sweep): {e}");
                    return 0;
                }
            };
            let keys: Vec<String> =
                match stmt.query_map(rusqlite::params![cutoff], |r| r.get::<_, String>(0)) {
                    Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                    Err(_) => vec![],
                };
            keys.into_iter()
                .filter(|k| !keep_keys.contains(k))
                .map(|k| {
                    let dir = self.universe_root(&k);
                    (k, dir)
                })
                .collect()
        };

        let mut deleted = 0;
        for (key, dir) in &to_delete {
            // Cascade: delete child rows in all tables with universe_key before
            // deleting the parent row (mirrors delete_universe in universe_routes.rs).
            let tables: Vec<String> = {
                let conn = &self.conn;
                match conn.prepare(
                    "SELECT m.name FROM sqlite_master m \
                     JOIN pragma_table_info(m.name) p \
                     WHERE m.type = 'table' AND m.name != 'universes' \
                       AND m.name NOT LIKE 'sqlite_%' AND p.name = 'universe_key'",
                ) {
                    Ok(mut s) => match s.query_map([], |r| r.get::<_, String>(0)) {
                        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                        Err(_) => vec![],
                    },
                    Err(_) => vec![],
                }
            };
            for table in &tables {
                let sql = format!("DELETE FROM \"{table}\" WHERE universe_key = ?1");
                let _ = self.conn.execute(&sql, rusqlite::params![key]);
            }
            if let Err(e) = self.conn.execute(
                "DELETE FROM universes WHERE key = ?1",
                rusqlite::params![key],
            ) {
                tracing::warn!("CO-379: sweep_test_namespaces delete row {key}: {e}");
                continue;
            }
            if dir.exists() {
                let _ = std::fs::remove_dir_all(dir);
            }
            deleted += 1;
        }

        deleted
    }

    /// CO-279: ensure a universe has at least one project entry so the kanban
    /// board never lands on the "no project found" empty state.
    ///
    /// Idempotent — returns false (no-op) when the universe already has any
    /// project. The project key follows the same `{first-4-of-universe-key
    /// uppercased}P` convention as `create_universe`, so it stays globally
    /// unique against the `project_universe_index` PK without colliding with
    /// neighbour universes' defaults.
    ///
    /// Returns true when a new default project was created.
    pub fn seed_default_project_if_missing(&mut self, universe_key: &str) -> bool {
        // Skip if the universe row itself doesn't exist yet — the caller is
        // expected to invoke this AFTER ensuring the universe is seeded.
        if self.get_universe(universe_key).is_none() {
            return false;
        }
        // Skip if any project entry already lives in this universe.
        if !self.list_projects_for_universe(universe_key).is_empty() {
            return false;
        }
        let proj_key: String = format!(
            "{}P",
            universe_key
                .to_uppercase()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(4)
                .collect::<String>()
        );
        if proj_key.len() < 2 {
            // Universe key has no usable alphanumerics — refuse silently
            // rather than emit an unusable single-letter project key.
            return false;
        }
        let now_str = Utc::now().to_rfc3339();
        let proj_path = format!("projects/{}/_project.md", proj_key);
        let proj_fm = json!({
            "type": "project",
            "key": proj_key,
            "title": "Bem-vindo",
            "status": "active",
            "next_id": 1,
            "created": now_str,
            "modified": now_str,
            "archived": false,
            "tags": []
        });
        let proj_entry = make_entry(&proj_path, proj_fm, "");
        let universe_root = self.universe_root(universe_key);
        let _ = co::entry::write_entry(&universe_root, &proj_entry);
        {
            let uc = self.universe_pool.get_or_open(universe_key);
            let uc_guard = uc.lock().expect("universe conn lock");
            let _ = upsert_entry_row(&uc_guard, universe_key, &proj_entry);
        }
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO project_universe_index \
             (project_key, universe_key) VALUES (?1, ?2)",
            params![proj_key, universe_key],
        );
        tracing::info!(
            "CO-279: seeded default project '{}' in universe '{}'",
            proj_key,
            universe_key
        );
        true
    }

    /// CO-279: walk every non-template universe and seed a default project
    /// when missing. Returns the number of universes that received a new
    /// project. Idempotent — universes that already have any project are
    /// no-ops.
    ///
    /// Skips:
    /// - template universes (templates are read-only / cloned, not user-driven)
    /// - anonymous-clone universes (`anon-*`, `u-*`) — these are short-lived
    ///   demo clones and the clone path already inherits the template's
    ///   project
    /// - timeline universes (tempo / humanity / universo) — content lives
    ///   under `events/`, not `projects/`
    /// - the `co` universe — its canonical project (`CO`) is seeded later
    ///   by `seed_co_universe_tasks` from `/app/seed-co/`; pre-seeding a
    ///   `COP` placeholder here would leave the universe with two projects
    pub fn backfill_default_projects(&mut self) -> usize {
        let candidates: Vec<String> = {
            let mut stmt = match self.conn.prepare(
                "SELECT key FROM universes \
                 WHERE is_template = 0 \
                   AND key NOT LIKE 'anon-%' \
                   AND key NOT LIKE 'u-%' \
                   AND key NOT IN ('tempo', 'humanity', 'universo', 'co')",
            ) {
                Ok(s) => s,
                Err(_) => return 0,
            };
            stmt.query_map([], |row| row.get::<_, String>(0))
                .into_iter()
                .flatten()
                .filter_map(|r| r.ok())
                .collect()
        };
        let mut seeded = 0usize;
        for key in &candidates {
            if self.seed_default_project_if_missing(key) {
                seeded += 1;
            }
        }
        seeded
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
    /// CO-317: ingest `.md` files from a local repo into a universe.
    ///
    /// For local dev: when `~/projects/<repo>/docs/`, `<repo>/content/`, or
    /// `<repo>/work/` contain markdown, mirror them into the matching universe
    /// so localhost shows the same content the deployed universe would.
    ///
    /// Idempotent: skips entirely when the universe already has more than
    /// `skip_if_count_above` entries (so this only does the initial seed and
    /// doesn't fight user-created content on later boots). Pass `0` to always
    /// re-ingest.
    pub fn seed_universe_from_local_repo(
        &mut self,
        universe_key: &str,
        repo_root: &std::path::Path,
        subdirs: &[&str],
        skip_if_count_above: i64,
    ) {
        if !repo_root.exists() {
            return;
        }
        if self.get_universe(universe_key).is_none() {
            tracing::warn!(
                "seed_universe_from_local_repo: universe '{universe_key}' missing — skipped"
            );
            return;
        }

        if skip_if_count_above > 0 {
            let existing: i64 = {
                let uc = self.universe_pool.get_or_open(universe_key);
                uc.lock()
                    .ok()
                    .and_then(|g| {
                        g.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get::<_, i64>(0))
                            .ok()
                    })
                    .unwrap_or(0)
            };
            if existing > skip_if_count_above {
                return;
            }
        }

        let universe_root = self.universe_root(universe_key);
        let now_str = Utc::now().to_rfc3339();
        let mut upserted = 0usize;

        for subdir in subdirs {
            let src = repo_root.join(subdir);
            if !src.exists() {
                continue;
            }
            for fs_path in walk_md_files(&src) {
                let rel = match fs_path.strip_prefix(repo_root) {
                    Ok(r) => r.to_path_buf(),
                    Err(_) => continue,
                };
                let raw = match std::fs::read_to_string(&fs_path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let entry_path = rel.to_string_lossy().replace('\\', "/");
                let entry = make_entry(
                    &entry_path,
                    seed_page_frontmatter(&raw, &now_str),
                    seed_page_body(&raw),
                );
                if co::entry::write_entry(&universe_root, &entry).is_err() {
                    continue;
                }
                let uc = self.universe_pool.get_or_open(universe_key);
                let conn = match uc.lock() {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                if upsert_entry_row(&conn, universe_key, &entry).is_ok() {
                    upserted += 1;
                    // CO-394: extract [[wikilink]] relations during seed, in parity with
                    // the vault/entry write path. Without this, CLI-seeded universes
                    // (`co launch`) have entries but an empty `entry_relations` table, so
                    // the knowledge-graph view renders nodes with zero edges.
                    //
                    // Same-universe targets are resolved relative to the linking entry's
                    // directory so the stored `to_path` matches stored entry paths (which
                    // carry the content-subdir prefix) — otherwise edges dangle and the
                    // graph builder drops them.
                    let relations: Vec<_> =
                        crate::relation_index::extract_body_wikilinks(&entry.body)
                            .into_iter()
                            .map(|(rt, to_path, to_univ, label)| {
                                let resolved = if to_univ.is_none() {
                                    resolve_entry_rel(&entry_path, &to_path)
                                } else {
                                    to_path
                                };
                                (rt, resolved, to_univ, label)
                            })
                            .collect();
                    let _ = crate::relation_index::RelationIndex::new(&conn).replace_all(
                        universe_key,
                        &entry_path,
                        &relations,
                    );
                }
            }
        }

        if upserted > 0 {
            let actual_count: i64 = {
                let uc = self.universe_pool.get_or_open(universe_key);
                uc.lock()
                    .ok()
                    .and_then(|g| {
                        g.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get::<_, i64>(0))
                            .ok()
                    })
                    .unwrap_or(upserted as i64)
            };
            let _ = self.conn.execute(
                "UPDATE universes SET content_count = ?1 WHERE key = ?2",
                rusqlite::params![actual_count, universe_key],
            );
            tracing::info!(
                "CO-317: seeded {upserted} entries into '{universe_key}' from {} (universe now has {actual_count} total)",
                repo_root.display()
            );
        }
    }

    /// CO-318: ingest `work/<space>/{PREFIX}-N.md` task specs from a local repo
    /// into a universe as kanban-board-compatible tasks.
    ///
    /// Walks `<work_root>/<space>/` recursively for files matching
    /// `{PREFIX}-{digits}.md`. The PREFIX (e.g. `AL`, `YG`, `RFQ`) becomes
    /// the project key — each unique prefix gets a `projects/<PREFIX>/_project.md`
    /// entry, then each task is upserted at `projects/<PREFIX>/<filename>` with
    /// `type: task` + `project: <PREFIX>` so it appears in the kanban view.
    ///
    /// Idempotent: skips entirely when the universe already has more than
    /// `skip_if_count_above` task entries.
    pub fn seed_universe_work_tasks_from_local(
        &mut self,
        universe_key: &str,
        work_root: &std::path::Path,
        skip_if_count_above: i64,
    ) {
        if !work_root.exists() {
            return;
        }
        if self.get_universe(universe_key).is_none() {
            return;
        }

        if skip_if_count_above > 0 {
            let existing: i64 = {
                let uc = self.universe_pool.get_or_open(universe_key);
                uc.lock()
                    .ok()
                    .and_then(|g| {
                        g.query_row(
                            "SELECT COUNT(*) FROM entries WHERE entry_type = 'task'",
                            [],
                            |r| r.get::<_, i64>(0),
                        )
                        .ok()
                    })
                    .unwrap_or(0)
            };
            if existing > skip_if_count_above {
                return;
            }
        }

        let universe_root = self.universe_root(universe_key);
        let now_str = Utc::now().to_rfc3339();
        let mut seeded_projects: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut upserted = 0usize;

        for fs_path in walk_md_files(work_root) {
            let filename = match fs_path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if !is_task_filename(&filename) {
                continue;
            }
            // Project key = prefix before the final '-{digits}.md'.
            let stem = match filename.strip_suffix(".md") {
                Some(s) => s,
                None => continue,
            };
            let project_key = match stem.rfind('-') {
                Some(pos) => stem[..pos].to_string(),
                None => continue,
            };

            // Seed project entry once per prefix.
            if !seeded_projects.contains(&project_key) {
                let proj_path = format!("projects/{}/_project.md", project_key);
                let proj_fm = json!({
                    "type": "project",
                    "key": project_key,
                    "title": format!("{} Board", project_key),
                    "status": "active",
                    "next_id": 1,
                    "created": now_str,
                    "modified": now_str,
                    "archived": false,
                    "tags": ["dev"],
                });
                let proj_entry = make_entry(
                    &proj_path,
                    proj_fm,
                    &format!(
                        "{} — tasks ingested from local repo work/ at boot (CO-318)",
                        project_key
                    ),
                );
                let _ = co::entry::write_entry(&universe_root, &proj_entry);
                let uc = self.universe_pool.get_or_open(universe_key);
                if let Ok(conn) = uc.lock() {
                    let _ = upsert_entry_row(&conn, universe_key, &proj_entry);
                }
                let _ = self.conn.execute(
                    "INSERT OR IGNORE INTO project_universe_index (project_key, universe_key) \
                     VALUES (?1, ?2)",
                    rusqlite::params![project_key, universe_key],
                );
                seeded_projects.insert(project_key.clone());
            }

            // Upsert the task itself under projects/<KEY>/<filename>.
            let raw = match std::fs::read_to_string(&fs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let entry_path = format!("projects/{}/{}", project_key, filename);
            let entry = make_entry(
                &entry_path,
                work_task_frontmatter(&raw, &now_str, &project_key),
                seed_page_body(&raw),
            );
            if co::entry::write_entry(&universe_root, &entry).is_err() {
                continue;
            }
            let uc = self.universe_pool.get_or_open(universe_key);
            if let Ok(conn) = uc.lock()
                && upsert_entry_row(&conn, universe_key, &entry).is_ok()
            {
                upserted += 1;
            }
        }

        if upserted > 0 {
            let actual_count: i64 = {
                let uc = self.universe_pool.get_or_open(universe_key);
                uc.lock()
                    .ok()
                    .and_then(|g| {
                        g.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get::<_, i64>(0))
                            .ok()
                    })
                    .unwrap_or(upserted as i64)
            };
            let _ = self.conn.execute(
                "UPDATE universes SET content_count = ?1 WHERE key = ?2",
                rusqlite::params![actual_count, universe_key],
            );
            tracing::info!(
                "CO-318: seeded {upserted} tasks across {} project(s) into '{universe_key}' from {}",
                seeded_projects.len(),
                work_root.display()
            );
        }
    }

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
        // CO-346: guard on universe row BEFORE source-dir check so the CO project
        // upsert below always runs even when task files are not yet available.
        // Without this, a fresh install where /app/seed-co/ is absent leaves `co`
        // with zero projects — `bootAppForUniverse` never calls `selectProject`
        // and the kanban renders empty despite 1000+ entries from `co push`.
        if self.get_universe("co").is_none() {
            tracing::warn!("seed_co_universe_tasks: 'co' universe row missing — skipped");
            return;
        }

        let universe_root = self.universe_root("co");
        let now_str = Utc::now().to_rfc3339();

        // CO-261 / CO-346: always upsert the CO Development Board project entry
        // so the kanban has a project to show even when source_dir is absent.
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

        if !source_dir.exists() {
            tracing::warn!(
                "seed_co_universe_tasks: source dir {} does not exist — \
                 CO project seeded, task files skipped",
                source_dir.display()
            );
            return;
        }

        let mut upserted = 0usize;
        let mut skipped = 0usize;

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

/// CO-317: recursively collect all `.md` files under `dir`. Skips common
/// developer-tool directories (`.git`, `target`, `node_modules`, `.next`,
/// `dist`, `build`) so we don't ingest 10k irrelevant readmes.
/// Resolve a same-universe wikilink `target` relative to the linking entry's
/// directory, normalizing `.`/`..`, so the resulting path matches stored entry
/// paths (e.g. from `content/documentos-fundacao/_index.md`, target
/// `01-capa.md` → `content/documentos-fundacao/01-capa.md`; `../index.md` →
/// `content/index.md`). Used by the seed relation-extraction (CO-394).
fn resolve_entry_rel(from_entry: &str, target: &str) -> String {
    let base = std::path::Path::new(from_entry)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let joined = if base.is_empty() {
        target.to_string()
    } else {
        format!("{base}/{target}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for comp in joined.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c),
        }
    }
    parts.join("/")
}

pub(super) fn walk_md_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    fn is_skip_dir(name: &str) -> bool {
        matches!(
            name,
            ".git" | "target" | "node_modules" | ".next" | "dist" | "build" | ".turbo" | "out"
        )
    }
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in read.flatten() {
        let p = entry.path();
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if p.is_dir() {
            if is_skip_dir(&name) {
                continue;
            }
            out.extend(walk_md_files(&p));
            continue;
        }
        if p.extension().and_then(|s| s.to_str()) == Some("md") {
            out.push(p);
        }
    }
    out
}

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
    fn test_ensure_personal_universe() {
        let data_dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(data_dir.path().to_str().unwrap());
        let user = storage.create_user("alice@example.com", "Alice").unwrap();

        // First login → private universe keyed by the email local-part.
        let key = storage
            .ensure_personal_universe(&user.id, "alice@example.com", "Alice")
            .unwrap();
        assert_eq!(key.as_deref(), Some("alice"));
        let u = storage.get_universe("alice").expect("created");
        assert_eq!(u.visibility, "private");
        assert!(
            storage
                .list_owned_universes(&user.id)
                .iter()
                .any(|x| x.universe.key == "alice")
        );

        // Idempotent — they already own one, so a second call is a no-op.
        assert_eq!(
            storage
                .ensure_personal_universe(&user.id, "alice@example.com", "Alice")
                .unwrap(),
            None
        );
    }

    #[test]
    fn test_register_universes_from_local_dir() {
        let data_dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(data_dir.path().to_str().unwrap());

        // Fake workspace exercising "all folders are universes":
        let ws = tempfile::tempdir().unwrap();
        // 1) folder WITH a manifest declaring visibility → honored (CO-467)
        let uni = ws.path().join("my-universe");
        std::fs::create_dir_all(uni.join("content")).unwrap();
        std::fs::write(
            uni.join("_universe.yaml"),
            "schema_version: 1\nname: My Universe\nvisibility: public-subscribable\n",
        )
        .unwrap();
        // 2) folder WITHOUT a manifest but WITH markdown content → still a universe
        let plain = ws.path().join("plain-notes");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(plain.join("hello.md"), "# Hi").unwrap();
        // 3) build dir → skipped even though it may contain files
        let target = ws.path().join("target");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("readme.md"), "noise").unwrap();
        // 4) empty/non-content folder → skipped
        std::fs::create_dir_all(ws.path().join("not-a-universe")).unwrap();
        // 5) CO-467: pure code repo (Cargo.toml + README, no manifest, no content/) → skipped
        let coderepo = ws.path().join("some-rust-tool");
        std::fs::create_dir_all(&coderepo).unwrap();
        std::fs::write(coderepo.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        std::fs::write(coderepo.join("README.md"), "# tool").unwrap();

        let n = storage.register_universes_from_local_dir(ws.path());
        assert_eq!(n, 2, "manifest folder + markdown-content folder register");
        let u = storage.get_universe("my-universe").expect("registered");
        assert_eq!(u.name, "My Universe");
        assert_eq!(
            u.visibility, "public-subscribable",
            "manifest visibility honored"
        );
        assert!(u.is_public, "public-* → is_public");
        let p = storage
            .get_universe("plain-notes")
            .expect("content registers");
        assert_eq!(p.name, "plain-notes", "name falls back to folder name");
        assert_eq!(p.visibility, "private", "no manifest → default private");
        assert!(
            storage.get_universe("target").is_none(),
            "build dir skipped"
        );
        assert!(
            storage.get_universe("not-a-universe").is_none(),
            "empty skipped"
        );
        assert!(
            storage.get_universe("some-rust-tool").is_none(),
            "pure code repo skipped (CO-467)"
        );

        // Idempotent: a second scan registers nothing new.
        assert_eq!(storage.register_universes_from_local_dir(ws.path()), 0);

        // CO-467: editing the manifest's visibility + re-scanning reconciles the
        // existing row (INSERT OR IGNORE alone never would).
        std::fs::write(
            uni.join("_universe.yaml"),
            "schema_version: 1\nname: My Universe\nvisibility: private\n",
        )
        .unwrap();
        storage.register_universes_from_local_dir(ws.path());
        let u2 = storage.get_universe("my-universe").expect("still there");
        assert_eq!(u2.visibility, "private", "re-scan reconciles visibility");
        assert!(!u2.is_public, "reconciled to non-public");
    }

    #[test]
    fn test_resolve_entry_rel_co394() {
        // sibling target → joined onto the from-entry's directory
        assert_eq!(
            resolve_entry_rel("content/documentos-fundacao/_index.md", "01-capa.md"),
            "content/documentos-fundacao/01-capa.md"
        );
        // subdir target from the universe root
        assert_eq!(
            resolve_entry_rel("content/index.md", "documentos-fundacao/_index.md"),
            "content/documentos-fundacao/_index.md"
        );
        // parent traversal normalizes `..`
        assert_eq!(
            resolve_entry_rel("content/documentos-fundacao/_index.md", "../index.md"),
            "content/index.md"
        );
        // `./` is a no-op
        assert_eq!(
            resolve_entry_rel("content/index.md", "./samba.md"),
            "content/samba.md"
        );
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

    /// CO-346: project must exist even when source dir is absent so the kanban
    /// board never renders empty.
    #[test]
    fn test_seed_co_universe_tasks_creates_project_without_source_dir() {
        let data_dir = tempfile::tempdir().unwrap();
        let missing_dir = tempfile::tempdir().unwrap();
        let missing_path = missing_dir.path().join("nonexistent");

        let mut storage = Storage::new(data_dir.path().to_str().unwrap());
        storage.seed_admin_content_universes();
        // Source dir does not exist — project should still be created.
        storage.seed_co_universe_tasks(&missing_path);

        let project = storage.get_project("CO");
        assert!(
            project.is_some(),
            "CO project must exist even when source dir is absent"
        );
        // No task files → zero tasks, but that is expected.
        let tasks = storage.list_tasks("CO");
        assert_eq!(tasks.len(), 0, "no task files → zero tasks");
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

    // CO-269: reseed_co_root_files seeds LICENSE (no extension) as LICENSE.md
    #[test]
    fn test_reseed_co_root_files_seeds_license() {
        let data_dir = tempfile::tempdir().unwrap();
        let root_dir = tempfile::tempdir().unwrap();

        // The real repo has `LICENSE` without extension — write the bare name.
        std::fs::write(
            root_dir.path().join("LICENSE"),
            "GNU AFFERO GENERAL PUBLIC LICENSE\nVersion 3, 19 November 2007\n",
        )
        .unwrap();

        let mut storage = Storage::new(data_dir.path().to_str().unwrap());
        storage.seed_admin_content_universes();
        storage.reseed_co_root_files(root_dir.path());

        let co_uc = storage.universe_pool.get_or_open("co");
        let uc_guard = co_uc.lock().unwrap();
        let idx = crate::entry_index::EntryIndex::new(&uc_guard);

        let license = idx.get("co", "LICENSE.md").unwrap();
        assert!(
            license.is_some(),
            "LICENSE entry must be seeded as LICENSE.md"
        );
        let license = license.unwrap();
        assert_eq!(license.entry_type, "page");
    }

    // CO-347: four content universes seeded with remote sync config.
    #[test]
    fn test_seed_admin_content_universes_co347_rows_exist() {
        let data_dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(data_dir.path().to_str().unwrap());
        storage.seed_admin_content_universes();

        for key in ["yuri", "retro-umarizal", "yoruba", "neuro"] {
            let u = storage.get_universe(key);
            assert!(u.is_some(), "universe '{key}' must exist after seed");
            let u = u.unwrap();
            assert_eq!(u.visibility, "public-subscribable", "{key}.visibility");
        }
    }

    #[test]
    fn test_seed_admin_content_universes_co347_parent_keys() {
        let data_dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(data_dir.path().to_str().unwrap());
        storage.seed_admin_content_universes();

        let yoruba = storage.get_universe("yoruba").unwrap();
        assert_eq!(
            yoruba.parent_key.as_deref(),
            Some("comunicacao"),
            "yoruba.parent_key must be 'comunicacao'"
        );

        let neuro = storage.get_universe("neuro").unwrap();
        assert_eq!(
            neuro.parent_key.as_deref(),
            Some("artelonga"),
            "neuro.parent_key must be 'artelonga'"
        );
    }

    #[test]
    fn test_seed_admin_content_universes_co347_remote_sync_fields() {
        let data_dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(data_dir.path().to_str().unwrap());
        storage.seed_admin_content_universes();

        struct RemoteRow {
            remote_url: Option<String>,
            remote_ref: Option<String>,
            content_subdirs: Option<String>,
            anon_published_only: i64,
        }

        let conn = storage.conn();
        let fetch = |key: &str| -> RemoteRow {
            conn.query_row(
                "SELECT remote_url, remote_ref, content_subdirs, anon_published_only \
                 FROM universes WHERE key = ?1",
                rusqlite::params![key],
                |row| {
                    Ok(RemoteRow {
                        remote_url: row.get(0)?,
                        remote_ref: row.get(1)?,
                        content_subdirs: row.get(2)?,
                        anon_published_only: row.get(3)?,
                    })
                },
            )
            .expect("universe row must exist")
        };

        let yuri = fetch("yuri");
        assert_eq!(
            yuri.remote_url.as_deref(),
            Some("https://github.com/artelonga/artelonga"),
            "yuri.remote_url"
        );
        assert_eq!(yuri.remote_ref.as_deref(), Some("main"), "yuri.remote_ref");
        assert_eq!(
            yuri.content_subdirs.as_deref(),
            Some("[\"yuri\"]"),
            "yuri.content_subdirs"
        );
        assert_eq!(
            yuri.anon_published_only, 1,
            "yuri.anon_published_only must be 1"
        );

        let retro = fetch("retro-umarizal");
        assert_eq!(
            retro.remote_url.as_deref(),
            Some("https://github.com/artelonga/retro-umarizal"),
            "retro-umarizal.remote_url"
        );
        assert_eq!(retro.remote_ref.as_deref(), Some("main"));
        assert_eq!(
            retro.anon_published_only, 0,
            "retro-umarizal.anon_published_only must be 0"
        );

        let yoruba = fetch("yoruba");
        assert_eq!(
            yoruba.remote_url.as_deref(),
            Some("https://github.com/artelonga/comunicacao"),
            "yoruba.remote_url"
        );
        assert_eq!(
            yoruba.content_subdirs.as_deref(),
            Some("[\"yoruba\"]"),
            "yoruba.content_subdirs"
        );
        assert_eq!(yoruba.anon_published_only, 0);

        let neuro = fetch("neuro");
        assert_eq!(
            neuro.remote_url.as_deref(),
            Some("https://github.com/artelonga/artelonga"),
            "neuro.remote_url"
        );
        assert_eq!(
            neuro.content_subdirs.as_deref(),
            Some("[\"neuro\"]"),
            "neuro.content_subdirs"
        );
        assert_eq!(neuro.anon_published_only, 0);
    }

    #[test]
    fn test_seed_admin_content_universes_co347_idempotent() {
        let data_dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(data_dir.path().to_str().unwrap());
        storage.seed_admin_content_universes();

        // Simulate operator setting a custom remote_url for yuri.
        storage
            .conn()
            .execute(
                "UPDATE universes SET remote_url = 'https://github.com/custom/repo' WHERE key = 'yuri'",
                [],
            )
            .unwrap();

        // Second boot — must not overwrite operator-set remote_url.
        storage.seed_admin_content_universes();

        let url: String = storage
            .conn()
            .query_row(
                "SELECT remote_url FROM universes WHERE key = 'yuri'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            url, "https://github.com/custom/repo",
            "operator-set remote_url must not be overwritten on re-seed"
        );
    }

    // CO-379: staging fixture universe seeding
    #[test]
    fn test_seed_staging_fixture_universes_creates_all_fixtures() {
        let data_dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(data_dir.path().to_str().unwrap());
        storage.seed_staging_fixture_universes();

        for key in [
            "recursion-a",
            "recursion-a-b",
            "recursion-a-b-c",
            "funnel-fixture",
            "mbya-staging",
            "yoruba-staging",
        ] {
            let u = storage.get_universe(key);
            assert!(
                u.is_some(),
                "fixture universe '{key}' must exist after seed"
            );
        }
    }

    #[test]
    fn test_seed_staging_fixture_universes_parent_keys() {
        let data_dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(data_dir.path().to_str().unwrap());
        storage.seed_staging_fixture_universes();

        let ab: Option<String> = storage
            .conn()
            .query_row(
                "SELECT parent_key FROM universes WHERE key = 'recursion-a-b'",
                [],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        assert_eq!(
            ab.as_deref(),
            Some("recursion-a"),
            "recursion-a-b.parent_key"
        );

        let abc: Option<String> = storage
            .conn()
            .query_row(
                "SELECT parent_key FROM universes WHERE key = 'recursion-a-b-c'",
                [],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        assert_eq!(
            abc.as_deref(),
            Some("recursion-a-b"),
            "recursion-a-b-c.parent_key"
        );
    }

    /// CO-401: synthetic funnel fixtures seed once, are flagged `is_synthetic`,
    /// and re-running is a no-op (no duplicates).
    #[test]
    fn test_seed_staging_funnel_fixtures_idempotent_and_flagged() {
        let data_dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(data_dir.path().to_str().unwrap());

        storage.seed_staging_funnel_fixtures();
        storage.seed_staging_funnel_fixtures(); // second run — must not duplicate

        let synthetic: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM leads WHERE is_synthetic = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(synthetic, 5, "exactly 5 synthetic leads after re-seed");

        let real: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM leads WHERE is_synthetic = 0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(real, 0, "fixtures must never masquerade as real leads");
    }

    /// CO-401: the staging-admin user is admin-tier and idempotent.
    #[test]
    fn test_ensure_staging_admin_user_is_admin_and_idempotent() {
        let data_dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(data_dir.path().to_str().unwrap());

        let id1 = storage.ensure_staging_admin_user().unwrap();
        let id2 = storage.ensure_staging_admin_user().unwrap();
        assert_eq!(id1, id2, "stable user id across calls");

        let (tier, count): (String, i64) = storage
            .conn()
            .query_row(
                "SELECT tier, (SELECT COUNT(*) FROM users WHERE id = ?1) \
                 FROM users WHERE id = ?1",
                rusqlite::params![id1],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(tier, "admin", "staging-admin must be admin tier");
        assert_eq!(count, 1, "idempotent: exactly one staging-admin row");
    }

    #[test]
    fn test_seed_staging_fixture_universes_is_idempotent() {
        let data_dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(data_dir.path().to_str().unwrap());
        storage.seed_staging_fixture_universes();
        storage.seed_staging_fixture_universes(); // second run

        let count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM universes WHERE key LIKE 'recursion-a%' OR key IN ('funnel-fixture','mbya-staging','yoruba-staging')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 6, "idempotent: exactly 6 fixture rows");
    }

    // CO-379: test namespace sweep
    #[test]
    fn test_sweep_test_namespaces_deletes_old_universes() {
        let data_dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(data_dir.path().to_str().unwrap());

        let old_date = "2020-01-01T00:00:00+00:00";
        storage
            .conn()
            .execute(
                "INSERT INTO universes (key, name, owner_id, created_at, visibility) \
                 VALUES ('u-test-old-abc123', 'Old Test', 'system', ?1, 'private')",
                rusqlite::params![old_date],
            )
            .unwrap();

        // keep_count=0 → delete all eligible (no forensic floor).
        let n = storage.sweep_test_namespaces(7, 0);
        assert_eq!(n, 1, "one old universe should be swept");

        let u = storage.get_universe("u-test-old-abc123");
        assert!(u.is_none(), "swept universe must be gone");
    }

    #[test]
    fn test_sweep_test_namespaces_keeps_recent_universes() {
        let data_dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(data_dir.path().to_str().unwrap());

        let recent_date = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO universes (key, name, owner_id, created_at, visibility) \
                 VALUES ('u-test-new-abc123', 'New Test', 'system', ?1, 'private')",
                rusqlite::params![recent_date],
            )
            .unwrap();

        // Recent universe is newer than 7-day cutoff — must not be swept.
        let n = storage.sweep_test_namespaces(7, 0);
        assert_eq!(n, 0, "recent universe must not be swept");

        let u = storage.get_universe("u-test-new-abc123");
        assert!(u.is_some(), "recent universe must still exist");
    }

    #[test]
    fn test_sweep_test_namespaces_respects_keep_count() {
        let data_dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(data_dir.path().to_str().unwrap());

        let old_date = "2020-01-01T00:00:00+00:00";
        // Insert 3 old u-test-* universes.
        for i in 0..3 {
            storage
                .conn()
                .execute(
                    "INSERT INTO universes (key, name, owner_id, created_at, visibility) \
                     VALUES (?1, 'Test', 'system', ?2, 'private')",
                    rusqlite::params![format!("u-test-keep-{i}"), old_date],
                )
                .unwrap();
        }

        // keep_count=2 → 3 old − 2 kept = 1 deleted.
        let n = storage.sweep_test_namespaces(7, 2);
        assert_eq!(n, 1, "keep_count=2 should leave 2 and delete 1");
    }

    // CO-429: claude-code universe metadata
    #[test]
    fn test_seed_co429_claude_code_metadata() {
        let data_dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(data_dir.path().to_str().unwrap());
        storage.seed_admin_content_universes();

        // CO-438 (Bug 1): claude-code is NO LONGER seeded. Seeding a private row
        // owned by the sentinel 'system' (no real owner) made it an orphan —
        // POST 409 "key taken" but GET 404. Importable private universes are now
        // created by `co source add` as the importing user. A fresh DB has no
        // claude-code row until then.
        assert!(
            storage.get_universe("claude-code").is_none(),
            "claude-code must not be seeded (would orphan: private + owner=system)"
        );
    }

    // CO-429: reconcile UPDATE corrects existing installs that were seeded by CO-364
    // with wrong visibility/parent. Simulates a fresh install with old values.
    #[test]
    fn test_seed_co429_claude_code_reconcile_existing() {
        let data_dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(data_dir.path().to_str().unwrap());

        // Simulate CO-364 state: claude-code exists with public-subscribable + no parent
        storage
            .conn()
            .execute(
                "INSERT INTO universes \
                 (key, name, owner_id, created_at, visibility, is_public) \
                 VALUES ('claude-code', 'Claude Code', 'system', '2026-01-01', \
                         'public-subscribable', 1)",
                [],
            )
            .unwrap();
        storage.conn().execute(
            "UPDATE universes SET content_subdirs = '[\"docs\",\"README.md\",\"CHANGELOG.md\"]' \
             WHERE key = 'claude-code'",
            [],
        ).unwrap();

        // Run seed — reconcile UPDATEs must tidy the stale row.
        storage.seed_admin_content_universes();

        let u = storage.get_universe("claude-code").unwrap();
        // CO-438 (Bug 1): a *system*-owned row must NOT be flipped to private —
        // that flip (private + no real owner) is exactly what orphaned it. It
        // stays public-subscribable, i.e. still GET-able, never "taken but 404".
        assert_eq!(
            u.visibility, "public-subscribable",
            "system-owned row must stay public-subscribable, not be orphaned to private"
        );
        // Owner-agnostic reconciles still apply: parent grouping …
        assert_eq!(
            u.parent_key.as_deref(),
            Some("co"),
            "reconcile must set parent_key=co"
        );
        // … and the stale-subdirs fix.
        let subdirs: Option<String> = storage
            .conn()
            .query_row(
                "SELECT content_subdirs FROM universes WHERE key = 'claude-code'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let subdirs = subdirs.unwrap_or_default();
        assert!(
            !subdirs.contains("\"docs\""),
            "reconcile must fix stale docs/ subdirs, got: {subdirs}"
        );
    }

    /// CO-438 (Bug 1): a claude-code row with a *real* owner IS privatized by
    /// the reconcile (the owner keeps access), confirming the guard targets only
    /// the system sentinel rather than disabling the flip entirely.
    #[test]
    fn test_seed_co438_real_owner_row_is_privatized() {
        let data_dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(data_dir.path().to_str().unwrap());
        storage
            .conn()
            .execute(
                "INSERT INTO universes \
                 (key, name, owner_id, created_at, visibility, is_public) \
                 VALUES ('claude-code', 'Claude Code', 'usr_yuri', '2026-01-01', \
                         'public-subscribable', 1)",
                [],
            )
            .unwrap();

        storage.seed_admin_content_universes();

        assert_eq!(
            storage.get_universe("claude-code").unwrap().visibility,
            "private",
            "a real-owner row must still be privatized by the reconcile"
        );
    }
}
