-- 0031_integration_adapters.sql
-- The integration domain: outside channels (marketplaces, carriers, payment providers,
-- fulfillment services) with their status mappings and hashed secrets; the copies of the orders
-- and claims a channel holds; product imports from outside feeds with one result per row; and
-- the events carriers and payment providers send.
-- Rules: docs/backend/database.md. Order: schema, tables, indexes, comments, row-level security,
-- grants.

CREATE SCHEMA IF NOT EXISTS integration;

COMMENT ON SCHEMA integration IS
  'Outside channels and their status mappings, copies of channel orders and claims, product imports from outside feeds, received carrier and payment events.';


CREATE TABLE integration.channel (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  channel_type text NOT NULL,
  url text NOT NULL,
  connection_status text NOT NULL DEFAULT 'connected',
  last_synced_at timestamptz,
  seller_id text,
  credential_hash text NOT NULL,
  signing_secret_hash text,
  status_mapping jsonb NOT NULL DEFAULT '{}',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT channel_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT channel_id_check CHECK (id ~ '^chn_[a-z0-9_]+$'),
  CONSTRAINT channel_name_url_key UNIQUE (tenant_id, name, url),
  CONSTRAINT channel_name_check CHECK (char_length(name) BETWEEN 1 AND 120),
  CONSTRAINT channel_channel_type_check CHECK (channel_type IN ('marketplace', 'carrier', 'payment', 'fulfillment')),
  CONSTRAINT channel_url_check CHECK (url ~ '^https?://'),
  CONSTRAINT channel_connection_status_check CHECK (connection_status IN ('connected', 'degraded', 'disconnected')),
  CONSTRAINT channel_seller_id_check CHECK (seller_id ~ '^slr_[a-z0-9_]+$'),
  CONSTRAINT channel_credential_hash_check CHECK (credential_hash ~ '^[0-9a-f]{64}$'),
  CONSTRAINT channel_signing_secret_hash_check CHECK (signing_secret_hash ~ '^[0-9a-f]{64}$'),
  CONSTRAINT channel_status_mapping_check CHECK (jsonb_typeof(status_mapping) = 'object')
);

CREATE TABLE integration.external_order (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  channel_id text NOT NULL,
  external_order_ref text NOT NULL,
  order_id text,
  external_status text NOT NULL,
  ordered_item jsonb NOT NULL DEFAULT '[]',
  order_date timestamptz NOT NULL,
  last_synced_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT external_order_pkey PRIMARY KEY (tenant_id, channel_id, external_order_ref),
  CONSTRAINT external_order_channel_id_fkey FOREIGN KEY (tenant_id, channel_id) REFERENCES integration.channel (tenant_id, id),
  CONSTRAINT external_order_order_id_fkey FOREIGN KEY (tenant_id, order_id) REFERENCES sales.sales_order (tenant_id, id),
  CONSTRAINT external_order_external_order_ref_check CHECK (char_length(external_order_ref) BETWEEN 1 AND 120),
  CONSTRAINT external_order_external_status_check CHECK (char_length(external_status) BETWEEN 1 AND 80),
  CONSTRAINT external_order_ordered_item_check CHECK (jsonb_typeof(ordered_item) = 'array')
);

CREATE TABLE integration.external_claim (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  channel_id text NOT NULL,
  external_claim_ref text NOT NULL,
  external_order_ref text NOT NULL,
  claim_type text NOT NULL,
  claim_id text,
  external_status text NOT NULL,
  ordered_item jsonb NOT NULL DEFAULT '[]',
  start_time timestamptz NOT NULL,
  last_synced_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT external_claim_pkey PRIMARY KEY (tenant_id, channel_id, external_claim_ref),
  CONSTRAINT external_claim_channel_id_fkey FOREIGN KEY (tenant_id, channel_id) REFERENCES integration.channel (tenant_id, id),
  CONSTRAINT external_claim_claim_id_fkey FOREIGN KEY (tenant_id, claim_id) REFERENCES fulfillment.claim (tenant_id, id),
  CONSTRAINT external_claim_external_claim_ref_check CHECK (char_length(external_claim_ref) BETWEEN 1 AND 120),
  CONSTRAINT external_claim_claim_type_check CHECK (claim_type IN ('cancel', 'return', 'exchange')),
  CONSTRAINT external_claim_external_status_check CHECK (char_length(external_status) BETWEEN 1 AND 80),
  CONSTRAINT external_claim_ordered_item_check CHECK (jsonb_typeof(ordered_item) = 'array')
);

