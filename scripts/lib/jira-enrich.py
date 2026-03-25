#!/usr/bin/env python3
"""
jira-enrich.py — Fetch descriptions and metadata from Jira for all tasks in a YAML export.

Usage:
    python3 jira-enrich.py <input_yaml> <output_yaml> [--concurrency N]

Reads the YAML produced by jira-export.py, fetches description/duedate/assignee/created/updated
for each issue via `acli jira workitem view`, and writes an enriched YAML file.
"""

import json
import re
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed

import yaml


# ── ADF to Markdown ──

def adf_to_markdown(node):
    """Convert Atlassian Document Format (ADF) to markdown."""
    if node is None:
        return ""
    if isinstance(node, str):
        return node
    if isinstance(node, list):
        return "".join(adf_to_markdown(item) for item in node)
    if not isinstance(node, dict):
        return ""

    ntype = node.get("type", "")
    content = node.get("content", [])
    text = node.get("text", "")
    marks = node.get("marks", [])

    # Apply marks (bold, italic, code, link)
    result = text
    for mark in marks:
        mtype = mark.get("type", "")
        if mtype == "strong":
            result = f"**{result}**"
        elif mtype == "em":
            result = f"*{result}*"
        elif mtype == "code":
            result = f"`{result}`"
        elif mtype == "link":
            href = mark.get("attrs", {}).get("href", "")
            result = f"[{result}]({href})"

    child_text = "".join(adf_to_markdown(c) for c in content)

    if ntype == "doc":
        return child_text
    elif ntype == "paragraph":
        return child_text + "\n\n"
    elif ntype == "heading":
        level = node.get("attrs", {}).get("level", 1)
        return "#" * level + " " + child_text + "\n\n"
    elif ntype == "bulletList":
        return child_text + "\n"
    elif ntype == "orderedList":
        return child_text + "\n"
    elif ntype == "listItem":
        # Indent nested content
        lines = child_text.strip().split("\n")
        first = "- " + lines[0] if lines else "- "
        rest = "\n".join("  " + l for l in lines[1:] if l.strip())
        return first + ("\n" + rest if rest else "") + "\n"
    elif ntype == "codeBlock":
        lang = node.get("attrs", {}).get("language", "")
        return f"```{lang}\n{child_text}```\n\n"
    elif ntype == "blockquote":
        lines = child_text.strip().split("\n")
        return "\n".join("> " + l for l in lines) + "\n\n"
    elif ntype == "rule":
        return "---\n\n"
    elif ntype == "hardBreak":
        return "\n"
    elif ntype == "mention":
        return "@" + node.get("attrs", {}).get("text", "someone")
    elif ntype == "emoji":
        return node.get("attrs", {}).get("text", "")
    elif ntype == "inlineCard":
        url = node.get("attrs", {}).get("url", "")
        return f"[link]({url})"
    elif ntype == "mediaSingle" or ntype == "mediaGroup":
        return child_text
    elif ntype == "media":
        return "[attachment]\n"
    elif ntype == "table":
        return child_text + "\n"
    elif ntype == "tableRow":
        cells = [adf_to_markdown(c).strip() for c in content]
        return "| " + " | ".join(cells) + " |\n"
    elif ntype == "tableHeader":
        return child_text
    elif ntype == "tableCell":
        return child_text
    else:
        return result + child_text


def clean_markdown(text):
    """Clean up markdown output."""
    # Remove excessive blank lines
    text = re.sub(r"\n{3,}", "\n\n", text)
    return text.strip()


# ── Jira Fetcher ──

