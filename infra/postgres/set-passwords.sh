#!/bin/sh
# Runs once, after init.sql, when the database volume is first created (deployed stack only,
# infra/compose/compose.deploy.yaml). Replaces the development passwords from init.sql with the
# ones in the server's .env, so they never live in the repository.
set -eu
psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" \
  -v owner="$ACOM_OWNER_PASSWORD" -v app="$ACOM_APP_PASSWORD" <<'SQL'
ALTER ROLE acom_owner PASSWORD :'owner';
ALTER ROLE acom_app PASSWORD :'app';
SQL
