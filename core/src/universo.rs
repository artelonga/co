//! Universo domain model (CO-431).
//!
//! A universo is a content workspace. This module holds the pure domain
//! types and the `Universo` trait so external services (a universe runtime,
//! Yggdrasil, a CLI) can depend on `core` alone — no server-framework,
//! database, or async-runtime dependencies — and provide their own backend.
//!
//! The trait provides standard views: quadro (tasks), jardim (notes),
//! eventos, membros, relatos, and raw content access. The filesystem
//! implementation (`UniversoLocal`) lives in co-web; alternative backends
//! plug in through [`UniversoFactory`].

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// ---- Domain types ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tarefa {
    pub id: String,
    pub titulo: String,
    pub status: String,
    pub tags: Vec<String>,
    pub data: String,
    pub conteudo: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Nota {
    pub id: String,
    pub titulo: String,
    pub tags: Vec<String>,
    pub data: String,
    pub preview: String,
    pub conteudo: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evento {
    pub id: String,
    pub titulo: String,
    pub data: String,
    pub hora: String,
    pub local: String,
    pub tags: Vec<String>,
    pub conteudo: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Membro {
    pub id: String,
    pub nome: String,
    pub papel: String,
    #[serde(default)]
    pub bio: String,
    #[serde(default)]
    pub foto_url: String,
    pub tags: Vec<String>,
    pub conteudo: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relato {
    pub id: String,
    pub titulo: String,
    pub descricao: String,
    pub data: String,
    pub slug: String,
    pub tags: Vec<String>,
    pub publicado: bool,
    pub conteudo: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conteudo {
    pub caminho: String,
    pub titulo: String,
    pub corpo: String,
    pub frontmatter: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entrada {
    pub nome: String,
    pub caminho: String,
    pub tipo: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filhos: Vec<Entrada>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversoInfo {
    pub nome: String,
    pub descricao: String,
    pub caminho: String,
}

// ---- Config ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversoConfig {
    pub nome: String,
    #[serde(default)]
    pub descricao: String,
    #[serde(default)]
    pub tipo: String,
    #[serde(default)]
    pub locale: String,
}

// ---- Trait ----

pub trait Universo: Send + Sync {
    fn nome(&self) -> &str;
    fn descricao(&self) -> &str;
    fn quadro(&self) -> Result<Vec<Tarefa>, String>;
    fn jardim(&self) -> Result<Vec<Nota>, String>;
    fn eventos(&self) -> Result<Vec<Evento>, String>;
    fn membros(&self) -> Result<Vec<Membro>, String>;
    fn relatos(&self) -> Result<Vec<Relato>, String>;
    fn conteudo(&self, caminho: &str) -> Result<Conteudo, String>;
    fn arvore(&self) -> Result<Vec<Entrada>, String>;

    /// Get content with locale fallback.
    ///
    /// Tries `{caminho}.{locale}.md` first, then falls back to `{caminho}.md`.
    /// For example, `conteudo_locale("sobre", "en")` tries `sobre.en.md`,
    /// then `sobre.md`.
    ///
    /// Default implementation delegates to `conteudo()` with fallback logic.
    fn conteudo_locale(&self, caminho: &str, locale: &str) -> Result<Conteudo, String> {
        let localized = format!("{}.{}", caminho, locale);
        match self.conteudo(&localized) {
            Ok(c) => Ok(c),
            Err(_) => self.conteudo(caminho),
        }
    }
}

// ---- Factory seam ----

/// Opens universos by key, decoupling consumers from any concrete backend.
///
/// `key` identifies the universo; `root` is the directory the caller resolves
/// it under (backends that are not filesystem-based may ignore it). The
/// default implementation injected into the co-web AppState opens
/// `root.join(key)` from disk; external services swap in their own backend
/// by implementing this trait — no co-web edits required.
pub trait UniversoFactory: Send + Sync {
    fn abrir(&self, key: &str, root: &Path) -> Result<Box<dyn Universo>, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal in-memory backend — doubles as the reference for external
    /// implementors: no filesystem, no server, just the trait.
    struct UniversoMemoria {
        conteudos: HashMap<String, Conteudo>,
    }

    impl Universo for UniversoMemoria {
        fn nome(&self) -> &str {
            "memoria"
        }
        fn descricao(&self) -> &str {
            "universo em memoria"
        }
        fn quadro(&self) -> Result<Vec<Tarefa>, String> {
            Ok(Vec::new())
        }
        fn jardim(&self) -> Result<Vec<Nota>, String> {
            Ok(Vec::new())
        }
        fn eventos(&self) -> Result<Vec<Evento>, String> {
            Ok(Vec::new())
        }
        fn membros(&self) -> Result<Vec<Membro>, String> {
            Ok(Vec::new())
        }
        fn relatos(&self) -> Result<Vec<Relato>, String> {
            Ok(Vec::new())
        }
        fn conteudo(&self, caminho: &str) -> Result<Conteudo, String> {
            self.conteudos
                .get(caminho)
                .cloned()
                .ok_or_else(|| format!("Conteudo nao encontrado: {caminho}"))
        }
        fn arvore(&self) -> Result<Vec<Entrada>, String> {
            Ok(Vec::new())
        }
    }

    struct FabricaMemoria;

    impl UniversoFactory for FabricaMemoria {
        fn abrir(&self, _key: &str, _root: &Path) -> Result<Box<dyn Universo>, String> {
            let mut conteudos = HashMap::new();
            conteudos.insert(
                "sobre".to_string(),
                Conteudo {
                    caminho: "sobre".into(),
                    titulo: "Sobre".into(),
                    corpo: "pt".into(),
                    frontmatter: HashMap::new(),
                },
            );
            conteudos.insert(
                "sobre.en".to_string(),
                Conteudo {
                    caminho: "sobre.en".into(),
                    titulo: "About".into(),
                    corpo: "en".into(),
                    frontmatter: HashMap::new(),
                },
            );
            Ok(Box::new(UniversoMemoria { conteudos }))
        }
    }

    #[test]
    fn conteudo_locale_prefers_localized_then_falls_back() {
        let u = FabricaMemoria
            .abrir("demo", Path::new("/nao-usado"))
            .unwrap();
        assert_eq!(u.conteudo_locale("sobre", "en").unwrap().corpo, "en");
        // No `sobre.fr` — falls back to the base content.
        assert_eq!(u.conteudo_locale("sobre", "fr").unwrap().corpo, "pt");
    }

    #[test]
    fn factory_is_object_safe_and_boxable() {
        let fabrica: Box<dyn UniversoFactory> = Box::new(FabricaMemoria);
        let u = fabrica.abrir("demo", Path::new("/nao-usado")).unwrap();
        assert_eq!(u.nome(), "memoria");
        assert!(u.quadro().unwrap().is_empty());
    }
}
