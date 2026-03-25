#!/usr/bin/env python3
"""
jira-export.py — Export PQ2 Jira data to structured YAML for board import.

Usage:
    python3 jira-export.py <project_key> <output_yaml>

Calls `acli` for flat CSV and `pst jira tree` for hierarchy, then merges
them into a single YAML file with nested parent-child relationships.
"""

import csv
import io
import re
import subprocess
import sys
import yaml


# ── Status & Priority Mapping ──

STATUS_MAP = {
    "Concluído": "done",
    "Em andamento": "in_progress",
    "Em análise": "in_review",
    "Tarefas pendentes": "todo",
}

TYPE_MAP = {
    "Epic": "epic",
    "Tarefa": "task",
    "Subtask": "subtask",
}


def map_status(jira_status):
    return STATUS_MAP.get(jira_status, "todo")


def map_priority(jira_priority):
    return jira_priority.lower() if jira_priority else "medium"


def map_type(jira_type):
    return TYPE_MAP.get(jira_type, "task")


# ── CSV Export ──

def fetch_csv(project_key):
    """Run acli to get all issues as CSV."""
    cmd = [
        "acli", "jira", "workitem", "search",
        "--jql", f"project = {project_key}",
        "--csv", "--paginate",
        "--fields", "issuetype,key,summary,status,priority",
    ]
    print(f"  Fetching all {project_key} issues via acli...", file=sys.stderr)
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    if result.returncode != 0:
        print(f"  ERROR: acli failed: {result.stderr}", file=sys.stderr)
        sys.exit(1)
    return result.stdout


def parse_csv(csv_text):
    """Parse CSV into a dict keyed by Jira key."""
    issues = {}
    reader = csv.DictReader(io.StringIO(csv_text))
    for row in reader:
        key = row["Key"]
        issues[key] = {
            "jira_key": key,
            "title": row["Summary"],
            "type": map_type(row["Type"]),
            "status": map_status(row["Status"]),
            "priority": map_priority(row["Priority"]),
        }
    return issues


# ── Tree Parsing ──

def fetch_tree(epic_key):
    """Run pst jira tree for a single epic."""
    cmd = ["pst", "jira", "tree", epic_key]
    print(f"  Fetching tree for {epic_key}...", file=sys.stderr)
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=300)
        if result.returncode != 0:
            print(f"  WARNING: pst jira tree {epic_key} failed: {result.stderr}", file=sys.stderr)
            return ""
        return result.stdout
    except subprocess.TimeoutExpired:
        print(f"  WARNING: pst jira tree {epic_key} timed out after 300s, skipping", file=sys.stderr)
        return ""


# Pattern to extract: key, type, status indicator, status text, summary
# Examples:
#   PQ2-2 (Epic) ● Em andamento — Deliver AIMS + pst v1
#   ├── PQ2-8 (Tarefa) ✓ Concluído — Meta AIMS V1
#   │   ├── PQ2-9 (Subtask) ✓ Concluído — Foundation Infrastructure
TREE_LINE_RE = re.compile(
    r"(PQ2-\d+)\s+\((\w+)\)\s+[✓●○◐]\s+(.+?)\s+—\s+(.+)"
)


def parse_tree(tree_text):
    """
    Parse pst jira tree output into hierarchy.

    Returns list of (key, type, depth) tuples where:
      depth 0 = epic (root)
      depth 1 = task (direct child of epic)
      depth 2 = subtask (child of task)
    """
    entries = []
    for line in tree_text.splitlines():
        m = TREE_LINE_RE.search(line)
        if not m:
            continue

        key = m.group(1)

        # Determine depth by counting indentation markers
        # Strip leading spaces
        stripped = line.lstrip()

        # Root level: starts with the key directly (e.g., "PQ2-2 (Epic)...")
        if stripped.startswith(key):
            depth = 0
        else:
            # Count the "│   " segments before the "├──" or "└──"
            # Each "│   " or "    " is one level of nesting
            # Find the position of the key in the line
            prefix = line[:line.index(key)]
            # Remove the connector (├── or └──) at the end
            prefix_clean = prefix.replace("├── ", "").replace("└── ", "")
            # Count remaining "│   " or "    " segments (each is 4 chars)
            # But the connector itself contributes to depth
            # Simpler: count how many "│" or tree connectors are before the key
            depth = prefix.count("├") + prefix.count("└") + prefix.count("│")
            # Normalize: the immediate child has one connector, grandchild has connector + pipe
            # ├── = depth 1
            # │   ├── = depth 2
            if depth > 2:
                depth = 2  # cap at subtask level

        entries.append((key, depth))

    return entries


