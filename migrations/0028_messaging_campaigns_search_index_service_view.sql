-- 0028_messaging_campaigns_search_index_service_view.sql
-- Campaigns that are set up and sent (create_campaign, send_bulk, the scheduled send) and the
-- messages they leave, the search lexicon and reindex jobs (get_lexicon, update_lexicon,
-- reindex, get_index_status), and the analytics read model of customer inquiries
-- (get_cs_overview). Rules: docs/backend/database.md. Order: tables and columns, indexes,
-- comments, row-level security, grants, then the view.


-- Who a campaign goes to besides a tier, and the template values it is sent with.
ALTER TABLE messaging.campaign ADD COLUMN segment_id text;
ALTER TABLE messaging.campaign ADD COLUMN recipient_ids text[];
ALTER TABLE messaging.campaign ADD COLUMN variable_values jsonb;
ALTER TABLE messaging.campaign ADD CONSTRAINT campaign_segment_id_check CHECK (segment_id ~ '^seg_[a-z0-9_]+$');

-- A campaign's messages: one message per shopper it reached, with the campaign it came from.
ALTER TABLE messaging.message ADD COLUMN campaign_id text;
ALTER TABLE messaging.message ADD CONSTRAINT message_campaign_id_fkey FOREIGN KEY (tenant_id, campaign_id) REFERENCES messaging.campaign (tenant_id, id);
ALTER TABLE messaging.message DROP CONSTRAINT message_message_kind_check;
ALTER TABLE messaging.message ADD CONSTRAINT message_message_kind_check CHECK (message_kind IN ('ask_for_photos', 'decline', 'refund_done', 'custom', 'campaign'));


CREATE TABLE search.lexicon (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  in_language text NOT NULL,
  analyzer_version text NOT NULL,
  synonyms jsonb NOT NULL DEFAULT '[]',
  neologisms jsonb NOT NULL DEFAULT '[]',
  boost_rules jsonb NOT NULL DEFAULT '[]',
  agent_id text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT lexicon_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT lexicon_id_check CHECK (id ~ '^lex_[a-z0-9_]+$'),
  CONSTRAINT lexicon_analyzer_version_key UNIQUE (tenant_id, in_language, analyzer_version),
  CONSTRAINT lexicon_in_language_check CHECK (in_language ~ '^[a-z]{2,3}$'),
  CONSTRAINT lexicon_analyzer_version_check CHECK (analyzer_version ~ '^v[0-9]+$')
);

CREATE TABLE search.reindex_job (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  reindex_mode text NOT NULL,
  in_language text NOT NULL,
  analyzer_version text NOT NULL,
  name text NOT NULL,
  product_group_ids text[],
  changed_since timestamptz,
  action_status text NOT NULL DEFAULT 'active',
  document_count integer NOT NULL DEFAULT 0,
  removed_count integer NOT NULL DEFAULT 0,
  result jsonb,
  error jsonb,
  agent_id text NOT NULL,
  start_time timestamptz NOT NULL,
  end_time timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT reindex_job_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT reindex_job_id_check CHECK (id ~ '^rij_[a-z0-9_]+$'),
  CONSTRAINT reindex_job_reindex_mode_check CHECK (reindex_mode IN ('full', 'incremental')),
  CONSTRAINT reindex_job_action_status_check CHECK (action_status IN ('potential', 'active', 'completed', 'failed')),
  CONSTRAINT reindex_job_analyzer_version_check CHECK (analyzer_version ~ '^v[0-9]+$'),
  CONSTRAINT reindex_job_document_count_check CHECK (document_count >= 0),
  CONSTRAINT reindex_job_removed_count_check CHECK (removed_count >= 0),
  CONSTRAINT reindex_job_end_time_check CHECK (end_time >= start_time)
);

CREATE INDEX message_campaign_id_idx ON messaging.message (tenant_id, campaign_id);
CREATE INDEX lexicon_in_language_idx ON search.lexicon (tenant_id, in_language, created_at);
CREATE INDEX reindex_job_start_time_idx ON search.reindex_job (tenant_id, start_time);

