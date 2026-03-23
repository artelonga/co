# ADR-0003: Vanilla JS with No Build Tooling

## Status

Accepted

## Context

The CO-Web frontend needs to support multiple UI variants that can be iterated on
quickly. Each variant explores a different interaction paradigm (Kanban, Table,
Timeline), and we need the ability to modify and compare them with minimal friction.

We considered React, Vue, Svelte, and vanilla HTML/CSS/JS.

## Decision

Use vanilla HTML, CSS, and JavaScript with zero build tooling.

- Each variant is a self-contained set of HTML, CSS, and JS files under `static/{variant}/`.
- No transpilation, bundling, or package manager is required.
- Shared utilities (API client, experiment widget) live in `static/shared/`.
- The browser loads files directly as served by the Rust backend.

## Consequences

- **No framework lock-in**: Variants can use completely different UI patterns without framework constraints.
- **Instant feedback**: Edit a file, refresh the browser. No build step, no hot-reload daemon.
- **Low barrier to entry**: Any developer can contribute without learning a framework or toolchain.
- **Larger JS files**: Without tree-shaking or minification, shipped JS may be larger than a bundled equivalent.
- **No type safety**: Without TypeScript, type errors are caught at runtime rather than compile time.
- **No component model**: UI reuse depends on manual patterns (template literals, DOM helpers) rather than framework components.
