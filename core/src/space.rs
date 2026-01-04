//! Space discovery and management
//!
//! Spaces are directories that form a hierarchical namespace for content.
//! They can be nested (e.g., `private/`, `work/`, `work/private/`).
//!
//! # Terminology
//!
//! - **Space** = Any namespace directory (hierarchical, fractal/recursive)
//! - **Context** = User-provided content/prompts only (kept separate)
//!
//! The term "scope" is deprecated and aliased to Space for backwards compatibility.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Represents where the user is working relative to CO infrastructure
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaceLocation {
    /// Inside a space directory (project namespace)
    InSpace {
        /// The space directory path
        space_path: PathBuf,
        /// The space name (directory name)
        space_name: String,
        /// Parent repository root (if detected)
        repo_root: Option<PathBuf>,
        /// Whether the repo root is a git submodule
        is_submodule: bool,
    },
    /// At the repository root (co home), not inside a space
    AtRepoRoot {
        /// The repository root path
        repo_root: PathBuf,
    },
    /// Unknown location (not in a repo or space)
    Unknown,
}

impl SpaceLocation {
    /// Detect the current space location from the given directory
    pub fn detect(from: &Path) -> Self {
        let cwd = if from.is_absolute() {
            from.to_path_buf()
        } else {
            std::env::current_dir()
                .ok()
                .map(|c| c.join(from))
                .unwrap_or_else(|| from.to_path_buf())
        };

        // Walk up the directory tree looking for markers
        let mut current = Some(cwd.as_path());
        let mut space_found: Option<(PathBuf, String)> = None;
        let mut repo_root: Option<PathBuf> = None;

        while let Some(dir) = current {
            // Check for .git (repository root marker)
            if dir.join(".git").exists() && repo_root.is_none() {
                repo_root = Some(dir.to_path_buf());
            }

            // Check for .co/ (CO workspace root marker)
            if dir.join(".co").is_dir() && repo_root.is_none() {
                repo_root = Some(dir.to_path_buf());
            }

            // Check for space marker (README.md with type: space)
            if space_found.is_none()
                && let Some(name) = Self::is_space_dir(dir)
            {
                space_found = Some((dir.to_path_buf(), name));
            }

            current = dir.parent();
        }

        match (space_found, repo_root) {
            (Some((space_path, space_name)), repo_root) => {
                let is_submodule = repo_root
                    .as_ref()
                    .map(|r| Self::is_git_submodule(r))
                    .unwrap_or(false);
                SpaceLocation::InSpace {
                    space_path,
                    space_name,
                    repo_root,
                    is_submodule,
                }
            }
            (None, Some(repo_root)) => SpaceLocation::AtRepoRoot { repo_root },
            (None, None) => SpaceLocation::Unknown,
        }
    }

    /// Check if a directory is a git submodule
    ///
    /// Submodules have a .git FILE (not directory) that points to the parent's .git/modules/
    pub fn is_git_submodule(dir: &Path) -> bool {
        let git_path = dir.join(".git");
        git_path.is_file()
    }

    /// Check if a directory is a space (has README.md with type: space)
    fn is_space_dir(dir: &Path) -> Option<String> {
        let readme = dir.join("README.md");
        if readme.exists()
            && let Ok(content) = std::fs::read_to_string(&readme)
            && content.contains("type: space")
        {
            return dir.file_name().map(|n| n.to_string_lossy().to_string());
        }
        None
    }

    /// Check if we're inside a space
    pub fn is_in_space(&self) -> bool {
        matches!(self, SpaceLocation::InSpace { .. })
    }

    /// Check if we're at the repo root (not in a space)
    pub fn is_at_repo_root(&self) -> bool {
        matches!(self, SpaceLocation::AtRepoRoot { .. })
    }

    /// Get the space name if we're in a space
    pub fn space_name(&self) -> Option<&str> {
        match self {
            SpaceLocation::InSpace { space_name, .. } => Some(space_name),
            _ => None,
        }
    }

    /// Get the repo root if known
    pub fn repo_root(&self) -> Option<&Path> {
        match self {
            SpaceLocation::InSpace { repo_root, .. } => repo_root.as_deref(),
            SpaceLocation::AtRepoRoot { repo_root } => Some(repo_root),
            SpaceLocation::Unknown => None,
        }
    }

