#!/usr/bin/env python3
"""
board-load.py — Load a YAML export into the board API.

Usage:
    python3 board-load.py <yaml_file> [--dry-run]

Reads the enriched YAML and creates project + tasks via the board REST API.
Handles long descriptions, special characters, and parent-child relationships.
"""

import json
import sys
import urllib.request
import urllib.error

import yaml

BOARD_API = "http://localhost:3000/api"
MAX_DESCRIPTION_LENGTH = 9900  # Board limit is 10,000; leave margin


def api_post(path, data):
    """POST JSON to the board API, return parsed response."""
    url = f"{BOARD_API}{path}"
    payload = json.dumps(data).encode("utf-8")
    req = urllib.request.Request(
        url, data=payload, headers={"Content-Type": "application/json"}, method="POST"
    )
    try:
        with urllib.request.urlopen(req) as resp:
            return json.loads(resp.read())
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"POST {path} failed ({e.code}): {body}")


def api_put(path, data):
    """PUT JSON to the board API."""
    url = f"{BOARD_API}{path}"
    payload = json.dumps(data).encode("utf-8")
    req = urllib.request.Request(
        url, data=payload, headers={"Content-Type": "application/json"}, method="PUT"
    )
    try:
        with urllib.request.urlopen(req) as resp:
            return json.loads(resp.read())
    except urllib.error.HTTPError:
        pass  # Non-critical (e.g., due_date update)


def api_get(path):
    """GET from the board API."""
    url = f"{BOARD_API}{path}"
    try:
        with urllib.request.urlopen(url) as resp:
            return json.loads(resp.read())
    except urllib.error.HTTPError:
        return None


def truncate_description(desc):
    """Truncate description to fit board limits (10,000 bytes)."""
    if not desc:
        return ""
    encoded = desc.encode("utf-8")
    if len(encoded) <= MAX_DESCRIPTION_LENGTH:
        return desc
    # Truncate at byte boundary, then decode safely
    truncated = encoded[:MAX_DESCRIPTION_LENGTH].decode("utf-8", errors="ignore")
    # Don't cut in the middle of a line
    last_newline = truncated.rfind("\n")
    if last_newline > MAX_DESCRIPTION_LENGTH - 200:
        truncated = truncated[:last_newline]
    return truncated + "\n\n... (truncated)"


def count_tasks(tasks):
    """Recursively count all tasks."""
    total = 0
    for t in tasks:
        total += 1
        total += count_tasks(t.get("children", []))
    return total


def process_task(project_key, task, parent_id, counter, total, dry_run):
    """Create a task and its children recursively. Returns the board ID."""
    counter[0] += 1
    n = counter[0]

    jira_key = task["jira_key"]
    title = task["title"]
    task_type = task.get("type", "task")
    status = task.get("status", "todo")
    priority = task.get("priority", "medium")
    description = truncate_description(task.get("description", ""))
    due_date = task.get("due_date")
    is_orphan = task.get("orphan", False)

    # Build labels
    labels = [task_type, f"jira:{jira_key}"]
    if is_orphan:
        labels.append("orphan")
    for lbl in task.get("extra_labels", []):
        labels.append(lbl)

    if dry_run:
        parent_info = f" (parent={parent_id})" if parent_id else ""
        desc_info = f" [{len(description)}ch]" if description else ""
        print(
            f"  [DRY-RUN] {n}/{total} {jira_key:<12} [{status:<11}] {task_type} — {title}{parent_info}{desc_info}",
            file=sys.stderr,
        )
        board_id = 0
    else:
        print(f"  Creating {n}/{total}: {jira_key} — {title}", file=sys.stderr)

        payload = {
            "title": title,
            "description": description,
            "status": status,
            "priority": priority,
            "labels": labels,
        }
        if parent_id:
            payload["parent"] = parent_id

        result = api_post(f"/projects/{project_key}/tasks", payload)
        board_id = result["id"]

        # Set due_date via update if present
        if due_date:
            api_put(f"/projects/{project_key}/tasks/{board_id}", {"due_date": due_date})

    # Process children
    for child in task.get("children", []):
        process_task(project_key, child, board_id, counter, total, dry_run)

    return board_id


def main():
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <yaml_file> [--dry-run]", file=sys.stderr)
        sys.exit(1)

    yaml_path = sys.argv[1]
    dry_run = "--dry-run" in sys.argv

    # Load YAML
    print(f"-> Loading from: {yaml_path}", file=sys.stderr)
    with open(yaml_path) as f:
        data = yaml.safe_load(f)

    if dry_run:
        print("-> DRY-RUN MODE — no changes will be made", file=sys.stderr)

    proj = data["project"]
    proj_name = proj["name"]
    proj_key = proj["key"]
    proj_desc = proj.get("description", "")
    tasks = data["tasks"]
    total = count_tasks(tasks)

    print(f"-> Project: {proj_key} ({proj_name})", file=sys.stderr)
    print(f"-> Total tasks to create: {total}", file=sys.stderr)
    print("", file=sys.stderr)

    # Create project if needed
    if dry_run:
        print(f"-> [DRY-RUN] Would create project: {proj_key} ({proj_name})", file=sys.stderr)
    else:
        existing = api_get(f"/projects/{proj_key}")
        if existing:
            print(f"-> Project {proj_key} already exists, skipping creation", file=sys.stderr)
        else:
            print(f"-> Creating project: {proj_key} ({proj_name})", file=sys.stderr)
            api_post("/projects", {"name": proj_name, "key": proj_key, "description": proj_desc})
            print(f"-> Project {proj_key} created", file=sys.stderr)

    print("", file=sys.stderr)

    # Process all tasks
    counter = [0]
    for task in tasks:
        process_task(proj_key, task, None, counter, total, dry_run)

    print("", file=sys.stderr)
    if dry_run:
        print(f"-> DRY-RUN complete. {counter[0]} tasks would be created.", file=sys.stderr)
    else:
        print(f"-> Load complete. {counter[0]} tasks created in project {proj_key}.", file=sys.stderr)


if __name__ == "__main__":
    main()