CREATE TABLE integration.webhook_event (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  channel_id text NOT NULL,
  carrier_event_type text,
  payment_event_type text,
  event_key text NOT NULL,
  external_order_ref text NOT NULL,
  order_id text NOT NULL,
  tracking_number text,
  payment_ref text,
  amount_value numeric(12,2),
  amount_currency char(3),
  start_time timestamptz NOT NULL,
  received_at timestamptz NOT NULL,
  action_status text NOT NULL DEFAULT 'potential',
  delivery_id text,
  payment_id text,
  refund_id text,
  result jsonb,
  error jsonb,
  end_time timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT webhook_event_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT webhook_event_id_check CHECK (id ~ '^whk_[a-z0-9_]+$'),
  CONSTRAINT webhook_event_event_key_key UNIQUE (tenant_id, event_key),
  CONSTRAINT webhook_event_channel_id_fkey FOREIGN KEY (tenant_id, channel_id) REFERENCES integration.channel (tenant_id, id),
  CONSTRAINT webhook_event_order_id_fkey FOREIGN KEY (tenant_id, order_id) REFERENCES sales.sales_order (tenant_id, id),
  CONSTRAINT webhook_event_delivery_id_fkey FOREIGN KEY (tenant_id, delivery_id) REFERENCES fulfillment.parcel_delivery (tenant_id, id),
  CONSTRAINT webhook_event_payment_id_fkey FOREIGN KEY (tenant_id, payment_id) REFERENCES sales.payment (tenant_id, id),
  CONSTRAINT webhook_event_refund_id_fkey FOREIGN KEY (tenant_id, refund_id) REFERENCES fulfillment.refund (tenant_id, id),
  CONSTRAINT webhook_event_carrier_event_type_check CHECK (carrier_event_type IN ('label_created', 'picked_up', 'in_transit', 'delivered', 'delivery_failed')),
  CONSTRAINT webhook_event_payment_event_type_check CHECK (payment_event_type IN ('authorized', 'deposit_confirmed', 'cancelled', 'refund_completed', 'refund_failed')),
  CONSTRAINT webhook_event_one_type_check CHECK (num_nonnulls(carrier_event_type, payment_event_type) = 1),
  CONSTRAINT webhook_event_action_status_check CHECK (action_status IN ('potential', 'active', 'completed', 'failed')),
  CONSTRAINT webhook_event_amount_value_check CHECK (amount_value >= 0),
  CONSTRAINT webhook_event_amount_currency_check CHECK (amount_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT webhook_event_amount_check CHECK ((amount_value IS NULL) = (amount_currency IS NULL)),
  CONSTRAINT webhook_event_period_check CHECK (end_time IS NULL OR end_time >= received_at)
);

CREATE TABLE integration.product_import (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  seller_id text,
  import_mode text NOT NULL,
  action_status text NOT NULL DEFAULT 'completed',
  category_mapping jsonb NOT NULL DEFAULT '{}',
  agent_id text NOT NULL,
  change_set_id text,
  start_time timestamptz NOT NULL,
  end_time timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT product_import_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT product_import_id_check CHECK (id ~ '^imp_[a-z0-9_]+$'),
  CONSTRAINT product_import_change_set_id_fkey FOREIGN KEY (tenant_id, change_set_id) REFERENCES platform.change_set (tenant_id, id),
  CONSTRAINT product_import_seller_id_check CHECK (seller_id ~ '^slr_[a-z0-9_]+$'),
  CONSTRAINT product_import_import_mode_check CHECK (import_mode IN ('upsert', 'price_only', 'stock_only', 'status_only')),
  CONSTRAINT product_import_action_status_check CHECK (action_status IN ('potential', 'active', 'completed', 'failed')),
  CONSTRAINT product_import_category_mapping_check CHECK (jsonb_typeof(category_mapping) = 'object'),
  CONSTRAINT product_import_period_check CHECK (end_time IS NULL OR end_time >= start_time)
);

CREATE TABLE integration.product_import_item (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  product_import_id text NOT NULL,
  position integer NOT NULL,
  external_id text NOT NULL,
  product_group_id text,
  item_status text NOT NULL,
  variant_mapping jsonb NOT NULL DEFAULT '[]',
  error jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT product_import_item_pkey PRIMARY KEY (tenant_id, product_import_id, position),
  CONSTRAINT product_import_item_product_import_id_fkey FOREIGN KEY (tenant_id, product_import_id) REFERENCES integration.product_import (tenant_id, id),
  CONSTRAINT product_import_item_position_check CHECK (position >= 1),
  CONSTRAINT product_import_item_external_id_check CHECK (char_length(external_id) BETWEEN 1 AND 120),
  CONSTRAINT product_import_item_item_status_check CHECK (item_status IN ('accepted', 'rejected')),
  CONSTRAINT product_import_item_variant_mapping_check CHECK (jsonb_typeof(variant_mapping) = 'array'),
  CONSTRAINT product_import_item_result_check CHECK ((item_status = 'accepted') = (error IS NULL))
);

CREATE INDEX channel_channel_type_idx ON integration.channel (tenant_id, channel_type);
CREATE INDEX external_order_order_id_idx ON integration.external_order (tenant_id, order_id);
CREATE INDEX external_order_order_date_idx ON integration.external_order (tenant_id, channel_id, order_date);
CREATE INDEX external_claim_claim_id_idx ON integration.external_claim (tenant_id, claim_id);
CREATE INDEX external_claim_start_time_idx ON integration.external_claim (tenant_id, channel_id, start_time);
CREATE INDEX webhook_event_order_id_idx ON integration.webhook_event (tenant_id, order_id);
CREATE INDEX webhook_event_action_status_idx ON integration.webhook_event (tenant_id, action_status);
CREATE INDEX product_import_seller_id_idx ON integration.product_import (tenant_id, seller_id);
CREATE INDEX product_import_change_set_id_idx ON integration.product_import (tenant_id, change_set_id);
CREATE INDEX product_import_item_external_id_idx ON integration.product_import_item (tenant_id, external_id);

COMMENT ON TABLE integration.channel IS
  'schema:WebAPI/acom:Channel — an outside system the store is connected to: a marketplace, a carrier, a payment provider or a fulfillment service. id prefix chn_.';
COMMENT ON COLUMN integration.channel.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN integration.channel.id IS
  '- id with prefix chn_; the same value the API, logs and events use.';
COMMENT ON COLUMN integration.channel.name IS
  'schema:name — the name people use for the channel, for example UPS; unique with its url.';
COMMENT ON COLUMN integration.channel.channel_type IS
  'acom:channelType — what the channel is: marketplace, carrier, payment or fulfillment. values: channel_type';
COMMENT ON COLUMN integration.channel.url IS
  'schema:url — the address of the outside system''s API.';
COMMENT ON COLUMN integration.channel.connection_status IS
  'acom:connectionStatus — whether the store can reach the channel now. values: connection_status';
COMMENT ON COLUMN integration.channel.last_synced_at IS
  'acom:lastSyncedAt — when the store last received data from the channel; NULL if never.';
COMMENT ON COLUMN integration.channel.seller_id IS
  'schema:seller — the marketplace seller (slr_ id) the channel belongs to; NULL for the store''s own channels.';
COMMENT ON COLUMN integration.channel.credential_hash IS
  '- SHA-256 (hex) of the credential reference the channel is called with; the reference itself is never stored or returned.';
COMMENT ON COLUMN integration.channel.signing_secret_hash IS
  '- SHA-256 (hex) of the secret inbound events are signed with; the hash is the HMAC key the signatures are checked with, and the secret itself is never stored; NULL when the channel sends no events.';
COMMENT ON COLUMN integration.channel.status_mapping IS
  'acom:statusMapping — the channel''s status words mapped to ours, {order: {shipped: in_transit}, claim: {return_requested: requested}} (enums.md order_status and claim_status).';
COMMENT ON COLUMN integration.channel.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN integration.channel.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN integration.channel.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE integration.external_order IS
  'schema:Order/acom:ExternalOrder — an order as an outside channel holds it, the copy last received from the channel. Key: channel and the channel''s order reference.';
COMMENT ON COLUMN integration.external_order.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN integration.external_order.channel_id IS
  'acom:channelId — the channel that holds the order (chn_ id).';
COMMENT ON COLUMN integration.external_order.external_order_ref IS
  'acom:externalOrderRef — the channel''s own order number, for example MK-55102; never used as our id.';
COMMENT ON COLUMN integration.external_order.order_id IS
  'acom:orderId — the store''s order it maps to (ord_ id); NULL while the order is not in the store.';
COMMENT ON COLUMN integration.external_order.external_status IS
  'acom:externalStatus — the order''s status word on the channel, as the channel writes it; channel.status_mapping turns it into an order_status.';
COMMENT ON COLUMN integration.external_order.ordered_item IS
  'schema:orderedItem — the lines on the channel, [{external_id, order_quantity}].';
COMMENT ON COLUMN integration.external_order.order_date IS
  'schema:orderDate — when the shopper ordered on the channel.';
COMMENT ON COLUMN integration.external_order.last_synced_at IS
  'acom:lastSyncedAt — when this copy was received from the channel or last sent to it.';
COMMENT ON COLUMN integration.external_order.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN integration.external_order.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN integration.external_order.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE integration.external_claim IS
  'schema:ReturnAction/acom:ExternalClaim — a cancel, return or exchange request as an outside channel holds it, the copy last received from the channel. Key: channel and the channel''s claim reference.';
COMMENT ON COLUMN integration.external_claim.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN integration.external_claim.channel_id IS
  'acom:channelId — the channel that holds the request (chn_ id).';
COMMENT ON COLUMN integration.external_claim.external_claim_ref IS
  'acom:externalClaimRef — the channel''s own request number, for example MK-R-2210; never used as our id.';
COMMENT ON COLUMN integration.external_claim.external_order_ref IS
  'acom:externalOrderRef — the channel''s order the request is about.';
COMMENT ON COLUMN integration.external_claim.claim_type IS
  'acom:claimType — cancel, return or exchange. values: claim_type';
COMMENT ON COLUMN integration.external_claim.claim_id IS
  'acom:claimId — the store''s claim it maps to (clm_ id); NULL while the request is not in the store.';
COMMENT ON COLUMN integration.external_claim.external_status IS
  'acom:externalStatus — the request''s status word on the channel; channel.status_mapping turns it into a claim_status.';
COMMENT ON COLUMN integration.external_claim.ordered_item IS
  'schema:orderedItem — the lines the request covers, [{external_id, order_quantity}].';
COMMENT ON COLUMN integration.external_claim.start_time IS
  'schema:startTime — when the shopper asked on the channel.';
COMMENT ON COLUMN integration.external_claim.last_synced_at IS
  'acom:lastSyncedAt — when this copy was received from the channel or last sent to it.';
COMMENT ON COLUMN integration.external_claim.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN integration.external_claim.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN integration.external_claim.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE integration.webhook_event IS
  'schema:ReceiveAction/acom:WebhookEvent — one event a carrier or a payment provider sent, checked and recorded once, and applied through the owning domain when its rules allow. id prefix whk_.';
COMMENT ON COLUMN integration.webhook_event.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN integration.webhook_event.id IS
  '- id with prefix whk_; the same value the API, logs and events use.';
COMMENT ON COLUMN integration.webhook_event.channel_id IS
  'acom:channelId — the carrier or payment channel that sent the event (chn_ id).';
COMMENT ON COLUMN integration.webhook_event.carrier_event_type IS
  'acom:carrierEventType — what the carrier reported; NULL for a payment event. values: carrier_event_type';
COMMENT ON COLUMN integration.webhook_event.payment_event_type IS
  'acom:paymentEventType — what the payment provider reported; NULL for a carrier event. values: payment_event_type';
COMMENT ON COLUMN integration.webhook_event.event_key IS
  '- channel, event type, tracking number or payment reference, and the time it happened, joined; unique, so a resent event is recognised and applied once.';
COMMENT ON COLUMN integration.webhook_event.external_order_ref IS
  'acom:externalOrderRef — the order reference the sender used: our order number, or a channel''s order reference.';
COMMENT ON COLUMN integration.webhook_event.order_id IS
  'acom:orderId — the store''s order the reference maps to (ord_ id).';
COMMENT ON COLUMN integration.webhook_event.tracking_number IS
  'schema:trackingNumber — the carrier''s tracking number; NULL for a payment event.';
COMMENT ON COLUMN integration.webhook_event.payment_ref IS
  'acom:paymentRef — the payment provider''s reference of the payment or refund; NULL for a carrier event.';
COMMENT ON COLUMN integration.webhook_event.amount_value IS
  'schema:amount — the amount the payment provider reported; NULL for a carrier event.';
COMMENT ON COLUMN integration.webhook_event.amount_currency IS
  'schema:amount — ISO 4217 code of amount_value.';
COMMENT ON COLUMN integration.webhook_event.start_time IS
  'schema:startTime — when it happened at the outside system.';
COMMENT ON COLUMN integration.webhook_event.received_at IS
  'acom:receivedAt — when the store received it, by the store clock; events.replay_window counts from start_time to this.';
COMMENT ON COLUMN integration.webhook_event.action_status IS
  'schema:actionStatus — potential while it waits for its parcel, payment or refund to be recorded; completed once applied; failed when it is kept but not applied (too old, or a move the owning domain does not allow). values: action_status';
COMMENT ON COLUMN integration.webhook_event.delivery_id IS
  'acom:deliveryId — the parcel a carrier event applied to (dlv_ id); NULL until it applies.';
COMMENT ON COLUMN integration.webhook_event.payment_id IS
  'acom:paymentId — the payment a payment event applied to (pay_ id); NULL for carrier and refund events.';
COMMENT ON COLUMN integration.webhook_event.refund_id IS
  'acom:refundId — the refund a refund event applied to (rfd_ id); NULL for other events.';
COMMENT ON COLUMN integration.webhook_event.result IS
  'schema:result — what applying the event changed, {entity, id, from, to}; NULL until it applies.';
COMMENT ON COLUMN integration.webhook_event.error IS
  'schema:error — why the event was not applied, {code, message}; NULL when it applied or still waits.';
COMMENT ON COLUMN integration.webhook_event.end_time IS
  'schema:endTime — when the event was applied or given up; NULL while it waits.';
COMMENT ON COLUMN integration.webhook_event.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN integration.webhook_event.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN integration.webhook_event.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE integration.product_import IS
  'schema:CreateAction/acom:ProductImport — one import_products run of an outside feed (a seller''s or the store''s own); the drafts it made are also in catalog.product_import under the same id. id prefix imp_. API: import_id.';
COMMENT ON COLUMN integration.product_import.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN integration.product_import.id IS
  '- id with prefix imp_; the same value the API, logs and events use.';
COMMENT ON COLUMN integration.product_import.seller_id IS
  'schema:seller — the marketplace seller (slr_ id) whose feed it was; NULL for the store''s own feed.';
COMMENT ON COLUMN integration.product_import.import_mode IS
  'acom:importMode — upsert creates and updates products; price_only, stock_only and status_only change only that. values: feed_import_mode';
COMMENT ON COLUMN integration.product_import.action_status IS
  'schema:actionStatus — whether the run finished. values: action_status';
COMMENT ON COLUMN integration.product_import.category_mapping IS
  'acom:categoryMapping — the feed''s category words mapped to standard categories in this run, {Trail tops: cat_tees}; later runs of the same feed start from it.';
COMMENT ON COLUMN integration.product_import.agent_id IS
  'schema:agent — who ran the import: the change-set runner (sys_) or an operator (adm_).';
COMMENT ON COLUMN integration.product_import.change_set_id IS
  'acom:changeSetId — the approved change set the import ran in (cs_ id); NULL when seeded.';
COMMENT ON COLUMN integration.product_import.start_time IS
  'schema:startTime — when the run started.';
COMMENT ON COLUMN integration.product_import.end_time IS
  'schema:endTime — when the run finished; NULL while it runs.';
COMMENT ON COLUMN integration.product_import.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN integration.product_import.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN integration.product_import.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE integration.product_import_item IS
  'schema:DataFeedItem — one row of an outside feed and what the import did with it. Key: import and row number.';
COMMENT ON COLUMN integration.product_import_item.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN integration.product_import_item.product_import_id IS
  'acom:productImportId — the import run that read the row (imp_ id).';
COMMENT ON COLUMN integration.product_import_item.position IS
  'schema:position — row number in the feed, from 1.';
COMMENT ON COLUMN integration.product_import_item.external_id IS
  'acom:externalId — the feed''s own code for the product, for example SKU-1182; never used as our id.';
COMMENT ON COLUMN integration.product_import_item.product_group_id IS
  'acom:productGroupId — the product the row created or changed (prd_ id; API product_id); NULL when the row was rejected.';
COMMENT ON COLUMN integration.product_import_item.item_status IS
  'acom:itemStatus — whether the row was taken. values: item_status';
COMMENT ON COLUMN integration.product_import_item.variant_mapping IS
  'acom:variantMapping — the feed''s variant codes and the variants they are, [{external_id, variant_id}]; later runs of the same feed find the variants by it.';
COMMENT ON COLUMN integration.product_import_item.error IS
  'schema:error — why the row was rejected, {code, message, next_tool}; NULL when it was taken.';
COMMENT ON COLUMN integration.product_import_item.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN integration.product_import_item.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN integration.product_import_item.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE integration.channel ENABLE ROW LEVEL SECURITY;
ALTER TABLE integration.channel FORCE ROW LEVEL SECURITY;
CREATE POLICY channel_tenant_isolation ON integration.channel
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE integration.external_order ENABLE ROW LEVEL SECURITY;
ALTER TABLE integration.external_order FORCE ROW LEVEL SECURITY;
CREATE POLICY external_order_tenant_isolation ON integration.external_order
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE integration.external_claim ENABLE ROW LEVEL SECURITY;
ALTER TABLE integration.external_claim FORCE ROW LEVEL SECURITY;
CREATE POLICY external_claim_tenant_isolation ON integration.external_claim
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE integration.webhook_event ENABLE ROW LEVEL SECURITY;
ALTER TABLE integration.webhook_event FORCE ROW LEVEL SECURITY;
CREATE POLICY webhook_event_tenant_isolation ON integration.webhook_event
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE integration.product_import ENABLE ROW LEVEL SECURITY;
ALTER TABLE integration.product_import FORCE ROW LEVEL SECURITY;
CREATE POLICY product_import_tenant_isolation ON integration.product_import
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE integration.product_import_item ENABLE ROW LEVEL SECURITY;
ALTER TABLE integration.product_import_item FORCE ROW LEVEL SECURITY;
CREATE POLICY product_import_item_tenant_isolation ON integration.product_import_item
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- The service role reaches the new schema like the others (infra/postgres/init.sql gives
-- default privileges on tables; the schema itself needs USAGE).
GRANT USAGE ON SCHEMA integration TO acom_app;

-- Undo removes what these writes created, as 0014 does for other tables:
--   integration.channel                undoing register_channel (while nothing refers to it)
--   integration.product_import,
--   integration.product_import_item    undoing import_products
GRANT DELETE ON
  integration.channel,
  integration.product_import,
  integration.product_import_item
TO acom_app;
