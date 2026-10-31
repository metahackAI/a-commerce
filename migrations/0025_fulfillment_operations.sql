-- 0025_fulfillment_operations.sql
-- Fulfillment operations: shipping zones and their rates, the event logs of parcels and
-- refunds, the extra charge a claim may need first, and the parcel that reships an exchange.
-- Rules: docs/backend/database.md. Order: tables, columns, indexes, comments, row-level security.


CREATE TABLE fulfillment.shipping_zone (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  zone_type text NOT NULL DEFAULT 'standard',
  shipping_destination jsonb NOT NULL DEFAULT '[]',
  fee_type text NOT NULL DEFAULT 'paid',
  shipping_rate_value numeric(12,2) NOT NULL,
  shipping_rate_currency char(3) NOT NULL,
  free_shipping_threshold_value numeric(12,2),
  free_shipping_threshold_currency char(3),
  surcharge_value numeric(12,2) NOT NULL DEFAULT 0,
  surcharge_currency char(3) NOT NULL,
  transit_business_days integer NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT shipping_zone_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT shipping_zone_id_check CHECK (id ~ '^zone_[a-z0-9_]+$'),
  CONSTRAINT shipping_zone_name_check CHECK (char_length(name) BETWEEN 1 AND 120),
  CONSTRAINT shipping_zone_zone_type_check CHECK (zone_type IN ('standard', 'remote')),
  CONSTRAINT shipping_zone_fee_type_check CHECK (fee_type IN ('paid', 'free', 'free_over_min_subtotal')),
  CONSTRAINT shipping_zone_shipping_rate_value_check CHECK (shipping_rate_value >= 0),
  CONSTRAINT shipping_zone_shipping_rate_currency_check CHECK (shipping_rate_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT shipping_zone_free_shipping_threshold_value_check CHECK (free_shipping_threshold_value > 0),
  CONSTRAINT shipping_zone_free_shipping_threshold_currency_check CHECK (free_shipping_threshold_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT shipping_zone_free_shipping_threshold_check CHECK ((fee_type = 'free_over_min_subtotal') = (free_shipping_threshold_value IS NOT NULL)),
  CONSTRAINT shipping_zone_surcharge_value_check CHECK (surcharge_value >= 0),
  CONSTRAINT shipping_zone_surcharge_currency_check CHECK (surcharge_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT shipping_zone_transit_business_days_check CHECK (transit_business_days BETWEEN 0 AND 60)
);

CREATE TABLE fulfillment.parcel_delivery_event (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  parcel_delivery_id text NOT NULL,
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
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT parcel_delivery_event_pkey PRIMARY KEY (tenant_id, parcel_delivery_id, position),
  CONSTRAINT parcel_delivery_event_parcel_delivery_id_fkey FOREIGN KEY (tenant_id, parcel_delivery_id) REFERENCES fulfillment.parcel_delivery (tenant_id, id),
  CONSTRAINT parcel_delivery_event_position_check CHECK (position >= 1),
  CONSTRAINT parcel_delivery_event_action_status_check CHECK (action_status IN ('potential', 'active', 'completed', 'failed')),
  CONSTRAINT parcel_delivery_event_from_state_check CHECK (from_state IN ('awaiting_stock', 'ship_instructed', 'packed', 'label_created', 'picked_up', 'in_transit', 'delivered', 'delivery_failed', 'cancelled')),
  CONSTRAINT parcel_delivery_event_to_state_check CHECK (to_state IN ('awaiting_stock', 'ship_instructed', 'packed', 'label_created', 'picked_up', 'in_transit', 'delivered', 'delivery_failed', 'cancelled'))
);

CREATE TABLE fulfillment.refund_event (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  refund_id text NOT NULL,
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
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT refund_event_pkey PRIMARY KEY (tenant_id, refund_id, position),
  CONSTRAINT refund_event_refund_id_fkey FOREIGN KEY (tenant_id, refund_id) REFERENCES fulfillment.refund (tenant_id, id),
  CONSTRAINT refund_event_position_check CHECK (position >= 1),
  CONSTRAINT refund_event_action_status_check CHECK (action_status IN ('potential', 'active', 'completed', 'failed')),
  CONSTRAINT refund_event_from_state_check CHECK (from_state IN ('requested', 'completed', 'failed')),
  CONSTRAINT refund_event_to_state_check CHECK (to_state IN ('requested', 'completed', 'failed'))
);

ALTER TABLE fulfillment.claim ADD COLUMN charge_value numeric(12,2);
ALTER TABLE fulfillment.claim ADD COLUMN charge_currency char(3);
ALTER TABLE fulfillment.claim ADD COLUMN charge_paid_at timestamptz;
ALTER TABLE fulfillment.claim ADD CONSTRAINT claim_charge_value_check CHECK (charge_value > 0);
ALTER TABLE fulfillment.claim ADD CONSTRAINT claim_charge_currency_check CHECK (charge_currency ~ '^[A-Z]{3}$');
ALTER TABLE fulfillment.claim ADD CONSTRAINT claim_charge_check CHECK ((charge_value IS NULL) = (charge_currency IS NULL));

ALTER TABLE fulfillment.parcel_delivery ADD COLUMN claim_id text;
ALTER TABLE fulfillment.parcel_delivery ADD CONSTRAINT parcel_delivery_claim_id_fkey FOREIGN KEY (tenant_id, claim_id) REFERENCES fulfillment.claim (tenant_id, id);

CREATE INDEX shipping_zone_zone_type_idx ON fulfillment.shipping_zone (tenant_id, zone_type);
CREATE INDEX parcel_delivery_event_start_time_idx ON fulfillment.parcel_delivery_event (tenant_id, start_time);
CREATE INDEX refund_event_start_time_idx ON fulfillment.refund_event (tenant_id, start_time);
CREATE INDEX parcel_delivery_claim_id_idx ON fulfillment.parcel_delivery (tenant_id, claim_id);

COMMENT ON TABLE fulfillment.shipping_zone IS
  'schema:ShippingRateSettings/acom:ShippingZone — a delivery area with its shipping rate, free-shipping minimum, surcharge and transit days. id prefix zone_.';
COMMENT ON COLUMN fulfillment.shipping_zone.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN fulfillment.shipping_zone.id IS
  '- id with prefix zone_; the same value the API, logs and events use.';
COMMENT ON COLUMN fulfillment.shipping_zone.name IS
  'schema:name — name people use for the zone, for example Contiguous US.';
COMMENT ON COLUMN fulfillment.shipping_zone.zone_type IS
  'acom:zoneType — standard or remote (remote zones usually carry a surcharge). values: zone_type';
COMMENT ON COLUMN fulfillment.shipping_zone.shipping_destination IS
  'schema:shippingDestination — DefinedRegion list [{address_country, address_region}]; an entry without address_region covers the whole country except regions another zone names.';
COMMENT ON COLUMN fulfillment.shipping_zone.fee_type IS
  'acom:feeType — how shipping is charged: paid, free, or free over a minimum subtotal. values: shipping_fee_type';
COMMENT ON COLUMN fulfillment.shipping_zone.shipping_rate_value IS
  'schema:shippingRate — shipping charged per parcel when it is not free, in major units of the currency.';
COMMENT ON COLUMN fulfillment.shipping_zone.shipping_rate_currency IS
  'schema:shippingRate — ISO 4217 code of shipping_rate_value.';
COMMENT ON COLUMN fulfillment.shipping_zone.free_shipping_threshold_value IS
  'schema:freeShippingThreshold — subtotal before member benefits from which shipping is free; set only for free_over_min_subtotal.';
COMMENT ON COLUMN fulfillment.shipping_zone.free_shipping_threshold_currency IS
  'schema:freeShippingThreshold — ISO 4217 code of free_shipping_threshold_value.';
COMMENT ON COLUMN fulfillment.shipping_zone.surcharge_value IS
  'acom:surcharge — remote-area amount added to every parcel leg to this zone (fulfillment.yml delivery.remote_area_surcharge: by_zone), 0 for none.';
COMMENT ON COLUMN fulfillment.shipping_zone.surcharge_currency IS
  'acom:surcharge — ISO 4217 code of surcharge_value.';
COMMENT ON COLUMN fulfillment.shipping_zone.transit_business_days IS
  'acom:transitBusinessDays — business days a parcel travels from ship day to arrival in this zone.';
COMMENT ON COLUMN fulfillment.shipping_zone.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN fulfillment.shipping_zone.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN fulfillment.shipping_zone.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE fulfillment.parcel_delivery_event IS
  'schema:Action — one state change of a parcel (scheduled, short of stock, labelled, carrier events); rows are only added.';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.parcel_delivery_id IS
  'acom:parcelDeliveryId — the parcel that changed (dlv_ id), the object of the event.';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.position IS
  'schema:position — 1-based order of this event within its aggregate.';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.action_type IS
  'acom:actionType — the schema.org Action subtype (@type), for example CreateAction or UpdateAction.';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.action_status IS
  'schema:actionStatus — lifecycle of the action. values: action_status';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.agent_id IS
  'schema:agent — person or system that acted (adm_ or sys_ id); NULL when only a model acted.';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.agent_name IS
  'schema:agent — name of the software agent (model) that acted; NULL for people.';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.instrument IS
  'schema:instrument — tool name or endpoint that caused the event.';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.from_state IS
  'acom:fromState — state before the event; NULL for the creating event. values: delivery_state';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.to_state IS
  'acom:toState — state after the event; NULL when the state did not change. values: delivery_state';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.result IS
  'schema:result — what the action produced, as JSON (lines, tracking number, carrier).';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.error IS
  'schema:error — failure as {code, class, message}; NULL when it succeeded.';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.start_time IS
  'schema:startTime — when the action started.';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.end_time IS
  'schema:endTime — when the action ended; NULL while it runs.';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.request_id IS
  '- request trace id (req_) from the gateway; joins agent turns and the action log.';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN fulfillment.parcel_delivery_event.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE fulfillment.refund_event IS
  'schema:Action — one state change of a refund (sent, retried, confirmed or failed by the payment provider); rows are only added.';
COMMENT ON COLUMN fulfillment.refund_event.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN fulfillment.refund_event.refund_id IS
  'acom:refundId — the refund that changed (rfd_ id), the object of the event.';
COMMENT ON COLUMN fulfillment.refund_event.position IS
  'schema:position — 1-based order of this event within its aggregate.';
COMMENT ON COLUMN fulfillment.refund_event.action_type IS
  'acom:actionType — the schema.org Action subtype (@type), for example PayAction.';
COMMENT ON COLUMN fulfillment.refund_event.action_status IS
  'schema:actionStatus — lifecycle of the action. values: action_status';
COMMENT ON COLUMN fulfillment.refund_event.agent_id IS
  'schema:agent — person or system that acted (adm_ or sys_ id); NULL when only a model acted.';
COMMENT ON COLUMN fulfillment.refund_event.agent_name IS
  'schema:agent — name of the software agent (model) that acted; NULL for people.';
COMMENT ON COLUMN fulfillment.refund_event.instrument IS
  'schema:instrument — tool name or endpoint that caused the event, for example retry_refund.';
COMMENT ON COLUMN fulfillment.refund_event.from_state IS
  'acom:fromState — state before the event. values: refund_status';
COMMENT ON COLUMN fulfillment.refund_event.to_state IS
  'acom:toState — state after the event. values: refund_status';
COMMENT ON COLUMN fulfillment.refund_event.result IS
  'schema:result — what the action produced, as JSON (attempt number, payment provider note).';
COMMENT ON COLUMN fulfillment.refund_event.error IS
  'schema:error — the provider''s failure as {code, class, message}; NULL when it succeeded.';
COMMENT ON COLUMN fulfillment.refund_event.start_time IS
  'schema:startTime — when the action started.';
COMMENT ON COLUMN fulfillment.refund_event.end_time IS
  'schema:endTime — when the action ended; NULL while it runs.';
COMMENT ON COLUMN fulfillment.refund_event.request_id IS
  '- request trace id (req_) from the gateway; joins agent turns and the action log.';
COMMENT ON COLUMN fulfillment.refund_event.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN fulfillment.refund_event.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN fulfillment.refund_event.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON COLUMN fulfillment.claim.charge_value IS
  'acom:charge — extra cost the shopper pays before the claim goes ahead (cancel shipping over the refund, exchange shipping); NULL when there is none.';
COMMENT ON COLUMN fulfillment.claim.charge_currency IS
  'acom:charge — ISO 4217 code of charge_value.';
COMMENT ON COLUMN fulfillment.claim.charge_paid_at IS
  'acom:chargePaidAt — when the shopper paid the charge; NULL while it is due or when there is none.';
COMMENT ON COLUMN fulfillment.parcel_delivery.claim_id IS
  'acom:claimId — exchange claim whose replacement this parcel reships (clm_ id); NULL for the order''s own shipments.';

ALTER TABLE fulfillment.shipping_zone ENABLE ROW LEVEL SECURITY;
ALTER TABLE fulfillment.shipping_zone FORCE ROW LEVEL SECURITY;
CREATE POLICY shipping_zone_tenant_isolation ON fulfillment.shipping_zone
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE fulfillment.parcel_delivery_event ENABLE ROW LEVEL SECURITY;
ALTER TABLE fulfillment.parcel_delivery_event FORCE ROW LEVEL SECURITY;
CREATE POLICY parcel_delivery_event_tenant_isolation ON fulfillment.parcel_delivery_event
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE fulfillment.refund_event ENABLE ROW LEVEL SECURITY;
ALTER TABLE fulfillment.refund_event FORCE ROW LEVEL SECURITY;
CREATE POLICY refund_event_tenant_isolation ON fulfillment.refund_event
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- Undo removes what these writes created, as 0014 does for other tables:
--   fulfillment.shipping_zone   undoing manage_shipping_zone that created a zone
--   fulfillment.inspection      undoing assess_return_condition (the claim's events keep the history)
GRANT DELETE ON
  fulfillment.shipping_zone,
  fulfillment.inspection
TO acom_app;
