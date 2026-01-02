# Agent Prompt: Implement v0.2.0 - Type System Foundation

## Context

You are implementing the first feature release for CO, an exegetic graph database for project development. CO is a Rust workspace with two crates:
- `core/` - Library crate with graph primitives
- `co-cli/` - CLI binary

**Current state**: v0.1.0 - Basic graph structure with NodeType enum, Languages, and file storage.

**Target state**: v0.2.0 - Type as universal abstraction, Scope isolation, Multilingual UI.

## Source of Truth

GitHub Issues are the source of truth. Review these before starting:
- **Issue #7**: EPIC 1 - Scope & Languages (contains US-1.1, US-1.2, US-1.3 in comments)
- **Plan file**: `/Users/yuri/.claude/plans/elegant-rolling-oasis.md`

## Core Principles (From Issue Comments)

1. **Type is the universal abstraction** - Everything in CO is a Type. Language is a subtype.
2. **Self-referential (fractal) type system** - CO describes itself using its own types.
3. **Translations are connections** - Between types, not inline data.
4. **Backwards compatibility** - All changes additive with deprecation warnings.
5. **UI vs Data separation** - Backend stays English, UI layer translates.

---

## Development Methodology

### TDD: Test-Driven Development

For each task, follow this cycle:

```
1. RED    → Write a failing test that defines expected behavior
2. GREEN  → Write minimal code to make the test pass
3. REFACTOR → Clean up while keeping tests green
```

### Red-Green-Refactor Example

```rust
// 1. RED - Write failing test first
#[test]
fn test_typekind_language_has_spec() {
    let spec = LanguageSpec {
        exegesis: ExegesisType::Natural,
        direction: Direction::LeftToRight,
        iso_code: Some("en".into()),
        is_default: true,
    };
    let kind = TypeKind::Language(spec.clone());

    match kind {
        TypeKind::Language(s) => assert!(s.is_default),
        _ => panic!("Expected Language variant"),
    }
}

// 2. GREEN - Implement minimal code
pub enum TypeKind {
    Language(LanguageSpec),
    // ... other variants
}

// 3. REFACTOR - Add derives, documentation, etc.
/// The kind of a Type - determines behavior and structure
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TypeKind {
    /// A language with lexicon and architecture
    Language(LanguageSpec),
    // ...
}
```

### Test File Structure

```
core/src/
  types.rs           # Implementation
  types/
    mod.rs           # If splitting into submodules
core/tests/
  types_test.rs      # Integration tests
  compatibility.rs   # Backwards compatibility tests
```

---

## Commit Structure: Conventional Commits

Use this format for all commits:

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

### Types

| Type | Use For |
|------|---------|
| `feat` | New feature |
| `fix` | Bug fix |
| `test` | Adding/updating tests |
| `refactor` | Code change that neither fixes nor adds |
| `docs` | Documentation only |
| `chore` | Build, config, tooling |
| `style` | Formatting, no code change |

### Scopes for v0.2.0

| Scope | Files Affected |
|-------|----------------|
| `types` | `core/src/types.rs` |
| `node` | `core/src/node.rs` |
| `edge` | `core/src/edge.rs` |
| `i18n` | `core/src/i18n.rs` |
| `scope` | `core/src/scope.rs` |
| `cli` | `co-cli/src/` |
| `lang` | `co-cli/src/commands/lang.rs` |

### Commit Sequence for TDD

```bash
# 1. Test first (RED)
git commit -m "test(types): add failing test for TypeKind enum"

# 2. Implementation (GREEN)
git commit -m "feat(types): implement TypeKind enum with Language variant"

# 3. Cleanup (REFACTOR)
git commit -m "refactor(types): add documentation and derives to TypeKind"
```

### Example Commit Messages

```bash
# Features
feat(types): add TypeKind enum as universal type abstraction
feat(types): add LanguageSpec with exegesis and direction
feat(types): add Lexicon struct with versioned entries
feat(node): add lexicon field to Node struct
feat(scope): implement Scope discovery and categorization
feat(i18n): add UI translation loading from YAML files
feat(cli): add 'co init' command for scope creation
feat(cli): add 'co list' command for scope listing
feat(cli): add 'co lang' command for system language

# Tests
test(types): add tests for TypeKind variants
test(types): add tests for Lexicon operations
test(node): add backwards compatibility tests for NodeType
test(i18n): add tests for translation fallback

# Deprecation
refactor(node): deprecate NodeType in favor of TypeKind
refactor(node): add From<NodeType> for TypeKind migration

# Documentation
docs(types): add module-level documentation for type system
```

---

## User Stories & Tasks

### US-1.1: Universal Type System

**AS** a developer
**I NEED** Type to be the universal abstraction
**TO** express all CO entities uniformly