    /// Check if the current location is within a git submodule
    pub fn is_submodule(&self) -> bool {
        match self {
            SpaceLocation::InSpace { is_submodule, .. } => *is_submodule,
            _ => false,
        }
    }
}

/// The type of a space detected from its README frontmatter
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpaceKind {
    /// A natural or formal language (e.g., english, math)
    Language,
    /// A user-defined space for content organization
    Space,
    /// Unknown or unrecognized type
    #[default]
    Unknown,
}

/// Backwards-compatible alias for SpaceKind
#[deprecated(since = "0.13.1", note = "Use SpaceKind instead")]
pub type ContextKind = SpaceKind;

/// Backwards-compatible alias for SpaceKind
#[deprecated(since = "0.13.1", note = "Use SpaceKind instead")]
pub type ScopeKind = SpaceKind;

/// Represents a discovered space in the repository
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Space {
    /// Unique identifier (directory name)
    pub id: String,
    /// Type of space (language or user-defined)
    pub space_kind: SpaceKind,
    /// Path to the space directory
    pub path: PathBuf,
    /// Whether this space is a git submodule
    pub is_submodule: bool,
}

/// Backwards-compatible alias for Space
#[deprecated(since = "0.13.1", note = "Use Space instead")]
pub type Context = Space;

/// Backwards-compatible alias for Space
#[deprecated(since = "0.13.1", note = "Use Space instead")]
pub type Scope = Space;

impl Space {
    /// Create a new space
    pub fn new(id: impl Into<String>, space_kind: SpaceKind, path: impl Into<PathBuf>) -> Self {
        Self {
            id: id.into(),
            space_kind,
            path: path.into(),
            is_submodule: false,
        }
    }

    /// Create a language space
    pub fn language(id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self::new(id, SpaceKind::Language, path)
    }

    /// Create a user-defined space
    pub fn new_space(id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self::new(id, SpaceKind::Space, path)
    }

    /// Mark this space as a git submodule
    pub fn with_submodule(mut self, is_submodule: bool) -> Self {
        self.is_submodule = is_submodule;
        self
    }

    /// Check if this is a language space
    pub fn is_language(&self) -> bool {
        matches!(self.space_kind, SpaceKind::Language)
    }

