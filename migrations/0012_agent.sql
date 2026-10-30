-- 0012_agent.sql
-- Agent: conversations, turns, steps and model calls. Owned by the agent service; ids of
-- commerce entities are plain text here because the agent never depends on a domain.
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security.


CREATE TABLE agent.conversation (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  surface text NOT NULL,
  customer_id text,
  admin_id text,
  session_id text,
  name text,
  agent_profile text,
  started_at timestamptz NOT NULL,
  last_turn_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT conversation_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT conversation_id_check CHECK (id ~ '^conv_[a-z0-9_]+$'),
  CONSTRAINT conversation_surface_check CHECK (surface IN ('bo', 'fo'))
);

CREATE TABLE agent.turn (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  conversation_id text NOT NULL,
  position integer NOT NULL,
  text text NOT NULL,
  context jsonb NOT NULL DEFAULT '[]',
  agent_profile text NOT NULL,
  reply_text text,
  blocks jsonb NOT NULL DEFAULT '[]',
  usage jsonb NOT NULL DEFAULT '{}',
  request_id text,
  error jsonb,
  started_at timestamptz NOT NULL,
  ended_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT turn_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT turn_id_check CHECK (id ~ '^turn_[a-z0-9_]+$'),
  CONSTRAINT turn_conversation_id_position_key UNIQUE (tenant_id, conversation_id, position),
  CONSTRAINT turn_conversation_id_fkey FOREIGN KEY (tenant_id, conversation_id) REFERENCES agent.conversation (tenant_id, id),
  CONSTRAINT turn_position_check CHECK (position >= 1)
);

CREATE TABLE agent.step (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  turn_id text NOT NULL,
  position integer NOT NULL,
  kind text NOT NULL,
  instrument text,
  action_status text NOT NULL DEFAULT 'active',
  summary text,
  input jsonb,
  output jsonb,
  preview_id text,
  change_set_id text,
  error jsonb,
  started_at timestamptz NOT NULL,
  ended_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT step_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT step_id_check CHECK (id ~ '^step_[a-z0-9_]+$'),
  CONSTRAINT step_turn_id_position_key UNIQUE (tenant_id, turn_id, position),
  CONSTRAINT step_turn_id_fkey FOREIGN KEY (tenant_id, turn_id) REFERENCES agent.turn (tenant_id, id),
  CONSTRAINT step_position_check CHECK (position >= 1),
  CONSTRAINT step_kind_check CHECK (kind IN ('route', 'search_tools', 'plan', 'call_tool', 'check')),
  CONSTRAINT step_action_status_check CHECK (action_status IN ('potential', 'active', 'completed', 'failed'))
);

CREATE TABLE agent.model_call (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  turn_id text NOT NULL,
  position integer NOT NULL,
  step_id text,
  model text NOT NULL,
  purpose text NOT NULL,
  input_tokens integer NOT NULL DEFAULT 0,
  output_tokens integer NOT NULL DEFAULT 0,
  latency_ms integer NOT NULL DEFAULT 0,
  cost_value numeric(12,2),
  cost_currency char(3),
  succeeded boolean NOT NULL,
  error jsonb,
  started_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT model_call_pkey PRIMARY KEY (tenant_id, turn_id, position),
  CONSTRAINT model_call_turn_id_fkey FOREIGN KEY (tenant_id, turn_id) REFERENCES agent.turn (tenant_id, id),
  CONSTRAINT model_call_step_id_fkey FOREIGN KEY (tenant_id, step_id) REFERENCES agent.step (tenant_id, id),
  CONSTRAINT model_call_position_check CHECK (position >= 1),
  CONSTRAINT model_call_input_tokens_check CHECK (input_tokens >= 0),
  CONSTRAINT model_call_output_tokens_check CHECK (output_tokens >= 0),
  CONSTRAINT model_call_latency_ms_check CHECK (latency_ms >= 0),
  CONSTRAINT model_call_cost_value_check CHECK (cost_value >= 0),
  CONSTRAINT model_call_cost_currency_check CHECK (cost_currency ~ '^[A-Z]{3}$')
);

