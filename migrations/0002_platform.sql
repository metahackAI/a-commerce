-- 0002_platform.sql
-- Platform: stores (tenants), operators, roles and sessions, policy overrides, previews,
-- change sets, the action log, operator tasks, the job queue, the outbox, idempotency keys and the demo clock.
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security.


CREATE TABLE platform.online_store (
  id text NOT NULL,
  name text NOT NULL,
  url text NOT NULL,
  currency char(3) NOT NULL,
  time_zone text NOT NULL,
  in_language text NOT NULL DEFAULT 'en',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT online_store_pkey PRIMARY KEY (id),
  CONSTRAINT online_store_id_check CHECK (id ~ '^store_[a-z0-9_]+$'),
  CONSTRAINT online_store_currency_check CHECK (currency ~ '^[A-Z]{3}$')
);

CREATE TABLE platform.admin_user (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  alternate_name text,
  email text NOT NULL,
  job_title text,
  admin_status text NOT NULL DEFAULT 'active',
  login_id text,
  password_hash text,
  last_login_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT admin_user_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT admin_user_id_check CHECK (id ~ '^adm_[a-z0-9_]+$'),
  CONSTRAINT admin_user_email_key UNIQUE (tenant_id, email),
  CONSTRAINT admin_user_login_id_key UNIQUE (tenant_id, login_id),
  CONSTRAINT admin_user_login_id_check CHECK (login_id ~ '^[a-z][a-z0-9_.-]{2,63}$'),
  CONSTRAINT admin_user_admin_status_check CHECK (admin_status IN ('active', 'suspended', 'locked'))
);

CREATE TABLE platform.role (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  description text,
  approver_roles text[] NOT NULL DEFAULT '{}',
  is_system boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT role_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT role_id_check CHECK (id ~ '^rol_[a-z0-9_]+$'),
  CONSTRAINT role_name_key UNIQUE (tenant_id, name)
);

CREATE TABLE platform.role_scope (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  role_id text NOT NULL,
  scope text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT role_scope_pkey PRIMARY KEY (tenant_id, role_id, scope),
  CONSTRAINT role_scope_role_id_fkey FOREIGN KEY (tenant_id, role_id) REFERENCES platform.role (tenant_id, id),
  CONSTRAINT role_scope_scope_check CHECK (scope ~ '^[a-z_]+(:[a-z_*]+)+$')
);

CREATE TABLE platform.admin_user_role (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  admin_user_id text NOT NULL,
  role_id text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT admin_user_role_pkey PRIMARY KEY (tenant_id, admin_user_id, role_id),
  CONSTRAINT admin_user_role_admin_user_id_fkey FOREIGN KEY (tenant_id, admin_user_id) REFERENCES platform.admin_user (tenant_id, id),
  CONSTRAINT admin_user_role_role_id_fkey FOREIGN KEY (tenant_id, role_id) REFERENCES platform.role (tenant_id, id)
);

CREATE TABLE platform.session (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  actor_kind text NOT NULL,
  subject_id text,
  token_hash text NOT NULL,
  csrf_token_hash text NOT NULL,
  expires_at timestamptz NOT NULL,
  last_seen_at timestamptz,
  revoked_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT session_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT session_id_check CHECK (id ~ '^ses_[a-z0-9_]+$'),
  CONSTRAINT session_token_hash_key UNIQUE (tenant_id, token_hash),
  CONSTRAINT session_actor_kind_check CHECK (actor_kind IN ('operator', 'shopper', 'agent_on_behalf', 'external_agent', 'system'))
);

CREATE TABLE platform.policy_override (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  file text NOT NULL,
  partial jsonb NOT NULL,
  policy_version integer NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT policy_override_pkey PRIMARY KEY (tenant_id, file),
  CONSTRAINT policy_override_file_check CHECK (file ~ '^[a-z_]+$'),
  CONSTRAINT policy_override_policy_version_check CHECK (policy_version >= 1)
);

CREATE TABLE platform.change_set (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  number integer NOT NULL,
  title text NOT NULL,
  change_set_state text NOT NULL DEFAULT 'drafting',
  request_text text[] NOT NULL,
  interpretation jsonb NOT NULL DEFAULT '[]',
  conversation_id text,
  created_by text NOT NULL,
  agent_name text,
  submitted_at timestamptz,
  approved_by text,
  approved_at timestamptz,
  rejected_at timestamptz,
  rejection_reason text,
  verified_at timestamptz,
  verification jsonb,
  undo_until timestamptz,
  undone_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT change_set_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT change_set_id_check CHECK (id ~ '^cs_[a-z0-9_]+$'),
  CONSTRAINT change_set_number_key UNIQUE (tenant_id, number),
  CONSTRAINT change_set_number_check CHECK (number >= 1),
  CONSTRAINT change_set_change_set_state_check CHECK (change_set_state IN ('drafting', 'waiting_for_approval', 'running', 'verifying', 'live', 'failed', 'rejected', 'undone'))
);