#### Task 1.1.1: Create TypeKind enum

```rust
// Test (RED)
#[test]
fn test_typekind_variants() {
    // All node types should be expressible
    let variants = vec![
        TypeKind::Root,
        TypeKind::Language(LanguageSpec::default()),
        TypeKind::Domain,
        TypeKind::Definition,
        TypeKind::Task,
        TypeKind::Project,
        TypeKind::Content,
        TypeKind::Custom("agent".into()),
    ];
    assert_eq!(variants.len(), 8);
}
```

- **GIVEN** `core/src/types.rs` does not exist
- **WHEN** file created with TypeKind enum
- **THEN** enum compiles with variants: Root, Language(LanguageSpec), Domain, Definition, Task, Project, Content, Custom(String)
- **Files**: `core/src/types.rs` (NEW)
- **Commit**: `feat(types): add TypeKind enum as universal type abstraction`

#### Task 1.1.2: Make Language a specialized Type

```rust
// Test (RED)
#[test]
fn test_language_spec_fields() {
    let spec = LanguageSpec {
        exegesis: ExegesisType::Natural,
        direction: Direction::LeftToRight,
        iso_code: Some("pt".into()),
        is_default: false,
    };
    assert_eq!(spec.iso_code, Some("pt".into()));
    assert!(!spec.is_default);
}
```

- **GIVEN** TypeKind::Language variant exists
- **WHEN** LanguageSpec includes: exegesis, direction, iso_code, is_default
- **THEN** English can be marked as default template
- **Files**: `core/src/types.rs`
- **Commit**: `feat(types): add LanguageSpec with exegesis type and direction`

#### Task 1.1.3: Add lexicon support

```rust
// Test (RED)
#[test]
fn test_lexicon_define_and_get() {
    let mut lexicon = Lexicon::new();
    lexicon.define("task", "An actionable item");

    let entry = lexicon.get("task").unwrap();
    assert_eq!(entry.term, "task");
    assert_eq!(entry.definition, "An actionable item");
}
```

- **GIVEN** Node struct exists
- **WHEN** `lexicon: Option<Lexicon>` field added
- **THEN** Language nodes can store versioned vocabulary
- **Files**: `core/src/types.rs`, `core/src/node.rs`
- **Commits**:
  - `feat(types): add Lexicon and LexiconEntry structs`
  - `feat(node): add optional lexicon field to Node`

#### Task 1.1.4: Deprecate NodeType

```rust
// Test (RED) - Deprecation warning should appear
#[test]
fn test_nodetype_deprecated_but_works() {
    #[allow(deprecated)]
    let old_type = NodeType::Task;
    let new_type: TypeKind = old_type.into();
    assert!(matches!(new_type, TypeKind::Task));
}
```

- **GIVEN** NodeType enum exists
- **WHEN** `#[deprecated]` attribute added
- **THEN** existing code compiles with warnings
- **Files**: `core/src/node.rs`
- **Commit**: `refactor(node): deprecate NodeType in favor of TypeKind`

#### Task 1.1.5: Add From<NodeType> for TypeKind

- **GIVEN** both enums exist
- **WHEN** `impl From<NodeType> for TypeKind`
- **THEN** incremental migration possible
- **Files**: `core/src/node.rs`
- **Commit**: `feat(node): add From<NodeType> for TypeKind migration path`

#### Task 1.1.6: Add #[non_exhaustive]

- **GIVEN** TypeKind and EdgeType exist
- **WHEN** `#[non_exhaustive]` added
- **THEN** future variants don't break downstream
- **Files**: `core/src/types.rs`, `core/src/edge.rs`
- **Commit**: `refactor(types): add non_exhaustive to enums for future extensibility`

#### Task 1.1.7: Export from lib.rs

- **GIVEN** `types.rs` created
- **WHEN** `pub mod types;` added with re-exports
- **THEN** `use co::{TypeKind, Lexicon}` works
- **Files**: `core/src/lib.rs`
- **Commit**: `feat(core): export types module from library root`

#### Task 1.1.8: Add schema_version to frontmatter

```rust
// Test (RED)
#[test]
fn test_frontmatter_schema_version_default() {
    let yaml = "type: task\nstatus: todo";
    let fm: Frontmatter = Frontmatter::parse(yaml).unwrap();
    assert_eq!(fm.schema_version, 1); // Default for v0.1.0 files
}

#[test]
fn test_frontmatter_schema_version_explicit() {
    let yaml = "schema_version: 2\ntype: task";
    let fm: Frontmatter = Frontmatter::parse(yaml).unwrap();
    assert_eq!(fm.schema_version, 2);
}
```

