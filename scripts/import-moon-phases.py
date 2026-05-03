#!/usr/bin/env python3
"""INMET lunar phase importer.

Usage:
    python3 scripts/import-moon-phases.py <year> [--time-dir DIR] [--html-file PATH]

Fetches the lunar phase table from portal.inmet.gov.br and writes one .md per
phase into <time-dir>/moon-phases/<year>/.

Options:
    --time-dir DIR      Path to the time universe root (default: ../../time relative
                        to this script, i.e. the sibling 'time' repo).
    --html-file PATH    Read HTML from a local file instead of fetching the URL
                        (useful for CI; see tests/fixtures/inmet-luas-2026.html).
"""

import sys
import os
import re
import subprocess
import datetime
from urllib.request import urlopen, Request
from html.parser import HTMLParser

INMET_BASE_URL = "https://portal.inmet.gov.br/paginas/luas"

MONTH_MAP = {
    "Jan": 1, "Fev": 2, "Mar": 3, "Abr": 4, "Mai": 5, "Jun": 6,
    "Jul": 7, "Ago": 8, "Set": 9, "Out": 10, "Nov": 11, "Dez": 12,
}

# column index → (kind, slug for filename, Portuguese title prefix)
COLUMNS = [
    ("moon.new",           "new",            "Lua nova"),
    ("moon.first-quarter", "first-quarter",  "Lua crescente"),
    ("moon.full",          "full",           "Lua cheia"),
    ("moon.last-quarter",  "last-quarter",   "Lua minguante"),
]

EXPECTED_HEADERS = ["LUA NOVA", "LUA CRESCENTE", "LUA CHEIA", "LUA MINGUANTE"]

SCRIPT_DIR = os.path.dirname(os.path.realpath(__file__))
DEFAULT_TIME_DIR = os.path.normpath(os.path.join(SCRIPT_DIR, "..", "..", "time"))


# ---------------------------------------------------------------------------
# HTML parsing
# ---------------------------------------------------------------------------

class _TableParser(HTMLParser):
    """Extracts the first <table> and the last <h3> heading from an HTML page."""

    def __init__(self):
        super().__init__()
        self._in_h3 = False
        self._in_table = False
        self._in_cell = False
        self._current_cell = ""
        self._current_row = []
        self.h3_text = ""
        self.rows = []

    def handle_starttag(self, tag, attrs):
        if tag == "h3":
            self._in_h3 = True
            self.h3_text = ""
        elif tag == "table" and not self._in_table:
            self._in_table = True
        elif tag == "tr" and self._in_table:
            self._current_row = []
        elif tag in ("td", "th") and self._in_table:
            self._current_cell = ""
            self._in_cell = True

    def handle_endtag(self, tag):
        if tag == "h3":
            self._in_h3 = False
        elif tag == "table":
            self._in_table = False
        elif tag == "tr" and self._in_table:
            if self._current_row:
                self.rows.append(list(self._current_row))
        elif tag in ("td", "th") and self._in_table:
            self._current_row.append(self._current_cell.strip())
            self._in_cell = False

    def handle_data(self, data):
        if self._in_h3:
            self.h3_text += data
        if self._in_cell:
            self._current_cell += data


def fetch_html(year: int, html_file) -> str:
    if html_file:
        with open(html_file, encoding="utf-8") as f:
            return f.read()
    url = f"{INMET_BASE_URL}?ano={year}"
    req = Request(url, headers={"User-Agent": "import-moon-phases/1.0 (CO platform)"})
    with urlopen(req, timeout=30) as resp:
        return resp.read().decode("utf-8", errors="replace")


def parse_phases(html: str, year: int) -> list[tuple]:
    """Return list of (kind, slug, title_prefix, brt_datetime).

    Raises ValueError loudly if the page structure doesn't match expectations.
    """
    parser = _TableParser()
    parser.feed(html)

    heading = parser.h3_text.strip()
    if str(year) not in heading:
        raise ValueError(
            f"INMET page heading {heading!r} does not contain year {year} — "
            f"wrong page loaded or format changed"
        )

    rows = parser.rows
    if not rows:
        raise ValueError("INMET table: no rows found — page format may have changed")

    # Locate and validate the column header row
    header_idx = None
    for i, row in enumerate(rows):
        normalized = [c.strip().upper() for c in row]
        if normalized == EXPECTED_HEADERS:
            header_idx = i
            break

    if header_idx is None:
        raise ValueError(
            f"INMET table: column headers not found — "
            f"expected {EXPECTED_HEADERS}, page format may have changed"
        )

    data_rows = rows[header_idx + 1:]
    if not data_rows:
        raise ValueError("INMET table: no data rows after header — format may have changed")

    date_pat = re.compile(
        r"^(\d{2})\s+([A-Za-záàãâéêíóôõúç]+)\s+(\d{4})\s+-\s+(\d{2}):(\d{2})$"
    )

    phases = []
    for row in data_rows:
        if len(row) != 4:
            # Footnote or structural row — skip
            continue

        for col_idx, (kind, slug, title_prefix) in enumerate(COLUMNS):
            cell = row[col_idx].strip()
            if not cell or cell == "--":
                continue

            m = date_pat.match(cell)
            if not m:
                raise ValueError(
                    f"INMET table: unexpected cell {cell!r} at column {col_idx} "
                    f"(expected 'DD Mon YYYY - HH:MM') — format may have changed"
                )

            day_s, mon_abbr, yr_s, hh_s, mm_s = m.groups()
            day, yr, hh, mm = int(day_s), int(yr_s), int(hh_s), int(mm_s)

            cap = mon_abbr.capitalize()
            if cap not in MONTH_MAP:
                raise ValueError(
                    f"INMET table: unknown month abbreviation {mon_abbr!r} "
                    f"— format may have changed"
                )
            month = MONTH_MAP[cap]

            if yr != year:
                raise ValueError(
                    f"INMET table: cell year {yr} ≠ requested year {year} "
                    f"— wrong page or format changed"
                )

            brt_dt = datetime.datetime(yr, month, day, hh, mm, 0)
            phases.append((kind, slug, title_prefix, brt_dt))

    if not phases:
        raise ValueError("INMET table: zero phases parsed — format may have changed")

    return phases


