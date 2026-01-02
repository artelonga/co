# CO Genesis

This document records the origins of CO through the conversation that shaped its design.

## Summary

CO is an **exegetic graph database** for project development. It emerged from a series of discussions about programming language design, project management systems, and the nature of meaning across languages.

---

## Phase 1: Language Design Discussion

**Context**: Evaluating programming languages for an efficient, scalable system.

**Requirements analyzed**:
- Parallelization for ETL, training, inference
- RAM and GPU compute
- Mac computers with Apple Silicon
- Multi-machine cluster support

**Languages compared**: C, C++, Rust, Go, Zig, Julia

**Conclusions**:
- Rust for core engine (memory safety, performance)
- Domain-specific REPL for interactive exploration
- YAML frontmatter for structured metadata
- Markdown for human-readable content

---

## Phase 2: Project Management System Requirements

**Key requirements identified**:
1. Files stored as Markdown with YAML frontmatter
2. Compiled types with real-time CLI evaluation
3. Direct memory interop (memory-mapped files)
4. Compression for archival
5. Future web/network API capability

**Key insight**: YAML frontmatter enables separation of queryable structured data from narrative context.

---

## Phase 3: Repository Analysis

Studied existing implementation (`cns` repository) to understand current state:
- Bash CLI wrapping Rust HTTP backend
- File-based storage with frontmatter
- No persistent index (scans on every request)
- No compression, no query DSL

**Gaps identified** for new implementation:
- Persistent binary index for fast startup
- Memory-mapped file access
- Query DSL for filtering
- zstd compression for archives

---

## Phase 4: Fresh Start

Created new repository: `github.com/institutional-pointset/co` (internal)

**Initial epic issues created**:
1. [#1](../../issues/1): Native Rust CLI with REPL
2. [#2](../../issues/2): Fast Index and Query Engine
3. [#3](../../issues/3): Memory-Mapped File Access
4. [#4](../../issues/4): Cold Storage with zstd Compression
5. [#5](../../issues/5): Documentation and Onboarding

---

## Phase 5: Exegetic Reframing

**The pivotal insight**: CO is not just a project manager, but an exegetic system.

### What is Exegesis?

From Greek ἐξηγέομαι (exegeomai):
- **ex** = "out of"
- **hegeomai** = "to lead/guide"
- Meaning: "to lead out" - extracting meaning through careful interpretation

Opposite of **eisegesis** (reading meaning INTO text).

### How Exegesis Shapes CO

1. **Project artifacts are texts to be interpreted** - not just stored
2. **The system teaches heuristics** - users learn HOW to think, not just WHAT to do
3. **Language is culture materialized** - multi-lingual support is fundamental
4. **Self-referential design** - CO can describe itself in any language
5. **Fractal structure** - recursive, where each part reflects the whole

---

## Phase 6: Graph Database Architecture

**Realization**: CO is fundamentally a graph database.

### Structure

- **Nodes** = definitions (exegetic abstractions)
- **Edges** = relationships (inherits, translates_to, references)

### Type Hierarchy

```
CO (root supertype)
├── Language (inherits CO, adds linguistic primitives)
│   ├── English
│   ├── Portuguese
│   ├── Guarani Mbya (indigenous, cultural exegesis)
│   ├── Music (non-verbal exegesis)
│   └── Math (formal/symbolic exegesis)
│
└── Domain (instantiation of Language)
    └── Project (group of people with an objective)
        └── adds vocabulary specific to that domain
```

### Key Concepts

**Language**: A base type providing exegetic primitives (symbols, grammar, cultural context)

**Domain**: An instantiation of a Language that:
- Inherits all vocabulary from parent language
- Adds new internal vocabulary specific to that group/project
- Can be in any language but "inherits" the parent

**Project**: A domain created by a group of people with an objective
- Every project creates its own vocabulary
- This vocabulary is stored as nodes in the graph
- Cross-references create edges

### Architectural Principles

| Element | Role |
|---------|------|
| **YAML** | Structured data (nodes and edges) |
| **Markdown** | Contextual narrative (the "why", heuristics) |
| **References** | Graph edges (links to other documents) |
| **Recursion** | Types describe types, up to CO itself |

---

## Languages as Subtypes

The initial five languages demonstrate different forms of exegesis:

| Language | Purpose |
|----------|---------|
| **English** | Primary natural language |
| **Portuguese** | Secondary natural language |
| **Guarani Mbya** | Indigenous language (demonstrates cultural diversity in exegesis) |
| **Music** | Notation as language (non-verbal exegesis) |
| **Math** | Symbolic language (formal exegesis) |

---

## Self-Reference: The Fractal Property

CO can define itself:

```yaml
type: definition
id: def-co
symbol: co
inherits: [language, math]
translations:
  english:
    term: "CO"
    definition: "The root supertype; a graph database of exegetic abstractions"
  math:
    term: "∀x: type(x) → inherits(x, CO)"
    definition: "For all x, if x is a type, then x inherits from CO"
```

This is **fractal**: the definition of CO uses CO's own structure.

---

## Epic 0: Exegetic Foundation

Based on this reframing, [Epic 0](../../issues/6) was created to establish:
- Graph primitives (Node, Edge, Type)
- Language type system
- Translation mapping
- Self-referential definitions
- Domain/project instantiation
- Heuristic prompts for users

---

## Sources

- [Exegesis - Wikipedia](https://en.wikipedia.org/wiki/Exegesis)
- [Hermeneutics - Wikipedia](https://en.wikipedia.org/wiki/Hermeneutics)
- [Biblical Exegesis | Britannica](https://www.britannica.com/topic/biblical-literature/The-critical-study-of-biblical-literature-exegesis-and-hermeneutics)

---

*This document is part of the CO graph and should be translated into all supported languages.*
