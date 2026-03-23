# ADR-0002: File-Based Storage with YAML Frontmatter

## Status

Accepted

## Context

CO-Web needs simple persistent storage for projects and tasks. The data model is
straightforward: projects contain tasks, and tasks have metadata (status, priority,
dates) plus free-form descriptions.

We considered SQLite, JSON files, and YAML/Markdown files.

## Decision

Use file-based storage with two formats:

- **Projects**: YAML files (`data/projects/{id}.yaml`) containing project metadata.
- **Tasks**: Markdown files with YAML frontmatter (`data/projects/{project_id}/tasks/{id}.md`)
  containing task metadata in the frontmatter and description in the body.

Example task file:

```markdown
---
id: "task-001"
title: "Implement login"
status: "doing"
priority: "high"
created: "2026-03-10"
---

Detailed description of the task goes here.
```

## Consequences

- **Human-readable**: Files can be inspected and edited with any text editor.
- **Git-friendly**: Changes produce clean diffs, enabling version control of project data.
- **No database dependency**: Eliminates setup complexity and runtime dependency on a DB engine.
- **Portable**: Data directory can be copied, backed up, or synced trivially.
- **Limited concurrency**: File-based I/O is not safe for concurrent writes without locking; acceptable for single-user or low-traffic scenarios.
- **No query engine**: Filtering and searching requires reading files into memory.