CREATE TABLE platform.preview (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  operation text NOT NULL,
  instrument text NOT NULL,
  input jsonb NOT NULL,
  input_hash text NOT NULL,
  plan jsonb NOT NULL,
  changes jsonb NOT NULL,
  checks jsonb NOT NULL DEFAULT '[]',
  needs_approval boolean NOT NULL,
  approval_reason text,
  basis jsonb NOT NULL DEFAULT '[]',
  policy_version text NOT NULL,
  actor_kind text NOT NULL,
  agent_id text,
  agent_name text,
  request_id text,
  change_set_id text,
  preview_state text NOT NULL DEFAULT 'open',
  expires_at timestamptz,
  superseded_by_id text,
  committed_at timestamptz,
  result jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT preview_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT preview_id_check CHECK (id ~ '^pv_[a-z0-9_]+$'),
  CONSTRAINT preview_change_set_id_fkey FOREIGN KEY (tenant_id, change_set_id) REFERENCES platform.change_set (tenant_id, id),
  CONSTRAINT preview_superseded_by_id_fkey FOREIGN KEY (tenant_id, superseded_by_id) REFERENCES platform.preview (tenant_id, id),
  CONSTRAINT preview_actor_kind_check CHECK (actor_kind IN ('operator', 'shopper', 'agent_on_behalf', 'external_agent', 'system')),
  CONSTRAINT preview_preview_state_check CHECK (preview_state IN ('open', 'committed', 'expired', 'superseded'))
);

CREATE TABLE platform.change_set_item (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  change_set_id text NOT NULL,
  position integer NOT NULL,
  preview_id text NOT NULL,
  depends_on text[] NOT NULL DEFAULT '{}',
  result_id text,
  verification jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT change_set_item_pkey PRIMARY KEY (tenant_id, change_set_id, position),
  CONSTRAINT change_set_item_change_set_id_preview_id_key UNIQUE (tenant_id, change_set_id, preview_id),
  CONSTRAINT change_set_item_preview_id_fkey FOREIGN KEY (tenant_id, preview_id) REFERENCES platform.preview (tenant_id, id),
  CONSTRAINT change_set_item_position_check CHECK (position >= 1)
);

CREATE TABLE platform.change_set_event (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  change_set_id text NOT NULL,
  position integer NOT NULL,
  action_type text NOT NULL,
  action_status text NOT NULL,
  agent_id text,
  agent_name text,
  instrument text,
  from_state text,
  to_state text,
  result jsonb,
  error jsonb,
  start_time timestamptz NOT NULL,
  end_time timestamptz,
  request_id text,
  change_set_version integer NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT change_set_event_pkey PRIMARY KEY (tenant_id, change_set_id, position),
  CONSTRAINT change_set_event_position_check CHECK (position >= 1),
  CONSTRAINT change_set_event_action_status_check CHECK (action_status IN ('potential', 'active', 'completed', 'failed')),
  CONSTRAINT change_set_event_from_state_check CHECK (from_state IN ('drafting', 'waiting_for_approval', 'running', 'verifying', 'live', 'failed', 'rejected', 'undone')),
  CONSTRAINT change_set_event_to_state_check CHECK (to_state IN ('drafting', 'waiting_for_approval', 'running', 'verifying', 'live', 'failed', 'rejected', 'undone')),
  CONSTRAINT change_set_event_change_set_version_check CHECK (change_set_version >= 1)
);

CREATE TABLE platform.action_log (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  action_type text NOT NULL,
  action_status text NOT NULL,
  actor_kind text NOT NULL,
  agent_id text,
  agent_name text,
  instrument text NOT NULL,
  object_type text,
  object_id text,
  result jsonb,
  error jsonb,
  start_time timestamptz NOT NULL,
  end_time timestamptz,
  change_set_id text,
  preview_id text,
  policy_version text,
  request_id text,
  usage jsonb,
  latency_ms integer,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT action_log_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT action_log_id_check CHECK (id ~ '^act_[a-z0-9_]+$'),
  CONSTRAINT action_log_change_set_id_fkey FOREIGN KEY (tenant_id, change_set_id) REFERENCES platform.change_set (tenant_id, id),
  CONSTRAINT action_log_preview_id_fkey FOREIGN KEY (tenant_id, preview_id) REFERENCES platform.preview (tenant_id, id),
  CONSTRAINT action_log_action_status_check CHECK (action_status IN ('potential', 'active', 'completed', 'failed')),
  CONSTRAINT action_log_actor_kind_check CHECK (actor_kind IN ('operator', 'shopper', 'agent_on_behalf', 'external_agent', 'system')),
  CONSTRAINT action_log_latency_ms_check CHECK (latency_ms >= 0)
);

CREATE TABLE platform.task (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  title text NOT NULL,
  task_state text NOT NULL DEFAULT 'needs_you',
  task_origin text NOT NULL,
  summary text,
  tag text,
  request_text text,
  watcher text,
  watcher_observation jsonb,
  related_entities jsonb NOT NULL DEFAULT '[]',
  change_set_id text,
  conversation_id text,
  start_time timestamptz NOT NULL,
  due_at timestamptz,
  end_time timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT task_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT task_id_check CHECK (id ~ '^task_[a-z0-9_]+$'),
  CONSTRAINT task_change_set_id_fkey FOREIGN KEY (tenant_id, change_set_id) REFERENCES platform.change_set (tenant_id, id),
  CONSTRAINT task_task_state_check CHECK (task_state IN ('needs_you', 'working', 'done', 'dismissed')),
  CONSTRAINT task_task_origin_check CHECK (task_origin IN ('you_asked', 'agent_noticed', 'automation'))
);

