# Spaces

Spaces are namespace directories that organize your content hierarchically.

## What is a Space?

A space is any directory with a `README.md` containing `type: space` in its frontmatter.
Spaces can be nested (e.g., `work/`, `work/private/`, `public/`).

## Creating Spaces

```
co init private    # Creates private/ with README.md
co init work       # Creates work/ with README.md
```

Each space is automatically added to `.gitignore` to prevent accidental commits.

## Space Structure

```
your-repo/
├── private/           # Private space (gitignored)
│   ├── README.md      # Space marker (type: space)
│   ├── user-stories/
│   └── tasks/
├── work/              # Work space
│   ├── README.md
│   └── use-cases/
└── public/            # Public space
    └── README.md
```

## Working with Spaces

**List spaces:**
```
co space list
```

**Show current space:**
```
co space current
```

**Create content in a space:**
```
co create user-story my-feature --in work
```

**Search within a space:**
```
co locate --in private status:todo
```

## Multi-Repo Support

Register multiple repositories for federated queries:

```
co repo add /path/to/project --alias myproject
co repo add . --alias current
co repo list
```

See also: `co help workflows`, `co help work-items`
