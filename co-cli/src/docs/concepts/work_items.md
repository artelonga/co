# Work Items

CO uses structured work items for development tracking.

## Work Item Types

| Type | Purpose | Format |
|------|---------|--------|
| **user-story** | Feature or fix request | `AS A <role>, I NEED <feature>, SO THAT <benefit>` |
| **task** | Sub-item of user-story | `GIVEN <context>, WHEN <action>, THEN <result>` |
| **epic** | Feature grouping | Collection of user-stories |
| **release** | Version milestone | Groups completed work |

## Hierarchy

```
epic
└── user-story (type:feat or type:fix)
    ├── task (commit 1)
    ├── task (commit 2)
    └── task (commit 3)
```

A **user-story** becomes a GitHub issue with `type:feat` or `type:fix` label.
**Tasks** are implemented as commits within the user-story's PR.

## Creating Work Items

**User Story:**
```
co create user-story add-auth --in work
```

Prompts for:
- AS A (who benefits)
- I NEED (what feature)
- SO THAT (what value)

**Task:**
```
co create task implement-login --in work --story add-auth
```

Prompts for:
- GIVEN (precondition)
- WHEN (action)
- THEN (expected result)

## Git Integration

| Label | Branch Prefix | Version Bump |
|-------|---------------|--------------|
| `type:feat` | `feat/issue-<n>-...` | Minor (x.**Y**.0) |
| `type:fix` | `fix/issue-<n>-...` | Patch (x.y.**Z**) |
| `type:docs` | `docs/issue-<n>-...` | Patch (x.y.**Z**) |
| `type:refactor` | `refactor/issue-<n>-...` | Patch (x.y.**Z**) |

See also: `co help workflows`, `co help spaces`
