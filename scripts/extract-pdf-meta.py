#!/usr/bin/env python3
"""Extract metadata from a PDF and write a reference-card .md sibling.

Usage:
    python3 scripts/extract-pdf-meta.py <pdf-path> [--out <md-path>] [--force]

If --out is omitted, writes to <pdf-basename>.md in the same directory.
If the .md already exists and --force is not set, prints a diff to stderr
and exits non-zero if differences exist.
With --force, updates the auto-generated block but preserves ## Notes and
everything after it (human-authored content).

Requires: pypdf, langdetect
    pip install pypdf langdetect
"""

import argparse
import difflib
import hashlib
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

from pypdf import PdfReader

try:
    from langdetect import detect as _langdetect, LangDetectException
except ImportError:
    _langdetect = None
    LangDetectException = Exception

DOI_RE = re.compile(r'\b(10\.\d{4,9}/[^\s"<>\]]+)')
ISBN_RE = re.compile(r'ISBN(?:-1[03])?:?\s*([\d][\d\s-]{8,16}[\dXx])', re.IGNORECASE)
YEAR_RE = re.compile(r'\b((?:19|20)\d{2})\b')
AUTHOR_RE = re.compile(r'(?i)(?:^by|^authors?|^auteurs?|^autores?)[\s:]+([A-Za-z][^\n\r]{2,80})', re.MULTILINE)
ABSTRACT_RE = re.compile(r'(?i)\babstract\b[\s:]*\n?(.{10,500}?)(?:\n\n|\Z)', re.DOTALL)

NOTES_HEADING = '## Notes'
NOTES_PLACEHOLDER = '- (placeholder for the human-author\'s editorial)'


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open('rb') as f:
        for chunk in iter(lambda: f.read(65536), b''):
            h.update(chunk)
    return h.hexdigest()


def get_commit_sha() -> str:
    try:
        result = subprocess.run(
            ['git', 'rev-parse', '--short', 'HEAD'],
            capture_output=True, text=True, check=True,
        )
        return result.stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return 'unknown'


def parse_pdf_year(raw: str) -> Optional[str]:
    """Extract 4-digit year from a PDF /Info date string (D:YYYYMMDDHHmmSS...)."""
    if not raw:
        return None
    m = re.match(r'D:(\d{4})', raw)
    if m:
        return m.group(1)
    m = YEAR_RE.search(raw)
    return m.group(1) if m else None


def heuristic_title(first_page_text: str) -> Optional[str]:
    """Extract the best title candidate from first-page text.

    Skips blank lines, pure numbers, URLs, and short/boilerplate tokens.
    Returns the first plausible title line, or None if nothing found.
    """
    if not first_page_text:
        return None
    for line in first_page_text.splitlines():
        line = line.strip()
        if len(line) < 5:
            continue
        if re.match(r'^\d+$', line):
            continue
        if re.match(r'^https?://', line, re.IGNORECASE):
            continue
        if re.match(r'^(by |copyright |©|\d{4})', line, re.IGNORECASE):
            continue
        return line
    return None