CREATE TABLE platform.job (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  kind text NOT NULL,
  run_at timestamptz NOT NULL,
  payload jsonb NOT NULL DEFAULT '{}',
  attempts integer NOT NULL DEFAULT 0,
  locked_at timestamptz,
  done_at timestamptz,
  last_error text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT job_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT job_id_check CHECK (id ~ '^job_[a-z0-9_]+$'),
  CONSTRAINT job_attempts_check CHECK (attempts >= 0)
);

CREATE TABLE platform.outbox (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  position bigint GENERATED ALWAYS AS IDENTITY,
  topic text NOT NULL,
  object_id text,
  payload jsonb NOT NULL,
  published_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT outbox_pkey PRIMARY KEY (tenant_id, position)
);

CREATE TABLE platform.idempotency_key (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  key text NOT NULL,
  endpoint text NOT NULL,
  subject_id text,
  request_hash text NOT NULL,
  response_status integer,
  response jsonb,
  expires_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT idempotency_key_pkey PRIMARY KEY (tenant_id, key),
  CONSTRAINT idempotency_key_response_status_check CHECK (response_status BETWEEN 100 AND 599)
);

CREATE TABLE platform.demo_clock (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  now timestamptz NOT NULL,
  is_running boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT demo_clock_pkey PRIMARY KEY (tenant_id)
);

CREATE INDEX admin_user_role_role_id_idx ON platform.admin_user_role (tenant_id, role_id);
CREATE INDEX session_subject_id_idx ON platform.session (tenant_id, subject_id);
CREATE INDEX session_expires_at_idx ON platform.session (tenant_id, expires_at);
CREATE INDEX change_set_change_set_state_idx ON platform.change_set (tenant_id, change_set_state);
CREATE INDEX change_set_created_by_idx ON platform.change_set (tenant_id, created_by);
CREATE INDEX change_set_conversation_id_idx ON platform.change_set (tenant_id, conversation_id);
CREATE INDEX preview_preview_state_idx ON platform.preview (tenant_id, preview_state);
CREATE INDEX preview_expires_at_idx ON platform.preview (tenant_id, expires_at);
CREATE INDEX preview_agent_id_idx ON platform.preview (tenant_id, agent_id);
CREATE INDEX preview_change_set_id_idx ON platform.preview (tenant_id, change_set_id);
CREATE INDEX preview_superseded_by_id_idx ON platform.preview (tenant_id, superseded_by_id);
CREATE INDEX change_set_item_preview_id_idx ON platform.change_set_item (tenant_id, preview_id);
CREATE INDEX change_set_event_start_time_idx ON platform.change_set_event (tenant_id, start_time);
CREATE INDEX action_log_start_time_idx ON platform.action_log (tenant_id, start_time);
CREATE INDEX action_log_object_id_idx ON platform.action_log (tenant_id, object_id);
CREATE INDEX action_log_instrument_idx ON platform.action_log (tenant_id, instrument);
CREATE INDEX action_log_agent_id_idx ON platform.action_log (tenant_id, agent_id);
CREATE INDEX action_log_request_id_idx ON platform.action_log (tenant_id, request_id);
CREATE INDEX action_log_change_set_id_idx ON platform.action_log (tenant_id, change_set_id);
CREATE INDEX action_log_preview_id_idx ON platform.action_log (tenant_id, preview_id);
CREATE INDEX task_task_state_idx ON platform.task (tenant_id, task_state);
CREATE INDEX task_start_time_idx ON platform.task (tenant_id, start_time);
CREATE INDEX task_change_set_id_idx ON platform.task (tenant_id, change_set_id);
CREATE INDEX job_run_at_idx ON platform.job (tenant_id, run_at);
CREATE INDEX job_done_at_idx ON platform.job (tenant_id, done_at);
CREATE INDEX outbox_published_at_idx ON platform.outbox (tenant_id, published_at);
CREATE INDEX idempotency_key_expires_at_idx ON platform.idempotency_key (tenant_id, expires_at);

COMMENT ON TABLE platform.online_store IS
  'schema:OnlineStore — one store; every other row belongs to exactly one store (tenant). id prefix store_. API: tenant.';
COMMENT ON COLUMN platform.online_store.id IS
  '- store id, prefix store_; the tenant claim in internal tokens and the tenant_id of every other row.';
COMMENT ON COLUMN platform.online_store.name IS
  'schema:name — store name shown to shoppers and operators.';
COMMENT ON COLUMN platform.online_store.url IS
  'schema:url — the storefront address, for example northwind.example.';
COMMENT ON COLUMN platform.online_store.currency IS
  'schema:currency — ISO 4217 code of the store currency; all prices and amounts use it.';
