## CO-172 — Fix quilombo forgot-password 404 (return_to allowlist sync)

The "Esqueci a senha" flow on quilomboaraucaria.org redirected to co's `/recover`
and then failed: the post-recovery redirect targeted `quilomboaraucaria.com.br`, a
**dead/unregistered domain**, and the client-side `isAllowedReturnTo` in `login.js`
listed `.com.br` but **not `.org`** (out of sync with the server safelist) — so a
corrected `.org` return_to would have been rejected client-side too.

- `login.js::isAllowedReturnTo` synced to the server: add `quilomboaraucaria.org`
  + the yggdrasil hosts, **remove the dead `quilomboaraucaria.com.br`**.
- `recovery_routes.rs::is_allowed_return_to` removed `quilomboaraucaria.com.br`
  (an allowlisted *unregistered* domain is an open-redirect/phishing vector if
  re-registered) + doc + test updated.

Companion frontend fix in `artelonga/quilomboaraucaria` points the recover links at
`https://quilomboaraucaria.org/auth/co-handover`, so after resetting the password
the user is redirected back and auto-logged-in (same path as the Google SSO flow).