# ---------------------------------------------------------------------------
# Frontmatter generation
# ---------------------------------------------------------------------------

def _git_sha() -> str:
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=SCRIPT_DIR,
            stderr=subprocess.DEVNULL,
        ).decode().strip()
    except Exception:
        return "unknown"


def _read_at_iso(path: str):
    """Return the at_iso value from an existing .md file, or None."""
    try:
        with open(path, encoding="utf-8") as f:
            in_fm = False
            for line in f:
                line = line.rstrip("\n")
                if line == "---":
                    if not in_fm:
                        in_fm = True
                        continue
                    else:
                        break
                if in_fm and line.startswith("at_iso:"):
                    return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return None


def _build_content(
    kind: str,
    slug: str,
    title_prefix: str,
    brt_dt: datetime.datetime,
    year: int,
    imported_at: str,
    imported_by: str,
) -> str:
    utc_dt = brt_dt + datetime.timedelta(hours=3)
    at_iso = utc_dt.strftime("%Y-%m-%dT%H:%M:%SZ")
    at_local = brt_dt.strftime("%Y-%m-%d %H:%M:%S -03:00")
    date_brt = brt_dt.strftime("%Y-%m-%d")
    title = f"{title_prefix} — {date_brt} (BRT)"

    fm = f"""\
---
title: "{title}"
type: moon-phase
kind: {kind}
at_iso: {at_iso}
at_local: {at_local}
geo:
  lat: -15.78
  lon: -47.93
  region: "Brasília (national reference)"
  timezone: America/Sao_Paulo
source: inmet.gov.br
source_url: {INMET_BASE_URL}
imported_at: {imported_at}
imported_by: {imported_by}
seed_status: draft
tags: [astronomy, moon, brazil, {year}]
---
"""

    body = f"""\
# {title}

Fase: **{title_prefix.lower()}** (`{kind}`).

Fonte: INMET — *Fases da Lua {year}*.
Horário BRT (UTC-3, sem horário de verão); `at_iso` = BRT + 3 h.
"""
    return fm + "\n" + body


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main(argv: list[str]) -> int:
    args = argv[1:]
    year_arg = None
    time_dir = DEFAULT_TIME_DIR
    html_file = None

    i = 0
    while i < len(args):
        a = args[i]
        if a == "--time-dir" and i + 1 < len(args):
            time_dir = args[i + 1]
            i += 2
        elif a == "--html-file" and i + 1 < len(args):
            html_file = args[i + 1]
            i += 2
        elif a.startswith("--time-dir="):
            time_dir = a.split("=", 1)[1]
            i += 1
        elif a.startswith("--html-file="):
            html_file = a.split("=", 1)[1]
            i += 1
        elif not a.startswith("--") and year_arg is None:
            year_arg = a
            i += 1
        else:
            print(f"error: unknown argument {a!r}", file=sys.stderr)
            return 1

    if year_arg is None:
        print("usage: import-moon-phases.py <year> [--time-dir DIR] [--html-file PATH]",
              file=sys.stderr)
        return 1

    try:
        year = int(year_arg)
    except ValueError:
        print(f"error: {year_arg!r} is not a valid year", file=sys.stderr)
        return 1

    # Fetch and parse
    try:
        html = fetch_html(year, html_file)
    except Exception as exc:
        print(f"error: could not load INMET page: {exc}", file=sys.stderr)
        return 1

    try:
        phases = parse_phases(html, year)
    except ValueError as exc:
        print(f"parse error: {exc}", file=sys.stderr)
        return 1

    # Prepare output directory
    out_dir = os.path.join(time_dir, "moon-phases", str(year))
    os.makedirs(out_dir, exist_ok=True)

    imported_at = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    imported_by = f"scripts/import-moon-phases.py@{_git_sha()}"

    n_new = 0
    n_updated = 0
    n_unchanged = 0

    for kind, slug, title_prefix, brt_dt in phases:
        date_str = brt_dt.strftime("%Y-%m-%d")
        filename = f"{date_str}-{slug}.md"
        path = os.path.join(out_dir, filename)

        utc_dt = brt_dt + datetime.timedelta(hours=3)
        new_at_iso = utc_dt.strftime("%Y-%m-%dT%H:%M:%SZ")

        existing_at_iso = _read_at_iso(path)
        if existing_at_iso is not None:
            if existing_at_iso == new_at_iso:
                n_unchanged += 1
                continue
            # at_iso differs — INMET revised the table; update
            content = _build_content(
                kind, slug, title_prefix, brt_dt, year, imported_at, imported_by
            )
            with open(path, "w", encoding="utf-8") as f:
                f.write(content)
            n_updated += 1
            print(f"  updated  {filename}  ({existing_at_iso} → {new_at_iso})")
        else:
            content = _build_content(
                kind, slug, title_prefix, brt_dt, year, imported_at, imported_by
            )
            with open(path, "w", encoding="utf-8") as f:
                f.write(content)
            n_new += 1
            print(f"  new      {filename}")

    total = n_new + n_updated + n_unchanged
    print(
        f"\nimported {total} phases for {year} "
        f"({n_new} new, {n_updated} updated, {n_unchanged} unchanged)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
