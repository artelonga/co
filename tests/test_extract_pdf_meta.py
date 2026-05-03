#!/usr/bin/env python3
"""Tests for scripts/extract-pdf-meta.py

Run with:  python3 tests/test_extract_pdf_meta.py
       or: pytest tests/test_extract_pdf_meta.py -v
"""

import hashlib
import importlib.util
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).parent.parent
SCRIPTS_DIR = REPO_ROOT / 'scripts'
FIXTURES_DIR = Path(__file__).parent / 'fixtures'
STUB_PDF = FIXTURES_DIR / 'stub.pdf'
SCRIPT = SCRIPTS_DIR / 'extract-pdf-meta.py'


def _load_module():
    spec = importlib.util.spec_from_file_location('extract_pdf_meta', SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


em = _load_module()


# ---------------------------------------------------------------------------
# Unit tests — pure functions, no subprocess
# ---------------------------------------------------------------------------

def test_sha256_matches_manual():
    expected = hashlib.sha256(STUB_PDF.read_bytes()).hexdigest()
    assert em.sha256_file(STUB_PDF) == expected


def test_doi_regex_catches_standard_form():
    m = em.DOI_RE.search('see doi 10.1234/test.paper.2024')
    assert m, 'DOI_RE should match a standard DOI'
    assert m.group(1) == '10.1234/test.paper.2024'


def test_doi_regex_four_digit_registrant():
    m = em.DOI_RE.search('10.1016/j.neuron.2024.01.001')
    assert m
    assert m.group(1).startswith('10.1016/')


def test_doi_regex_no_match():
    assert em.DOI_RE.search('no DOI here') is None


def test_heuristic_title_picks_first_meaningful_line():
    text = '   \n   \nMy Research Paper Title\nAuthor: Someone\n'
    assert em.heuristic_title(text) == 'My Research Paper Title'


def test_heuristic_title_skips_short_and_numeric():
    text = '1\n2024\nActual Title Here\n'
    assert em.heuristic_title(text) == 'Actual Title Here'


def test_heuristic_title_skips_urls():
    text = 'https://example.com/paper\nReal Title\n'
    assert em.heuristic_title(text) == 'Real Title'


def test_heuristic_title_returns_none_on_empty():
    assert em.heuristic_title('') is None
    assert em.heuristic_title('   \n\n  \n') is None


def test_parse_pdf_year_d_format():
    assert em.parse_pdf_year('D:20240101120000') == '2024'


def test_parse_pdf_year_returns_none_on_empty():
    assert em.parse_pdf_year('') is None


def test_split_at_notes_splits_correctly():
    content = 'frontmatter\n\n## Notes\n\nhuman content\n'
    auto, human = em.split_at_notes(content)
    assert auto == 'frontmatter\n\n', repr(auto)
    assert human == '## Notes\n\nhuman content\n', repr(human)


def test_split_at_notes_returns_none_when_absent():
    _, human = em.split_at_notes('just auto content here')
    assert human is None


def test_stable_for_diff_removes_volatile_fields():
    content = 'title: foo\nextracted_at: 2024-01-01T00:00:00Z\nextracted_by: scripts/extract-pdf-meta.py@abc\npages: 1\n'
    stable = em.stable_for_diff(content)
    assert 'extracted_at' not in stable
    assert 'extracted_by' not in stable
    assert 'title: foo' in stable
    assert 'pages: 1' in stable


# ---------------------------------------------------------------------------
# Integration tests — extract_metadata on the fixture PDF
# ---------------------------------------------------------------------------

def test_extract_metadata_required_fields():
    meta = em.extract_metadata(STUB_PDF)
    assert meta['title'], 'title must be non-empty'
    assert meta['pages'] == 1, f'expected 1 page, got {meta["pages"]}'
    assert len(meta['sha256']) == 64, 'sha256 must be 64 hex chars'
    assert 'file' in meta
    assert 'size_bytes' in meta
    assert 'language' in meta  # may be None — field must exist


def test_extract_metadata_title_from_info():
    meta = em.extract_metadata(STUB_PDF)
    assert meta['title'] == 'Test Reference'


def test_extract_metadata_authors_from_info():
    meta = em.extract_metadata(STUB_PDF)
    assert meta['authors'] == ['Test Author']


def test_extract_metadata_year_from_creation_date():
    meta = em.extract_metadata(STUB_PDF)
    assert meta['year'] == '2024'


def test_extract_metadata_keywords_from_info():
    meta = em.extract_metadata(STUB_PDF)
    assert 'test' in meta['keywords']
    assert 'fixture' in meta['keywords']


def test_render_full_card_contains_required_frontmatter_keys():
    meta = em.extract_metadata(STUB_PDF)
    card = em.render_full_card(meta, 'abc1234')
    for key in ('title:', 'pages:', 'sha256:', 'extracted_at:', 'language:'):
        assert key in card, f'missing key: {key}'


# ---------------------------------------------------------------------------
# End-to-end subprocess tests
# ---------------------------------------------------------------------------

def test_write_creates_md_sibling():
    with tempfile.TemporaryDirectory() as td:
        pdf = Path(td) / 'PICH0255-T.pdf'
        shutil.copy(STUB_PDF, pdf)
        result = subprocess.run(
            [sys.executable, str(SCRIPT), str(pdf)],
            capture_output=True, text=True,
        )
        assert result.returncode == 0, result.stderr
        md = Path(td) / 'PICH0255-T.md'
        assert md.exists(), 'md sibling was not created'
        content = md.read_text()
        for key in ('title:', 'pages:', 'sha256:', 'extracted_at:', 'language:'):
            assert key in content, f'missing frontmatter key: {key}'


def test_diff_mode_exits_nonzero_when_different():
    with tempfile.TemporaryDirectory() as td:
        pdf = Path(td) / 'stub.pdf'
        shutil.copy(STUB_PDF, pdf)
        md = Path(td) / 'stub.md'
        md.write_text('---\ntitle: Something Different\n---\n\nDifferent body\n')
        result = subprocess.run(
            [sys.executable, str(SCRIPT), str(pdf)],
            capture_output=True, text=True,
        )
        assert result.returncode != 0, 'diff mode should exit non-zero when differences exist'
        assert result.stderr, 'diff output should appear on stderr'


def test_diff_mode_exits_zero_when_same():
    with tempfile.TemporaryDirectory() as td:
        pdf = Path(td) / 'stub.pdf'
        shutil.copy(STUB_PDF, pdf)
        # First write
        subprocess.run([sys.executable, str(SCRIPT), str(pdf)], check=True)
        # Second run: same PDF, same .md — stable fields should match
        result = subprocess.run(
            [sys.executable, str(SCRIPT), str(pdf)],
            capture_output=True, text=True,
        )
        assert result.returncode == 0, f'identical re-run should exit 0; stderr: {result.stderr}'


def test_force_preserves_human_notes():
    with tempfile.TemporaryDirectory() as td:
        pdf = Path(td) / 'stub.pdf'
        shutil.copy(STUB_PDF, pdf)
        md = Path(td) / 'stub.md'
        # First write
        subprocess.run([sys.executable, str(SCRIPT), str(pdf)], check=True)
        # Simulate human editing the Notes section
        content = md.read_text()
        content = content.replace(em.NOTES_PLACEHOLDER, '- My important editorial note')
        md.write_text(content)
        # Force re-run
        result = subprocess.run(
            [sys.executable, str(SCRIPT), str(pdf), '--force'],
            capture_output=True, text=True,
        )
        assert result.returncode == 0, result.stderr
        new_content = md.read_text()
        assert '- My important editorial note' in new_content, 'human notes must be preserved'
        assert 'extracted_at:' in new_content, 'auto fields must be updated'
        assert 'sha256:' in new_content


def test_force_does_not_double_notes():
    with tempfile.TemporaryDirectory() as td:
        pdf = Path(td) / 'stub.pdf'
        shutil.copy(STUB_PDF, pdf)
        subprocess.run([sys.executable, str(SCRIPT), str(pdf)], check=True)
        subprocess.run([sys.executable, str(SCRIPT), str(pdf), '--force'], check=True)
        content = (Path(td) / 'stub.md').read_text()
        assert content.count('## Notes') == 1, 'should not duplicate ## Notes heading'


def test_out_flag_writes_to_custom_path():
    with tempfile.TemporaryDirectory() as td:
        pdf = Path(td) / 'stub.pdf'
        out = Path(td) / 'custom-output.md'
        shutil.copy(STUB_PDF, pdf)
        result = subprocess.run(
            [sys.executable, str(SCRIPT), str(pdf), '--out', str(out)],
            capture_output=True, text=True,
        )
        assert result.returncode == 0, result.stderr
        assert out.exists(), '--out path must be created'


# ---------------------------------------------------------------------------
# Runner
# ---------------------------------------------------------------------------

if __name__ == '__main__':
    tests = [(name, obj) for name, obj in sorted(globals().items()) if name.startswith('test_') and callable(obj)]
    passed = failed = 0
    for name, fn in tests:
        try:
            fn()
            print(f'  OK  {name}')
            passed += 1
        except Exception as exc:
            print(f'FAIL  {name}: {exc}')
            failed += 1
    print(f'\n{passed} passed, {failed} failed')
    sys.exit(0 if failed == 0 else 1)
