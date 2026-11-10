-- 0036_customer_bot.sql
-- Ailism Customer Bot, the shoppers that keep a demo store busy (docs/api/platform.md#customer_bot):
-- each run with its workers, each shopping session one worker had as one member, every action of
-- a session in order, and the persona each member shops as.
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security.

CREATE TABLE platform.customer_bot_run (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  bot_trigger text NOT NULL,
  requested_by text,
  run_state text NOT NULL DEFAULT 'queued',
  workers integer NOT NULL,
  start_time timestamptz NOT NULL,
  end_time timestamptz,
  summary jsonb NOT NULL DEFAULT '{}',
  error text,
  claimed_at timestamptz,
  finished_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT customer_bot_run_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT customer_bot_run_id_check CHECK (id ~ '^cbr_[a-z0-9_]+$'),
  CONSTRAINT customer_bot_run_bot_trigger_check CHECK (bot_trigger IN ('schedule', 'manual')),
  CONSTRAINT customer_bot_run_requested_by_check CHECK (requested_by ~ '^adm_[a-z0-9_]+$'),
  CONSTRAINT customer_bot_run_run_state_check CHECK (run_state IN ('queued', 'running', 'done', 'failed')),
  CONSTRAINT customer_bot_run_workers_check CHECK (workers BETWEEN 1 AND 20),
  CONSTRAINT customer_bot_run_summary_check CHECK (jsonb_typeof(summary) = 'object')
);

CREATE TABLE platform.customer_bot_session (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  run_id text NOT NULL,
  customer_id text,
  account_origin text NOT NULL,
  session_state text NOT NULL DEFAULT 'running',
  persona_built boolean NOT NULL DEFAULT false,
  goal text,
  outcome jsonb NOT NULL DEFAULT '{}',
  conversation_ids jsonb NOT NULL DEFAULT '[]',
  error text,
  start_time timestamptz NOT NULL,
  end_time timestamptz,
  finished_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT customer_bot_session_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT customer_bot_session_id_check CHECK (id ~ '^cbs_[a-z0-9_]+$'),
  CONSTRAINT customer_bot_session_run_id_fkey FOREIGN KEY (tenant_id, run_id) REFERENCES platform.customer_bot_run (tenant_id, id),
  CONSTRAINT customer_bot_session_customer_id_check CHECK (customer_id ~ '^cus_[a-z0-9_]+$'),
  CONSTRAINT customer_bot_session_account_origin_check CHECK (account_origin IN ('existing', 'created')),
  CONSTRAINT customer_bot_session_session_state_check CHECK (session_state IN ('running', 'done', 'failed')),
  CONSTRAINT customer_bot_session_goal_check CHECK (char_length(goal) BETWEEN 1 AND 500),
  CONSTRAINT customer_bot_session_outcome_check CHECK (jsonb_typeof(outcome) = 'object'),
  CONSTRAINT customer_bot_session_conversation_ids_check CHECK (jsonb_typeof(conversation_ids) = 'array')
);

CREATE TABLE platform.customer_bot_step (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  session_id text NOT NULL,
  position integer NOT NULL,
  step_kind text NOT NULL,
  step_status text NOT NULL,
  title text NOT NULL,
  request jsonb,
  response jsonb,
  detail jsonb NOT NULL DEFAULT '{}',
  duration_ms integer NOT NULL DEFAULT 0,
  start_time timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT customer_bot_step_pkey PRIMARY KEY (tenant_id, session_id, position),
  CONSTRAINT customer_bot_step_session_id_fkey FOREIGN KEY (tenant_id, session_id) REFERENCES platform.customer_bot_session (tenant_id, id),
  CONSTRAINT customer_bot_step_position_check CHECK (position >= 1),
  CONSTRAINT customer_bot_step_step_kind_check CHECK (step_kind IN ('sign_up', 'sign_in', 'persona', 'visit', 'read', 'think', 'chat', 'press', 'review', 'sign_out')),
  CONSTRAINT customer_bot_step_step_status_check CHECK (step_status IN ('ok', 'failed', 'skipped')),
  CONSTRAINT customer_bot_step_title_check CHECK (char_length(title) BETWEEN 1 AND 300),
  CONSTRAINT customer_bot_step_request_check CHECK (jsonb_typeof(request) = 'object'),
  CONSTRAINT customer_bot_step_response_check CHECK (jsonb_typeof(response) IN ('object', 'array')),
  CONSTRAINT customer_bot_step_detail_check CHECK (jsonb_typeof(detail) = 'object'),
  CONSTRAINT customer_bot_step_duration_ms_check CHECK (duration_ms >= 0)
);

CREATE TABLE platform.shopper_persona (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  customer_id text NOT NULL,
  description text NOT NULL,
  persona jsonb NOT NULL,
  built_by_session_id text,
  session_count integer NOT NULL DEFAULT 0,
  last_session_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT shopper_persona_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT shopper_persona_id_check CHECK (id ~ '^spn_[a-z0-9_]+$'),
  CONSTRAINT shopper_persona_customer_id_key UNIQUE (tenant_id, customer_id),
  CONSTRAINT shopper_persona_customer_id_check CHECK (customer_id ~ '^cus_[a-z0-9_]+$'),
  CONSTRAINT shopper_persona_description_check CHECK (char_length(description) BETWEEN 1 AND 500),
  CONSTRAINT shopper_persona_persona_check CHECK (jsonb_typeof(persona) = 'object'),
  CONSTRAINT shopper_persona_built_by_session_id_fkey FOREIGN KEY (tenant_id, built_by_session_id) REFERENCES platform.customer_bot_session (tenant_id, id),
  CONSTRAINT shopper_persona_session_count_check CHECK (session_count >= 0)
);

CREATE INDEX customer_bot_run_created_at_idx ON platform.customer_bot_run (tenant_id, created_at DESC);
CREATE INDEX customer_bot_session_run_id_idx ON platform.customer_bot_session (tenant_id, run_id);
CREATE INDEX customer_bot_session_customer_id_idx ON platform.customer_bot_session (tenant_id, customer_id);
CREATE INDEX customer_bot_session_created_at_idx ON platform.customer_bot_session (tenant_id, created_at DESC);

COMMENT ON TABLE platform.customer_bot_run IS
  'acom:CustomerBotRun — one round of Ailism Customer Bot: its workers each shop once as a member. id prefix cbr_.';
COMMENT ON COLUMN platform.customer_bot_run.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.customer_bot_run.id IS
  '- id with prefix cbr_; the same value the API, logs and events use.';
COMMENT ON COLUMN platform.customer_bot_run.bot_trigger IS
  'acom:botTrigger — what started the run: its schedule or a person. values: bot_trigger';
COMMENT ON COLUMN platform.customer_bot_run.requested_by IS
  'acom:requestedBy — the operator who ran it now (adm_ id); NULL for a scheduled run.';
COMMENT ON COLUMN platform.customer_bot_run.run_state IS
  'acom:customerBotRunState — queued until the gateway takes it, running while its sessions shop, then done or failed. values: customer_bot_run_state';
COMMENT ON COLUMN platform.customer_bot_run.workers IS
  'acom:workers — how many sessions shop at the same time in this run.';
COMMENT ON COLUMN platform.customer_bot_run.start_time IS
  'schema:startTime — when the run opened, in store time (API at).';
COMMENT ON COLUMN platform.customer_bot_run.end_time IS
  'schema:endTime — when its last session ended, in store time.';
COMMENT ON COLUMN platform.customer_bot_run.summary IS
  'acom:summary — what its sessions did together: {sessions, done, failed, accounts_created, personas_built, chat_turns, orders, spent, reviews}.';
COMMENT ON COLUMN platform.customer_bot_run.error IS
  'acom:error — why the run failed; NULL otherwise.';
COMMENT ON COLUMN platform.customer_bot_run.claimed_at IS
  '- when the gateway took the run, by the server''s clock (the schedule follows real minutes).';
COMMENT ON COLUMN platform.customer_bot_run.finished_at IS
  '- when the run ended, by the server''s clock.';
COMMENT ON COLUMN platform.customer_bot_run.created_at IS
  '- row creation time, by the server''s clock; the schedule counts from it.';
COMMENT ON COLUMN platform.customer_bot_run.updated_at IS
  '- last update time.';
COMMENT ON COLUMN platform.customer_bot_run.version IS
  '- row version, +1 on every change.';

COMMENT ON TABLE platform.customer_bot_session IS
  'acom:CustomerBotSession — one shopping visit a worker made as one member, start to sign-out. id prefix cbs_.';
COMMENT ON COLUMN platform.customer_bot_session.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.customer_bot_session.id IS
  '- id with prefix cbs_; the same value the API, logs and events use.';
COMMENT ON COLUMN platform.customer_bot_session.run_id IS
  'acom:customerBotRun — the run the session belongs to (cbr_ id).';
COMMENT ON COLUMN platform.customer_bot_session.customer_id IS
  'schema:customer — the member the session shopped as (cus_ id); NULL until a new account exists.';
COMMENT ON COLUMN platform.customer_bot_session.account_origin IS
  'acom:accountOrigin — an existing member or an account the session signed up. values: customer_bot_account_origin';
COMMENT ON COLUMN platform.customer_bot_session.session_state IS
  'acom:customerBotSessionState — running, then done or failed. values: customer_bot_session_state';
COMMENT ON COLUMN platform.customer_bot_session.persona_built IS
  'acom:personaBuilt — whether this session made the member''s persona before shopping.';
COMMENT ON COLUMN platform.customer_bot_session.goal IS
  'acom:goal — what the shopper came for this time, in its own words.';
COMMENT ON COLUMN platform.customer_bot_session.outcome IS
  'acom:outcome — what the session did: {chat_turns, orders[{order_id, total}], spent, reviews, memories, tools[]}.';
COMMENT ON COLUMN platform.customer_bot_session.conversation_ids IS
  'acom:conversation — the storefront conversations it had (conv_ ids).';
COMMENT ON COLUMN platform.customer_bot_session.error IS
  'acom:error — why the session stopped early; NULL otherwise.';
COMMENT ON COLUMN platform.customer_bot_session.start_time IS
  'schema:startTime — when the session started, in store time.';
COMMENT ON COLUMN platform.customer_bot_session.end_time IS
  'schema:endTime — when it ended, in store time.';
COMMENT ON COLUMN platform.customer_bot_session.finished_at IS
  '- when it ended, by the server''s clock.';
COMMENT ON COLUMN platform.customer_bot_session.created_at IS
  '- row creation time, by the server''s clock.';
COMMENT ON COLUMN platform.customer_bot_session.updated_at IS
  '- last update time.';
COMMENT ON COLUMN platform.customer_bot_session.version IS
  '- row version, +1 on every change.';

COMMENT ON TABLE platform.customer_bot_step IS
  'acom:CustomerBotStep — one thing a session did, in order: a request it sent, a message it wrote, a decision it took. Kept whole and never changed.';
COMMENT ON COLUMN platform.customer_bot_step.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.customer_bot_step.session_id IS
  'acom:customerBotSession — the session (cbs_ id).';
COMMENT ON COLUMN platform.customer_bot_step.position IS
  'schema:position — the step''s place in the session, from 1.';
COMMENT ON COLUMN platform.customer_bot_step.step_kind IS
  'acom:customerBotStepKind — what kind of action it was. values: customer_bot_step_kind';
COMMENT ON COLUMN platform.customer_bot_step.step_status IS
  'acom:customerBotStepStatus — whether it worked. values: customer_bot_step_status';
COMMENT ON COLUMN platform.customer_bot_step.title IS
  'schema:name — one sentence saying what it did.';
COMMENT ON COLUMN platform.customer_bot_step.request IS
  'acom:request — what it sent: {method, path, body} for a request, {message} for a chat turn.';
COMMENT ON COLUMN platform.customer_bot_step.response IS
  'acom:response — what came back, shortened: {status, body}.';
COMMENT ON COLUMN platform.customer_bot_step.detail IS
  'acom:detail — the rest worth keeping: the reply and blocks of a chat turn, the reason for a decision, the model''s usage.';
COMMENT ON COLUMN platform.customer_bot_step.duration_ms IS
  'schema:duration — how long it took, in milliseconds.';
COMMENT ON COLUMN platform.customer_bot_step.start_time IS
  'schema:startTime — when it happened, in store time.';
COMMENT ON COLUMN platform.customer_bot_step.created_at IS
  '- row creation time, by the server''s clock.';
COMMENT ON COLUMN platform.customer_bot_step.updated_at IS
  '- last update time.';
COMMENT ON COLUMN platform.customer_bot_step.version IS
  '- row version, +1 on every change.';

COMMENT ON TABLE platform.shopper_persona IS
  'acom:ShopperPersona — who a member is when Ailism Customer Bot shops as them: style, sizes, budget, habits and voice. One per member. id prefix spn_.';
COMMENT ON COLUMN platform.shopper_persona.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.shopper_persona.id IS
  '- id with prefix spn_; the same value the API, logs and events use.';
COMMENT ON COLUMN platform.shopper_persona.customer_id IS
  'schema:customer — the member (cus_ id).';
COMMENT ON COLUMN platform.shopper_persona.description IS
  'schema:description — the persona in one line.';
COMMENT ON COLUMN platform.shopper_persona.persona IS
  'acom:persona — the persona: {age_range, location, occupation, style[], colors[], sizes{}, budget{}, categories[], shopping_style, voice, interests[]}.';
COMMENT ON COLUMN platform.shopper_persona.built_by_session_id IS
  'acom:customerBotSession — the session that made it (cbs_ id).';
COMMENT ON COLUMN platform.shopper_persona.session_count IS
  'acom:sessionCount — how many sessions have shopped as this persona.';
COMMENT ON COLUMN platform.shopper_persona.last_session_at IS
  '- when a session last shopped as it, by the server''s clock.';
COMMENT ON COLUMN platform.shopper_persona.created_at IS
  '- row creation time, by the server''s clock.';
COMMENT ON COLUMN platform.shopper_persona.updated_at IS
  '- last update time.';
COMMENT ON COLUMN platform.shopper_persona.version IS
  '- row version, +1 on every change.';

ALTER TABLE platform.customer_bot_run ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.customer_bot_run FORCE ROW LEVEL SECURITY;
CREATE POLICY customer_bot_run_tenant_isolation ON platform.customer_bot_run
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.customer_bot_session ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.customer_bot_session FORCE ROW LEVEL SECURITY;
CREATE POLICY customer_bot_session_tenant_isolation ON platform.customer_bot_session
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.customer_bot_step ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.customer_bot_step FORCE ROW LEVEL SECURITY;
CREATE POLICY customer_bot_step_tenant_isolation ON platform.customer_bot_step
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.shopper_persona ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.shopper_persona FORCE ROW LEVEL SECURITY;
CREATE POLICY shopper_persona_tenant_isolation ON platform.shopper_persona
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