- **GIVEN** Frontmatter struct exists
- **WHEN** `schema_version: Option<u32>` added with `#[serde(default)]`
- **THEN** v0.1.0 files parse as version 1, v0.2.0 can specify 2
- **Files**: `core/src/frontmatter.rs`
- **Commit**: `feat(frontmatter): add schema_version field with backwards-compatible default`

---

### US-1.2: Scope Isolation

**AS** a user with multiple projects
**I NEED** to create scope namespaces
**TO** organize content separately

#### Task 1.2.1: Initialize scope directory

```rust
// Integration test (RED)
#[test]
fn test_init_creates_scope_structure() {
    let temp = tempdir().unwrap();
    let result = init_scope(temp.path(), "yuri");

    assert!(result.is_ok());
    assert!(temp.path().join("yuri").exists());
    assert!(temp.path().join("yuri/tasks").exists());
    assert!(temp.path().join("yuri/definitions").exists());
    assert!(temp.path().join("yuri/projects").exists());
}
```

- **GIVEN** CO installed, no `yuri/` exists
- **WHEN** `co init yuri`
- **THEN** `yuri/` created with `tasks/`, `definitions/`, `projects/`
- **Files**: `co-cli/src/commands/init.rs` (NEW), `co-cli/src/main.rs`
- **Commits**:
  - `test(cli): add integration test for scope initialization`
  - `feat(cli): add 'co init' command for scope creation`

#### Task 1.2.2: Prevent duplicate scope

```rust
// Test (RED)
#[test]
fn test_init_fails_if_exists() {
    let temp = tempdir().unwrap();
    fs::create_dir(temp.path().join("yuri")).unwrap();

    let result = init_scope(temp.path(), "yuri");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("already exists"));
}
```

- **GIVEN** `yuri/` exists
- **WHEN** `co init yuri`
- **THEN** error "Scope 'yuri' already exists"
- **Files**: `co-cli/src/commands/init.rs`
- **Commit**: `feat(cli): add duplicate scope detection to init command`

#### Task 1.2.3: Create scope README

- **GIVEN** `co init yuri` executed
- **WHEN** directory created
- **THEN** `yuri/README.md` has frontmatter with type, id, language
- **Files**: `co-cli/src/commands/init.rs`
- **Commit**: `feat(cli): generate README.md with frontmatter on scope init`

#### Task 1.2.4: List scopes command

```rust
// Test (RED)
#[test]
fn test_list_shows_scopes_with_type() {
    // Setup: en/ (language), yuri/ (scope)
    let scopes = discover_scopes(temp.path());

    assert_eq!(scopes.len(), 2);
    assert!(scopes.iter().any(|s| s.id == "en" && s.is_language()));
    assert!(scopes.iter().any(|s| s.id == "yuri" && !s.is_language()));
}
```

- **GIVEN** `en/`, `yuri/` exist
- **WHEN** `co list`
- **THEN** shows `en (language)`, `yuri (scope)`
- **Files**: `co-cli/src/commands/list.rs` (NEW), `core/src/scope.rs` (NEW)
- **Commits**:
  - `feat(scope): add Scope struct and discovery`
  - `feat(cli): add 'co list' command for scope listing`

#### Task 1.2.5: List with stats

- **GIVEN** scopes have files
- **WHEN** `co list --stats`
- **THEN** shows file counts
- **Files**: `co-cli/src/commands/list.rs`
- **Commit**: `feat(cli): add --stats flag to list command`

#### Task 1.2.6: Add Scope type to core

- **GIVEN** `types.rs` exists
- **WHEN** Scope struct added
- **THEN** scopes discoverable with type categorization
- **Files**: `core/src/scope.rs` (NEW)
- **Commit**: `feat(scope): implement scope discovery and categorization`

---

### US-1.3: Multilingual UI Support

**AS** a non-English speaker
**I NEED** the CLI to display in my language
**TO** understand CO without knowing English

#### Task 1.3.1: System language configuration

```rust
// Test (RED)
#[test]
fn test_lang_command_sets_config() {
    let config_path = temp.path().join("config.yaml");
    set_system_language(&config_path, "pt").unwrap();

    let config = load_config(&config_path).unwrap();
    assert_eq!(config.system_language, "pt");
}
```

- **GIVEN** config doesn't exist
- **WHEN** `co lang pt`
- **THEN** config created with `system_language: pt`
- **Files**: `co-cli/src/commands/lang.rs` (NEW)
- **Commit**: `feat(cli): add 'co lang' command for system language setting`

#### Task 1.3.2: UI label translation loading

```rust
// Test (RED)
#[test]
fn test_i18n_loads_translations() {
    let i18n = I18n::load("pt", &translations_dir).unwrap();
    assert_eq!(i18n.t("fields.type"), "tipo");
    assert_eq!(i18n.t("types.task"), "tarefa");
}

#[test]
fn test_i18n_fallback_to_english() {
    let i18n = I18n::load("pt", &translations_dir).unwrap();
    // If "some.missing.key" not in pt.yaml, falls back to en.yaml
    assert_eq!(i18n.t("some.missing.key"), "some.missing.key");
}
```

