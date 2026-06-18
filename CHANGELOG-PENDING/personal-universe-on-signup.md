## personal-universe-on-signup — every new user gets a private universe (CO-465)

On first login, a new user is automatically given **their own private personal
universe** (key = sanitised email local-part, collision-suffixed), owned by them,
with a default project + a welcome page. Private by default — only they see it.
Pairs with the existing public-subscribable discovery (`GET /api/v1/universes/public`):
a freshly-invited user lands in their own space *and* can browse + subscribe to
public universes.

### How
`Storage::ensure_personal_universe(user_id, email, display_name)` —
idempotent (no-op if they already own one), called best-effort from the magic-code
`verify` handler so it can never block login. Reuses `create_universe`
(private + owner membership + default project).