def build_hierarchy_from_tree(tree_entries, issues_dict):
    """
    Given tree entries [(key, depth), ...] and the full issues dict,
    build a nested structure: epic -> tasks -> subtasks.

    Returns the epic node with children populated.
    """
    if not tree_entries:
        return None

    epic_key, _ = tree_entries[0]
    epic_node = issues_dict.get(epic_key)
    if not epic_node:
        return None
    epic_node = dict(epic_node)  # copy
    epic_node["children"] = []

    current_task = None
    for key, depth in tree_entries[1:]:
        node = issues_dict.get(key)
        if not node:
            continue
        node = dict(node)  # copy

        if depth == 1:
            # Direct child of epic (task)
            node["children"] = []
            epic_node["children"].append(node)
            current_task = node
        elif depth == 2 and current_task is not None:
            # Subtask under current task
            current_task["children"].append(node)

    return epic_node


# ── Main Export Logic ──

def export_project(project_key, output_path):
    # Step 1: Get flat CSV of all issues
    csv_text = fetch_csv(project_key)
    issues = parse_csv(csv_text)
    total = len(issues)
    print(f"  Found {total} issues in CSV", file=sys.stderr)

    # Step 2: Identify epics
    epics = {k: v for k, v in issues.items() if v["type"] == "epic"}
    epic_keys = sorted(epics.keys(), key=lambda k: int(k.split("-")[1]))
    print(f"  Found {len(epic_keys)} epics: {', '.join(epic_keys)}", file=sys.stderr)

    # Step 3: Fetch tree for each epic and build hierarchy
    all_seen_keys = set()
    task_trees = []

    for epic_key in epic_keys:
        tree_text = fetch_tree(epic_key)
        tree_entries = parse_tree(tree_text)

        if not tree_entries:
            # Epic with no children — just add it directly
            node = dict(issues[epic_key])
            task_trees.append(node)
            all_seen_keys.add(epic_key)
            continue

        for key, _ in tree_entries:
            all_seen_keys.add(key)

        epic_node = build_hierarchy_from_tree(tree_entries, issues)
        if epic_node:
            task_trees.append(epic_node)

    # Step 4: Find orphans (issues not in any tree)
    orphan_keys = set(issues.keys()) - all_seen_keys
    orphan_tasks = []
    for key in sorted(orphan_keys, key=lambda k: int(k.split("-")[1])):
        node = dict(issues[key])
        node["orphan"] = True
        orphan_tasks.append(node)

    if orphan_tasks:
        print(f"  Found {len(orphan_tasks)} orphan issues", file=sys.stderr)

    # Step 5: Build YAML structure
    data = {
        "project": {
            "name": "PointSet Q1 2026",
            "key": "PST",
            "description": f"{project_key} Jira project — Q1 2026 work",
        },
        "tasks": task_trees + orphan_tasks,
    }

    # Step 6: Write YAML
    # Custom representer to handle multi-line strings and ordering
    class CustomDumper(yaml.SafeDumper):
        pass

    def str_representer(dumper, data):
        if "\n" in data:
            return dumper.represent_scalar("tag:yaml.org,2002:str", data, style="|")
        return dumper.represent_scalar("tag:yaml.org,2002:str", data)

    CustomDumper.add_representer(str, str_representer)

    with open(output_path, "w") as f:
        yaml.dump(data, f, Dumper=CustomDumper, default_flow_style=False,
                  allow_unicode=True, sort_keys=False, width=120)

    in_tree = len(all_seen_keys)
    orphans = len(orphan_tasks)
    print(f"  Wrote {output_path}", file=sys.stderr)
    print(f"  Summary: {total} issues, {len(epic_keys)} epics, "
          f"{in_tree} in hierarchy, {orphans} orphans", file=sys.stderr)


# ── Entry Point ──

if __name__ == "__main__":
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <project_key> <output_yaml>", file=sys.stderr)
        sys.exit(1)

    project_key = sys.argv[1]
    output_path = sys.argv[2]
    export_project(project_key, output_path)
