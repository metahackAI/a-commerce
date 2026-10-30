-- 0009_messaging.sql
-- Messaging: transactional messages to shoppers and the message templates operators write.
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security.


CREATE TABLE messaging.message (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  recipient_id text NOT NULL,
  about_id text,
  message_kind text NOT NULL,
  message_channel text NOT NULL DEFAULT 'email',
  headline text,
  text text NOT NULL,
  sender_id text,
  preview_id text,
  change_set_id text,
  date_sent timestamptz,
  date_read timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT message_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT message_id_check CHECK (id ~ '^msg_[a-z0-9_]+$'),
  CONSTRAINT message_recipient_id_fkey FOREIGN KEY (tenant_id, recipient_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT message_preview_id_fkey FOREIGN KEY (tenant_id, preview_id) REFERENCES platform.preview (tenant_id, id),
  CONSTRAINT message_change_set_id_fkey FOREIGN KEY (tenant_id, change_set_id) REFERENCES platform.change_set (tenant_id, id),
  CONSTRAINT message_message_kind_check CHECK (message_kind IN ('ask_for_photos', 'decline', 'refund_done', 'custom')),
  CONSTRAINT message_message_channel_check CHECK (message_channel IN ('email', 'sms', 'push', 'in_app')),
  CONSTRAINT message_text_check CHECK (char_length(text) BETWEEN 1 AND 2000)
);

CREATE TABLE messaging.message_template (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  message_channel text NOT NULL,
  message_purpose text NOT NULL,
  headline text,
  text text NOT NULL,
  variables text[] NOT NULL DEFAULT '{}',
  in_language text NOT NULL DEFAULT 'en-US',
  template_status text NOT NULL DEFAULT 'draft',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT message_template_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT message_template_id_check CHECK (id ~ '^tpl_[a-z0-9_]+$'),
  CONSTRAINT message_template_message_channel_check CHECK (message_channel IN ('email', 'sms', 'push', 'in_app')),
  CONSTRAINT message_template_text_check CHECK (char_length(text) BETWEEN 1 AND 2000)
);

CREATE INDEX message_recipient_id_date_sent_idx ON messaging.message (tenant_id, recipient_id, date_sent);
CREATE INDEX message_about_id_idx ON messaging.message (tenant_id, about_id);
CREATE INDEX message_change_set_id_idx ON messaging.message (tenant_id, change_set_id);
CREATE INDEX message_preview_id_idx ON messaging.message (tenant_id, preview_id);
CREATE INDEX message_template_message_channel_idx ON messaging.message_template (tenant_id, message_channel);

COMMENT ON TABLE messaging.message IS
  'schema:Message — a transactional message to one shopper about an order or claim; a sent message cannot be taken back. id prefix msg_.';
COMMENT ON COLUMN messaging.message.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN messaging.message.id IS
  '- id with prefix msg_; the same value the API, logs and events use.';
COMMENT ON COLUMN messaging.message.recipient_id IS
  'schema:recipient — shopper the message goes to (cus_ id).';
COMMENT ON COLUMN messaging.message.about_id IS
  'schema:about — order or claim the message is about (ord_ or clm_ id).';
COMMENT ON COLUMN messaging.message.message_kind IS
  'acom:messageKind — what the message does. values: message_kind';
COMMENT ON COLUMN messaging.message.message_channel IS
  'acom:messageChannel — channel it is sent on. values: message_channel';
COMMENT ON COLUMN messaging.message.headline IS
  'schema:headline — subject line for email and push.';
COMMENT ON COLUMN messaging.message.text IS
  'schema:text — body exactly as the shopper reads it, up to 2,000 characters.';
COMMENT ON COLUMN messaging.message.sender_id IS
  'schema:sender — operator or system that sent it (adm_ or sys_ id).';
COMMENT ON COLUMN messaging.message.preview_id IS
  'acom:previewId — send_customer_message preview that was run (pv_ id).';
COMMENT ON COLUMN messaging.message.change_set_id IS
  'acom:changeSetId — change set that approved the message.';
COMMENT ON COLUMN messaging.message.date_sent IS
  'schema:dateSent — when it was sent; NULL while queued.';
COMMENT ON COLUMN messaging.message.date_read IS
  'schema:dateRead — when the shopper opened it, if the channel reports it.';
COMMENT ON COLUMN messaging.message.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN messaging.message.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN messaging.message.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE messaging.message_template IS
  'schema:CreativeWork/acom:MessageTemplate — a reusable message for one channel, with named variables; creating one sends nothing. id prefix tpl_.';
COMMENT ON COLUMN messaging.message_template.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN messaging.message_template.id IS
  '- id with prefix tpl_; the same value the API, logs and events use.';
COMMENT ON COLUMN messaging.message_template.name IS
  'schema:name — name operators pick the template by, for example Back in stock.';
COMMENT ON COLUMN messaging.message_template.message_channel IS
  'acom:messageChannel — channel the template is written for; a campaign must use the same channel. values: message_channel';
COMMENT ON COLUMN messaging.message_template.message_purpose IS
  'acom:messagePurpose — marketing (needs opt-in consent, held in quiet hours) or transactional (about an order or claim).';
COMMENT ON COLUMN messaging.message_template.headline IS
  'schema:headline — subject line for email and push; NULL for sms and in-app.';
COMMENT ON COLUMN messaging.message_template.text IS
  'schema:text — body with variables written as {{name}}, up to 2,000 characters.';
COMMENT ON COLUMN messaging.message_template.variables IS
  'acom:variables — variable names the headline and text may use, for example {customer_name,product_name}.';
COMMENT ON COLUMN messaging.message_template.in_language IS
  'schema:inLanguage — BCP 47 language tag of the text, for example en-US.';
COMMENT ON COLUMN messaging.message_template.template_status IS
  'acom:templateStatus — draft until a campaign uses the template.';
COMMENT ON COLUMN messaging.message_template.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN messaging.message_template.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN messaging.message_template.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE messaging.message ENABLE ROW LEVEL SECURITY;
ALTER TABLE messaging.message FORCE ROW LEVEL SECURITY;
CREATE POLICY message_tenant_isolation ON messaging.message
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE messaging.message_template ENABLE ROW LEVEL SECURITY;
ALTER TABLE messaging.message_template FORCE ROW LEVEL SECURITY;
CREATE POLICY message_template_tenant_isolation ON messaging.message_template
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