    /// Check if this is a user-defined space
    pub fn is_space(&self) -> bool {
        matches!(self.space_kind, SpaceKind::Space)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_space_struct() {
        let space = Space::new("private", SpaceKind::Space, "/path/to/private");
        assert_eq!(space.id, "private");
        assert_eq!(space.space_kind, SpaceKind::Space);
        assert_eq!(space.path, PathBuf::from("/path/to/private"));
        assert!(!space.is_submodule);
    }

    #[test]
    fn test_space_language_constructor() {
        let lang = Space::language("en", "/path/to/en");
        assert!(lang.is_language());
        assert!(!lang.is_space());
    }

    #[test]
    fn test_space_with_submodule() {
        let space = Space::new_space("shared", "/path/to/shared").with_submodule(true);
        assert!(space.is_submodule);
    }

    #[test]
    fn test_space_kind_default() {
        let kind: SpaceKind = Default::default();
        assert_eq!(kind, SpaceKind::Unknown);
    }

    #[test]
    fn test_space_location_detect_at_repo_root() {
        let tmp = TempDir::new().unwrap();

        // Create a .git directory to simulate repo root
        std::fs::create_dir(tmp.path().join(".git")).unwrap();

        let location = SpaceLocation::detect(tmp.path());
        assert!(location.is_at_repo_root());
        assert!(!location.is_in_space());
        assert_eq!(location.repo_root(), Some(tmp.path()));
    }

    #[test]
    fn test_space_location_detect_in_space() {
        let tmp = TempDir::new().unwrap();

        // Create repo with .git
        std::fs::create_dir(tmp.path().join(".git")).unwrap();

        // Create a space directory with README
        let space_path = tmp.path().join("myspace");
        std::fs::create_dir(&space_path).unwrap();
        std::fs::write(
            space_path.join("README.md"),
            "---\ntype: space\nid: myspace\n---\n",
        )
        .unwrap();

        // Detect from inside the space
        let location = SpaceLocation::detect(&space_path);
        assert!(location.is_in_space());
        assert!(!location.is_at_repo_root());
        assert_eq!(location.space_name(), Some("myspace"));
        assert_eq!(location.repo_root(), Some(tmp.path()));
    }

    #[test]
    fn test_space_location_detect_in_subdir_of_space() {
        let tmp = TempDir::new().unwrap();

        // Create repo with .git
        std::fs::create_dir(tmp.path().join(".git")).unwrap();

        // Create a space directory with README
        let space_path = tmp.path().join("myspace");
        std::fs::create_dir(&space_path).unwrap();
        std::fs::write(
            space_path.join("README.md"),
            "---\ntype: space\nid: myspace\n---\n",
        )
        .unwrap();

        // Create a subdirectory inside the space
        let subdir = space_path.join("notes");
        std::fs::create_dir(&subdir).unwrap();

        // Detect from the subdirectory
        let location = SpaceLocation::detect(&subdir);
        assert!(location.is_in_space());
        assert_eq!(location.space_name(), Some("myspace"));
    }

    #[test]
    fn test_space_location_unknown() {
        let tmp = TempDir::new().unwrap();
        // No .git, no space markers
        let location = SpaceLocation::detect(tmp.path());
        assert!(matches!(location, SpaceLocation::Unknown));
        assert!(!location.is_in_space());
        assert!(!location.is_at_repo_root());
    }

    #[test]
    fn test_space_location_detect_at_co_root() {
        let tmp = TempDir::new().unwrap();

        // Create .co directory (CO workspace without git)
        std::fs::create_dir(tmp.path().join(".co")).unwrap();

        let location = SpaceLocation::detect(tmp.path());
        assert!(location.is_at_repo_root());
        assert_eq!(location.repo_root(), Some(tmp.path()));
    }

    #[test]
    fn test_space_location_git_and_co_present() {
        let tmp = TempDir::new().unwrap();

        // Create both .git and .co directories
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        std::fs::create_dir(tmp.path().join(".co")).unwrap();

        let location = SpaceLocation::detect(tmp.path());
        // Should detect as repo root (both markers present)
        assert!(location.is_at_repo_root());
        assert_eq!(location.repo_root(), Some(tmp.path()));
    }

    #[test]
    fn test_space_location_co_root_with_space() {
        let tmp = TempDir::new().unwrap();

        // Create .co directory (CO workspace without git)
        std::fs::create_dir(tmp.path().join(".co")).unwrap();

        // Create a space directory with README
        let space_path = tmp.path().join("myspace");
        std::fs::create_dir(&space_path).unwrap();
        std::fs::write(
            space_path.join("README.md"),
            "---\ntype: space\nid: myspace\n---\n",
        )
        .unwrap();

        // Detect from inside the space
        let location = SpaceLocation::detect(&space_path);
        assert!(location.is_in_space());
        assert_eq!(location.space_name(), Some("myspace"));
        // repo_root should be the .co directory parent
        assert_eq!(location.repo_root(), Some(tmp.path()));
    }

    #[test]
    fn test_submodule_detection_file_based() {
        let tmp = TempDir::new().unwrap();

        // Create parent repo
        std::fs::create_dir(tmp.path().join(".git")).unwrap();

        // Create submodule directory with .git FILE (not directory)
        let submodule_path = tmp.path().join("submodule");
        std::fs::create_dir(&submodule_path).unwrap();
        std::fs::write(
            submodule_path.join(".git"),
            "gitdir: ../.git/modules/submodule\n",
        )
        .unwrap();

        assert!(SpaceLocation::is_git_submodule(&submodule_path));
    }

    #[test]
    fn test_regular_repo_not_detected_as_submodule() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();

        assert!(!SpaceLocation::is_git_submodule(tmp.path()));
    }

    #[test]
    fn test_space_location_in_space_detects_submodule() {
        let tmp = TempDir::new().unwrap();

        // Create parent repo
        std::fs::create_dir(tmp.path().join(".git")).unwrap();

        // Create submodule with .git file
        let submodule_path = tmp.path().join("mysubmodule");
        std::fs::create_dir(&submodule_path).unwrap();
        std::fs::write(
            submodule_path.join(".git"),
            "gitdir: ../.git/modules/mysubmodule\n",
        )
        .unwrap();

        // Create a space inside the submodule
        let space_path = submodule_path.join("myspace");
        std::fs::create_dir(&space_path).unwrap();
        std::fs::write(
            space_path.join("README.md"),
            "---\ntype: space\nid: myspace\n---\n",
        )
        .unwrap();

        // Detect from inside the space
        let location = SpaceLocation::detect(&space_path);
        assert!(location.is_in_space());
        assert!(location.is_submodule());
    }
}
