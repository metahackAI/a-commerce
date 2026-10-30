-- deploy/postgres/init.sql
-- Run once, as a superuser, in a new database before the migrations (for example from
-- /docker-entrypoint-initdb.d). Creates the three roles of docs/backend/database.md §8; the
-- migrations never create roles. The passwords below are for local development only: set real ones
-- with ALTER ROLE ... PASSWORD anywhere else, and keep them out of the repository.

-- acom_owner: runs migrations and owns every schema, table and view. Not a superuser, so the
-- FORCE ROW LEVEL SECURITY on each table applies to it too.
CREATE ROLE acom_owner LOGIN PASSWORD 'acom_owner' NOSUPERUSER NOCREATEROLE NOBYPASSRLS;

-- acom_app: what the commerce and agent services connect as. Row-level security always applies
-- (NOBYPASSRLS); each transaction sets app.tenant_id. It can read, insert and update, and may
-- delete only from the tables listed at the end of this file.
CREATE ROLE acom_app LOGIN PASSWORD 'acom_app' NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;

-- acom_readonly: insight queries and outside analysis tools; reads the analytics views only.
CREATE ROLE acom_readonly LOGIN PASSWORD 'acom_readonly' NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;

COMMENT ON ROLE acom_owner IS 'Owns the schema objects and runs migrations; subject to forced row-level security.';
COMMENT ON ROLE acom_app IS 'Service connections; row-level security by app.tenant_id; DELETE only on listed tables.';
COMMENT ON ROLE acom_readonly IS 'Read-only access to analytics views, scoped by app.tenant_id.';

-- pgvector usually needs a superuser to install; 0001_foundation.sql then finds it in place.
CREATE EXTENSION IF NOT EXISTS vector;

-- The local database is named acom (deploy/docker-compose.yml); change the name for other setups.
GRANT CONNECT ON DATABASE acom TO acom_app, acom_readonly;
GRANT CREATE ON DATABASE acom TO acom_owner;

-- Objects that acom_owner creates later (the migrations) get these grants automatically.
ALTER DEFAULT PRIVILEGES FOR ROLE acom_owner GRANT USAGE ON SCHEMAS TO acom_app;
ALTER DEFAULT PRIVILEGES FOR ROLE acom_owner GRANT SELECT, INSERT, UPDATE ON TABLES TO acom_app;
ALTER DEFAULT PRIVILEGES FOR ROLE acom_owner GRANT USAGE, SELECT ON SEQUENCES TO acom_app;

-- The analytics schema is created here so that acom_readonly can be limited to it. Its views run
-- with acom_owner's rights, so acom_readonly needs no grant on the tables behind them.
CREATE SCHEMA IF NOT EXISTS analytics AUTHORIZATION acom_owner;
GRANT USAGE ON SCHEMA analytics TO acom_app, acom_readonly;
ALTER DEFAULT PRIVILEGES FOR ROLE acom_owner IN SCHEMA analytics GRANT SELECT ON TABLES TO acom_readonly;

-- DELETE is granted per table after the migrations have run (the tables do not exist yet here):
--   GRANT DELETE ON sales.cart_line, customer.preference, platform.change_set_item,
--                   platform.session, platform.idempotency_key TO acom_app;
-- cart_line: remove_cart_item / clear_cart; preference: memory undo and remove; change_set_item:
-- items taken out of a draft change set; session and idempotency_key: expiry cleanup.
