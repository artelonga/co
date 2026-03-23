# ADR-0004: Built-In Experiment Framework

## Status

Accepted

## Context

Before committing to a final UI design, we need to compare multiple approaches with
real user feedback. The experiment must be lightweight, self-contained, and not depend
on external analytics or A/B testing services.

## Decision

Build an integrated A/B/C testing framework into CO-Web with the following components:

- **Variant assignment**: Random assignment on first visit via the `co_variant` cookie (see ADR-0001).
- **Feedback collection**: Each variant includes an experiment widget where users rate their experience (1-5 stars) and leave optional comments. Feedback is stored as JSON files in `data/feedback/`.
- **Summary API**: A `/api/experiment/summary` endpoint aggregates feedback across variants, returning average ratings, preference counts, and sample comments.
- **Manual switching**: Users can switch variants at any time via the experiment widget to compare experiences directly.

## Consequences

- **Data-driven decisions**: Design choices are backed by quantitative ratings and qualitative feedback rather than opinion.
- **Self-contained**: No dependency on third-party analytics; all data stays local.
- **Overhead**: Maintaining multiple variants in parallel requires discipline to keep shared logic in sync.
- **Bias risk**: Users who switch variants manually may skew results; the summary API should distinguish assigned vs. switched users if needed.
- **Temporary by design**: Once a winning variant is selected, the experiment framework and losing variants can be removed cleanly.