def extract_metadata(pdf_path: Path) -> dict:
    """Read a PDF and return a metadata dict ready for frontmatter rendering."""
    reader = PdfReader(str(pdf_path))
    info = reader.metadata or {}
    page_count = len(reader.pages)

    full_text = ''
    extraction_failed = False
    try:
        for page in reader.pages:
            full_text += (page.extract_text() or '') + '\n'
    except Exception as exc:
        print(f'WARNING: text extraction error: {exc}', file=sys.stderr)
        extraction_failed = True

    first_page_text = ''
    try:
        first_page_text = reader.pages[0].extract_text() or ''
    except Exception:
        pass

    has_text = bool(full_text.strip())

    # Title: /Info.Title > first-page heuristic > filename stem
    info_title = (info.get('/Title') or '').strip()
    if info_title:
        title = info_title
    else:
        title = heuristic_title(first_page_text)
        if not title:
            print(
                f'WARNING: /Info.Title absent and first-page heuristic found nothing; '
                f'falling back to filename stem "{pdf_path.stem}"',
                file=sys.stderr,
            )
            title = pdf_path.stem

    # Authors
    info_author = (info.get('/Author') or '').strip()
    if info_author:
        authors = [a.strip() for a in re.split(r'[;,]', info_author) if a.strip()]
    else:
        m = AUTHOR_RE.search(first_page_text)
        authors = [m.group(1).strip()] if m else []

    # Year
    creation_raw = info.get('/CreationDate') or info.get('/ModDate') or ''
    year = parse_pdf_year(creation_raw)
    if not year:
        m = YEAR_RE.search(first_page_text)
        year = m.group(1) if m else None

    # Keywords
    kw_raw = (info.get('/Keywords') or '').strip()
    keywords = [k.strip() for k in re.split(r'[;,]', kw_raw) if k.strip()]

    # DOI
    doi_m = DOI_RE.search(full_text)
    doi = doi_m.group(1).rstrip('.,;:)') if doi_m else None

    # ISBN
    isbn_m = ISBN_RE.search(full_text)
    isbn = isbn_m.group(1).strip() if isbn_m else None

    # Abstract
    abstract = None
    abs_m = ABSTRACT_RE.search(full_text)
    if abs_m:
        abstract = abs_m.group(1).strip()[:500]

    # Language
    language = None
    if _langdetect and has_text:
        try:
            language = _langdetect(full_text[:2000])
        except LangDetectException:
            pass

    extraction_flags = []
    if extraction_failed or not has_text:
        extraction_flags.append('text-only-failed')

    return {
        'title': title,
        'file': pdf_path.name,
        'size_bytes': pdf_path.stat().st_size,
        'sha256': sha256_file(pdf_path),
        'language': language,
        'authors': authors,
        'year': year,
        'pages': page_count,
        'keywords': keywords,
        'doi': doi,
        'isbn': isbn,
        'abstract': abstract,
        'extraction_flags': extraction_flags or None,
    }


def _yaml_str(val: object) -> str:
    """Render a scalar as a YAML block-mapping value."""
    if val is None:
        return 'null'
    s = str(val)
    needs_quote = (
        not s
        or s[0] in '-?:,[]{}#&*!|>\'"@`'
        or ': ' in s
        or ' #' in s
        or s.endswith(':')
        or '\n' in s
    )
    if needs_quote:
        escaped = s.replace('\\', '\\\\').replace('"', '\\"').replace('\n', '\\n')
        return f'"{escaped}"'
    return s


def _yaml_flow_list(items: list) -> str:
    """Render a YAML flow sequence  [a, b, c]."""
    if not items:
        return '[]'

    def quote_item(v: str) -> str:
        if any(c in v for c in ',[]{}"\n'):
            escaped = v.replace('\\', '\\\\').replace('"', '\\"')
            return f'"{escaped}"'
        return v

    return '[' + ', '.join(quote_item(str(i)) for i in items) + ']'


def render_frontmatter(meta: dict, commit_sha: str) -> str:
    now = datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
    lines = ['---']
    lines.append(f'title: {_yaml_str(meta["title"])}')
    lines.append('type: reference')
    lines.append('medium: pdf')
    lines.append(f'file: {meta["file"]}')
    lines.append('mime: application/pdf')
    lines.append(f'size_bytes: {meta["size_bytes"]}')
    lines.append(f'sha256: {meta["sha256"]}')
    lines.append(f'language: {_yaml_str(meta["language"])}')
    lines.append(f'authors: {_yaml_flow_list(meta["authors"])}')
    lines.append(f'year: {_yaml_str(meta["year"])}')
    lines.append(f'pages: {meta["pages"]}')
    lines.append(f'keywords: {_yaml_flow_list(meta["keywords"])}')
    lines.append(f'doi: {_yaml_str(meta["doi"])}')
    lines.append(f'isbn: {_yaml_str(meta["isbn"])}')

    if meta['abstract']:
        indented = meta['abstract'].replace('\n', '\n  ')
        lines.append(f'abstract: |\n  {indented}')
    else:
        lines.append('abstract: null')

    if meta['extraction_flags']:
        flags = '\n'.join(f'- {f}' for f in meta['extraction_flags'])
        lines.append(f'extraction_flags:\n{flags}')

    lines.append(f'extracted_by: scripts/extract-pdf-meta.py@{commit_sha}')
    lines.append(f'extracted_at: {now}')
    lines.append('seed_status: stub')
    lines.append('tags:\n- reference\n- pdf\n- card\n- auto-extracted')
    lines.append('---')
    return '\n'.join(lines)


