# Workflows

CO supports structured development workflows through the Plan & Execute pattern.

## Plan & Execute Workflow

### 1. Plan Phase

Create a structured use-case from an objective:

```
co conduct plan "Add user authentication"
co conduct plan "Feature X" --context requirements.md
```

This creates a use-case file with:
- User story (AS A / I NEED / SO THAT)
- Acceptance criteria
- GitHub issue (auto-created)

### 2. Execute Phase

Drive the plan through git workflow states:

```
co conduct execute my-plan-id
```

States: `todo` -> `in-progress` -> `review` -> `done`

Each state change:
- Creates/updates git branch
- Updates issue status
- Manages PR lifecycle

## Writer Workflow

Generate content using writer agents:

```
co write user-story --agent writer --in work
co write task --agent writer --context notes.md
```

Agent backends:
- `manual` - Interactive prompts
- `claude` - Creates skeleton for Claude Code
- `ollama` - Local model (planned)

## Archive Workflow

Move completed content to archive:

```
co archive task-12.1.1           # Archive content
co archive list                  # List archived items
co archive restore task-12.1.1   # Restore from archive
```

Archived items have `indexed: false` and are excluded from locate/validate.

See also: `co help spaces`, `co help work-items`
