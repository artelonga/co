---
id: 3
title: Define Plugin trait and manifest format
status: todo
priority: critical
parent: 1
labels:
  - base-app
  - plugin-system
  - core
created_at: 2026-03-22T00:00:00Z
updated_at: 2026-03-22T00:00:00Z
---

GIVEN universes are self-contained plugins with their own logic, themes, and data,
WHEN I define the plugin interface,
THEN:
- [ ] `Plugin` trait defined in `core/` with methods: `name()`, `version()`, `manifest()`, `routes()`, `on_load()`, `on_unload()`
- [ ] Plugin manifest is a `plugin.toml` file with fields: name, version, description, author, universe_config (map size, rules, theme)
- [ ] `UniverseConfig` struct defined: map dimensions, tile data (JSON), rules, theme colors, portals, entities
- [ ] `PluginRegistry` struct manages loaded plugins with `register()`, `get()`, `list()` methods
- [ ] Plugins can register their own API routes via `routes() -> Router`
- [ ] One unit test: register a mock plugin, verify it appears in the registry
- [ ] Version: core v0.1.0