- **GIVEN** system language is `pt`
- **WHEN** CO starts
- **THEN** labels loaded from `en/ui/pt.yaml`
- **Files**: `core/src/i18n.rs` (NEW)
- **Commit**: `feat(i18n): implement UI translation loading with fallback`

#### Task 1.3.3: Create English UI labels

- **GIVEN** need default labels
- **WHEN** `en/ui/en.yaml` created
- **THEN** English labels available: type, language, status, etc.
- **Files**: `en/ui/en.yaml` (NEW)
- **Commit**: `feat(i18n): add English UI translation file as default`

#### Task 1.3.4: Create Portuguese translation

- **GIVEN** need Portuguese UI
- **WHEN** `en/ui/pt.yaml` created
- **THEN** Portuguese labels: tipo, linguagem, estado, etc.
- **Files**: `en/ui/pt.yaml` (NEW)
- **Commit**: `feat(i18n): add Portuguese UI translation`

#### Task 1.3.5: CLI uses translated labels

```rust
// Test (RED)
#[test]
fn test_content_show_uses_i18n() {
    let i18n = I18n::load("pt", &dir).unwrap();
    let output = format_frontmatter(&node, &i18n);

    assert!(output.contains("tipo:"));
    assert!(output.contains("linguagem:"));
    assert!(!output.contains("type:"));
}
```

- **GIVEN** system language is `pt`
- **WHEN** `co content show` runs
- **THEN** output shows `tipo: tarefa`
- **Files**: `co-cli/src/commands/content/show.rs` (when implemented in v0.3.0, but prepare i18n now)
- **Commit**: `feat(cli): integrate i18n into CLI output formatting`

#### Task 1.3.6: Fallback for missing translations

- **GIVEN** system language is `gun` but no translation file
- **WHEN** CO displays UI
- **THEN** falls back to English with warning
- **Files**: `core/src/i18n.rs`
- **Commit**: `feat(i18n): add graceful fallback with warning for missing translations`

---

## File Creation Order

Execute in this order to satisfy dependencies:

```
1. core/src/types.rs         # TypeKind, LanguageSpec, Lexicon
2. core/src/node.rs          # Deprecate NodeType, add lexicon field
3. core/src/edge.rs          # Add #[non_exhaustive]
4. core/src/scope.rs         # Scope struct and discovery
5. core/src/i18n.rs          # Translation loading
6. core/src/lib.rs           # Export new modules
7. core/src/frontmatter.rs   # Add schema_version
8. en/ui/en.yaml             # English UI labels
9. en/ui/pt.yaml             # Portuguese UI labels
10. co-cli/src/commands/init.rs   # co init
11. co-cli/src/commands/list.rs   # co list
12. co-cli/src/commands/lang.rs   # co lang
13. co-cli/src/main.rs       # Wire up new commands
```

---

## Git Workflow

```bash
# Create feature branch
git checkout -b v0.2.0/type-system

# Implement with TDD commits (see sequence above)
# ...

# Final version bump
git commit -m "chore: bump version to 0.2.0"

# Tag release
git tag -a v0.2.0 -m "EPIC 1: Type System Foundation

- feat: TypeKind as universal type abstraction
- feat: Scope isolation with co init/list
- feat: Multilingual UI with co lang
- refactor: Deprecate NodeType (backwards compatible)

User Stories:
- US-1.1: Universal Type System
- US-1.2: Scope Isolation
- US-1.3: Multilingual UI Support"

# Merge to main
git checkout main
git merge --no-ff v0.2.0/type-system
git push origin main --tags
```

---

## Backwards Compatibility Checklist

Before merging, verify:

- [ ] All existing tests pass
- [ ] v0.1.0 YAML files parse without modification
- [ ] `NodeType` usable with deprecation warning
- [ ] `schema_version` defaults to 1 for old files
- [ ] No breaking changes to public API

---

## Success Criteria

When complete, these should work:

```bash
# Type system
cargo test -p co

# Scope commands
co init myproject
co list
co list --stats

# Language switching
co lang pt
co lang --list
co lang  # shows current

# Backwards compatibility
cargo test --test compatibility
```

---

## Questions to Resolve During Implementation

1. Should `TypeKind` include a `Scope` variant, or is Scope a separate concept?
2. Where should UI translation files live: `en/ui/` or `.co/translations/`?
3. Should `co lang` require the lexicon to exist, or just set the preference?

Make decisions, document in commit messages, and proceed.

---

🤖 Generated with [Claude Code](https://claude.com/claude-code)