COMMENT ON COLUMN platform.online_store.time_zone IS
  'acom:timeZone — IANA time zone that store times and dates are shown in, for example America/New_York.';
COMMENT ON COLUMN platform.online_store.in_language IS
  'schema:inLanguage — BCP 47 language of store text, for example en.';
COMMENT ON COLUMN platform.online_store.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.online_store.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.online_store.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.admin_user IS
  'schema:Person — an operator account that signs in to the back office. id prefix adm_. API: admin_id.';
COMMENT ON COLUMN platform.admin_user.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.admin_user.id IS
  '- id with prefix adm_; the same value the API, logs and events use.';
COMMENT ON COLUMN platform.admin_user.name IS
  'schema:name — full name shown in change sets and the action log.';
COMMENT ON COLUMN platform.admin_user.alternate_name IS
  'schema:alternateName — short name colleagues use, for example SJ.';
COMMENT ON COLUMN platform.admin_user.email IS
  'schema:email — sign-in email address, unique in the store.';
COMMENT ON COLUMN platform.admin_user.job_title IS
  'schema:jobTitle — job title shown next to the name.';
COMMENT ON COLUMN platform.admin_user.admin_status IS
  'acom:adminStatus — whether the account can sign in. values: admin_status';
COMMENT ON COLUMN platform.admin_user.login_id IS
  'acom:loginId — what the operator types to sign in (for example admin); NULL means the account cannot sign in with a password.';
COMMENT ON COLUMN platform.admin_user.password_hash IS
  '- argon2 hash of the password; NULL until the person sets one (demo login does not use it).';
COMMENT ON COLUMN platform.admin_user.last_login_at IS
  'acom:lastLoginAt — last successful sign-in; NULL if never.';
COMMENT ON COLUMN platform.admin_user.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.admin_user.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.admin_user.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.role IS
  'schema:Role — a named bundle of scopes that the gateway puts in operator tokens. id prefix rol_.';
COMMENT ON COLUMN platform.role.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.role.id IS
  '- id with prefix rol_; the same value the API, logs and events use.';
COMMENT ON COLUMN platform.role.name IS
  'schema:name — role name shown to administrators, for example Merchandiser.';
COMMENT ON COLUMN platform.role.description IS
  'schema:description — what people with this role do.';
COMMENT ON COLUMN platform.role.approver_roles IS
  'acom:approverRoles — approver_role values from approval.yml thresholds this role may approve, for example {merchandiser,operator}.';
COMMENT ON COLUMN platform.role.is_system IS
  'acom:isSystem — true for built-in roles that cannot be edited.';
COMMENT ON COLUMN platform.role.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.role.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.role.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.role_scope IS
  '- one scope granted by a role (gateway.md §2); the gateway reads these to build tokens.';
COMMENT ON COLUMN platform.role_scope.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.role_scope.role_id IS
  '- role that grants the scope (rol_ id).';
COMMENT ON COLUMN platform.role_scope.scope IS
  '- scope string, for example tools:read:promotion or human:approve.';
COMMENT ON COLUMN platform.role_scope.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.role_scope.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.role_scope.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.admin_user_role IS
  '- assignment of a role to an operator account.';
COMMENT ON COLUMN platform.admin_user_role.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.admin_user_role.admin_user_id IS
  '- operator account (adm_ id; API admin_id).';
COMMENT ON COLUMN platform.admin_user_role.role_id IS
  '- role given to the account (rol_ id).';
COMMENT ON COLUMN platform.admin_user_role.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.admin_user_role.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.admin_user_role.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.session IS
  '- a signed-in browser session held by the gateway (operator, shopper or guest). id prefix ses_.';
COMMENT ON COLUMN platform.session.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.session.id IS
  '- id with prefix ses_; the same value the API, logs and events use.';
COMMENT ON COLUMN platform.session.actor_kind IS
  '- who holds the session. values: actor_kind';
COMMENT ON COLUMN platform.session.subject_id IS
  '- person behind the session (adm_ or cus_ id); NULL for an anonymous guest.';
COMMENT ON COLUMN platform.session.token_hash IS
  '- hash of the session cookie value; the cookie itself is never stored.';
COMMENT ON COLUMN platform.session.csrf_token_hash IS
  '- hash of the CSRF token paired with the session.';
COMMENT ON COLUMN platform.session.expires_at IS
  '- when the session stops being accepted.';
COMMENT ON COLUMN platform.session.last_seen_at IS
  '- last request made with the session.';
COMMENT ON COLUMN platform.session.revoked_at IS
  '- when the session was signed out or revoked; NULL while active.';
COMMENT ON COLUMN platform.session.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.session.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.session.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.policy_override IS
  '- store-specific values deep-merged over one policies/*.yml file (commerce.md §4).';
COMMENT ON COLUMN platform.policy_override.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.policy_override.file IS
  '- policy file name without .yml, for example promotion.';
COMMENT ON COLUMN platform.policy_override.partial IS
  '- the partial YAML document as JSON, same key paths as the file, for example {max_discount: {by_tier: {vip: 0.30}}}.';