def render_auto_block(meta: dict, commit_sha: str) -> str:
    """Render the auto-generated portion of the card (frontmatter + prose + abstract section)."""
    frontmatter = render_frontmatter(meta, commit_sha)
    title = meta['title'] or meta['file']

    parts = [frontmatter, '', f'# {title}', '']

    source = f'Reference card auto-extracted from `{meta["file"]}`.'
    if meta['authors']:
        source += f' Authors: {", ".join(meta["authors"])}.'
    if meta['year']:
        source += f' ({meta["year"]})'
    parts.append(source)
    parts.append('')
    parts.append('## Auto-extracted abstract')
    parts.append('')
    parts.append(meta['abstract'] if meta['abstract'] else '_No abstract found._')
    parts.append('')

    return '\n'.join(parts)


def render_full_card(meta: dict, commit_sha: str) -> str:
    auto_block = render_auto_block(meta, commit_sha)
    return auto_block + f'\n{NOTES_HEADING}\n\n{NOTES_PLACEHOLDER}\n'


def split_at_notes(content: str):
    """Split a card at the ## Notes heading.

    Returns (auto_part, human_part) where human_part starts with the heading
    line and includes everything after, or (content, None) if not found.
    """
    lines = content.splitlines(keepends=True)
    for i, line in enumerate(lines):
        if line.rstrip() == NOTES_HEADING:
            return ''.join(lines[:i]), ''.join(lines[i:])
    return content, None


def stable_for_diff(content: str) -> str:
    """Strip volatile fields so diffs focus on extractable content."""
    lines = content.splitlines(keepends=True)
    return ''.join(
        l for l in lines
        if not l.startswith(('extracted_at:', 'extracted_by:'))
    )


def main() -> None:
    parser = argparse.ArgumentParser(
        description='Extract PDF metadata and write a reference-card .md'
    )
    parser.add_argument('pdf_path', type=Path, help='Path to the PDF file')
    parser.add_argument('--out', type=Path, default=None, help='Output .md path')
    parser.add_argument(
        '--force',
        action='store_true',
        help='Overwrite existing .md; human-authored ## Notes content is preserved',
    )
    args = parser.parse_args()

    pdf_path: Path = args.pdf_path.resolve()
    if not pdf_path.exists():
        print(f'ERROR: {pdf_path} does not exist', file=sys.stderr)
        sys.exit(1)

    md_path: Path = args.out if args.out else pdf_path.with_suffix('.md')

    try:
        meta = extract_metadata(pdf_path)
    except Exception as exc:
        print(f'ERROR extracting metadata: {exc}', file=sys.stderr)
        sys.exit(1)

    commit_sha = get_commit_sha()

    if md_path.exists() and not args.force:
        # Diff mode: compare stable fields and exit non-zero if there are differences
        new_card = render_full_card(meta, commit_sha)
        existing = md_path.read_text(encoding='utf-8')
        diff_lines = list(difflib.unified_diff(
            stable_for_diff(existing).splitlines(keepends=True),
            stable_for_diff(new_card).splitlines(keepends=True),
            fromfile=str(md_path),
            tofile='<extracted>',
        ))
        if diff_lines:
            sys.stderr.write(''.join(diff_lines))
            sys.exit(1)
        sys.exit(0)

    if md_path.exists() and args.force:
        existing = md_path.read_text(encoding='utf-8')
        _, human_part = split_at_notes(existing)
        auto_block = render_auto_block(meta, commit_sha)
        content = auto_block + '\n' + human_part if human_part else render_full_card(meta, commit_sha)
    else:
        content = render_full_card(meta, commit_sha)

    md_path.write_text(content, encoding='utf-8')
    print(f'Written: {md_path}')


if __name__ == '__main__':
    main()
