# Self-Hosting CO

Run the CO platform (`co-web`) on your own machine — a home Mac, a mini, a VPS —
instead of (or alongside) the managed Fly deploy.

> **Parallel path, same code (CO-497).** Self-host is NOT a fork or a stripped
> build: it runs the *same* `target/release/co-web` binary the Fly image runs.
> Fly (`fly.toml`) stays the managed option; self-host is the do-it-yourself
> option. Whatever works on one works on the other — the only difference is who
> runs the process and where the `$CO_WEB_DATA` directory lives.

The kit lives in [`scripts/selfhost/`](../scripts/selfhost/):

| File | Purpose |
|------|---------|
| `run.sh` | Build + boot co-web with the right env (network or `--localhost`). |
| `com.artelonga.co.plist` | launchd LaunchAgent — keep it running, restart on crash. |
| `cloudflared-config.example.yml` | CGNAT-proof public HTTPS via Cloudflare Tunnel (+ a Caddy alternative). |
| `litestream.yml` | Continuous off-box backup of `meta.db` (+ companion per-universe snapshot). |
| `verify-restore.sh` | Prove the backup actually restores (read-only). |

---

## 1. Prerequisites

- **Rust toolchain** (`cargo`/`rustc`) to build co-web — `rustup` or Homebrew.
  *(When CO-484 ships a prebuilt binary, you can skip the toolchain and drop the
  binary in `target/release/co-web` + run with `--no-build`.)*
- **`cloudflared`** if you want public HTTPS from behind home CGNAT
  (`brew install cloudflared`). Skip if you only need LAN access or you have a
  public IP and use Caddy.