COMMENT ON COLUMN platform.policy_override.policy_version IS
  '- version of the default policy file this override was written against.';
COMMENT ON COLUMN platform.policy_override.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.policy_override.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.policy_override.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.change_set IS
  'schema:ItemList/acom:ChangeSet — previews from one request that a person approves, runs, verifies and can undo as a unit. id prefix cs_.';
COMMENT ON COLUMN platform.change_set.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.change_set.id IS
  '- id with prefix cs_; the same value the API, logs and events use.';
COMMENT ON COLUMN platform.change_set.number IS
  'acom:number — number people see (#1042), unique in the store; same as the id suffix in seed data.';
COMMENT ON COLUMN platform.change_set.title IS
  'acom:title — one-line title shown on the change set card.';
COMMENT ON COLUMN platform.change_set.change_set_state IS
  'acom:changeSetState — where the set is in its lifecycle. values: change_set_state';
COMMENT ON COLUMN platform.change_set.request_text IS
  'acom:requestText — the person''s words, first request then each edit by conversation.';
COMMENT ON COLUMN platform.change_set.interpretation IS
  'acom:interpretation — how the runtime understood the request: [{slot, text, value}], slot is who, products, benefit, when, where or other.';
COMMENT ON COLUMN platform.change_set.conversation_id IS
  'acom:conversationId — agent conversation (conv_ id) the set came from; NULL for automation.';
COMMENT ON COLUMN platform.change_set.created_by IS
  'schema:creator — operator or system principal that asked for the set (adm_ or sys_ id).';
COMMENT ON COLUMN platform.change_set.agent_name IS
  'schema:agent — model that drafted the set, for example muse-glimmer-30b; NULL when no model was involved.';
COMMENT ON COLUMN platform.change_set.submitted_at IS
  'acom:submittedAt — when the latest version was submitted for approval.';
COMMENT ON COLUMN platform.change_set.approved_by IS
  'acom:approvedBy — operator who approved (adm_ id); NULL before approval or under a standing approval.';
COMMENT ON COLUMN platform.change_set.approved_at IS
  'acom:approvedAt — when the set was approved; for standing approvals, when the automation started it.';
COMMENT ON COLUMN platform.change_set.rejected_at IS
  'acom:rejectedAt — when a person rejected the set.';
COMMENT ON COLUMN platform.change_set.rejection_reason IS
  'acom:rejectionReason — the reason the person gave when rejecting.';
COMMENT ON COLUMN platform.change_set.verified_at IS
  'acom:verifiedAt — when the executor finished re-reading the results.';
COMMENT ON COLUMN platform.change_set.verification IS
  'acom:verification — summary from the verifying step: {state, matched, total}.';
COMMENT ON COLUMN platform.change_set.undo_until IS
  'acom:undoUntil — last moment a person can undo the set; NULL when an item cannot be undone.';
COMMENT ON COLUMN platform.change_set.undone_at IS
  'acom:undoneAt — when a person undid the set.';
COMMENT ON COLUMN platform.change_set.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.change_set.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.change_set.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.preview IS
  'schema:Action — a stored write preview (actionStatus potential); the operation decides the Action subtype. id prefix pv_.';
COMMENT ON COLUMN platform.preview.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.preview.id IS
  '- id with prefix pv_; the same value the API, logs and events use.';
COMMENT ON COLUMN platform.preview.operation IS
  'acom:operation — logical address of the write, for example promotion.offer.set_sale_prices.';
COMMENT ON COLUMN platform.preview.instrument IS
  'schema:instrument — tool name of the write, for example set_sale_prices.';
COMMENT ON COLUMN platform.preview.input IS
  'acom:input — normalized operation input.';
COMMENT ON COLUMN platform.preview.input_hash IS
  '- hash of the normalized input; detects a repeated preview of the same input.';
COMMENT ON COLUMN platform.preview.plan IS
  'acom:plan — the WriteOp plan that commit applies unchanged.';
COMMENT ON COLUMN platform.preview.changes IS
  'acom:changes — changes[] as returned to the caller: [{op, entity, id, from, to}].';
COMMENT ON COLUMN platform.preview.checks IS
  'acom:checks — policy checks as returned in the API field checks: [{rule, passed, detail, fix_tool}].';
COMMENT ON COLUMN platform.preview.needs_approval IS
  'acom:needsApproval — true when only an approved change set may run it.';
COMMENT ON COLUMN platform.preview.approval_reason IS
  'acom:approvalReason — why approval is needed, shown to the approver; NULL when not needed.';
COMMENT ON COLUMN platform.preview.basis IS
  'acom:basis — entities read to plan and their versions: [{entity, id, version}]; compared at execution.';
COMMENT ON COLUMN platform.preview.policy_version IS
  'acom:policyVersion — hash of the policy snapshot the checks used.';
COMMENT ON COLUMN platform.preview.actor_kind IS
  'acom:actorKind — kind of principal that asked for the preview. values: actor_kind';
COMMENT ON COLUMN platform.preview.agent_id IS
  'schema:agent — person or system behind the preview (adm_, cus_ or sys_ id).';
