---
type: agent
id: writer
backend: claude
model: claude-3-opus
capabilities: write, edit, format
status: active
provider: anthropic
context: work items, schemas
---

# Writer Agent

An AI-powered agent specialized in content writing tasks.

## Capabilities

- **write**: Generate new content from prompts and context
- **edit**: Revise and improve existing content
- **format**: Apply consistent formatting and structure

## Usage

Invoke this agent to write content using Claude Code:

```bash
co write user-story --agent writer --in private
co write task --agent writer --context notes.md
```

## Backend

This agent uses the `claude` backend, which creates skeleton files with a `## Prompt` section for Claude Code to complete.
