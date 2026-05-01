FROM alpine:3.21

# busybox-suid provides crond
RUN apk add --no-cache \
    bash \
    busybox-suid \
    curl \
    aws-cli \
    sqlite \
    tar

# Install flyctl
RUN curl -L https://fly.io/install.sh | sh
ENV PATH="/root/.fly/bin:$PATH"

# Build context must be the repo root:
#   docker build -f infra/backup-cron/Dockerfile -t co-backup-cron .
COPY scripts/backup-prod.sh /scripts/backup-prod.sh
COPY infra/backup-cron/entrypoint.sh /entrypoint.sh
RUN chmod +x /scripts/backup-prod.sh /entrypoint.sh

# Daily at 03:17 UTC — off-minute to spread load.
RUN echo "17 3 * * * /scripts/backup-prod.sh >> /var/log/backup.log 2>&1" \
    > /etc/crontabs/root

# crond in foreground (-f), syslog level 8 = debug.
CMD ["crond", "-f", "-d", "8"]