COMMENT ON COLUMN platform.preview.agent_name IS
  'schema:agent — model that produced the call, when a model did.';
COMMENT ON COLUMN platform.preview.request_id IS
  '- request trace id (req_).';
COMMENT ON COLUMN platform.preview.change_set_id IS
  'acom:changeSetId — change set the preview belongs to; NULL for a stand-alone preview.';
COMMENT ON COLUMN platform.preview.preview_state IS
  'acom:previewState — open until committed, expired or superseded. values: preview_state';
COMMENT ON COLUMN platform.preview.expires_at IS
  'acom:expiresAt — when a stand-alone preview stops being runnable; NULL for previews in a change set (rechecked at approval).';
COMMENT ON COLUMN platform.preview.superseded_by_id IS
  'acom:supersededBy — preview that replaced this one in its change set.';
COMMENT ON COLUMN platform.preview.committed_at IS
  'acom:committedAt — when the preview was executed.';
COMMENT ON COLUMN platform.preview.result IS
  'schema:result — execution response {ok, id, undo}, returned again on a repeated execution.';
COMMENT ON COLUMN platform.preview.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.preview.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.preview.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.change_set_item IS
  'schema:ListItem — one preview inside a change set, in run order.';
COMMENT ON COLUMN platform.change_set_item.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.change_set_item.change_set_id IS
  'acom:changeSetId — the change set.';
COMMENT ON COLUMN platform.change_set_item.position IS
  'schema:position — 1-based run order inside the set.';
COMMENT ON COLUMN platform.change_set_item.preview_id IS
  'acom:previewId — the preview this item runs (pv_ id).';
COMMENT ON COLUMN platform.change_set_item.depends_on IS
  'acom:dependsOn — preview ids whose results this item consumes, for example a promotion_id placeholder.';
COMMENT ON COLUMN platform.change_set_item.result_id IS
  'acom:resultId — id the item produced when it ran, for example promo_1042.';
COMMENT ON COLUMN platform.change_set_item.verification IS
  'acom:verification — verify result for the item: {matched, checked, mismatches[]}.';
COMMENT ON COLUMN platform.change_set_item.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.change_set_item.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.change_set_item.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.change_set_event IS
  'schema:Action — one state transition of a change set; rows are only added.';
COMMENT ON COLUMN platform.change_set_event.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.change_set_event.change_set_id IS
  'acom:changeSetId — the change set that changed (the object of the event).';
COMMENT ON COLUMN platform.change_set_event.position IS
  'schema:position — 1-based order of this event within its aggregate.';
COMMENT ON COLUMN platform.change_set_event.action_type IS
  'acom:actionType — the schema.org Action subtype (@type), for example UpdateAction or AuthorizeAction.';
COMMENT ON COLUMN platform.change_set_event.action_status IS
  'schema:actionStatus — lifecycle of the action. values: action_status';
COMMENT ON COLUMN platform.change_set_event.agent_id IS
  'schema:agent — person or system that acted (adm_, cus_ or sys_ id); NULL when only a model acted.';
COMMENT ON COLUMN platform.change_set_event.agent_name IS
  'schema:agent — name of the software agent (model) that acted, for example muse-glimmer-30b; NULL for people.';
COMMENT ON COLUMN platform.change_set_event.instrument IS
  'schema:instrument — tool name or person-only endpoint that caused the event.';
COMMENT ON COLUMN platform.change_set_event.from_state IS
  'acom:fromState — state before the event; NULL for the creating event. values: change_set_state';
COMMENT ON COLUMN platform.change_set_event.to_state IS
  'acom:toState — state after the event; NULL when the state did not change. values: change_set_state';
COMMENT ON COLUMN platform.change_set_event.result IS
  'schema:result — what the action produced, as JSON.';
COMMENT ON COLUMN platform.change_set_event.error IS
  'schema:error — failure as {code, class, message}; NULL when it succeeded.';
COMMENT ON COLUMN platform.change_set_event.start_time IS
  'schema:startTime — when the action started.';
COMMENT ON COLUMN platform.change_set_event.end_time IS
  'schema:endTime — when the action ended; NULL while it runs.';
COMMENT ON COLUMN platform.change_set_event.request_id IS
  '- request trace id (req_) from the gateway; joins agent turns and the action log.';
COMMENT ON COLUMN platform.change_set_event.change_set_version IS
  'acom:changeSetVersion — version of the set when the event happened.';
COMMENT ON COLUMN platform.change_set_event.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.change_set_event.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.change_set_event.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.action_log IS
  'schema:Action — every tool call, preview, approval, run, verification and undo, in one shape; rows are only added. id prefix act_.';
COMMENT ON COLUMN platform.action_log.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.action_log.id IS
  '- id with prefix act_; the same value the API, logs and events use.';
COMMENT ON COLUMN platform.action_log.action_type IS
  'acom:actionType — the schema.org Action subtype (@type), for example UpdateAction.';
COMMENT ON COLUMN platform.action_log.action_status IS
  'schema:actionStatus — potential for previews, then active, completed or failed. values: action_status';
