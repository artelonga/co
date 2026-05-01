FROM clickhouse/clickhouse-server:24.8

# Server config: listen on all interfaces, iceberg table function enabled.
COPY infra/clickhouse/config/config.xml /etc/clickhouse-server/config.d/co-config.xml

# User config: co_admin (password from CH_ADMIN_PASSWORD Fly secret); default user
# restricted to localhost for the healthcheck only.
COPY infra/clickhouse/config/users.xml /etc/clickhouse-server/users.d/co-users.xml

# Init SQL runs once on first start via the ClickHouse entrypoint.
COPY infra/clickhouse/init/init.sql /docker-entrypoint-initdb.d/00-co-init.sql
