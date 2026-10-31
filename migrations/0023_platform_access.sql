-- 0023_platform_access.sql
-- Platform: who may do what in a store. Organizations operators belong to, the back-office menu
-- and each operator's favorites, API keys for outside agents and integration clients, and the
-- client sessions those keys sign in to (login, refresh_token, logout). Stores get a status and
-- feature flags (get_tenant). docs/api/platform.md §tenant, §role, §menu, §admin_user, §org,
-- §api_key, §auth.
-- Secrets are kept only as SHA-256 hashes: an API key's secret is shown once when it is issued,
-- a client's tokens once when they are handed out.
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security, grants.


ALTER TABLE platform.online_store ADD COLUMN store_status text NOT NULL DEFAULT 'active';
ALTER TABLE platform.online_store ADD CONSTRAINT online_store_store_status_check CHECK (store_status IN ('active', 'suspended'));
ALTER TABLE platform.online_store ADD COLUMN feature_flags jsonb NOT NULL DEFAULT '{}';

CREATE TABLE platform.organization (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  organization_type text NOT NULL,
  parent_organization_id text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT organization_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT organization_id_check CHECK (id ~ '^org_[a-z0-9_]+$'),
  CONSTRAINT organization_name_key UNIQUE (tenant_id, name),
  CONSTRAINT organization_organization_type_check CHECK (organization_type IN ('department', 'vendor', 'branch')),
  CONSTRAINT organization_parent_organization_id_fkey FOREIGN KEY (tenant_id, parent_organization_id) REFERENCES platform.organization (tenant_id, id),
  CONSTRAINT organization_parent_organization_id_check CHECK (parent_organization_id <> id)
);

ALTER TABLE platform.admin_user ADD COLUMN organization_id text;
ALTER TABLE platform.admin_user ADD CONSTRAINT admin_user_organization_id_fkey FOREIGN KEY (tenant_id, organization_id) REFERENCES platform.organization (tenant_id, id);

CREATE TABLE platform.site_navigation_element (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  url text NOT NULL,
  required_scope text NOT NULL,
  position integer NOT NULL DEFAULT 0,
  is_visible boolean NOT NULL DEFAULT true,
  is_part_of_id text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT site_navigation_element_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT site_navigation_element_id_check CHECK (id ~ '^mnu_[a-z0-9_]+$'),
  CONSTRAINT site_navigation_element_url_check CHECK (url ~ '^/'),
  CONSTRAINT site_navigation_element_required_scope_check CHECK (required_scope ~ '^[a-z_]+(:[a-z_*]+)+$'),
  CONSTRAINT site_navigation_element_is_part_of_id_fkey FOREIGN KEY (tenant_id, is_part_of_id) REFERENCES platform.site_navigation_element (tenant_id, id),
  CONSTRAINT site_navigation_element_is_part_of_id_check CHECK (is_part_of_id <> id)
);

CREATE TABLE platform.menu_favorite (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  admin_user_id text NOT NULL,
  site_navigation_element_id text NOT NULL,
  is_favorite boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT menu_favorite_pkey PRIMARY KEY (tenant_id, admin_user_id, site_navigation_element_id),
  CONSTRAINT menu_favorite_admin_user_id_fkey FOREIGN KEY (tenant_id, admin_user_id) REFERENCES platform.admin_user (tenant_id, id),
  CONSTRAINT menu_favorite_site_navigation_element_id_fkey FOREIGN KEY (tenant_id, site_navigation_element_id) REFERENCES platform.site_navigation_element (tenant_id, id)
);

CREATE TABLE platform.api_key (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  key_prefix text NOT NULL,
  secret_hash text NOT NULL,
  scopes text[] NOT NULL DEFAULT '{}',
  api_key_status text NOT NULL DEFAULT 'active',
  expires timestamptz,
  creator_id text,
  last_used_at timestamptz,
  revoked_at timestamptz,
  revocation_reason text,
  failed_attempt_count integer NOT NULL DEFAULT 0,
  first_failed_at timestamptz,
  locked_until timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT api_key_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT api_key_id_check CHECK (id ~ '^key_[a-z0-9_]+$'),
  CONSTRAINT api_key_secret_hash_key UNIQUE (tenant_id, secret_hash),
  CONSTRAINT api_key_secret_hash_check CHECK (secret_hash ~ '^[0-9a-f]{64}$'),
  CONSTRAINT api_key_api_key_status_check CHECK (api_key_status IN ('active', 'revoked')),
  CONSTRAINT api_key_creator_id_fkey FOREIGN KEY (tenant_id, creator_id) REFERENCES platform.admin_user (tenant_id, id),
  CONSTRAINT api_key_failed_attempt_count_check CHECK (failed_attempt_count >= 0)
);

