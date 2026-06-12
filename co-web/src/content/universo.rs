//! Filesystem implementation of the `Universo` domain (CO-431).
//!
//! The `Universo` trait and its domain types (`Tarefa`, `Nota`, `Evento`,
//! `Membro`, `Relato`, `Conteudo`, `Entrada`) live in `core::universo` so
//! external services can depend on `core` alone. This module re-exports them
//! for compatibility and keeps the disk-backed implementation
//! (`UniversoLocal`) plus the default [`UniversoFactory`] used by AppState.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub use co::universo::*;

// ---- Filesystem implementation ----

pub struct UniversoLocal {
    pub caminho: PathBuf,
    pub config: UniversoConfig,
}

impl UniversoLocal {
    pub fn abrir(caminho: &Path) -> Result<Self, String> {
        let config_path = caminho.join(".universo.yaml");
        let config_str = fs::read_to_string(&config_path)
            .map_err(|e| format!("Erro ao ler .universo.yaml em {}: {}", caminho.display(), e))?;
        let config: UniversoConfig = serde_yaml::from_str(&config_str)
            .map_err(|e| format!("Erro ao parsear .universo.yaml: {}", e))?;
        Ok(Self {
            caminho: caminho.to_path_buf(),
            config,
        })
    }
}

/// Default `UniversoFactory`: opens `root.join(key)` from disk.
pub struct UniversoLocalFactory;

impl UniversoFactory for UniversoLocalFactory {
    fn abrir(&self, key: &str, root: &Path) -> Result<Box<dyn Universo>, String> {
        Ok(Box::new(UniversoLocal::abrir(&root.join(key))?))
    }
}

// ---- Frontmatter parsing ----

fn parse_frontmatter(content: &str) -> (HashMap<String, String>, String) {
    let mut frontmatter = HashMap::new();
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (frontmatter, content.to_string());
    }
    let after_opening = &trimmed[3..];
    if let Some(end_idx) = after_opening.find("\n---") {
        let fm_block = &after_opening[..end_idx];
        for line in fm_block.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim().to_string();
                let value = value.trim().to_string();
                frontmatter.insert(key, value);
            }
        }
        let body_start = 3 + end_idx + 4;
        let body = if body_start < trimmed.len() {
            trimmed[body_start..].trim_start_matches('\n').to_string()
        } else {
            String::new()
        };
        (frontmatter, body)
    } else {
        (frontmatter, content.to_string())
    }
}

fn parse_tags(value: &str) -> Vec<String> {
    let v = value.trim();
    let v = v.strip_prefix('[').unwrap_or(v);
    let v = v.strip_suffix(']').unwrap_or(v);
    v.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

fn collect_md_files(dir: &Path, base: &Path) -> Vec<(PathBuf, String)> {
    let mut results = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return results,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            results.extend(collect_md_files(&path, base));
        } else if path.extension().is_some_and(|e| e == "md") {
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            results.push((path, rel));
        }
    }
    results
}

/// Read markdown files from a specific subdirectory.
fn read_dir_md(base: &Path, subdir: &str) -> Vec<(String, HashMap<String, String>, String)> {
    let dir = base.join(subdir);
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut results = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let id = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let (fm, body) = parse_frontmatter(&content);
        results.push((id, fm, body));
    }
    results
}

impl Universo for UniversoLocal {
    fn nome(&self) -> &str {
        &self.config.nome
    }

    fn descricao(&self) -> &str {
        &self.config.descricao
    }

    fn quadro(&self) -> Result<Vec<Tarefa>, String> {
        let files = collect_md_files(&self.caminho, &self.caminho);
        let mut tarefas = Vec::new();
        for (path, rel) in files {
            let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let (fm, body) = parse_frontmatter(&content);
            if let Some(status) = fm.get("status") {
                let id = rel.trim_end_matches(".md").replace('/', "-");
                let titulo = fm
                    .get("titulo")
                    .or_else(|| fm.get("title"))
                    .cloned()
                    .unwrap_or_else(|| id.clone());
                let tags = fm
                    .get("tags")
                    .or_else(|| fm.get("etiquetas"))
                    .map(|t| parse_tags(t))
                    .unwrap_or_default();
                let data = fm
                    .get("data")
                    .or_else(|| fm.get("date"))
                    .or_else(|| fm.get("criado"))
                    .cloned()
                    .unwrap_or_default();
                tarefas.push(Tarefa {
                    id,
                    titulo,
                    status: status.clone(),
                    tags,
                    data,
                    conteudo: body,
                });
            }
        }
        Ok(tarefas)
    }

