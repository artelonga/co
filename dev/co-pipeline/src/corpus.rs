//! CO-88 — source corpora loaded from local content repos.
//!
//! Maps the three matrix universes to their on-disk repos. A missing repo is
//! not an error: the universe simply contributes zero files, so the harness
//! still runs end-to-end on whatever content is present.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// A universe in the matrix, with where its markdown lives and how it is shared.
#[derive(Debug, Clone)]
pub struct Universe {
    /// Matrix key (`artelonga`, `quilomboaraucaria`, `rfq`).
    pub key: String,
    /// Visibility label echoed into the report.
    pub visibility: String,
    /// Repo directory under the corpus root (relative name).
    pub repo_dir: String,
}

/// One authoring file in a corpus.
#[derive(Debug, Clone)]
pub struct CorpusFile {
    /// Vault-relative path (forward slashes).
    pub rel_path: String,
    /// Raw file bytes (markdown body).
    pub bytes: Vec<u8>,
}

impl Universe {
    /// The three universes the matrix targets, in report order.
    pub fn matrix() -> Vec<Universe> {
        vec![
            Universe {
                key: "artelonga".into(),
                visibility: "public-subscribable".into(),
                repo_dir: "ArteLonga".into(),
            },
            Universe {
                key: "quilomboaraucaria".into(),
                visibility: "public-subscribable".into(),
                repo_dir: "quilomboaraucaria".into(),
            },
            Universe {
                key: "rfq".into(),
                visibility: "private".into(),
                repo_dir: "rfq-gateway".into(),
            },
        ]
    }

    /// Absolute repo path under `corpus_root`.
    pub fn repo_path(&self, corpus_root: &Path) -> PathBuf {
        corpus_root.join(&self.repo_dir)
    }

    /// Load up to `limit` markdown files from this universe's repo.
    ///
    /// Returns an empty vec if the repo is absent. Files are sorted by path so
    /// the matrix is deterministic.
    pub fn load_files(&self, corpus_root: &Path, limit: usize) -> Vec<CorpusFile> {
        let root = self.repo_path(corpus_root);
        if !root.is_dir() {
            return Vec::new();
        }
        let mut files = collect_markdown(&root, limit);
        files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        files.truncate(limit);
        files
    }
}

/// Walk `root` collecting `.md` files (skipping VCS/hidden dirs), up to `limit`.
pub fn collect_markdown(root: &Path, limit: usize) -> Vec<CorpusFile> {
    let mut out = Vec::new();
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| !is_hidden_or_vcs(e.path()))
        .filter_map(|e| e.ok())
    {
        if out.len() >= limit {
            break;
        }
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        out.push(CorpusFile {
            rel_path: rel.to_string_lossy().replace('\\', "/"),
            bytes,
        });
    }
    out
}

fn is_hidden_or_vcs(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n == ".git" || n == "node_modules" || n == "target" || n == "_source")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_has_three_universes() {
        let m = Universe::matrix();
        assert_eq!(m.len(), 3);
        assert_eq!(m[2].visibility, "private");
    }

    #[test]
    fn missing_repo_yields_no_files() {
        let dir = tempfile::tempdir().unwrap();
        let u = &Universe::matrix()[0];
        assert!(u.load_files(dir.path(), 100).is_empty());
    }

    #[test]
    fn collects_markdown_sorted_and_skips_git() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ArteLonga");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::write(root.join("b.md"), b"# B").unwrap();
        std::fs::write(root.join("notes/a.md"), b"# A").unwrap();
        std::fs::write(root.join(".git/config"), b"x").unwrap();
        std::fs::write(root.join("c.txt"), b"not md").unwrap();
        let u = &Universe::matrix()[0];
        let files = u.load_files(dir.path(), 100);
        let paths: Vec<_> = files.iter().map(|f| f.rel_path.as_str()).collect();
        assert_eq!(paths, vec!["b.md", "notes/a.md"]);
    }
}
