# Getting Started with CO

CO is a graph-based content management CLI for structured development workflows.

## Quick Start

1. **Initialize a space**
   ```
   co init private    # Create a private workspace
   co init work       # Create a work workspace
   ```

2. **Create content**
   ```
   co create user-story my-feature --in work
   co create task implement-api --in work --story my-feature
   ```

3. **Search and locate**
   ```
   co locate status:todo           # Find items by status
   co locate "api endpoint"        # Full-text search
   co locate --in work priority:high
   ```

4. **Validate your content**
   ```
   co validate all    # Check all content for errors
   ```

## Key Commands

| Command | Purpose |
|---------|---------|
| `co init <name>` | Create a new space |
| `co create <type> <name>` | Create content interactively |
| `co locate <query>` | Search content |
| `co validate all` | Validate all content |
| `co status` | Show workspace status |
| `co help <topic>` | Get conceptual help |

## Next Steps

- `co help spaces` - Learn about spaces and namespaces
- `co help workflows` - Understand Plan & Execute workflow
- `co help work-items` - Learn about user-stories and tasks