ALTER TABLE platform.session ADD COLUMN scopes text[];
ALTER TABLE platform.session ADD COLUMN refresh_token_hash text;
ALTER TABLE platform.session ADD COLUMN refresh_expires_at timestamptz;
ALTER TABLE platform.session ADD COLUMN replaced_by_id text;
ALTER TABLE platform.session ADD CONSTRAINT session_refresh_token_hash_key UNIQUE (tenant_id, refresh_token_hash);
ALTER TABLE platform.session ADD CONSTRAINT session_replaced_by_id_fkey FOREIGN KEY (tenant_id, replaced_by_id) REFERENCES platform.session (tenant_id, id);

CREATE INDEX organization_parent_organization_id_idx ON platform.organization (tenant_id, parent_organization_id);
CREATE INDEX admin_user_organization_id_idx ON platform.admin_user (tenant_id, organization_id);
CREATE INDEX site_navigation_element_is_part_of_id_idx ON platform.site_navigation_element (tenant_id, is_part_of_id);
-- An active key's name is unique, so people can tell keys apart; a revoked key frees its name.
CREATE UNIQUE INDEX api_key_active_name_key ON platform.api_key (tenant_id, lower(name)) WHERE api_key_status = 'active';

COMMENT ON COLUMN platform.online_store.store_status IS
  'acom:storeStatus — whether the store is open for business; a suspended store keeps its data but serves no one. values: store_status';
COMMENT ON COLUMN platform.online_store.feature_flags IS
  'acom:featureFlags — features switched on for this store, as {flag: true|false}; {} when none.';

COMMENT ON TABLE platform.organization IS
  'schema:Organization — a department, vendor or branch that operator accounts belong to; organizations nest. id prefix org_. API: org.';
COMMENT ON COLUMN platform.organization.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.organization.id IS
  '- id with prefix org_; the same value the API, logs and events use (API: org_id).';
COMMENT ON COLUMN platform.organization.name IS
  'schema:name — organization name shown to administrators, unique in the store.';
COMMENT ON COLUMN platform.organization.organization_type IS
  'acom:organizationType — what kind of organization this is (API: org_type). values: organization_type';
COMMENT ON COLUMN platform.organization.parent_organization_id IS
  'schema:parentOrganization — the organization this one belongs to; NULL at the top. Never itself or one of its own sub-organizations (orgs.no_cycle).';
COMMENT ON COLUMN platform.organization.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.organization.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.organization.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON COLUMN platform.admin_user.organization_id IS
  'schema:worksFor — the organization (org_ id) the operator belongs to; NULL when none. API: org_id.';

COMMENT ON TABLE platform.site_navigation_element IS
  'schema:SiteNavigationElement — one entry of the back-office menu; entries nest one level under a parent. id prefix mnu_. API: menu.';
COMMENT ON COLUMN platform.site_navigation_element.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.site_navigation_element.id IS
  '- id with prefix mnu_; the same value the API, logs and events use (API: menu_id).';
COMMENT ON COLUMN platform.site_navigation_element.name IS
  'schema:name — the label operators see, for example Promotions.';
COMMENT ON COLUMN platform.site_navigation_element.url IS
  'schema:url — the back-office path the entry opens, for example /bo/promotions (API: path).';
COMMENT ON COLUMN platform.site_navigation_element.required_scope IS
  'acom:requiredScope — the scope an operator needs to see the entry, for example tools:read:promotion.';
COMMENT ON COLUMN platform.site_navigation_element.position IS
  'schema:position — order among the entries with the same parent, smallest first.';
COMMENT ON COLUMN platform.site_navigation_element.is_visible IS
  'acom:isVisible — false hides the entry from every operator without removing it.';
COMMENT ON COLUMN platform.site_navigation_element.is_part_of_id IS
  'schema:isPartOf — the parent entry (mnu_ id); NULL for a top-level entry.';
COMMENT ON COLUMN platform.site_navigation_element.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.site_navigation_element.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.site_navigation_element.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.menu_favorite IS
  '- whether an operator keeps a back-office menu entry among their favorites (set_favorite).';
COMMENT ON COLUMN platform.menu_favorite.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.menu_favorite.admin_user_id IS
  '- the operator account (adm_ id; API admin_id).';
COMMENT ON COLUMN platform.menu_favorite.site_navigation_element_id IS
  '- the menu entry (mnu_ id; API menu_id).';
COMMENT ON COLUMN platform.menu_favorite.is_favorite IS
  '- true while the entry is a favorite; turning it off keeps the row.';
