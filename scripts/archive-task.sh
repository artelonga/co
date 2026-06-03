#!/usr/bin/env bash
# archive-task.sh <TASK-ID>
# Creates (or overwrites) docs/task-archive/<TASK-ID>.json for a merged task.
#
# Usage:
#   scripts/archive-task.sh CO-279
#
# Reads:
#   work/**/<TASK-ID>.md          spec frontmatter + summary
#   CHANGELOG-PENDING/<TASK-ID>.md  changelog entry if still present
#   gh pr list                    PR metadata (branch, URL, stats, mergedAt)
#   git log                       merge commit SHA
#
# Writes: docs/task-archive/<TASK-ID>.json
#
# Idempotent — re-running overwrites the file.
#
# Exit codes:
#   0  archive written
#   1  missing TASK-ID argument

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR/.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

TASK_ID="${1:-}"
if [[ -z "$TASK_ID" ]]; then
  echo "Usage: $0 <TASK-ID>  (e.g. CO-279)" >&2
  exit 1
fi

REPO="artelonga/co"
ARCHIVE_DIR="docs/task-archive"
ARCHIVE_FILE="$ARCHIVE_DIR/${TASK_ID}.json"
mkdir -p "$ARCHIVE_DIR"

echo "→ Archiving $TASK_ID..." >&2

# ── 1. Spec frontmatter ──────────────────────────────────────────────────────

SPEC_FILE=$(find work/ -name "${TASK_ID}.md" 2>/dev/null | head -1 || true)

if [[ -n "$SPEC_FILE" ]]; then
  FRONTMATTER_JSON=$(SPEC_FILE="$SPEC_FILE" python3 - <<'PYEOF'
import sys, os, re, json

path = os.environ['SPEC_FILE']
with open(path) as f:
    content = f.read()

m = re.match(r'^---\s*\n(.*?)\n---', content, re.DOTALL)
if not m:
    print(json.dumps({}))
    sys.exit(0)

fm = {}
labels = []
in_labels = False
for line in m.group(1).split('\n'):
    stripped = line.strip()
    if not stripped:
        continue
    if in_labels:
        if stripped.startswith('- '):
            labels.append(stripped[2:].strip())
            continue
        else:
            in_labels = False
    if ':' in stripped and not stripped.startswith('-'):
        k, _, v = stripped.partition(':')
        k = k.strip()
        v = v.strip().strip('"')
        if k == 'labels':
            in_labels = True
        else:
            fm[k] = v

fm['labels'] = labels
print(json.dumps(fm))
PYEOF
)

  TITLE=$(echo "$FRONTMATTER_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin).get('title',''))")
  ITEM_TYPE=$(echo "$FRONTMATTER_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin).get('type','user-story'))")
  STATUS=$(echo "$FRONTMATTER_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin).get('status',''))")
  LABELS=$(echo "$FRONTMATTER_JSON" | python3 -c "import sys,json; d=json.load(sys.stdin); print(json.dumps(d.get('labels',[])))")
  MODULE=$(echo "$FRONTMATTER_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin).get('module',''))")
  CREATED_AT=$(echo "$FRONTMATTER_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin).get('created_at',''))")
  SEMVER_BUMP=$(echo "$FRONTMATTER_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin).get('semver_bump',''))")

  # First paragraph after frontmatter as summary (≤200 chars)
  SUMMARY=$(SPEC_FILE="$SPEC_FILE" python3 - <<'PYEOF'
import os, re

path = os.environ['SPEC_FILE']
with open(path) as f:
    content = f.read()

m = re.match(r'^---.*?---\s*\n(.*)', content, re.DOTALL)
body = m.group(1) if m else content

for para in body.split('\n\n'):
    para = para.strip()
    para = re.sub(r'^#+\s+', '', para, flags=re.MULTILINE)
    para = re.sub(r'\s+', ' ', para)
    if para and not para.startswith('```') and len(para) > 10:
        print(para[:200])
        break
PYEOF
)
else
  echo "  Warning: spec file not found for $TASK_ID" >&2
  TITLE=""
  ITEM_TYPE="user-story"
  STATUS=""
  LABELS="[]"
  MODULE=""
  CREATED_AT=""
  SEMVER_BUMP=""
  SUMMARY=""
  SPEC_FILE=""
fi

# ── 2. PR metadata ───────────────────────────────────────────────────────────

