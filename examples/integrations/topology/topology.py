#!/usr/bin/env python3
"""topology — CO-503 deterministic meaning-topology tool (feeds the CO-517 Vectorizer).

Contract (matches co/src/canon_tool.rs::SubprocessInvoker):
  * Reads ONE JSON object from stdin: {"word": "...", "lang"?: "...", "k"?: 8}.
  * Writes ONE JSON object to stdout:
      {"word", "lang", "neighbors":[{term,lang,score}],
       "translations":[{term,lang}], "method", "note"}.
  * Exits non-zero with a message on stderr on error.

What this demonstrates: comunicacao's cross-language meaning-topology becomes a
portable CO tool that ANY surface can call (yggdrasil/topologia, co agents, the
bot) — decoupled from yggdrasil-core and from the ../comunicacao sibling-dir.

Two backends, a swap (CO-517 Vectorizer):
  * STATISTICAL — co-occurrence / PPMI over a shipped artifact. Runs OFFLINE.
                  This script ships its own tiny EMBEDDED sample lexicon so a
                  reviewer can run it with no ../comunicacao dir and no Ollama.
  * NEURAL      — the predictive overlay (nomic embeddings via Ollama). Not
                  exercised here; it is the CO-517 Vectorizer variant.

Stdlib only; no third-party deps, no network, no sibling-dir dependency.
"""

import json
import math
import sys

# --- Embedded sample lexicon -------------------------------------------------
# A handful of cross-language concept entries. Each term is tagged with its
# language and the shared CONCEPTS it participates in. Concepts are the bridge:
# terms that share concepts co-occur. This stands in for the production
# topologia.json artifact (gloss/def/examples/verses/co-occ per term).
#
#   lang codes: pt (Portuguese), es (Spanish), yo (Yoruba), gn (Guarani)
LEXICON = [
    # concept: WORD/SPEECH
    {"term": "palavra", "lang": "pt", "concepts": {"word": 9, "soul": 2}},
    {"term": "palabra", "lang": "es", "concepts": {"word": 9, "soul": 2}},
    {"term": "ọrọ", "lang": "yo", "concepts": {"word": 7, "soul": 3}},
    {"term": "ñe'ẽ", "lang": "gn", "concepts": {"word": 8, "soul": 6}},
    # concept: SOUL/BREATH (ñe'ẽ in Guarani = word AND soul — the bridge term)
    {"term": "alma", "lang": "pt", "concepts": {"soul": 8, "breath": 3}},
    {"term": "alma", "lang": "es", "concepts": {"soul": 8, "breath": 3}},
    {"term": "ẹ̀mí", "lang": "yo", "concepts": {"soul": 7, "breath": 6}},
    {"term": "ã", "lang": "gn", "concepts": {"soul": 7, "breath": 4}},
    # concept: WATER
    {"term": "água", "lang": "pt", "concepts": {"water": 9}},
    {"term": "agua", "lang": "es", "concepts": {"water": 9}},
    {"term": "omi", "lang": "yo", "concepts": {"water": 9, "soul": 1}},
    {"term": "y", "lang": "gn", "concepts": {"water": 9}},
    # concept: EARTH/LAND
    {"term": "terra", "lang": "pt", "concepts": {"earth": 9}},
    {"term": "tierra", "lang": "es", "concepts": {"earth": 9}},
    {"term": "ilẹ̀", "lang": "yo", "concepts": {"earth": 9}},
    {"term": "yvy", "lang": "gn", "concepts": {"earth": 9, "water": 1}},
]


def fail(msg):
    sys.stderr.write(msg + "\n")
    sys.exit(1)


def find_entry(word, lang):
    """Locate the lexicon entry for (word[, lang]). Returns entry or None."""
    word_l = word.strip().lower()
    for e in LEXICON:
        if e["term"].lower() == word_l and (lang is None or e["lang"] == lang):
            return e
    # second pass: ignore lang if a lang was supplied but didn't match
    for e in LEXICON:
        if e["term"].lower() == word_l:
            return e
    return None


def ppmi_weights():
    """Pre-compute total co-occurrence mass per concept for PPMI.

    PPMI(term, concept) = max(0, log2( p(term,concept) / (p(term) p(concept)) )).
    We treat the concept counts as a term×concept co-occurrence matrix.
    """
    concept_totals = {}
    grand_total = 0
    for e in LEXICON:
        for c, n in e["concepts"].items():
            concept_totals[c] = concept_totals.get(c, 0) + n
            grand_total += n
    return concept_totals, grand_total


def term_vector(entry, concept_totals, grand_total):
    """PPMI-weighted concept vector for one term."""
    term_total = sum(entry["concepts"].values())
    vec = {}
    for c, n in entry["concepts"].items():
        p_tc = n / grand_total
        p_t = term_total / grand_total
        p_c = concept_totals[c] / grand_total
        pmi = math.log2(p_tc / (p_t * p_c)) if p_tc > 0 else 0.0
        if pmi > 0:
            vec[c] = pmi
    return vec


def cosine(a, b):
    keys = set(a) | set(b)
    dot = sum(a.get(k, 0.0) * b.get(k, 0.0) for k in keys)
    na = math.sqrt(sum(v * v for v in a.values()))
    nb = math.sqrt(sum(v * v for v in b.values()))
    if na == 0 or nb == 0:
        return 0.0
    return dot / (na * nb)


def main():
    raw = sys.stdin.read() or "{}"
    try:
        args = json.loads(raw)
    except json.JSONDecodeError as e:
        fail(f"invalid JSON on stdin: {e}")

    word = args.get("word")
    if not isinstance(word, str) or not word.strip():
        fail("`word` (non-empty string) is required")
    lang = args.get("lang")
    if lang is not None and not isinstance(lang, str):
        fail("`lang` must be a string when present")
    k = args.get("k", 8)
    if not isinstance(k, int) or k < 1:
        fail("`k` must be a positive integer")

    target = find_entry(word, lang)
    if target is None:
        fail(f"word {word!r} not found in the embedded sample lexicon")

    concept_totals, grand_total = ppmi_weights()
    tvec = term_vector(target, concept_totals, grand_total)

    # neighbors: every other term, ranked by PPMI-cosine over shared concepts.
    scored = []
    for e in LEXICON:
        if e is target:
            continue
        score = cosine(tvec, term_vector(e, concept_totals, grand_total))
        if score > 0:
            scored.append((score, e))
    scored.sort(key=lambda s: (-s[0], s[1]["term"]))
    neighbors = [
        {"term": e["term"], "lang": e["lang"], "score": round(score, 4)}
        for score, e in scored[:k]
    ]

    # translations: same/overlapping concepts in OTHER languages, sharing the
    # term's dominant concept. (A simpler, direct cross-language map.)
    dominant = max(target["concepts"], key=target["concepts"].get)
    translations = [
        {"term": e["term"], "lang": e["lang"]}
        for e in LEXICON
        if e is not target
        and e["lang"] != target["lang"]
        and dominant in e["concepts"]
    ]

    out = {
        "word": target["term"],
        "lang": target["lang"],
        "neighbors": neighbors,
        "translations": translations,
        "method": "co-occurrence/PPMI",
        "note": "neural overlay = the CO-517 Vectorizer variant",
    }
    sys.stdout.write(json.dumps(out, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