    fn jardim(&self) -> Result<Vec<Nota>, String> {
        let files = collect_md_files(&self.caminho, &self.caminho);
        let mut notas = Vec::new();
        for (path, rel) in files {
            let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let (fm, body) = parse_frontmatter(&content);
            if fm.contains_key("status") {
                continue;
            }
            // Skip typed content that has its own view
            let tipo = fm.get("type").cloned().unwrap_or_default();
            if matches!(tipo.as_str(), "evento" | "membro" | "relato") {
                continue;
            }
            let id = rel.trim_end_matches(".md").replace('/', "-");
            let titulo = fm
                .get("titulo")
                .or_else(|| fm.get("title"))
                .cloned()
                .unwrap_or_else(|| id.clone());
            let tags = fm.get("tags").map(|t| parse_tags(t)).unwrap_or_default();
            let data = fm
                .get("data")
                .or_else(|| fm.get("date"))
                .cloned()
                .unwrap_or_default();
            let preview = body
                .chars()
                .take(140)
                .collect::<String>()
                .replace(['#', '*', '`', '[', ']'], "")
                .trim()
                .to_string();
            notas.push(Nota {
                id,
                titulo,
                tags,
                data,
                preview,
                conteudo: body,
            });
        }
        Ok(notas)
    }

    fn eventos(&self) -> Result<Vec<Evento>, String> {
        let mut eventos: Vec<Evento> = read_dir_md(&self.caminho, "eventos")
            .into_iter()
            .map(|(id, fm, body)| {
                let titulo = fm.get("titulo").cloned().unwrap_or_else(|| id.clone());
                let data = fm.get("data").cloned().unwrap_or_default();
                let hora = fm.get("hora").cloned().unwrap_or_default();
                let local = fm.get("local").cloned().unwrap_or_default();
                let tags = fm.get("tags").map(|t| parse_tags(t)).unwrap_or_default();
                Evento {
                    id,
                    titulo,
                    data,
                    hora,
                    local,
                    tags,
                    conteudo: body,
                }
            })
            .collect();
        eventos.sort_by(|a, b| a.data.cmp(&b.data).then(a.hora.cmp(&b.hora)));
        Ok(eventos)
    }

    fn membros(&self) -> Result<Vec<Membro>, String> {
        let membros: Vec<Membro> = read_dir_md(&self.caminho, "membros")
            .into_iter()
            .map(|(id, fm, body)| {
                let nome = fm.get("nome").cloned().unwrap_or_else(|| id.clone());
                let papel = fm.get("papel").cloned().unwrap_or_else(|| "membro".into());
                let bio = fm.get("bio").cloned().unwrap_or_default();
                let foto_url = fm.get("foto_url").cloned().unwrap_or_default();
                let tags = fm.get("tags").map(|t| parse_tags(t)).unwrap_or_default();
                Membro {
                    id,
                    nome,
                    papel,
                    bio,
                    foto_url,
                    tags,
                    conteudo: body,
                }
            })
            .collect();
        Ok(membros)
    }

    fn relatos(&self) -> Result<Vec<Relato>, String> {
        let mut pubs: Vec<Relato> = read_dir_md(&self.caminho, "relatos")
            .into_iter()
            .map(|(id, fm, body)| {
                let titulo = fm.get("titulo").cloned().unwrap_or_else(|| id.clone());
                let descricao = fm.get("descricao").cloned().unwrap_or_default();
                let data = fm.get("data").cloned().unwrap_or_default();
                let slug = fm.get("slug").cloned().unwrap_or_else(|| id.clone());
                let tags = fm.get("tags").map(|t| parse_tags(t)).unwrap_or_default();
                let publicado = fm.get("publicado").map(|v| v == "true").unwrap_or(false);
                Relato {
                    id,
                    titulo,
                    descricao,
                    data,
                    slug,
                    tags,
                    publicado,
                    conteudo: body,
                }
            })
            .collect();
        pubs.sort_by(|a, b| b.data.cmp(&a.data));
        Ok(pubs)
    }

    fn conteudo(&self, caminho: &str) -> Result<Conteudo, String> {
        let file_path = if caminho.ends_with(".md") {
            self.caminho.join(caminho)
        } else {
            self.caminho.join(format!("{caminho}.md"))
        };
        if !file_path.exists() {
            return Err(format!("Conteudo nao encontrado: {caminho}"));
        }
        let content = fs::read_to_string(&file_path).map_err(|e| e.to_string())?;
        let (fm, body) = parse_frontmatter(&content);
        let titulo = fm
            .get("titulo")
            .or_else(|| fm.get("title"))
            .cloned()
            .unwrap_or_default();
        Ok(Conteudo {
            caminho: caminho.to_string(),
            titulo,
            corpo: body,
            frontmatter: fm,
        })
    }