CREATE INDEX conversation_customer_id_idx ON agent.conversation (tenant_id, customer_id);
CREATE INDEX conversation_admin_id_idx ON agent.conversation (tenant_id, admin_id);
CREATE INDEX conversation_started_at_idx ON agent.conversation (tenant_id, started_at);
CREATE INDEX turn_request_id_idx ON agent.turn (tenant_id, request_id);
CREATE INDEX turn_started_at_idx ON agent.turn (tenant_id, started_at);
CREATE INDEX step_instrument_idx ON agent.step (tenant_id, instrument);
CREATE INDEX model_call_step_id_idx ON agent.model_call (tenant_id, step_id);
CREATE INDEX model_call_model_idx ON agent.model_call (tenant_id, model);
CREATE INDEX model_call_started_at_idx ON agent.model_call (tenant_id, started_at);

COMMENT ON TABLE agent.conversation IS
  'schema:Conversation — one chat between a person and the agent runtime, on the back office (bo) or the storefront (fo). id prefix conv_.';
COMMENT ON COLUMN agent.conversation.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN agent.conversation.id IS
  '- id with prefix conv_; the same value the API, logs and events use.';
COMMENT ON COLUMN agent.conversation.surface IS
  'acom:surface — bo for operators, fo for shoppers. values: chat_surface';
COMMENT ON COLUMN agent.conversation.customer_id IS
  'schema:customer — shopper in the conversation (cus_ id), NULL on bo and for guests; plain text because the agent service owns this schema.';
COMMENT ON COLUMN agent.conversation.admin_id IS
  'acom:adminId — operator in the conversation (adm_ id), NULL on fo; plain text like customer_id.';
COMMENT ON COLUMN agent.conversation.session_id IS
  'acom:sessionId — gateway session the conversation runs in (ses_ id).';
COMMENT ON COLUMN agent.conversation.name IS
  'schema:name — short title for history lists.';
COMMENT ON COLUMN agent.conversation.agent_profile IS
  'acom:agentProfile — specialized agent last routed to, for example promotion or shopper.';
COMMENT ON COLUMN agent.conversation.started_at IS
  'acom:startedAt — first turn.';
COMMENT ON COLUMN agent.conversation.last_turn_at IS
  'acom:lastTurnAt — latest turn.';
COMMENT ON COLUMN agent.conversation.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN agent.conversation.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN agent.conversation.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE agent.turn IS
  'schema:Message — one request and its answer inside a conversation. id prefix turn_.';
COMMENT ON COLUMN agent.turn.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN agent.turn.id IS
  '- id with prefix turn_; the same value the API, logs and events use.';
COMMENT ON COLUMN agent.turn.conversation_id IS
  'acom:conversationId — the conversation (conv_ id).';
COMMENT ON COLUMN agent.turn.position IS
  'schema:position — turn number in the conversation.';
COMMENT ON COLUMN agent.turn.text IS
  'schema:text — what the person said, or the intent a UI button sent.';
COMMENT ON COLUMN agent.turn.context IS
  'acom:context — screen elements the person pointed at: [{element_ref, ids}].';
COMMENT ON COLUMN agent.turn.agent_profile IS
  'acom:agentProfile — specialized agent that handled the turn.';
COMMENT ON COLUMN agent.turn.reply_text IS
  'acom:replyText — short text answer shown with the blocks.';
COMMENT ON COLUMN agent.turn.blocks IS
  'acom:blocks — UI blocks returned: [{block, schema, data}].';
COMMENT ON COLUMN agent.turn.usage IS
  'acom:usage — {model_calls, tool_calls, input_tokens, output_tokens, latency_ms, cost: {value, currency}}.';
COMMENT ON COLUMN agent.turn.request_id IS
  '- request trace id (req_); joins the commerce action log.';
COMMENT ON COLUMN agent.turn.error IS
  'schema:error — why the turn stopped early: {code, class, message}.';
COMMENT ON COLUMN agent.turn.started_at IS
  'acom:startedAt — when the request arrived.';
COMMENT ON COLUMN agent.turn.ended_at IS
  'acom:endedAt — when the answer finished streaming.';
COMMENT ON COLUMN agent.turn.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN agent.turn.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN agent.turn.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE agent.step IS
  'schema:Action — one step of a turn: routing, tool search, planning, a tool call or an intent check. id prefix step_.';
