# CO Deploy — Ansible Playbooks

Reproducible deployment of `co-web` to a VPS or Fly.io.

## Directory structure

```
co-deploy/
├── inventory/
│   ├── fly.yml           # Fly.io target (flyctl, local connection)
│   └── vps.yml           # Generic VPS (DigitalOcean, Hetzner, etc.)
├── playbooks/
│   ├── provision.yml     # One-time server setup
│   ├── deploy.yml        # Binary deploy + service restart
│   ├── backup.yml        # SQLite backup + rotation + cron
│   └── fly-deploy.yml    # Fly.io: backup → flyctl deploy → health check
├── templates/
│   ├── co-web.service.j2 # systemd unit
│   └── caddy.conf.j2     # Reverse proxy (auto-SSL, compression, security headers)
├── group_vars/
│   ├── all.yml           # Shared config (version, port, domain)
│   └── production.yml    # Secrets (ansible-vault encrypted)
├── molecule/
│   └── default/          # Docker-based integration test
└── requirements.yml      # Ansible Galaxy collections
```

## Prerequisites

```bash
pip install ansible molecule "molecule-plugins[docker]"
ansible-galaxy collection install -r co-deploy/requirements.yml
```

For cross-compilation (macOS host → Linux target):
```bash
cargo install cross
```

For Fly.io:
```bash
brew install flyctl
fly auth login
```

## Quickstart — VPS

### 1. Configure inventory

Set environment variables or edit `inventory/vps.yml`:

```bash
export CO_VPS_HOST=203.0.113.42
export CO_VPS_USER=root
export CO_SSH_KEY=~/.ssh/id_ed25519
```

### 2. Encrypt secrets

```bash
ansible-vault encrypt co-deploy/group_vars/production.yml
# Edit with: ansible-vault edit co-deploy/group_vars/production.yml
```

Set at minimum:
- `jwt_secret` — strong random string (≥ 32 chars)
- `resend_api_key` — from resend.com dashboard

### 3. Provision (once)

```bash
ansible-playbook -i co-deploy/inventory/vps.yml co-deploy/playbooks/provision.yml \
  --ask-vault-pass
```

Creates: `co` user, `/opt/co/`, `/var/lib/co/data/`, UFW rules, Caddy reverse proxy.

### 4. Deploy

```bash
ansible-playbook -i co-deploy/inventory/vps.yml co-deploy/playbooks/deploy.yml \
  --ask-vault-pass
```

Cross-compiles `co-web` for `x86_64-unknown-linux-musl`, copies binary, restarts systemd service, and verifies `/api/health`.

### 5. Backup (manual)

```bash
ansible-playbook -i co-deploy/inventory/vps.yml co-deploy/playbooks/backup.yml
```

Cron is installed automatically (daily at 03:00 UTC) on first run.

### Off-site backup (optional)

Set `backup_rclone_remote` in `group_vars/all.yml`:

```yaml
backup_rclone_remote: "b2:my-bucket/co-backups"
```

Configure rclone on the server: `rclone config`.

## Quickstart — Fly.io

```bash
export FLY_API_TOKEN=$(fly auth token)
ansible-playbook -i co-deploy/inventory/fly.yml co-deploy/playbooks/fly-deploy.yml
```

Runs: pre-deploy backup → `flyctl deploy --remote-only` → health check.

## Local testing (Molecule)

Requires Docker running locally.

```bash
cd co-deploy
molecule test
```

Tests: provision + stub deploy on a Debian 12 container, then verifies idempotency.

## Idempotency

All playbooks are safe to run multiple times. Running any playbook twice produces the same result — Ansible's declarative model ensures this for all tasks.