COMMENT ON TABLE messaging.message IS
  'schema:Message — a message to one shopper: a transactional one about an order or claim, or one shopper''s copy of a campaign; a sent message cannot be taken back. id prefix msg_.';
COMMENT ON COLUMN messaging.message.campaign_id IS
  'acom:campaignId — campaign this message is one shopper''s copy of (cmp_ id); NULL for a transactional message.';
COMMENT ON COLUMN messaging.campaign.valid_for_member_tier IS
  'schema:validForMemberTier — tier the campaign goes to; NULL when it goes to a segment, a list of shoppers, or every member who agreed to marketing messages.';
COMMENT ON COLUMN messaging.campaign.segment_id IS
  'acom:segmentId — segment the campaign goes to (seg_ id); NULL unless it was set up for a segment.';
COMMENT ON COLUMN messaging.campaign.recipient_ids IS
  'schema:recipient — shoppers the campaign goes to when it was set up for a list (cus_ ids); NULL otherwise.';
COMMENT ON COLUMN messaging.campaign.variable_values IS
  'acom:variableValues — values of the template variables that are the same for every shopper, as [{name, value}]; per-shopper ones (customer_name) are filled at sending.';

COMMENT ON TABLE search.lexicon IS
  'schema:DefinedTermSet/acom:Lexicon — one version of the store''s search dictionary for one language: synonyms, new words and boost or exclude rules; a version takes effect when a reindex uses it. id prefix lex_.';
COMMENT ON COLUMN search.lexicon.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN search.lexicon.id IS
  '- id with prefix lex_; the same value the API, logs and events use.';
COMMENT ON COLUMN search.lexicon.in_language IS
  'schema:inLanguage — language the dictionary is for, as a two- or three-letter code, for example en.';
COMMENT ON COLUMN search.lexicon.analyzer_version IS
  'acom:analyzerVersion — version of the dictionary, v1 being the store policy''s default; it goes up by one with every change.';
COMMENT ON COLUMN search.lexicon.synonyms IS
  'acom:synonyms — the whole synonym set, as [{terms, bidirectional}]; a one-way entry widens its first term to the others.';
COMMENT ON COLUMN search.lexicon.neologisms IS
  'acom:neologisms — new words and what they mean in common words, as [{surface, normalize}].';
COMMENT ON COLUMN search.lexicon.boost_rules IS
  'acom:boostRules — words that raise products in the ranking or exclude them, as [{term, effect}]; an excluded word finds nothing.';
COMMENT ON COLUMN search.lexicon.agent_id IS
  'schema:agent — who saved this version (adm_ or sys_ id).';
COMMENT ON COLUMN search.lexicon.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN search.lexicon.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN search.lexicon.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE search.reindex_job IS
  'schema:UpdateAction/acom:ReindexJob — one rebuild of the store''s search documents, whole or only what changed, with the dictionary version it used; the documents switch over when it commits. id prefix rij_.';
COMMENT ON COLUMN search.reindex_job.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN search.reindex_job.id IS
  '- id with prefix rij_; the same value the API, logs and events use.';
COMMENT ON COLUMN search.reindex_job.reindex_mode IS
  'acom:reindexMode — full rebuilds every document; incremental only the products named, changed since a time, or out of date. values: reindex_mode';
COMMENT ON COLUMN search.reindex_job.in_language IS
  'schema:inLanguage — language of the index, for example en.';
COMMENT ON COLUMN search.reindex_job.analyzer_version IS
  'acom:analyzerVersion — dictionary version the index uses from this job on (search.lexicon, or v1 for the policy default).';
COMMENT ON COLUMN search.reindex_job.name IS
  'schema:name — name of the index, after search.yml index.naming, for example products-store_northwind-en-v3.';
COMMENT ON COLUMN search.reindex_job.product_group_ids IS
  'schema:object — products an incremental job was limited to (prd_ ids; API product_ids); NULL when not limited to a list.';
COMMENT ON COLUMN search.reindex_job.changed_since IS
  'acom:changedSince — an incremental job rebuilt the products changed at or after this time; NULL when not limited by time.';
COMMENT ON COLUMN search.reindex_job.action_status IS
  'schema:actionStatus — active while it runs, then completed or failed. values: action_status';
