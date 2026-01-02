# CO

Graph-based content management in Rust. Markdown files with YAML frontmatter, version controlled, human readable.

## Quick Start

```bash
git clone git@github.com:institutional-pointset/co.git
cd co
cargo build --workspace
cargo install --path co-cli
co --help
```

## Using as a Dependency

```toml
[dependencies]
co = { git = "https://github.com/institutional-pointset/co", branch = "main" }
```

### Example

```rust
use co::{Graph, LANGUAGES};
use co::language::Language;

fn main() {
    let graph = Graph::new();
    println!("Root: {:?}", graph.root());

    for lang in Language::initial_languages() {
        println!("{}: {}", lang.id, lang.name);
    }
}
```

## Structure

```
co/
├── core/             # Library crate
│   └── src/
│       ├── graph.rs      # Graph operations
│       ├── node.rs       # Node types
│       ├── edge.rs       # Edge types
│       ├── language.rs   # Language system
│       ├── index.rs      # Binary index
│       ├── query.rs      # Query DSL
│       ├── storage.rs    # File I/O
│       ├── frontmatter.rs
│       └── archive.rs
├── co-cli/           # CLI binary
│   └── src/
│       ├── main.rs
│       └── commands/
└── graph/            # Content
    ├── languages/
    └── definitions/
```

## Commands

```bash
co status              # Graph status
co languages           # List languages
co query "status:todo" # Query content
co define <id>         # Create definition
co index build         # Rebuild index
co repl                # Interactive mode
```

## License

MIT