    fn arvore(&self) -> Result<Vec<Entrada>, String> {
        fn build_tree(dir: &Path, base: &Path) -> Result<Vec<Entrada>, String> {
            let mut entries = Vec::new();
            let read = fs::read_dir(dir).map_err(|e| e.to_string())?;
            for entry in read.flatten() {
                let path = entry.path();
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if name.starts_with('.') {
                    continue;
                }
                let rel = path
                    .strip_prefix(base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();
                if path.is_dir() {
                    let filhos = build_tree(&path, base)?;
                    entries.push(Entrada {
                        nome: name,
                        caminho: rel,
                        tipo: "diretorio".to_string(),
                        filhos,
                    });
                } else {
                    entries.push(Entrada {
                        nome: name,
                        caminho: rel,
                        tipo: "arquivo".to_string(),
                        filhos: Vec::new(),
                    });
                }
            }
            entries.sort_by(|a, b| a.nome.cmp(&b.nome));
            Ok(entries)
        }
        build_tree(&self.caminho, &self.caminho)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn criar_universo_teste() -> (tempfile::TempDir, UniversoLocal) {
        let dir = tempdir().unwrap();

        fs::write(
            dir.path().join(".universo.yaml"),
            "nome: teste\ndescricao: Universo de teste\ntipo: comunidade\nlocale: pt-BR\n",
        )
        .unwrap();

        // quadro/ - tasks with status
        fs::create_dir_all(dir.path().join("quadro")).unwrap();
        fs::write(
            dir.path().join("quadro/tarefa-alpha.md"),
            "---\ntitulo: Tarefa Alpha\nstatus: todo\netiquetas: [feat]\ncriado: 2026-03-20\n---\nFazer a coisa alpha.\n",
        )
        .unwrap();

        // jardim/ - notes without status
        fs::create_dir_all(dir.path().join("jardim")).unwrap();
        fs::write(
            dir.path().join("jardim/sobre.md"),
            "---\ntype: pagina\ntitulo: Sobre\ntags: [info]\n---\nSobre o projeto.\n",
        )
        .unwrap();

        // eventos/
        fs::create_dir_all(dir.path().join("eventos")).unwrap();
        fs::write(
            dir.path().join("eventos/oficina.md"),
            "---\ntype: evento\ntitulo: Oficina\ndata: 2026-04-01\nhora: 10:00\nlocal: Sede\ntags: [ambiental]\n---\nDescricao.\n",
        )
        .unwrap();

        // membros/
        fs::create_dir_all(dir.path().join("membros")).unwrap();
        fs::write(
            dir.path().join("membros/ana.md"),
            "---\ntype: membro\nnome: Ana\npapel: admin\ntags: [fundador]\n---\n",
        )
        .unwrap();

        // publicacoes/
        fs::create_dir_all(dir.path().join("relatos")).unwrap();
        fs::write(
            dir.path().join("relatos/post-um.md"),
            "---\ntype: relato\ntitulo: Post Um\ndata: 2026-03-28\npublicado: true\nslug: post-um\ntags: [blog]\n---\nConteudo do post.\n",
        )
        .unwrap();

        let universo = UniversoLocal::abrir(dir.path()).unwrap();
        (dir, universo)
    }

    #[test]
    fn test_abrir_universo() {
        let (_dir, u) = criar_universo_teste();
        assert_eq!(u.nome(), "teste");
        assert_eq!(u.descricao(), "Universo de teste");
    }

    #[test]
    fn test_quadro_retorna_tarefas() {
        let (_dir, u) = criar_universo_teste();
        let tarefas = u.quadro().unwrap();
        assert_eq!(tarefas.len(), 1);
        assert_eq!(tarefas[0].titulo, "Tarefa Alpha");
        assert_eq!(tarefas[0].status, "todo");
    }

    #[test]
    fn test_jardim_exclui_tipos_especificos() {
        let (_dir, u) = criar_universo_teste();
        let notas = u.jardim().unwrap();
        // Should include pagina (sobre.md) but NOT evento, membro, publicacao
        let ids: Vec<&str> = notas.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"jardim-sobre"));
        assert!(!ids.iter().any(|id| id.contains("oficina")));
        assert!(!ids.iter().any(|id| id.contains("ana")));
        assert!(!ids.iter().any(|id| id.contains("post-um")));
    }