COMMENT ON COLUMN platform.menu_favorite.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.menu_favorite.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.menu_favorite.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.api_key IS
  'schema:Intangible/acom:ApiKey — a credential an outside agent or integration client signs in with (login); the secret is shown once when issued and kept only as a hash. id prefix key_.';
COMMENT ON COLUMN platform.api_key.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.api_key.id IS
  '- id with prefix key_; the same value the API, logs and events use; the client_id at login.';
COMMENT ON COLUMN platform.api_key.name IS
  'schema:name — what the key is for, for example Outside shopping agent; unique among active keys.';
COMMENT ON COLUMN platform.api_key.key_prefix IS
  'acom:keyPrefix — the first characters of the secret, shown in lists so people can recognize a key.';
COMMENT ON COLUMN platform.api_key.secret_hash IS
  '- SHA-256 (hex) of the secret; the secret itself is never stored.';
COMMENT ON COLUMN platform.api_key.scopes IS
  'acom:scopes — the scopes a session of this key may get, for example {external:read,external:cart}.';
COMMENT ON COLUMN platform.api_key.api_key_status IS
  'acom:apiKeyStatus — whether the key can still sign in. values: api_key_status';
COMMENT ON COLUMN platform.api_key.expires IS
  'schema:expires — when the key stops working; NULL means it does not expire.';
COMMENT ON COLUMN platform.api_key.creator_id IS
  'schema:creator — the operator (adm_ id) who asked for the key.';
COMMENT ON COLUMN platform.api_key.last_used_at IS
  'acom:lastUsedAt — the last successful sign-in with the key; NULL if never.';
COMMENT ON COLUMN platform.api_key.revoked_at IS
  'acom:revokedAt — when the key was revoked; NULL while active.';
COMMENT ON COLUMN platform.api_key.revocation_reason IS
  'acom:revocationReason — why the key was revoked, in a sentence; NULL while active.';
COMMENT ON COLUMN platform.api_key.failed_attempt_count IS
  '- wrong secrets since first_failed_at; customer.yml password.lockout locks the key when it reaches failed_attempts.';
COMMENT ON COLUMN platform.api_key.first_failed_at IS
  '- the first wrong secret of the current lockout window; NULL when there is none.';
COMMENT ON COLUMN platform.api_key.locked_until IS
  'acom:lockedUntil — sign-in is refused until then after too many wrong secrets; NULL when not locked.';
COMMENT ON COLUMN platform.api_key.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.api_key.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.api_key.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON COLUMN platform.session.subject_id IS
  '- who holds the session: a person (adm_ or cus_ id), an API client (key_ id), or NULL for an anonymous guest.';
COMMENT ON COLUMN platform.session.scopes IS
  '- scopes of an API client session, fixed at login; NULL for a browser session, whose scopes come from roles.';
COMMENT ON COLUMN platform.session.refresh_token_hash IS
  '- SHA-256 (hex) of an API client session''s refresh token; NULL for a browser session.';
COMMENT ON COLUMN platform.session.refresh_expires_at IS
  '- when the refresh token stops being accepted; NULL for a browser session.';
COMMENT ON COLUMN platform.session.replaced_by_id IS
  '- the session its refresh token was exchanged for; a refresh token that comes back after that is reuse and ends every session of the client.';

ALTER TABLE platform.organization ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.organization FORCE ROW LEVEL SECURITY;
CREATE POLICY organization_tenant_isolation ON platform.organization
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.site_navigation_element ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.site_navigation_element FORCE ROW LEVEL SECURITY;
CREATE POLICY site_navigation_element_tenant_isolation ON platform.site_navigation_element
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.menu_favorite ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.menu_favorite FORCE ROW LEVEL SECURITY;
CREATE POLICY menu_favorite_tenant_isolation ON platform.menu_favorite
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.api_key ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.api_key FORCE ROW LEVEL SECURITY;
CREATE POLICY api_key_tenant_isolation ON platform.api_key
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- DELETE for the service role (acom_app), where undoing a change set removes what it made:
--   platform.admin_user_role       revoking a role (assign_role), undoing a grant
--   platform.role, platform.role_scope
--                                  undoing create_role while nobody holds the role
--   platform.admin_user            undoing create_admin_user while the account never signed in
--   platform.organization          undoing set_org that created an organization nobody belongs to
--   platform.site_navigation_element, platform.menu_favorite
--                                  undoing set_menu that created an entry
GRANT DELETE ON
  platform.admin_user_role,
  platform.role,
  platform.role_scope,
  platform.admin_user,
  platform.organization,
  platform.site_navigation_element,
  platform.menu_favorite
TO acom_app;
