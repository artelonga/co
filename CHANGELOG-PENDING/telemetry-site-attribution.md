## telemetry-site-attribution — co-web routes bucket under "co"; drop scanner probes

Fixed the write-side `universe_key` pollution that made the per-site engagement
breakdown list co-web's own SPA routes and exploit-scanner probes as if they were
separate sites.

### What changed
- The server-side pageview middleware now resolves `universe_key` to a distinct
  site **only when the first path segment is a registered universe** (`/yuri/…`,
  `/grcsamazonia/…`). Every other route on co.artelonga.com.br (`/agora`,
  `/sala`, `/deployments`, `/recover`, `/entrar`, root) buckets under the
  platform key `"co"`. The full `path` is still stored, so top-pages keeps
  per-route detail.
- Exploit/vuln scanner probes are dropped before any telemetry is written
  (`/wp-admin`, `/wp-includes`, `/env`, `/cgi-bin`, `/credentials`, `/actuator`,
  `/phpmyadmin`, `/xmlrpc`, …). They reach the SPA shell with HTTP 200, so
  neither the User-Agent nor a status filter caught them.

### Why
With `universe_key = first-path-segment`, the network-wide breakdown surfaced
~40 bogus "sites" (SPA route names + bot probes), drowning the real surfaces.
Resolution is DB-driven (registered universes), so adding a universe needs no
code change. The site key is resolved under the storage lock already taken for
the insert — no new lock on the response hot path.

### Note
Historical events (last 30d) keep their old `universe_key`; they age out via
telemetry retention. New traffic is attributed correctly immediately.
