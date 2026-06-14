# 🏁 Sprint Review — Template & Ritual
### a reusable, gamified end-of-sprint review · *copy this file, fill the blanks*

> A sprint review isn't a status meeting — it's a **moment of wonder**: look at what the
> universe built, measure it honestly, and leave more curious than you arrived. Optimize
> the ritual for **curiosity, inspiration, and play**. Keep it to 20 minutes.

---

## 0 · Setup (1 min)
- Copy this file to `docs/sprint/REVIEW-<date>.md`.
- Open the **SHANNON dashboard** (`/shannon`) — it's the scoreboard. Information shipped, in bits.
- Have the cross-repo release note open (`SPRINT-<date>.md`).

## 1 · 🎬 The Demo, not the Report (5 min)
Show, don't tell. For each shipped thing, **make it do something live** on screen.
- [ ] What can a user *do today* that they couldn't last sprint? (one sentence each)
- [ ] The single most **delightful** moment of the sprint — replay it.

## 2 · 📊 The Bits Ledger (3 min)
Measure the sprint in its true currency.
| Metric | This sprint | Source |
|---|---|---|
| Information shipped (bits) | ____ | SHANNON · commit entropy |
| Commits / repos touched | ____ | `git log` |
| Production releases | ____ | CHANGELOG |
| Outages / rollbacks | ____ | ops log |
| Redundancy of our own work (%) | ____ | SHANNON · "did we repeat ourselves?" |
| Migration frontier reached | v____ | migrations/ |

> *Curiosity prompt:* which number surprised you? Why?

## 3 · 🟢🟡🔴 The Honest Board (4 min)
- **🟢 Shipped & live** — deployed, smoke-tested, a user touched it.
- **🟡 Built, staged** — done but held for the batched deploy (say *why* held).
- **🔴 Caught before it shipped** — the bugs/divergences we *didn't* ship. **Celebrate these loudest** — a caught bug is a gift. (e.g. this sprint: the parallel-build surface divergence, the privilege-escalation in token scopes, the disk-at-85% gate.)

## 4 · 🧩 What We Learned (3 min)
Convert pain into durable rules (write them to memory / `CLAUDE.md`).
- [ ] One **gotcha** that cost time → the rule that prevents it next time.
- [ ] One thing that **worked surprisingly well** → do more of it.
- [ ] One **assumption that broke** when we looked closely.

## 5 · ✨ The Spark (2 min)
End on inspiration, not a backlog.
- [ ] One **wild idea** the sprint surfaced (this sprint: *Shannon as a playable character* in ÑE'Ẽ).
- [ ] What would make next sprint *fun*?
- [ ] Match the next sprint's ambition to the **available Claude budget** (session % + weekly %) — plan waves that fit the window, sequence the migrations, and pick one thing that makes you smile.

---

## 🎮 The Gamification Layer (optional, recommended)
Make the review a game you *want* to play:
- **🏆 Bug Bounty (internal):** the best **caught-before-shipped** finding wins the sprint. Honest > heroic.
- **📡 Bandwidth high score:** beat last sprint's bits-shipped (but watch redundancy — volume ≠ value).
- **🎵 Theme song:** every sprint gets a name and a vibe (this one: *"rock and roll all night, everything is bits"*).
- **🧭 Curiosity tax:** every review must surface **one question nobody can answer yet**. Carry it forward.

---

## Why this shape
A status meeting drains energy; a review should *restore* it. We measure honestly (the bits don't flatter us), we celebrate the bugs we caught more than the features we shipped, and we always leave with a spark and an open question. Build things that make people **curious**, and review them in a way that keeps **you** curious too.

> *"Information is the resolution of uncertainty."* — Shannon. A good sprint resolves a little
> uncertainty about the world, and opens a more interesting one.

---
*Template v1 · authored 2026-06-14 · first run: `SPRINT-2026-06-14.md`*