echo "  Searching for merged PR..." >&2
PR_JSON=$(gh pr list -R "$REPO" --state merged --limit 300 \
  --json number,url,title,headRefName,mergedAt,labels,additions,deletions,changedFiles \
  | python3 -c "
import sys, json
task_id = '${TASK_ID}'
prs = json.load(sys.stdin)
for p in prs:
    if task_id in p.get('headRefName','') or task_id in p.get('title',''):
        print(json.dumps(p))
        sys.exit(0)
print('null')
")

if [[ "$PR_JSON" == "null" ]]; then
  echo "  Warning: no merged PR found for $TASK_ID" >&2
  PR_NUMBER="null"
  PR_URL="null"
  BRANCH=""
  MERGED_AT="null"
  ADDITIONS=0
  DELETIONS=0
  CHANGED_FILES=0
  PR_LABELS="[]"
else
  PR_NUMBER=$(echo "$PR_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['number'])")
  PR_URL=$(echo   "$PR_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['url'])")
  BRANCH=$(echo   "$PR_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['headRefName'])")
  MERGED_AT=$(echo "$PR_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['mergedAt'])")
  ADDITIONS=$(echo "$PR_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin).get('additions',0))")
  DELETIONS=$(echo "$PR_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin).get('deletions',0))")
  CHANGED_FILES=$(echo "$PR_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin).get('changedFiles',0))")
  PR_LABELS=$(echo "$PR_JSON" | python3 -c "import sys,json; d=json.load(sys.stdin); print(json.dumps([l['name'] for l in d.get('labels',[])]))")
  echo "  PR #${PR_NUMBER}: $PR_URL" >&2
fi

# ── 3. Merge commit SHA ──────────────────────────────────────────────────────

MERGE_SHA=""
if [[ "$PR_NUMBER" != "null" ]]; then
  MERGE_SHA=$(git log --format="%H %s" | grep -m1 "(#${PR_NUMBER})" | awk '{print $1}' || true)
fi
if [[ -z "$MERGE_SHA" ]]; then
  MERGE_SHA=$(git log --format="%H %s" | grep -m1 "${TASK_ID}" | awk '{print $1}' || true)
fi
if [[ -n "$MERGE_SHA" ]]; then
  echo "  Merge commit: ${MERGE_SHA:0:7}" >&2
else
  echo "  Warning: no merge commit found in git log" >&2
  MERGE_SHA=""
fi

# ── 4. CHANGELOG-PENDING entry ───────────────────────────────────────────────

CL_FILE="CHANGELOG-PENDING/${TASK_ID}.md"
if [[ -f "$CL_FILE" ]]; then
  CL_CONTENT=$(cat "$CL_FILE")
else
  CL_CONTENT=""
fi

# ── 5. Write JSON ─────────────────────────────────────────────────────────────

export TASK_ID TITLE ITEM_TYPE STATUS LABELS MODULE CREATED_AT SEMVER_BUMP \
       SUMMARY SPEC_FILE BRANCH MERGE_SHA PR_NUMBER PR_URL MERGED_AT \
       ADDITIONS DELETIONS CHANGED_FILES PR_LABELS CL_CONTENT ARCHIVE_FILE

python3 - <<'PYEOF'
import json, os

def e(k, default=''):
    return os.environ.get(k, default) or default or None

def ei(k):
    try:
        return int(os.environ.get(k, ''))
    except (ValueError, TypeError):
        return None

def ej(k):
    try:
        return json.loads(os.environ.get(k, '[]'))
    except Exception:
        return []

pr_num_raw = os.environ.get('PR_NUMBER', 'null')
pr_number = int(pr_num_raw) if pr_num_raw not in ('null', '') else None
pr_url = e('PR_URL')
merged_at = e('MERGED_AT') if os.environ.get('MERGED_AT', 'null') != 'null' else None

data = {
    "task_id":        os.environ['TASK_ID'],
    "title":          e('TITLE'),
    "type":           e('ITEM_TYPE') or 'user-story',
    "status":         e('STATUS'),
    "semver_bump":    e('SEMVER_BUMP'),
    "spec_path":      e('SPEC_FILE'),
    "branch":         e('BRANCH'),
    "merge_sha":      e('MERGE_SHA'),
    "pr": {"url": pr_url, "number": pr_number} if pr_number else None,
    "started_at":     e('CREATED_AT'),
    "merged_at":      merged_at,
    "files_changed":  ei('CHANGED_FILES'),
    "lines_added":    ei('ADDITIONS'),
    "lines_removed":  ei('DELETIONS'),
    "labels":         ej('LABELS'),
    "changelog_entry": e('CL_CONTENT'),
    "summary":        e('SUMMARY'),
    "co_auto_version": "0.1.0",
}

archive_file = os.environ['ARCHIVE_FILE']
with open(archive_file, 'w') as f:
    json.dump(data, f, indent=2, ensure_ascii=False)
    f.write('\n')

print(f'  Written: {archive_file}')
PYEOF