COMMENT ON COLUMN agent.step.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN agent.step.id IS
  '- id with prefix step_; the same value the API, logs and events use.';
COMMENT ON COLUMN agent.step.turn_id IS
  'schema:isPartOf — the turn (turn_ id).';
COMMENT ON COLUMN agent.step.position IS
  'schema:position — step order in the turn.';
COMMENT ON COLUMN agent.step.kind IS
  'acom:stepKind — what the step does. values: step_kind';
COMMENT ON COLUMN agent.step.instrument IS
  'schema:instrument — tool called, for call_tool steps.';
COMMENT ON COLUMN agent.step.action_status IS
  'schema:actionStatus — state of the step. values: action_status';
COMMENT ON COLUMN agent.step.summary IS
  'acom:summary — one-line description for traces and evaluation.';
COMMENT ON COLUMN agent.step.input IS
  'acom:input — step input, for example corrected tool arguments.';
COMMENT ON COLUMN agent.step.output IS
  'acom:output — step output, for example the tool result or the plan.';
COMMENT ON COLUMN agent.step.preview_id IS
  'acom:previewId — preview the step created (pv_ id).';
COMMENT ON COLUMN agent.step.change_set_id IS
  'acom:changeSetId — change set the step attached to (cs_ id).';
COMMENT ON COLUMN agent.step.error IS
  'schema:error — {code, class, message} when the step failed.';
COMMENT ON COLUMN agent.step.started_at IS
  'acom:startedAt — when the step started.';
COMMENT ON COLUMN agent.step.ended_at IS
  'acom:endedAt — when it ended.';
COMMENT ON COLUMN agent.step.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN agent.step.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN agent.step.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE agent.model_call IS
  'schema:Action — one call to a language model, with tokens, latency and cost for evaluation.';
COMMENT ON COLUMN agent.model_call.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN agent.model_call.turn_id IS
  'schema:isPartOf — the turn (turn_ id).';
COMMENT ON COLUMN agent.model_call.position IS
  'schema:position — call order in the turn.';
COMMENT ON COLUMN agent.model_call.step_id IS
  'acom:stepId — step that made the call (step_ id); NULL for calls outside a step.';
COMMENT ON COLUMN agent.model_call.model IS
  'acom:model — model name, for example muse-glimmer-30b.';
COMMENT ON COLUMN agent.model_call.purpose IS
  'acom:purpose — why it was called, for example route, plan, arguments or reply.';
COMMENT ON COLUMN agent.model_call.input_tokens IS
  'acom:inputTokens — tokens sent.';
COMMENT ON COLUMN agent.model_call.output_tokens IS
  'acom:outputTokens — tokens received.';
COMMENT ON COLUMN agent.model_call.latency_ms IS
  'acom:latency — time to the full response, in milliseconds.';
COMMENT ON COLUMN agent.model_call.cost_value IS
  'acom:cost — price of the call from agents/models.yml, in major units of the currency.';
COMMENT ON COLUMN agent.model_call.cost_currency IS
  'acom:cost — ISO 4217 code of cost_value.';
COMMENT ON COLUMN agent.model_call.succeeded IS
  'acom:succeeded — false when the call errored or its output failed validation.';
COMMENT ON COLUMN agent.model_call.error IS
  'schema:error — {code, class, message} when it failed.';
COMMENT ON COLUMN agent.model_call.started_at IS
  'acom:startedAt — when the call was sent.';
COMMENT ON COLUMN agent.model_call.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN agent.model_call.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN agent.model_call.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE agent.conversation ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent.conversation FORCE ROW LEVEL SECURITY;
CREATE POLICY conversation_tenant_isolation ON agent.conversation
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE agent.turn ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent.turn FORCE ROW LEVEL SECURITY;
CREATE POLICY turn_tenant_isolation ON agent.turn
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE agent.step ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent.step FORCE ROW LEVEL SECURITY;
CREATE POLICY step_tenant_isolation ON agent.step
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE agent.model_call ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent.model_call FORCE ROW LEVEL SECURITY;
CREATE POLICY model_call_tenant_isolation ON agent.model_call
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
