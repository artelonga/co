---
id: 4
title: Plugin loader and dynamic registration
status: todo
priority: high
parent: 1
labels:
  - base-app
  - plugin-system
  - server
created_at: 2026-03-22T00:00:00Z
updated_at: 2026-03-22T00:00:00Z
---

GIVEN plugins are separate crates with a `plugin.toml` manifest,
WHEN the server starts,
THEN:
- [ ] Server scans a configurable `plugins/` directory for `plugin.toml` files
- [ ] Each discovered plugin is loaded and registered in `PluginRegistry`
- [ ] Plugin routes are mounted under `/api/v1/universes/{plugin_name}/`
- [ ] Plugin universe config is stored in the database on first load
- [ ] Server logs each loaded plugin: name, version, route count
- [ ] Missing or malformed `plugin.toml` produces a warning, does not crash server
- [ ] `GET /api/v1/plugins` returns list of loaded plugins with name, version, description
- [ ] Version: server v0.1.0