def fetch_issue_metadata(jira_key):
    """Fetch description, duedate, assignee, created, updated for a single issue."""
    cmd = [
        "acli", "jira", "workitem", "view", jira_key,
        "--fields", "description,duedate,assignee,created,updated",
        "--json",
    ]
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
        if result.returncode != 0:
            return jira_key, {}
        data = json.loads(result.stdout)
        fields = data.get("fields", {})

        metadata = {}

        # Description: ADF → markdown
        desc = fields.get("description")
        if desc and isinstance(desc, dict):
            md = clean_markdown(adf_to_markdown(desc))
            if md:
                metadata["description"] = md

        # Due date
        duedate = fields.get("duedate")
        if duedate:
            metadata["due_date"] = duedate

        # Assignee
        assignee = fields.get("assignee")
        if assignee and isinstance(assignee, dict):
            name = assignee.get("displayName", "")
            if name:
                metadata["assignee"] = name

        # Created/Updated timestamps
        created = fields.get("created")
        if created:
            metadata["created"] = created

        updated = fields.get("updated")
        if updated:
            metadata["updated"] = updated

        return jira_key, metadata

    except (subprocess.TimeoutExpired, json.JSONDecodeError, Exception) as e:
        print(f"  WARNING: Failed to fetch {jira_key}: {e}", file=sys.stderr)
        return jira_key, {}


# ── YAML Processing ──

def collect_jira_keys(tasks):
    """Recursively collect all jira_key values from the task tree."""
    keys = []
    for task in tasks:
        keys.append(task["jira_key"])
        if task.get("children"):
            keys.extend(collect_jira_keys(task["children"]))
    return keys


def enrich_tasks(tasks, metadata_map):
    """Recursively enrich tasks with metadata."""
    enriched_count = 0
    for task in tasks:
        jira_key = task["jira_key"]
        meta = metadata_map.get(jira_key, {})
        if meta.get("description"):
            task["description"] = meta["description"]
            enriched_count += 1
        if meta.get("due_date"):
            task["due_date"] = meta["due_date"]
        if meta.get("assignee"):
            # Add assignee as a label
            if "labels" not in task:
                task["labels"] = []
            task.setdefault("extra_labels", [])
            task["extra_labels"].append(f"assignee:{meta['assignee']}")
        if task.get("children"):
            enriched_count += enrich_tasks(task["children"], metadata_map)
    return enriched_count


# ── Main ──

def main():
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} <input_yaml> <output_yaml> [--concurrency N]", file=sys.stderr)
        sys.exit(1)

    input_path = sys.argv[1]
    output_path = sys.argv[2]

    concurrency = 10
    if "--concurrency" in sys.argv:
        idx = sys.argv.index("--concurrency")
        concurrency = int(sys.argv[idx + 1])

    # Load YAML
    print(f"  Loading {input_path}...", file=sys.stderr)
    with open(input_path) as f:
        data = yaml.safe_load(f)

    # Collect all Jira keys
    all_keys = collect_jira_keys(data["tasks"])
    total = len(all_keys)
    print(f"  Found {total} issues to enrich", file=sys.stderr)

    # Fetch metadata in parallel
    metadata_map = {}
    completed = 0
    start_time = time.time()

    print(f"  Fetching metadata ({concurrency} concurrent)...", file=sys.stderr)

    with ThreadPoolExecutor(max_workers=concurrency) as executor:
        futures = {executor.submit(fetch_issue_metadata, key): key for key in all_keys}

        for future in as_completed(futures):
            jira_key, meta = future.result()
            metadata_map[jira_key] = meta
            completed += 1

            if completed % 50 == 0 or completed == total:
                elapsed = time.time() - start_time
                rate = completed / elapsed if elapsed > 0 else 0
                eta = (total - completed) / rate if rate > 0 else 0
                print(
                    f"  Progress: {completed}/{total} ({rate:.1f}/s, ETA {eta:.0f}s)",
                    file=sys.stderr,
                )

    # Count how many actually have descriptions
    with_desc = sum(1 for m in metadata_map.values() if m.get("description"))
    with_due = sum(1 for m in metadata_map.values() if m.get("due_date"))
    with_assignee = sum(1 for m in metadata_map.values() if m.get("assignee"))
    print(f"  Fetched: {with_desc} descriptions, {with_due} due dates, {with_assignee} assignees", file=sys.stderr)

    # Enrich the task tree
    enriched = enrich_tasks(data["tasks"], metadata_map)
    print(f"  Enriched {enriched} tasks with descriptions", file=sys.stderr)

    # Write output YAML
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

    print(f"  Wrote {output_path}", file=sys.stderr)


if __name__ == "__main__":
    main()