- **`litestream`** for off-box backups (`brew install litestream`).
- **A B2 or S3 bucket** for the backup replica (Backblaze B2 is cheapest).
- **A UPS.** SQLite + a sudden power loss is the classic corruption path; a UPS
  (or a laptop's own battery) turns "yank the cord" into a clean shutdown.
- **`JWT_SECRET`** — a stable HS256 signing key. Generate once and keep it:
  `openssl rand -base64 48`. co-web (CO-469) **panics at boot** in a prod env if
  this is unset or left at the insecure dev fallback — every session would
  otherwise be forgeable.

---

## 2. First deploy

```bash
# 1. Clone (or pull) the repo
git clone https://github.com/artelonga/co.git ~/projects/co
cd ~/projects/co

# 2. Make a STABLE secret and keep it out of the repo (0600 file)
mkdir -p ~/.co && umask 077
printf 'export JWT_SECRET=%s\n' "$(openssl rand -base64 48)" > ~/.co/secrets.env
chmod 600 ~/.co/secrets.env
source ~/.co/secrets.env

# 3. (Optional) seed an admin user for password-login + the admin dashboard.
#    Generate the argon2 hash locally:
HASH=$(printf 'myStrongPassword' | argon2 "$(openssl rand -hex 16)" -id -t 3 -m 16 -p 1 -e)
export CO_SEED_ADMIN_EMAIL=you@example.com
export CO_SEED_ADMIN_PASSWORD_HASH="$HASH"

# 4. Build + boot (defaults: data ~/.co/data, port 8742, bind 0.0.0.0, env prod)
scripts/selfhost/run.sh

# 5. In another shell: verify the served surface has no draft-leak (CO-439)
cargo run -p co-web --bin audit_serve -- ~/.co/data
#    → exit 0 = clean; non-zero lists files on disk but not in the published index.

# 6. Health check
curl -s http://localhost:8742/api/health
#    → {"status":"ok","version":"...","env":"production"}
```

Data lands in `$CO_WEB_DATA` (default `~/.co/data`) with the CO-77 layout:

```
~/.co/data/
├── meta.db                       # global: universes, users, schema_version, …
└── universes/<key>/data.db       # per-universe content (WAL)
```

### Keep it running (launchd)

```bash
# Edit the plist: replace every <USER> with your `whoami`, fix the repo path.
cp scripts/selfhost/com.artelonga.co.plist ~/Library/LaunchAgents/
launchctl load -w ~/Library/LaunchAgents/com.artelonga.co.plist

# Manage
launchctl list | grep artelonga.co
launchctl kickstart -k gui/$(id -u)/com.artelonga.co     # restart
tail -f ~/Library/Logs/co/web.log                        # logs
```

Provide `JWT_SECRET` to launchd via the 0600 file (source it from `run.sh`, e.g.
add `[ -f ~/.co/secrets.env ] && . ~/.co/secrets.env` near the top) rather than
inlining the secret in the plist — see the plist header for both options.

> A LaunchAgent runs only while you're logged in. For headless 24/7 operation,
> also keep the Mac awake: `caffeinate -dimsu` (foreground) or
> `sudo pmset -a sleep 0 disablesleep 1`.

---

## 3. Network reach

co-web picks its bind host from `CO_WEB_HOST`; when unset it derives from
`CO_ENV` (`local` → `127.0.0.1`, anything else → `0.0.0.0`). `run.sh` sets these
for you:

| Goal | How | Bind |
|------|-----|------|
| Public site (home/CGNAT) | `run.sh` (default) + **Cloudflare Tunnel** | `0.0.0.0`, tunnel does TLS |
| Public site (public IP) | `run.sh` + **Caddy** reverse proxy | `127.0.0.1` (Caddy fronts it) |
| LAN only | `run.sh` (default) reachable on the LAN | `0.0.0.0` |
| Loopback only | `run.sh --localhost` | `127.0.0.1` |

### Cloudflare Tunnel (CGNAT-proof, recommended for home)

A home Mac has no public IP and can't port-forward. A tunnel makes an *outbound*
connection to Cloudflare's edge, which proxies public HTTPS back down to
`localhost:8742` — public reach **and** managed TLS with **zero inbound ports
open**. Full setup (login → create → route-dns → service install) is in the
header of [`cloudflared-config.example.yml`](../scripts/selfhost/cloudflared-config.example.yml).

### Caddy (only with a public IP)

If you can forward inbound 80/443, Caddy gives auto Let's Encrypt TLS without
Cloudflare. Minimal Caddyfile + commands are in the "Alternative" block at the
bottom of the same example file. With Caddy you can run co-web loopback-only
(`run.sh --localhost`).

### LAN-only hardening

If you don't expose the site, bind loopback (`--localhost`) or firewall the port
so only your LAN can reach it. See the [LAN hardening pointer](#6-lan-hardening)
below.

---

## 4. Backup & restore

The machine is a single point of failure; SQLite is the operational source of
truth. Use Litestream for the global DB plus a periodic snapshot for the dynamic
per-universe DBs.

```bash
# Install + configure (edit placeholders: <USER>, <BUCKET>, region/endpoint)
brew install litestream
export LITESTREAM_ACCESS_KEY_ID=...        # bucket creds via env, not in the file
export LITESTREAM_SECRET_ACCESS_KEY=...
sudo cp scripts/selfhost/litestream.yml /etc/litestream.yml
sudo brew services start litestream         # streams meta.db continuously
```

> **CO-77 nuance.** Litestream needs an *explicit path per database* and the
> per-universe `universes/<key>/data.db` files appear **dynamically** as you
> create universes. So `litestream.yml` covers **`meta.db`** directly and
> documents a **companion tar/rsync** of the whole `universes/` tree (cron /
> launchd) for the per-universe DBs. For a small, stable set of universes you can
> instead add one `dbs:` entry per universe. Details + example cron are in the
> file header.

Prove the backup restores (do this on a schedule — an unverified backup is a
guess):

```bash
scripts/selfhost/verify-restore.sh          # restores meta.db to a temp file,
                                            # PRAGMA integrity_check, PASS/FAIL
```

Disaster recovery on a fresh machine:

```bash
litestream restore -config scripts/selfhost/litestream.yml ~/.co/data/meta.db
# then extract your latest universes/ snapshot into ~/.co/data/universes/
scripts/selfhost/run.sh
```

---

## 5. Upgrade

```bash
# 1. BACK UP FIRST — confirm the replica is healthy before changing anything
scripts/selfhost/verify-restore.sh

# 2. Pull the new code
git -C ~/projects/co pull

# 3. CO-446 disk pre-flight — a migration that can't write schema_version on a
#    near-full disk crash-loops the server. Make sure $CO_WEB_DATA's volume has
#    headroom (rule of thumb: keep it < 85% full) before upgrading.
df -h ~/.co/data

# 4. Rebuild + restart (migrations run automatically at boot)
launchctl kickstart -k gui/$(id -u)/com.artelonga.co      # plist runs run.sh
#    or, without launchd:
scripts/selfhost/run.sh

# 5. Re-check health + leak surface
curl -s http://localhost:8742/api/health
cargo run -p co-web --bin audit_serve -- ~/.co/data
```

Migrations are applied automatically on boot (source of truth:
`co-web/src/storage/migrations/`). Because they mutate the DB, the
verify-restore in step 1 is your rollback insurance.

---

## 6. LAN hardening

For binding, firewall rules, kill-switch, and exposure-scan guidance on a
self-hosted box, see **`infra/SELF-HOST-SECURITY.md`** (the security kit:
`pf` firewall rules, kill-switch, exposure-scan e2e guard — born from the
`.git`-over-`:8000` postmortem). co-web binding `0.0.0.0` is for reachability,
not a security boundary — the firewall and tunnel/proxy in front of it are.

---

## See also

- Managed deploy (Fly): [`CLAUDE.md` → Deployment](../CLAUDE.md) and `fly.toml`.
- Operations runbook: [`docs/OPERATIONS.md`](OPERATIONS.md) (disk-full recovery,
  smoke checks — the self-host disk pre-flight mirrors CO-446 there).
- Local development (not a deploy): [`docs/LOCAL-DEV.md`](LOCAL-DEV.md).
