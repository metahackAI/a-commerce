-- 0033_ailism_bot.sql
-- Ailism Bot, the back-office manager AI (docs/api/platform.md#bot): each cycle it ran with
-- the snapshot it read, its report and what it cost; each proposal it opened as a task; and what
-- people told it back.
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security.

CREATE TABLE platform.bot_cycle (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  bot_trigger text NOT NULL,
  requested_by text,
  cycle_state text NOT NULL DEFAULT 'observing',
  bot_mode text NOT NULL,
  start_time timestamptz NOT NULL,
  end_time timestamptz,
  snapshot jsonb NOT NULL DEFAULT '{}',
  anomalies jsonb NOT NULL DEFAULT '[]',
  report jsonb,
  usage jsonb NOT NULL DEFAULT '{}',
  error text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT bot_cycle_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT bot_cycle_id_check CHECK (id ~ '^bcy_[a-z0-9_]+$'),
  CONSTRAINT bot_cycle_bot_trigger_check CHECK (bot_trigger IN ('schedule', 'manual')),
  CONSTRAINT bot_cycle_requested_by_check CHECK (requested_by ~ '^adm_[a-z0-9_]+$'),
  CONSTRAINT bot_cycle_cycle_state_check CHECK (cycle_state IN ('observing', 'done', 'failed')),
  CONSTRAINT bot_cycle_bot_mode_check CHECK (bot_mode IN ('off', 'observe', 'propose')),
  CONSTRAINT bot_cycle_snapshot_check CHECK (jsonb_typeof(snapshot) = 'object'),
  CONSTRAINT bot_cycle_anomalies_check CHECK (jsonb_typeof(anomalies) = 'array'),
  CONSTRAINT bot_cycle_report_check CHECK (jsonb_typeof(report) = 'object'),
  CONSTRAINT bot_cycle_usage_check CHECK (jsonb_typeof(usage) = 'object')
);

CREATE TABLE platform.bot_decision (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  cycle_id text NOT NULL,
  task_id text NOT NULL,
  bot_area text NOT NULL,
  title text NOT NULL,
  request_text text NOT NULL,
  reason text NOT NULL,
  evidence jsonb NOT NULL DEFAULT '[]',
  expected_effect jsonb NOT NULL DEFAULT '{}',
  bot_risk text NOT NULL,
  related_entities jsonb NOT NULL DEFAULT '[]',
  cause_key text NOT NULL,
  revises_id text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT bot_decision_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT bot_decision_id_check CHECK (id ~ '^bdc_[a-z0-9_]+$'),
  CONSTRAINT bot_decision_cycle_id_fkey FOREIGN KEY (tenant_id, cycle_id) REFERENCES platform.bot_cycle (tenant_id, id),
  CONSTRAINT bot_decision_task_id_fkey FOREIGN KEY (tenant_id, task_id) REFERENCES platform.task (tenant_id, id),
  CONSTRAINT bot_decision_bot_area_check CHECK (bot_area IN ('pricing', 'inventory', 'promotions', 'merchandising', 'catalog_content', 'returns', 'questions_reviews', 'community')),
  CONSTRAINT bot_decision_title_check CHECK (char_length(title) BETWEEN 1 AND 200),
  CONSTRAINT bot_decision_request_text_check CHECK (char_length(request_text) BETWEEN 1 AND 1000),
  CONSTRAINT bot_decision_reason_check CHECK (char_length(reason) BETWEEN 1 AND 1000),
  CONSTRAINT bot_decision_evidence_check CHECK (jsonb_typeof(evidence) = 'array'),
  CONSTRAINT bot_decision_expected_effect_check CHECK (jsonb_typeof(expected_effect) = 'object'),
  CONSTRAINT bot_decision_bot_risk_check CHECK (bot_risk IN ('low', 'medium', 'high')),
  CONSTRAINT bot_decision_related_entities_check CHECK (jsonb_typeof(related_entities) = 'array'),
  CONSTRAINT bot_decision_cause_key_check CHECK (char_length(cause_key) BETWEEN 1 AND 400),
  CONSTRAINT bot_decision_revises_id_fkey FOREIGN KEY (tenant_id, revises_id) REFERENCES platform.bot_decision (tenant_id, id)
);

CREATE TABLE platform.bot_feedback (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  decision_id text,
  feedback_kind text NOT NULL,
  text text NOT NULL,
  feedback_state text NOT NULL DEFAULT 'open',
  created_by text NOT NULL,
  closed_by text,
  closed_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT bot_feedback_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT bot_feedback_id_check CHECK (id ~ '^bfb_[a-z0-9_]+$'),
  CONSTRAINT bot_feedback_decision_id_fkey FOREIGN KEY (tenant_id, decision_id) REFERENCES platform.bot_decision (tenant_id, id),
  CONSTRAINT bot_feedback_feedback_kind_check CHECK (feedback_kind IN ('feedback', 'directive', 'correction')),
  CONSTRAINT bot_feedback_text_check CHECK (char_length(text) BETWEEN 1 AND 1000),
  CONSTRAINT bot_feedback_feedback_state_check CHECK (feedback_state IN ('open', 'closed')),
  CONSTRAINT bot_feedback_created_by_check CHECK (created_by ~ '^adm_[a-z0-9_]+$'),
  CONSTRAINT bot_feedback_closed_by_check CHECK (closed_by ~ '^adm_[a-z0-9_]+$')
);

CREATE INDEX bot_cycle_start_time_idx ON platform.bot_cycle (tenant_id, start_time DESC);
CREATE INDEX bot_decision_cycle_id_idx ON platform.bot_decision (tenant_id, cycle_id);
CREATE INDEX bot_decision_task_id_idx ON platform.bot_decision (tenant_id, task_id);
CREATE INDEX bot_decision_cause_key_idx ON platform.bot_decision (tenant_id, cause_key);
CREATE INDEX bot_feedback_decision_id_idx ON platform.bot_feedback (tenant_id, decision_id);

COMMENT ON TABLE platform.bot_cycle IS
  'acom:BotCycle — one look Ailism Bot took at the store: the snapshot it read, its report and what it cost. id prefix bcy_.';
COMMENT ON COLUMN platform.bot_cycle.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.bot_cycle.id IS
  '- id with prefix bcy_; the same value the API, logs and events use.';
COMMENT ON COLUMN platform.bot_cycle.bot_trigger IS
  'acom:botTrigger — what started the cycle: its schedule or a person. values: bot_trigger';
COMMENT ON COLUMN platform.bot_cycle.requested_by IS
  'acom:requestedBy — the operator who ran it now (adm_ id); NULL for a scheduled cycle.';
COMMENT ON COLUMN platform.bot_cycle.cycle_state IS
  'acom:botCycleState — observing while the bot reads and analyses, then done or failed. values: bot_cycle_state';
COMMENT ON COLUMN platform.bot_cycle.bot_mode IS
  'acom:botMode — the mode the cycle ran in. values: bot_mode';
COMMENT ON COLUMN platform.bot_cycle.start_time IS
  'schema:startTime — when the cycle opened (API at).';
COMMENT ON COLUMN platform.bot_cycle.end_time IS
  'schema:endTime — when the report came back or the cycle failed.';
COMMENT ON COLUMN platform.bot_cycle.snapshot IS
  'acom:snapshot — what the bot read, in order: {areas, reads[{tool, args, result}]}.';
COMMENT ON COLUMN platform.bot_cycle.anomalies IS
  'acom:anomalies — warning signs found by rule in the snapshot: [{area, code, text, severity}].';
COMMENT ON COLUMN platform.bot_cycle.report IS
  'acom:report — the analysis: {summary, findings[], proposals[]} with each proposal''s outcome (opened or why not).';
COMMENT ON COLUMN platform.bot_cycle.usage IS
  'acom:usage — what the cycle cost: {model, model_calls, input_tokens, output_tokens, reads}.';
COMMENT ON COLUMN platform.bot_cycle.error IS
  'acom:error — why a failed cycle failed.';
COMMENT ON COLUMN platform.bot_cycle.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.bot_cycle.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.bot_cycle.version IS
  '- row version, +1 on every change.';

COMMENT ON TABLE platform.bot_decision IS
  'acom:BotDecision — one proposal Ailism Bot opened as a task for the operations agent to draft and a person to decide. id prefix bdc_.';
COMMENT ON COLUMN platform.bot_decision.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.bot_decision.id IS
  '- id with prefix bdc_; the same value the API, logs and events use.';
COMMENT ON COLUMN platform.bot_decision.cycle_id IS
  'acom:botCycleId — the cycle that proposed it.';
COMMENT ON COLUMN platform.bot_decision.task_id IS
  'acom:taskId — the task it opened; its change set and state are the decision''s.';
COMMENT ON COLUMN platform.bot_decision.bot_area IS
  'acom:botArea — the area it is about. values: bot_area';
COMMENT ON COLUMN platform.bot_decision.title IS
  'acom:title — one line shown on the decision list.';
COMMENT ON COLUMN platform.bot_decision.request_text IS
  'acom:requestText — the sentence the operations agent drafts the change from.';
COMMENT ON COLUMN platform.bot_decision.reason IS
  'acom:reason — why the bot proposes it, from the snapshot.';
COMMENT ON COLUMN platform.bot_decision.evidence IS
  'acom:evidence — figures from the snapshot it rests on: [text].';
COMMENT ON COLUMN platform.bot_decision.expected_effect IS
  'acom:expectedEffect — what should change and by when: {metric, direction, horizon_days}.';
COMMENT ON COLUMN platform.bot_decision.bot_risk IS
  'acom:botRisk — how much could go wrong if it is wrong. values: bot_risk';
COMMENT ON COLUMN platform.bot_decision.related_entities IS
  'schema:object — entities the decision is about: [{entity, id}].';
COMMENT ON COLUMN platform.bot_decision.cause_key IS
  'acom:causeKey — area and entities, to find an open or recently turned down decision about the same thing.';
COMMENT ON COLUMN platform.bot_decision.revises_id IS
  'acom:revises — the decision a person asked again with a correction; this one replaces it.';
COMMENT ON COLUMN platform.bot_decision.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.bot_decision.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.bot_decision.version IS
  '- row version, +1 on every change.';

COMMENT ON TABLE platform.bot_feedback IS
  'acom:BotFeedback — what a person told Ailism Bot: an opinion, a standing directive, or a correction asked of a decision. id prefix bfb_.';
COMMENT ON COLUMN platform.bot_feedback.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN platform.bot_feedback.id IS
  '- id with prefix bfb_; the same value the API, logs and events use.';
COMMENT ON COLUMN platform.bot_feedback.decision_id IS
  'acom:botDecisionId — the decision it is about; NULL for the bot as a whole.';
COMMENT ON COLUMN platform.bot_feedback.feedback_kind IS
  'acom:botFeedbackKind — feedback, directive or correction. values: bot_feedback_kind';
COMMENT ON COLUMN platform.bot_feedback.text IS
  'schema:text — what the person wrote.';
COMMENT ON COLUMN platform.bot_feedback.feedback_state IS
  'acom:botFeedbackState — open feedback goes into every analysis until a person closes it. values: bot_feedback_state';
COMMENT ON COLUMN platform.bot_feedback.created_by IS
  'acom:createdBy — the operator who wrote it (adm_ id).';
COMMENT ON COLUMN platform.bot_feedback.closed_by IS
  'acom:closedBy — the operator who closed it.';
COMMENT ON COLUMN platform.bot_feedback.closed_at IS
  'acom:closedAt — when it was closed.';
COMMENT ON COLUMN platform.bot_feedback.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN platform.bot_feedback.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN platform.bot_feedback.version IS
  '- row version, +1 on every change.';

ALTER TABLE platform.bot_cycle ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.bot_cycle FORCE ROW LEVEL SECURITY;
CREATE POLICY bot_cycle_tenant_isolation ON platform.bot_cycle
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.bot_decision ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.bot_decision FORCE ROW LEVEL SECURITY;
CREATE POLICY bot_decision_tenant_isolation ON platform.bot_decision
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE platform.bot_feedback ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.bot_feedback FORCE ROW LEVEL SECURITY;
CREATE POLICY bot_feedback_tenant_isolation ON platform.bot_feedback
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
