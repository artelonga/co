FROM alpine:3.21

RUN apk add --no-cache \
    bash \
    busybox-suid \
    curl \
    python3

COPY scripts/wae-to-clickhouse.sh /scripts/wae-to-clickhouse.sh
RUN chmod +x /scripts/wae-to-clickhouse.sh

# Daily at 04:07 UTC — after the S3 backup cron (03:17) to avoid I/O contention.
RUN echo "7 4 * * * /scripts/wae-to-clickhouse.sh >> /var/log/wae-export.log 2>&1" \
    > /etc/crontabs/root

CMD ["crond", "-f", "-d", "8"]