COMMENT ON COLUMN search.reindex_job.document_count IS
  'acom:documentCount — search documents written.';
COMMENT ON COLUMN search.reindex_job.removed_count IS
  'acom:removedCount — search documents removed because their product is gone.';
COMMENT ON COLUMN search.reindex_job.result IS
  'schema:result — what the job reports besides the counts, for example that no embedding model is connected.';
COMMENT ON COLUMN search.reindex_job.error IS
  'schema:error — why the job failed, as {code, message}; NULL unless action_status is failed.';
COMMENT ON COLUMN search.reindex_job.agent_id IS
  'schema:agent — who ran it (adm_ or sys_ id).';
COMMENT ON COLUMN search.reindex_job.start_time IS
  'schema:startTime — when the job started.';
COMMENT ON COLUMN search.reindex_job.end_time IS
  'schema:endTime — when it finished; NULL while it runs.';
COMMENT ON COLUMN search.reindex_job.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN search.reindex_job.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN search.reindex_job.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE search.lexicon ENABLE ROW LEVEL SECURITY;
ALTER TABLE search.lexicon FORCE ROW LEVEL SECURITY;
CREATE POLICY lexicon_tenant_isolation ON search.lexicon
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE search.reindex_job ENABLE ROW LEVEL SECURITY;
ALTER TABLE search.reindex_job FORCE ROW LEVEL SECURITY;
CREATE POLICY reindex_job_tenant_isolation ON search.reindex_job
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- DELETE where removing a row is part of the design (see 0014_app_delete_grants.sql):
--   search.search_document   a reindex removes the documents of products that are gone
--   search.lexicon           undoing update_lexicon removes a version no index has used
-- Reindex jobs are only ever added to.
GRANT DELETE ON
  search.search_document,
  search.lexicon
TO acom_app;


-- One row per customer inquiry with only what service figures need (day, type, state, when it
-- was first answered and resolved), never who asked or what they wrote.
CREATE VIEW analytics.inquiry_summary AS
SELECT
  q.tenant_id,
  q.id AS inquiry_id,
  (q.date_received AT TIME ZONE s.time_zone)::date AS observation_date,
  q.inquiry_type,
  q.inquiry_status,
  q.date_received,
  r.first_response_time,
  q.resolved_at
FROM customer.customer_inquiry AS q
JOIN platform.online_store AS s ON s.id = q.tenant_id
LEFT JOIN LATERAL (
  SELECT min(c.date_created) AS first_response_time FROM customer.comment AS c
  WHERE c.tenant_id = q.tenant_id AND c.inquiry_id = q.id AND c.author_type IN ('staff', 'agent')
) AS r ON true;

COMMENT ON VIEW analytics.inquiry_summary IS
  'schema:Observation/acom:InquirySummary — one row per customer inquiry: the store day it came in, its type and state, when staff or the agent first answered and when it was resolved; never who asked or what they wrote.';
COMMENT ON COLUMN analytics.inquiry_summary.tenant_id IS
  '- store (tenant) the inquiry belongs to.';
COMMENT ON COLUMN analytics.inquiry_summary.inquiry_id IS
  'acom:inquiryId — the inquiry (inq_ id).';
COMMENT ON COLUMN analytics.inquiry_summary.observation_date IS
  'schema:observationDate — calendar day in the store''s time zone on which the inquiry came in.';
COMMENT ON COLUMN analytics.inquiry_summary.inquiry_type IS
  'acom:inquiryType — what the inquiry is about. values: inquiry_type';
COMMENT ON COLUMN analytics.inquiry_summary.inquiry_status IS
  'acom:inquiryStatus — where the inquiry is now. values: inquiry_status';
COMMENT ON COLUMN analytics.inquiry_summary.date_received IS
  'schema:dateReceived — when the inquiry came in.';
COMMENT ON COLUMN analytics.inquiry_summary.first_response_time IS
  'acom:firstResponseTime — when staff or the agent first wrote in the thread; NULL while nobody has.';
COMMENT ON COLUMN analytics.inquiry_summary.resolved_at IS
  'acom:resolvedAt — when the inquiry was resolved; NULL while it is open.';