    #[test]
    fn test_eventos() {
        let (_dir, u) = criar_universo_teste();
        let eventos = u.eventos().unwrap();
        assert_eq!(eventos.len(), 1);
        assert_eq!(eventos[0].titulo, "Oficina");
        assert_eq!(eventos[0].hora, "10:00");
    }

    #[test]
    fn test_membros() {
        let (_dir, u) = criar_universo_teste();
        let membros = u.membros().unwrap();
        assert_eq!(membros.len(), 1);
        assert_eq!(membros[0].nome, "Ana");
        assert_eq!(membros[0].papel, "admin");
    }

    #[test]
    fn test_relatos() {
        let (_dir, u) = criar_universo_teste();
        let pubs = u.relatos().unwrap();
        assert_eq!(pubs.len(), 1);
        assert_eq!(pubs[0].titulo, "Post Um");
        assert!(pubs[0].publicado);
    }

    #[test]
    fn test_conteudo() {
        let (_dir, u) = criar_universo_teste();
        let c = u.conteudo("jardim/sobre").unwrap();
        assert_eq!(c.titulo, "Sobre");
        assert!(c.corpo.contains("Sobre o projeto"));
    }

    #[test]
    fn test_arvore() {
        let (_dir, u) = criar_universo_teste();
        let arvore = u.arvore().unwrap();
        let nomes: Vec<&str> = arvore.iter().map(|e| e.nome.as_str()).collect();
        assert!(nomes.contains(&"quadro"));
        assert!(nomes.contains(&"eventos"));
        assert!(nomes.contains(&"membros"));
        assert!(!nomes.contains(&".universo.yaml"));
    }

    #[test]
    fn test_factory_local_abre_root_join_key() {
        let dir = tempdir().unwrap();
        let demo = dir.path().join("demo");
        fs::create_dir_all(&demo).unwrap();
        fs::write(
            demo.join(".universo.yaml"),
            "nome: demo\ndescricao: aberto pela factory\n",
        )
        .unwrap();

        let fabrica: Box<dyn UniversoFactory> = Box::new(UniversoLocalFactory);
        let u = fabrica.abrir("demo", dir.path()).unwrap();
        assert_eq!(u.nome(), "demo");
        assert_eq!(u.descricao(), "aberto pela factory");
    }

    #[test]
    fn test_factory_local_universo_inexistente_erra() {
        let dir = tempdir().unwrap();
        let err = match UniversoLocalFactory.abrir("nao-existe", dir.path()) {
            Ok(_) => panic!("abrir deveria falhar para universo inexistente"),
            Err(e) => e,
        };
        assert!(err.contains(".universo.yaml"));
    }
}

/// CO-431 swap proof: the same handler, mounted on the same route, serves a
/// filesystem universo with the default factory and an in-memory universo
/// when a fake `UniversoFactory` is plugged into AppState — no route or
/// handler edits between the two scenarios.
#[cfg(test)]
mod factory_swap_tests {
    use super::*;
    use crate::server::{
        AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState,
    };
    use axum::body::Body;
    use axum::extract::State;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::{Json, Router};
    use std::sync::{Arc, Mutex as StdMutex};
    use tempfile::tempdir;
    use tower::ServiceExt;

    /// Handler under test: obtains the universo through the AppState factory,
    /// never via `UniversoLocal::abrir` hardcoded.
    async fn quadro_handler(
        State(state): State<AppState>,
    ) -> Result<Json<Vec<Tarefa>>, (StatusCode, String)> {
        let root = PathBuf::from(&state.core.config.universo_dir);
        let universo = state
            .core
            .universo_factory
            .abrir("demo", &root)
            .map_err(|e| (StatusCode::NOT_FOUND, e))?;
        universo
            .quadro()
            .map(Json)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
    }

    fn rotas(state: AppState) -> Router {
        Router::new()
            .route("/universos/demo/quadro", get(quadro_handler))
            .with_state(state)
    }