COMMENT ON COLUMN platform.action_log.actor_kind IS
  'acom:actorKind — kind of principal that acted. values: actor_kind';
COMMENT ON COLUMN platform.action_log.agent_id IS
  'schema:agent — person or system that acted (adm_, cus_ or sys_ id).';
COMMENT ON COLUMN platform.action_log.agent_name IS
  'schema:agent — model name when a model made the call, for example muse-glimmer-30b.';
COMMENT ON COLUMN platform.action_log.instrument IS
  'schema:instrument — tool name or person-only endpoint.';
COMMENT ON COLUMN platform.action_log.object_type IS
  'acom:objectType — schema.org type of the object, for example Offer.';
COMMENT ON COLUMN platform.action_log.object_id IS
  'schema:object — id of the entity acted on, for example off_windbreaker_m_slate.';
COMMENT ON COLUMN platform.action_log.result IS
  'schema:result — what the action produced.';
COMMENT ON COLUMN platform.action_log.error IS
  'schema:error — {code, class, message} when the action failed.';
COMMENT ON COLUMN platform.action_log.start_time IS
  'schema:startTime — when the action started.';
COMMENT ON COLUMN platform.action_log.end_time IS
  'schema:endTime — when the action ended.';
COMMENT ON COLUMN platform.action_log.change_set_id IS
  'acom:changeSetId — change set the action belongs to.';
COMMENT ON COLUMN platform.action_log.preview_id IS
  'acom:previewId — preview the action created or ran.';
COMMENT ON COLUMN platform.action_log.policy_version IS
  'acom:policyVersion — policy snapshot hash used by checks.';
COMMENT ON COLUMN platform.action_log.request_id IS
  '- request trace id (req_).';
COMMENT ON COLUMN platform.action_log.usage IS
  'acom:usage — model usage for the call: {input_tokens, output_tokens}.';
COMMENT ON COLUMN platform.action_log.latency_ms IS
  'acom:latency — time the call took, in milliseconds.';
COMMENT ON COLUMN platform.action_log.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.action_log.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.action_log.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.task IS
  'schema:Action/acom:Task — one item on the operator home list, from a watcher, an automation or a request. id prefix task_.';
COMMENT ON COLUMN platform.task.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.task.id IS
  '- id with prefix task_; the same value the API, logs and events use.';
COMMENT ON COLUMN platform.task.title IS
  'acom:title — one line shown on the home list.';
COMMENT ON COLUMN platform.task.task_state IS
  'acom:taskState — whether the task waits for a person, is being worked, done or dismissed. values: task_state';
COMMENT ON COLUMN platform.task.task_origin IS
  'acom:taskOrigin — how the task started. values: task_origin';
COMMENT ON COLUMN platform.task.summary IS
  'acom:summary — current one-line status, for example 15 of 18 checked.';
COMMENT ON COLUMN platform.task.tag IS
  'acom:tag — area label shown on the list, for example returns or pricing.';
COMMENT ON COLUMN platform.task.request_text IS
  'acom:requestText — the sentence the agent starts a turn with when the task is opened.';
COMMENT ON COLUMN platform.task.watcher IS
  'acom:watcher — platform.yml watchers key that created the task; NULL for requests.';
COMMENT ON COLUMN platform.task.watcher_observation IS
  'acom:watcherObservation — threshold and what was observed: {threshold, observed}.';
COMMENT ON COLUMN platform.task.related_entities IS
  'schema:object — entities the task is about: [{entity, id}].';
COMMENT ON COLUMN platform.task.change_set_id IS
  'acom:changeSetId — change set holding the task''s writes; NULL until one exists.';
COMMENT ON COLUMN platform.task.conversation_id IS
  'acom:conversationId — agent conversation working on the task (conv_ id).';
COMMENT ON COLUMN platform.task.start_time IS
  'schema:startTime — when the task appeared (API at).';
COMMENT ON COLUMN platform.task.due_at IS
  'acom:dueAt — when it should be done, if a watcher sets one.';
COMMENT ON COLUMN platform.task.end_time IS
  'schema:endTime — when the task became done or dismissed.';
COMMENT ON COLUMN platform.task.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.task.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.task.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.job IS
  '- scheduled work run by the commerce job queue (FOR UPDATE SKIP LOCKED): watchers, banner and promotion start and end. id prefix job_.';
COMMENT ON COLUMN platform.job.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.job.id IS
  '- id with prefix job_; the same value the API, logs and events use.';
COMMENT ON COLUMN platform.job.kind IS
  '- job kind the dispatcher routes on, for example promotion.start or watcher.price_below_cost.';
COMMENT ON COLUMN platform.job.run_at IS
  '- when the job becomes due, on the store clock (the demo clock in demo mode).';
COMMENT ON COLUMN platform.job.payload IS
  '- job input, for example {promotion_id}.';
COMMENT ON COLUMN platform.job.attempts IS
  '- runs tried so far.';
COMMENT ON COLUMN platform.job.locked_at IS
  '- when a worker took the job; NULL when free.';
COMMENT ON COLUMN platform.job.done_at IS
  '- when the job finished successfully; NULL while pending.';
