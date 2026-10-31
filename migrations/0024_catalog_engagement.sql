-- 0024_catalog_engagement.sql
-- Catalog: what shoppers and the store add around a product. Product questions and the store's
-- answers (ask_question, answer_question), reports on reviews (report_review), restock alerts
-- (subscribe_restock), bundles made of other variants (create_bundle), and drafts of generated
-- copy and images (generate_copy, generate_image). Generated drafts stay off the product page
-- until a person or an agent uses them (update_product_group).
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security, grants.


CREATE TABLE catalog.question (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  product_group_id text NOT NULL,
  author_id text NOT NULL,
  author_name text NOT NULL,
  text text NOT NULL,
  is_private boolean NOT NULL DEFAULT false,
  creative_work_status text NOT NULL DEFAULT 'visible',
  date_created timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT question_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT question_id_check CHECK (id ~ '^qst_[a-z0-9_]+$'),
  CONSTRAINT question_product_group_id_fkey FOREIGN KEY (tenant_id, product_group_id) REFERENCES catalog.product_group (tenant_id, id),
  CONSTRAINT question_author_id_fkey FOREIGN KEY (tenant_id, author_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT question_creative_work_status_check CHECK (creative_work_status IN ('visible', 'hidden'))
);

CREATE TABLE catalog.answer (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  question_id text NOT NULL,
  author_id text NOT NULL,
  author_name text,
  text text NOT NULL,
  date_created timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT answer_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT answer_id_check CHECK (id ~ '^ans_[a-z0-9_]+$'),
  CONSTRAINT answer_question_id_fkey FOREIGN KEY (tenant_id, question_id) REFERENCES catalog.question (tenant_id, id) ON DELETE CASCADE,
  CONSTRAINT answer_question_id_key UNIQUE (tenant_id, question_id)
);

CREATE TABLE catalog.review_report (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  review_id text NOT NULL,
  customer_id text NOT NULL,
  report_reason text NOT NULL,
  description text,
  start_time timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT review_report_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT review_report_id_check CHECK (id ~ '^rpt_[a-z0-9_]+$'),
  CONSTRAINT review_report_review_id_fkey FOREIGN KEY (tenant_id, review_id) REFERENCES catalog.review (tenant_id, id) ON DELETE CASCADE,
  CONSTRAINT review_report_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT review_report_review_id_customer_id_key UNIQUE (tenant_id, review_id, customer_id),
  CONSTRAINT review_report_report_reason_check CHECK (report_reason IN ('spam', 'abuse', 'obscene', 'advertisement', 'fake', 'irrelevant', 'other'))
);

CREATE TABLE catalog.restock_alert (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  customer_id text NOT NULL,
  product_id text NOT NULL,
  message_channel text NOT NULL,
  start_time timestamptz NOT NULL,
  expires timestamptz NOT NULL,
  notified_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT restock_alert_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT restock_alert_id_check CHECK (id ~ '^rsa_[a-z0-9_]+$'),
  CONSTRAINT restock_alert_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT restock_alert_product_id_fkey FOREIGN KEY (tenant_id, product_id) REFERENCES catalog.product (tenant_id, id),
  CONSTRAINT restock_alert_message_channel_check CHECK (message_channel IN ('email', 'sms', 'push', 'in_app')),
  CONSTRAINT restock_alert_period_check CHECK (expires > start_time)
);

CREATE TABLE catalog.type_and_quantity_node (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  product_group_id text NOT NULL,
  product_id text NOT NULL,
  amount_of_this_good integer NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT type_and_quantity_node_pkey PRIMARY KEY (tenant_id, product_group_id, product_id),
  CONSTRAINT type_and_quantity_node_product_group_id_fkey FOREIGN KEY (tenant_id, product_group_id) REFERENCES catalog.product_group (tenant_id, id),
  CONSTRAINT type_and_quantity_node_product_id_fkey FOREIGN KEY (tenant_id, product_id) REFERENCES catalog.product (tenant_id, id),
  CONSTRAINT type_and_quantity_node_amount_of_this_good_check CHECK (amount_of_this_good > 0)
);

CREATE TABLE catalog.copy_draft (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  product_group_id text NOT NULL,
  copy_type text NOT NULL,
  text text NOT NULL,
  keywords text[] NOT NULL DEFAULT '{}',
  tone text NOT NULL,
  is_ai_generated boolean NOT NULL DEFAULT true,
  date_created timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT copy_draft_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT copy_draft_id_check CHECK (id ~ '^cpd_[a-z0-9_]+$'),
  CONSTRAINT copy_draft_product_group_id_fkey FOREIGN KEY (tenant_id, product_group_id) REFERENCES catalog.product_group (tenant_id, id),
  CONSTRAINT copy_draft_copy_type_check CHECK (copy_type IN ('title', 'summary', 'description', 'seo'))
);

CREATE TABLE catalog.image_object (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  product_group_id text NOT NULL,
  content_url text NOT NULL,
  encoding_format text NOT NULL,
  caption text NOT NULL,
  prompt text NOT NULL,
  image_style text NOT NULL,
  is_ai_generated boolean NOT NULL DEFAULT true,
  date_created timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT image_object_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT image_object_id_check CHECK (id ~ '^img_[a-z0-9_]+$'),
  CONSTRAINT image_object_product_group_id_fkey FOREIGN KEY (tenant_id, product_group_id) REFERENCES catalog.product_group (tenant_id, id),
  CONSTRAINT image_object_image_style_check CHECK (image_style IN ('studio', 'lifestyle', 'flat_lay'))
);

CREATE INDEX question_product_group_id_idx ON catalog.question (tenant_id, product_group_id);
CREATE INDEX question_author_id_idx ON catalog.question (tenant_id, author_id);
CREATE INDEX review_report_customer_id_idx ON catalog.review_report (tenant_id, customer_id);
CREATE INDEX restock_alert_customer_id_idx ON catalog.restock_alert (tenant_id, customer_id);
CREATE INDEX restock_alert_open_idx ON catalog.restock_alert (tenant_id, product_id) WHERE notified_at IS NULL;
CREATE INDEX type_and_quantity_node_product_id_idx ON catalog.type_and_quantity_node (tenant_id, product_id);
CREATE INDEX copy_draft_product_group_id_idx ON catalog.copy_draft (tenant_id, product_group_id);
CREATE INDEX image_object_product_group_id_idx ON catalog.image_object (tenant_id, product_group_id);

COMMENT ON TABLE catalog.question IS
  'schema:Question — a shopper''s question about a product; a hidden question hides its answer too. id prefix qst_.';
COMMENT ON COLUMN catalog.question.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.question.id IS
  '- id with prefix qst_; the same value the API, logs and events use (API question_id).';
COMMENT ON COLUMN catalog.question.product_group_id IS
  'schema:about — the product asked about (prd_ id, API product_id).';
COMMENT ON COLUMN catalog.question.author_id IS
  'schema:author — the shopper who asked (cus_ id); only they may edit or delete it.';
COMMENT ON COLUMN catalog.question.author_name IS
  'schema:author — the name shown with the question, masked as catalog.yml author_masking says when it was asked, for example Ja***.';
COMMENT ON COLUMN catalog.question.text IS
  'schema:text — the question as the shopper wrote it.';
COMMENT ON COLUMN catalog.question.is_private IS
  'acom:isPrivate — true when only the asker and the store may read it (catalog.yml question.private_allowed).';
COMMENT ON COLUMN catalog.question.creative_work_status IS
  'schema:creativeWorkStatus — visible, or hidden by moderate_question or held by question.profanity_filter. values: question_status';
COMMENT ON COLUMN catalog.question.date_created IS
  'schema:dateCreated — when the shopper asked (store clock).';
COMMENT ON COLUMN catalog.question.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.question.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.question.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE catalog.answer IS
  'schema:Answer — the accepted answer to a product question; one per question. id prefix ans_.';
COMMENT ON COLUMN catalog.answer.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.answer.id IS
  '- id with prefix ans_; the same value the API, logs and events use (API answer_id).';
COMMENT ON COLUMN catalog.answer.question_id IS
  'schema:parentItem — the question answered (qst_ id); it shows the answer as acceptedAnswer.';
COMMENT ON COLUMN catalog.answer.author_id IS
  'schema:author — who answered: an operator (adm_), the system (sys_), or a shopper (cus_) when catalog.yml question.answered_by allows it.';
COMMENT ON COLUMN catalog.answer.author_name IS
  'schema:author — the masked name shown for a shopper''s answer; NULL when the store answered, shown as the store.';
COMMENT ON COLUMN catalog.answer.text IS
  'schema:text — the answer shoppers read, word for word as previewed.';
COMMENT ON COLUMN catalog.answer.date_created IS
  'schema:dateCreated — when the answer was posted (store clock).';
COMMENT ON COLUMN catalog.answer.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.answer.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.answer.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE catalog.review_report IS
  'schema:Action/acom:ContentReport — a shopper''s report that a review is inappropriate; it puts the review in the moderation queue. id prefix rpt_.';
COMMENT ON COLUMN catalog.review_report.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.review_report.id IS
  '- id with prefix rpt_; the same value the API, logs and events use.';
COMMENT ON COLUMN catalog.review_report.review_id IS
  'schema:object — the review reported (rev_ id).';
COMMENT ON COLUMN catalog.review_report.customer_id IS
  'schema:agent — the shopper who reported it (cus_ id); one report per shopper and review.';
COMMENT ON COLUMN catalog.review_report.report_reason IS
  'acom:reportReason — why it was reported (catalog.yml review.report_reasons). values: report_reason';
COMMENT ON COLUMN catalog.review_report.description IS
  'schema:description — the reporter''s own words; NULL when they gave none.';
COMMENT ON COLUMN catalog.review_report.start_time IS
  'schema:startTime — when the report was made (store clock).';
COMMENT ON COLUMN catalog.review_report.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.review_report.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.review_report.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE catalog.restock_alert IS
  'schema:SubscribeAction/acom:RestockAlert — a shopper asks to be told when a sold-out variant is back; a transactional alert, not marketing. id prefix rsa_.';
COMMENT ON COLUMN catalog.restock_alert.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.restock_alert.id IS
  '- id with prefix rsa_; the same value the API, logs and events use (API restock_alert_id).';
COMMENT ON COLUMN catalog.restock_alert.customer_id IS
  'schema:agent — the shopper who asked (cus_ id); only they see it.';
COMMENT ON COLUMN catalog.restock_alert.product_id IS
  'schema:object — the variant waited for (var_ id, API variant_id).';
COMMENT ON COLUMN catalog.restock_alert.message_channel IS
  'acom:messageChannel — how the shopper wants to hear (catalog.yml restock_alert.channels). values: message_channel';
COMMENT ON COLUMN catalog.restock_alert.start_time IS
  'schema:startTime — when the shopper asked (store clock).';
COMMENT ON COLUMN catalog.restock_alert.expires IS
  'schema:expires — when the request lapses if the variant is not back by then (start_time + restock_alert.expires_after_days).';
COMMENT ON COLUMN catalog.restock_alert.notified_at IS
  'acom:notifiedAt — when the variant came back and the alert was handed to messaging (platform.outbox topic restock_alert.notify); NULL while waiting.';
COMMENT ON COLUMN catalog.restock_alert.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.restock_alert.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.restock_alert.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE catalog.type_and_quantity_node IS
  'schema:TypeAndQuantityNode — one component of a bundle (create_bundle): a variant and how many of it; the bundle''s price and stock are derived from its components.';
COMMENT ON COLUMN catalog.type_and_quantity_node.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.type_and_quantity_node.product_group_id IS
  'acom:productGroupId — the bundle this is a component of (prd_ id); schema:includesObject seen from the bundle.';
COMMENT ON COLUMN catalog.type_and_quantity_node.product_id IS
  'schema:typeOfGood — the component variant (var_ id, API variant_id).';
COMMENT ON COLUMN catalog.type_and_quantity_node.amount_of_this_good IS
  'schema:amountOfThisGood — units of the variant in one bundle (API order_quantity).';
COMMENT ON COLUMN catalog.type_and_quantity_node.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.type_and_quantity_node.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.type_and_quantity_node.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE catalog.copy_draft IS
  'schema:CreativeWork/acom:CopyDraft — generated text for a product (generate_copy), kept as a draft until update_product_group uses it. id prefix cpd_.';
COMMENT ON COLUMN catalog.copy_draft.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.copy_draft.id IS
  '- id with prefix cpd_; the same value the API, logs and events use (API copy_draft_id).';
COMMENT ON COLUMN catalog.copy_draft.product_group_id IS
  'schema:about — the product the copy is for (prd_ id, API product_id).';
COMMENT ON COLUMN catalog.copy_draft.copy_type IS
  'acom:copyType — which text of the product this drafts. values: copy_type';
COMMENT ON COLUMN catalog.copy_draft.text IS
  'schema:text — the drafted text.';
COMMENT ON COLUMN catalog.copy_draft.keywords IS
  'schema:keywords — words the draft was asked to use; empty when none.';
COMMENT ON COLUMN catalog.copy_draft.tone IS
  'acom:tone — the tone the draft was written in: the request''s, or catalog.yml content.copy.tone.';
COMMENT ON COLUMN catalog.copy_draft.is_ai_generated IS
  'acom:aiGenerated — true for generated content (catalog.yml content.ai_content_disclosure).';
COMMENT ON COLUMN catalog.copy_draft.date_created IS
  'schema:dateCreated — when the draft was made (store clock).';
COMMENT ON COLUMN catalog.copy_draft.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.copy_draft.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.copy_draft.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE catalog.image_object IS
  'schema:ImageObject — a generated product image (generate_image), kept as a draft off the product page until it is used. id prefix img_.';
COMMENT ON COLUMN catalog.image_object.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.image_object.id IS
  '- id with prefix img_; the same value the API, logs and events use (API image_id).';
COMMENT ON COLUMN catalog.image_object.product_group_id IS
  'schema:about — the product pictured (prd_ id, API product_id).';
COMMENT ON COLUMN catalog.image_object.content_url IS
  'schema:contentUrl — where the image is; while no image generator is connected (catalog.yml content.generator: placeholder), a placeholder picture that says so.';
COMMENT ON COLUMN catalog.image_object.encoding_format IS
  'schema:encodingFormat — media type of the image, for example image/svg+xml.';
COMMENT ON COLUMN catalog.image_object.caption IS
  'schema:caption — the alternative text shown with the image.';
COMMENT ON COLUMN catalog.image_object.prompt IS
  'acom:prompt — what the operator asked the image to show.';
COMMENT ON COLUMN catalog.image_object.image_style IS
  'acom:imageStyle — how the image is staged. values: image_style';
COMMENT ON COLUMN catalog.image_object.is_ai_generated IS
  'acom:aiGenerated — true for generated content (catalog.yml content.ai_content_disclosure).';
COMMENT ON COLUMN catalog.image_object.date_created IS
  'schema:dateCreated — when the image was made (store clock).';
COMMENT ON COLUMN catalog.image_object.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.image_object.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.image_object.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE catalog.question ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.question FORCE ROW LEVEL SECURITY;
CREATE POLICY question_tenant_isolation ON catalog.question
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE catalog.answer ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.answer FORCE ROW LEVEL SECURITY;
CREATE POLICY answer_tenant_isolation ON catalog.answer
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE catalog.review_report ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.review_report FORCE ROW LEVEL SECURITY;
CREATE POLICY review_report_tenant_isolation ON catalog.review_report
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE catalog.restock_alert ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.restock_alert FORCE ROW LEVEL SECURITY;
CREATE POLICY restock_alert_tenant_isolation ON catalog.restock_alert
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE catalog.type_and_quantity_node ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.type_and_quantity_node FORCE ROW LEVEL SECURITY;
CREATE POLICY type_and_quantity_node_tenant_isolation ON catalog.type_and_quantity_node
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE catalog.copy_draft ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.copy_draft FORCE ROW LEVEL SECURITY;
CREATE POLICY copy_draft_tenant_isolation ON catalog.copy_draft
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE catalog.image_object ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.image_object FORCE ROW LEVEL SECURITY;
CREATE POLICY image_object_tenant_isolation ON catalog.image_object
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- DELETE where removing a row is part of the design (see 0014_app_delete_grants.sql):
--   catalog.review, catalog.review_vote, catalog.review_report
--                                  delete_review: a shopper removes their own review
--   catalog.question, catalog.answer
--                                  delete_question: a shopper removes their own question
--   catalog.copy_draft, catalog.image_object, catalog.type_and_quantity_node
--                                  undoing generate_copy, generate_image, create_bundle
GRANT DELETE ON
  catalog.review,
  catalog.review_vote,
  catalog.review_report,
  catalog.question,
  catalog.answer,
  catalog.copy_draft,
  catalog.image_object,
  catalog.type_and_quantity_node
TO acom_app;