    fn build_state(
        dir: &Path,
        universo_dir: &Path,
        factory: Option<Arc<dyn UniversoFactory>>,
    ) -> AppState {
        let config = crate::config::WebConfig {
            port: 3000,
            data_dir: dir.to_str().unwrap().to_string(),
            static_dir: "co-web/static".to_string(),
            default_variant: "a".to_string(),
            experiments: false,
            plugins_dir: "plugins".to_string(),
            game_db_path: None,
            universo_dir: universo_dir.to_str().unwrap().to_string(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "prod".into(),
            wae_api_key: None,
            wae_endpoint: None,
            cookie_domain: None,
            quilombo_legacy_login: true,
            bypass_rate_limit: false,
        };
        let storage = crate::storage::Storage::new(&config.data_dir);
        let experiment = crate::experiment::ExperimentStore::new(&config.data_dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_storage = Arc::new(
            game_core::storage::Storage::open(&dir.join("game_test.db"))
                .expect("Failed to open test game storage"),
        );
        let (embedding_tx, _embedding_rx) = crate::embedding_worker::channel();
        let mut core_state = CoreState::from_storage(storage, config, auth_store);
        if let Some(factory) = factory {
            core_state = core_state.with_universo_factory(factory);
        }
        AppState::new(AppStateInner {
            core: Arc::new(core_state),
            realtime: Arc::new(RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: StdMutex::new(std::collections::HashMap::new()),
                chat_presence: StdMutex::new(std::collections::HashMap::new()),
            }),
            index: Arc::new(IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx,
            }),
            integrations: Arc::new(IntegrationsState {
                mail,
                geo: Arc::new(crate::geo::GeoDb::disabled()),
                plugin_registry: game_core::plugin::PluginRegistry::new(),
                game_storage,
                wae: crate::wae::WaeEmitter::new(None, None),
                jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
                rate_limiter: StdMutex::new(crate::rate_limit::RateLimiter::new()),
                experiment: StdMutex::new(experiment),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        })
    }

    async fn get_quadro(app: Router) -> (StatusCode, Vec<Tarefa>) {
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/universos/demo/quadro")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let tarefas = serde_json::from_slice(&body).unwrap_or_default();
        (status, tarefas)
    }

    #[tokio::test]
    async fn default_factory_serve_universo_de_filesystem() {
        let dir = tempdir().unwrap();
        let universos = dir.path().join("universos");
        let demo = universos.join("demo");
        fs::create_dir_all(demo.join("quadro")).unwrap();
        fs::write(demo.join(".universo.yaml"), "nome: demo\n").unwrap();
        fs::write(
            demo.join("quadro/do-disco.md"),
            "---\ntitulo: Tarefa do disco\nstatus: todo\n---\nCorpo.\n",
        )
        .unwrap();

        let state = build_state(dir.path(), &universos, None);
        let (status, tarefas) = get_quadro(rotas(state)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(tarefas.len(), 1);
        assert_eq!(tarefas[0].titulo, "Tarefa do disco");
    }

    struct UniversoMemoria;

    impl Universo for UniversoMemoria {
        fn nome(&self) -> &str {
            "memoria"
        }
        fn descricao(&self) -> &str {
            "sem filesystem"
        }
        fn quadro(&self) -> Result<Vec<Tarefa>, String> {
            Ok(vec![Tarefa {
                id: "mem-1".into(),
                titulo: "Tarefa da memoria".into(),
                status: "doing".into(),
                tags: vec![],
                data: String::new(),
                conteudo: String::new(),
            }])
        }
        fn jardim(&self) -> Result<Vec<Nota>, String> {
            Ok(vec![])
        }
        fn eventos(&self) -> Result<Vec<Evento>, String> {
            Ok(vec![])
        }
        fn membros(&self) -> Result<Vec<Membro>, String> {
            Ok(vec![])
        }
        fn relatos(&self) -> Result<Vec<Relato>, String> {
            Ok(vec![])
        }
        fn conteudo(&self, caminho: &str) -> Result<Conteudo, String> {
            Err(format!("Conteudo nao encontrado: {caminho}"))
        }
        fn arvore(&self) -> Result<Vec<Entrada>, String> {
            Ok(vec![])
        }
    }

    struct FabricaMemoria;

    impl UniversoFactory for FabricaMemoria {
        fn abrir(&self, _key: &str, _root: &Path) -> Result<Box<dyn Universo>, String> {
            Ok(Box::new(UniversoMemoria))
        }
    }

    #[tokio::test]
    async fn fabrica_fake_serve_universo_nao_filesystem_pelos_mesmos_handlers() {
        let dir = tempdir().unwrap();
        // No universe on disk at all — only the in-memory factory.
        let universos = dir.path().join("universos");

        let state = build_state(dir.path(), &universos, Some(Arc::new(FabricaMemoria)));
        let (status, tarefas) = get_quadro(rotas(state)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(tarefas.len(), 1);
        assert_eq!(tarefas[0].titulo, "Tarefa da memoria");
        assert_eq!(tarefas[0].status, "doing");
    }
}
