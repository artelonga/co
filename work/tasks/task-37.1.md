---
type: task
id: "37.1"
story: "[[37]]"
status: done
github: 37
language: english
---

# Implement Spaces & Multi-Repo SSH

## Given
the acceptance criteria from issue #37:
- Global space at `~/.co/` for cross-repo config
- Local space at `.co/` for project-specific config
- `co repo add <path> --alias <name> --ssh-host <host>` registers repo with SSH identity
- Context auto-detection from current directory
- `co space list` shows all registered spaces

## When
the implementation is complete

## Then
- `co space list` shows all registered spaces with types
- `co space current` shows current detected space
- `co repo add --ssh-host` stores SSH host identity
- Backward compatibility maintained for existing configs
