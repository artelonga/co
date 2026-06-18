## cli-email-login — `co login` by emailed magic-code (passwordless)

`co login` now defaults to **passwordless email login**: enter your email, a
6-digit code is sent to your inbox (`POST /api/v1/auth/login`), you type it in,
and you're authenticated (`/api/v1/auth/verify`) with a saved session/API token.
Works for brand-new signups too — no password to set first. `co login --password`
(and `co auth login --password`) keep the classic password flow.

### Why
The CLI previously only did password-login, which a magic-code signup user has no
password for — blocking the "install the CLI and sync" onboarding. This makes the
CLI usable by any user the moment they have an email, matching the web onboarding.