COMMENT ON COLUMN platform.job.last_error IS
  '- error of the last failed attempt.';
COMMENT ON COLUMN platform.job.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.job.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.job.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.outbox IS
  '- domain events waiting to be published to other domains and to the event stream; at least once delivery.';
COMMENT ON COLUMN platform.outbox.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.outbox.position IS
  '- increasing event number; the SSE event id.';
COMMENT ON COLUMN platform.outbox.topic IS
  '- event name, for example change_set.updated or order_delivered.';
COMMENT ON COLUMN platform.outbox.object_id IS
  '- id of the aggregate the event is about.';
COMMENT ON COLUMN platform.outbox.payload IS
  '- event body.';
COMMENT ON COLUMN platform.outbox.published_at IS
  '- when the dispatcher delivered the event; NULL while waiting.';
COMMENT ON COLUMN platform.outbox.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.outbox.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.outbox.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.idempotency_key IS
  '- first response to a person-only request, returned again when the same Idempotency-Key is retried.';
COMMENT ON COLUMN platform.idempotency_key.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.idempotency_key.key IS
  '- the Idempotency-Key header value.';
COMMENT ON COLUMN platform.idempotency_key.endpoint IS
  '- method and path the key was used with.';
COMMENT ON COLUMN platform.idempotency_key.subject_id IS
  '- person who sent the request (adm_ or cus_ id).';
COMMENT ON COLUMN platform.idempotency_key.request_hash IS
  '- hash of the request body; a retry with another body is refused.';
COMMENT ON COLUMN platform.idempotency_key.response_status IS
  '- HTTP status of the first response.';
COMMENT ON COLUMN platform.idempotency_key.response IS
  '- body of the first response.';
COMMENT ON COLUMN platform.idempotency_key.expires_at IS
  '- when the key may be forgotten.';
COMMENT ON COLUMN platform.idempotency_key.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.idempotency_key.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.idempotency_key.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE platform.demo_clock IS
  '- the store''s demo time; in demo mode every Clock read and due job uses it.';
COMMENT ON COLUMN platform.demo_clock.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.demo_clock.now IS
  '- current demo time for the store.';
COMMENT ON COLUMN platform.demo_clock.is_running IS
  '- true when demo time advances with real time from now; false when it stays fixed.';
COMMENT ON COLUMN platform.demo_clock.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.demo_clock.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.demo_clock.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE platform.online_store ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.online_store FORCE ROW LEVEL SECURITY;
CREATE POLICY online_store_tenant_isolation ON platform.online_store
  USING (id = current_setting('app.tenant_id', true))
  WITH CHECK (id = current_setting('app.tenant_id', true));

ALTER TABLE platform.admin_user ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.admin_user FORCE ROW LEVEL SECURITY;
CREATE POLICY admin_user_tenant_isolation ON platform.admin_user
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.role ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.role FORCE ROW LEVEL SECURITY;
CREATE POLICY role_tenant_isolation ON platform.role
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.role_scope ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.role_scope FORCE ROW LEVEL SECURITY;
CREATE POLICY role_scope_tenant_isolation ON platform.role_scope
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.admin_user_role ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.admin_user_role FORCE ROW LEVEL SECURITY;
CREATE POLICY admin_user_role_tenant_isolation ON platform.admin_user_role
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.session ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.session FORCE ROW LEVEL SECURITY;
CREATE POLICY session_tenant_isolation ON platform.session
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.policy_override ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.policy_override FORCE ROW LEVEL SECURITY;
CREATE POLICY policy_override_tenant_isolation ON platform.policy_override
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.change_set ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.change_set FORCE ROW LEVEL SECURITY;
CREATE POLICY change_set_tenant_isolation ON platform.change_set
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.preview ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.preview FORCE ROW LEVEL SECURITY;
CREATE POLICY preview_tenant_isolation ON platform.preview
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.change_set_item ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.change_set_item FORCE ROW LEVEL SECURITY;
CREATE POLICY change_set_item_tenant_isolation ON platform.change_set_item
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.change_set_event ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.change_set_event FORCE ROW LEVEL SECURITY;
CREATE POLICY change_set_event_tenant_isolation ON platform.change_set_event
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.action_log ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.action_log FORCE ROW LEVEL SECURITY;
CREATE POLICY action_log_tenant_isolation ON platform.action_log
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.task ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.task FORCE ROW LEVEL SECURITY;
CREATE POLICY task_tenant_isolation ON platform.task
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.job ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.job FORCE ROW LEVEL SECURITY;
CREATE POLICY job_tenant_isolation ON platform.job
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.outbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.outbox FORCE ROW LEVEL SECURITY;
CREATE POLICY outbox_tenant_isolation ON platform.outbox
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.idempotency_key ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.idempotency_key FORCE ROW LEVEL SECURITY;
CREATE POLICY idempotency_key_tenant_isolation ON platform.idempotency_key
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.demo_clock ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.demo_clock FORCE ROW LEVEL SECURITY;
CREATE POLICY demo_clock_tenant_isolation ON platform.demo_clock
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
