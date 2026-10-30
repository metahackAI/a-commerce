-- 0008_fulfillment.sql
-- Fulfillment: parcel deliveries, claims (cancel, return, exchange) with their items and
-- events, inspections and refunds.
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security.


CREATE TABLE fulfillment.parcel_delivery (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  order_id text NOT NULL,
  item_shipped jsonb,
  delivery_state text NOT NULL DEFAULT 'ship_instructed',
  delivery_address_id text,
  delivery_address jsonb NOT NULL,
  has_delivery_method text NOT NULL DEFAULT 'standard',
  carrier text,
  tracking_number text,
  tracking_url text,
  expected_arrival_from timestamptz,
  expected_arrival_until timestamptz,
  shipped_at timestamptz,
  delivered_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT parcel_delivery_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT parcel_delivery_id_check CHECK (id ~ '^dlv_[a-z0-9_]+$'),
  CONSTRAINT parcel_delivery_order_id_fkey FOREIGN KEY (tenant_id, order_id) REFERENCES sales.sales_order (tenant_id, id),
  CONSTRAINT parcel_delivery_delivery_address_id_fkey FOREIGN KEY (tenant_id, delivery_address_id) REFERENCES customer.postal_address (tenant_id, id),
  CONSTRAINT parcel_delivery_delivery_state_check CHECK (delivery_state IN ('awaiting_stock', 'ship_instructed', 'packed', 'label_created', 'picked_up', 'in_transit', 'delivered', 'delivery_failed', 'cancelled'))
);

CREATE TABLE fulfillment.claim (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  order_id text NOT NULL,
  customer_id text NOT NULL,
  claim_type text NOT NULL,
  claim_status text NOT NULL DEFAULT 'requested',
  claim_path text NOT NULL DEFAULT 'store_review',
  claim_reason text,
  cancel_reason text,
  reason_text text,
  liability text NOT NULL,
  return_fees text,
  send_back_method text,
  pickup_window jsonb,
  evidence jsonb NOT NULL DEFAULT '[]',
  amount_value numeric(12,2) NOT NULL,
  amount_currency char(3) NOT NULL,
  refund_estimate_value numeric(12,2),
  refund_estimate_currency char(3),
  start_time timestamptz NOT NULL,
  end_time timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT claim_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT claim_id_check CHECK (id ~ '^clm_[a-z0-9_]+$'),
  CONSTRAINT claim_order_id_fkey FOREIGN KEY (tenant_id, order_id) REFERENCES sales.sales_order (tenant_id, id),
  CONSTRAINT claim_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT claim_claim_type_check CHECK (claim_type IN ('cancel', 'return', 'exchange')),
  CONSTRAINT claim_claim_status_check CHECK (claim_status IN ('charge_due', 'requested', 'photos_requested', 'approved', 'pickup_instructed', 'picked_up', 'inspected', 'reship_requested', 'completed', 'declined', 'withdrawn', 'converted')),
  CONSTRAINT claim_claim_path_check CHECK (claim_path IN ('self_service', 'store_review', 'not_allowed')),
  CONSTRAINT claim_claim_reason_check CHECK (claim_reason IN ('defect', 'damaged_in_transit', 'not_as_described', 'wrong_item_sent', 'wrong_option_sent', 'missing_parts', 'wrong_size', 'different_option_wanted', 'changed_mind')),
  CONSTRAINT claim_cancel_reason_check CHECK (cancel_reason IN ('changed_mind', 'wrong_option_ordered', 'found_lower_price', 'discount_changed', 'payment_method_change', 'duplicate_order', 'payment_not_completed', 'shipping_delay', 'out_of_stock', 'free_gift_unavailable', 'product_info_inaccurate', 'seller_cancelled', 'store_cancelled')),
  CONSTRAINT claim_reason_text_check CHECK (char_length(reason_text) <= 500),
  CONSTRAINT claim_liability_check CHECK (liability IN ('store_fault', 'seller_fault', 'customer_fault', 'carrier_fault', 'none')),
  CONSTRAINT claim_return_fees_check CHECK (return_fees IN ('free_return', 'return_fees_customer_responsibility', 'return_shipping_fees')),
  CONSTRAINT claim_amount_value_check CHECK (amount_value >= 0),
  CONSTRAINT claim_amount_currency_check CHECK (amount_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT claim_refund_estimate_value_check CHECK (refund_estimate_value >= 0),
  CONSTRAINT claim_refund_estimate_currency_check CHECK (refund_estimate_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT claim_reason_check CHECK ((claim_type = 'cancel') = (cancel_reason IS NOT NULL) AND (claim_type <> 'cancel') = (claim_reason IS NOT NULL))
);

CREATE TABLE fulfillment.claim_item (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  claim_id text NOT NULL,
  order_item_id text NOT NULL,
  order_quantity integer NOT NULL,
  replacement_product_id text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT claim_item_pkey PRIMARY KEY (tenant_id, claim_id, order_item_id),
  CONSTRAINT claim_item_claim_id_fkey FOREIGN KEY (tenant_id, claim_id) REFERENCES fulfillment.claim (tenant_id, id),
  CONSTRAINT claim_item_order_item_id_fkey FOREIGN KEY (tenant_id, order_item_id) REFERENCES sales.order_item (tenant_id, id),
  CONSTRAINT claim_item_replacement_product_id_fkey FOREIGN KEY (tenant_id, replacement_product_id) REFERENCES catalog.product (tenant_id, id),
  CONSTRAINT claim_item_order_quantity_check CHECK (order_quantity >= 1)
);

CREATE TABLE fulfillment.claim_event (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  claim_id text NOT NULL,
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
  CONSTRAINT claim_event_pkey PRIMARY KEY (tenant_id, claim_id, position),
  CONSTRAINT claim_event_claim_id_fkey FOREIGN KEY (tenant_id, claim_id) REFERENCES fulfillment.claim (tenant_id, id),
  CONSTRAINT claim_event_position_check CHECK (position >= 1),
  CONSTRAINT claim_event_action_status_check CHECK (action_status IN ('potential', 'active', 'completed', 'failed')),
  CONSTRAINT claim_event_from_state_check CHECK (from_state IN ('charge_due', 'requested', 'photos_requested', 'approved', 'pickup_instructed', 'picked_up', 'inspected', 'reship_requested', 'completed', 'declined', 'withdrawn', 'converted')),
  CONSTRAINT claim_event_to_state_check CHECK (to_state IN ('charge_due', 'requested', 'photos_requested', 'approved', 'pickup_instructed', 'picked_up', 'inspected', 'reship_requested', 'completed', 'declined', 'withdrawn', 'converted'))
);

CREATE TABLE fulfillment.inspection (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  claim_id text NOT NULL,
  inspection_condition text NOT NULL,
  liability text,
  description text,
  agent_id text NOT NULL,
  end_time timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT inspection_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT inspection_id_check CHECK (id ~ '^ins_[a-z0-9_]+$'),
  CONSTRAINT inspection_claim_id_key UNIQUE (tenant_id, claim_id),
  CONSTRAINT inspection_claim_id_fkey FOREIGN KEY (tenant_id, claim_id) REFERENCES fulfillment.claim (tenant_id, id),
  CONSTRAINT inspection_inspection_condition_check CHECK (inspection_condition IN ('as_new', 'damaged', 'missing_parts')),
  CONSTRAINT inspection_liability_check CHECK (liability IN ('store_fault', 'seller_fault', 'customer_fault', 'carrier_fault', 'none'))
);

CREATE TABLE fulfillment.refund (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  claim_id text NOT NULL,
  order_id text NOT NULL,
  payment_id text,
  amount_value numeric(12,2) NOT NULL,
  amount_currency char(3) NOT NULL,
  refund_method text NOT NULL DEFAULT 'original_payment',
  refund_type text NOT NULL DEFAULT 'full_refund',
  refund_status text NOT NULL DEFAULT 'requested',
  external_id text,
  preview_id text,
  change_set_id text,
  start_time timestamptz NOT NULL,
  end_time timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT refund_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT refund_id_check CHECK (id ~ '^rfd_[a-z0-9_]+$'),
  CONSTRAINT refund_claim_id_key UNIQUE (tenant_id, claim_id),
  CONSTRAINT refund_claim_id_fkey FOREIGN KEY (tenant_id, claim_id) REFERENCES fulfillment.claim (tenant_id, id),
  CONSTRAINT refund_order_id_fkey FOREIGN KEY (tenant_id, order_id) REFERENCES sales.sales_order (tenant_id, id),
  CONSTRAINT refund_payment_id_fkey FOREIGN KEY (tenant_id, payment_id) REFERENCES sales.payment (tenant_id, id),
  CONSTRAINT refund_preview_id_fkey FOREIGN KEY (tenant_id, preview_id) REFERENCES platform.preview (tenant_id, id),
  CONSTRAINT refund_change_set_id_fkey FOREIGN KEY (tenant_id, change_set_id) REFERENCES platform.change_set (tenant_id, id),
  CONSTRAINT refund_amount_value_check CHECK (amount_value >= 0),
  CONSTRAINT refund_amount_currency_check CHECK (amount_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT refund_refund_method_check CHECK (refund_method IN ('original_payment', 'bank_account', 'store_credit')),
  CONSTRAINT refund_refund_type_check CHECK (refund_type IN ('full_refund', 'exchange_refund', 'store_credit_refund')),
  CONSTRAINT refund_refund_status_check CHECK (refund_status IN ('requested', 'completed', 'failed'))
);

CREATE INDEX parcel_delivery_order_id_idx ON fulfillment.parcel_delivery (tenant_id, order_id);
CREATE INDEX parcel_delivery_delivery_address_id_idx ON fulfillment.parcel_delivery (tenant_id, delivery_address_id);
CREATE INDEX parcel_delivery_delivery_state_idx ON fulfillment.parcel_delivery (tenant_id, delivery_state);
CREATE INDEX parcel_delivery_delivered_at_idx ON fulfillment.parcel_delivery (tenant_id, delivered_at);
CREATE INDEX claim_order_id_idx ON fulfillment.claim (tenant_id, order_id);
CREATE INDEX claim_customer_id_start_time_idx ON fulfillment.claim (tenant_id, customer_id, start_time);
CREATE INDEX claim_claim_status_claim_type_idx ON fulfillment.claim (tenant_id, claim_status, claim_type);
CREATE INDEX claim_start_time_idx ON fulfillment.claim (tenant_id, start_time);
CREATE INDEX claim_item_order_item_id_idx ON fulfillment.claim_item (tenant_id, order_item_id);
CREATE INDEX claim_item_replacement_product_id_idx ON fulfillment.claim_item (tenant_id, replacement_product_id);
CREATE INDEX claim_event_start_time_idx ON fulfillment.claim_event (tenant_id, start_time);
CREATE INDEX refund_order_id_idx ON fulfillment.refund (tenant_id, order_id);
CREATE INDEX refund_payment_id_idx ON fulfillment.refund (tenant_id, payment_id);
CREATE INDEX refund_refund_status_idx ON fulfillment.refund (tenant_id, refund_status);
CREATE INDEX refund_change_set_id_idx ON fulfillment.refund (tenant_id, change_set_id);
CREATE INDEX refund_preview_id_idx ON fulfillment.refund (tenant_id, preview_id);

COMMENT ON TABLE fulfillment.parcel_delivery IS
  'schema:ParcelDelivery — one shipment of order items to an address, with its carrier progress. id prefix dlv_.';
COMMENT ON COLUMN fulfillment.parcel_delivery.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN fulfillment.parcel_delivery.id IS
  '- id with prefix dlv_; the same value the API, logs and events use.';
COMMENT ON COLUMN fulfillment.parcel_delivery.order_id IS
  'schema:partOfOrder — order shipped (ord_ id).';
COMMENT ON COLUMN fulfillment.parcel_delivery.item_shipped IS
  'schema:itemShipped — lines in this parcel: [{order_item_id, order_quantity}]; NULL means every line of the order.';
COMMENT ON COLUMN fulfillment.parcel_delivery.delivery_state IS
  'acom:deliveryState — warehouse step or carrier event the parcel is at. values: delivery_state';
COMMENT ON COLUMN fulfillment.parcel_delivery.delivery_address_id IS
  'acom:deliveryAddressId — saved address used (adr_ id); NULL for a one-time address such as a gift recipient''s.';
COMMENT ON COLUMN fulfillment.parcel_delivery.delivery_address IS
  'schema:deliveryAddress — address as printed on the label: {name, street_address, address_locality, address_region, postal_code, address_country}.';
COMMENT ON COLUMN fulfillment.parcel_delivery.has_delivery_method IS
  'schema:hasDeliveryMethod — delivery method word from fulfillment.yml delivery.methods.';
COMMENT ON COLUMN fulfillment.parcel_delivery.carrier IS
  'schema:carrier — carrier name.';
COMMENT ON COLUMN fulfillment.parcel_delivery.tracking_number IS
  'schema:trackingNumber — carrier tracking number.';
COMMENT ON COLUMN fulfillment.parcel_delivery.tracking_url IS
  'schema:trackingUrl — carrier tracking page.';
COMMENT ON COLUMN fulfillment.parcel_delivery.expected_arrival_from IS
  'schema:expectedArrivalFrom — earliest expected arrival.';
COMMENT ON COLUMN fulfillment.parcel_delivery.expected_arrival_until IS
  'schema:expectedArrivalUntil — latest expected arrival.';
COMMENT ON COLUMN fulfillment.parcel_delivery.shipped_at IS
  'acom:shippedAt — when the carrier took the parcel.';
COMMENT ON COLUMN fulfillment.parcel_delivery.delivered_at IS
  'acom:deliveredAt — when the carrier reported delivery; return windows count from it.';
COMMENT ON COLUMN fulfillment.parcel_delivery.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN fulfillment.parcel_delivery.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN fulfillment.parcel_delivery.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE fulfillment.claim IS
  'schema:ReturnAction/acom:Claim — a cancel, return or exchange request; @type is ReturnAction for returns and exchanges and CancelAction for cancels (claim_type decides). id prefix clm_.';
COMMENT ON COLUMN fulfillment.claim.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN fulfillment.claim.id IS
  '- id with prefix clm_; the same value the API, logs and events use.';
COMMENT ON COLUMN fulfillment.claim.order_id IS
  'acom:orderId — order the claim is about (ord_ id).';
COMMENT ON COLUMN fulfillment.claim.customer_id IS
  'schema:customer — shopper who sent the request (cus_ id), the agent of the action.';
COMMENT ON COLUMN fulfillment.claim.claim_type IS
  'acom:claimType — cancel, return or exchange. values: claim_type';
COMMENT ON COLUMN fulfillment.claim.claim_status IS
  'acom:claimStatus — where the claim is. values: claim_status';
COMMENT ON COLUMN fulfillment.claim.claim_path IS
  'acom:claimPath — who decides the claim. values: claim_path';
COMMENT ON COLUMN fulfillment.claim.claim_reason IS
  'acom:claimReason — reason for a return or exchange; NULL for cancels. values: claim_reason';
COMMENT ON COLUMN fulfillment.claim.cancel_reason IS
  'acom:cancelReason — reason for a cancel; NULL for returns and exchanges. values: cancel_reason';
COMMENT ON COLUMN fulfillment.claim.reason_text IS
  'acom:reasonText — the shopper''s own words, up to 500 characters.';
COMMENT ON COLUMN fulfillment.claim.liability IS
  'acom:liability — who is at fault; decides who pays shipping. values: liability';
COMMENT ON COLUMN fulfillment.claim.return_fees IS
  'schema:returnFees — who pays return shipping for this claim. values: return_fees';
COMMENT ON COLUMN fulfillment.claim.send_back_method IS
  'acom:sendBackMethod — pickup or drop_off, from fulfillment.yml return.send_back_methods.';
COMMENT ON COLUMN fulfillment.claim.pickup_window IS
  'acom:pickupWindow — carrier pickup window {start, end}; NULL for drop-off.';
COMMENT ON COLUMN fulfillment.claim.evidence IS
  'acom:evidence — photos and documents attached: [{id, evidence_type, content_url}], evd_ ids.';
COMMENT ON COLUMN fulfillment.claim.amount_value IS
  'schema:amount — value of the claimed lines (price paid after line discounts), in major units of the currency.';
COMMENT ON COLUMN fulfillment.claim.amount_currency IS
  'schema:amount — ISO 4217 code of amount_value.';
COMMENT ON COLUMN fulfillment.claim.refund_estimate_value IS
  'acom:refundEstimate — refund the shopper confirmed when sending the request; NULL for exchanges, in major units of the currency.';
COMMENT ON COLUMN fulfillment.claim.refund_estimate_currency IS
  'acom:refundEstimate — ISO 4217 code of refund_estimate_value.';
COMMENT ON COLUMN fulfillment.claim.start_time IS
  'schema:startTime — when the shopper sent the request (API requested_at).';
COMMENT ON COLUMN fulfillment.claim.end_time IS
  'schema:endTime — when the claim reached completed, declined, withdrawn or converted.';
COMMENT ON COLUMN fulfillment.claim.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN fulfillment.claim.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN fulfillment.claim.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE fulfillment.claim_item IS
  'schema:OrderItem/acom:ClaimItem — one order line and quantity inside a claim.';
COMMENT ON COLUMN fulfillment.claim_item.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN fulfillment.claim_item.claim_id IS
  'acom:claimId — the claim (clm_ id).';
COMMENT ON COLUMN fulfillment.claim_item.order_item_id IS
  'acom:orderItemId — order line claimed (oln_ id).';
COMMENT ON COLUMN fulfillment.claim_item.order_quantity IS
  'schema:orderQuantity — units claimed from the line.';
COMMENT ON COLUMN fulfillment.claim_item.replacement_product_id IS
  'acom:replacement — variant sent instead for an exchange (var_ id); NULL otherwise.';
COMMENT ON COLUMN fulfillment.claim_item.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN fulfillment.claim_item.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN fulfillment.claim_item.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE fulfillment.claim_event IS
  'schema:Action — one state change of a claim; rows are only added.';
COMMENT ON COLUMN fulfillment.claim_event.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN fulfillment.claim_event.claim_id IS
  'acom:claimId — the claim that changed (clm_ id), the object of the event.';
COMMENT ON COLUMN fulfillment.claim_event.position IS
  'schema:position — 1-based order of this event within its aggregate.';
COMMENT ON COLUMN fulfillment.claim_event.action_type IS
  'acom:actionType — the schema.org Action subtype (@type), for example UpdateAction or AuthorizeAction.';
COMMENT ON COLUMN fulfillment.claim_event.action_status IS
  'schema:actionStatus — lifecycle of the action. values: action_status';
COMMENT ON COLUMN fulfillment.claim_event.agent_id IS
  'schema:agent — person or system that acted (adm_, cus_ or sys_ id); NULL when only a model acted.';
COMMENT ON COLUMN fulfillment.claim_event.agent_name IS
  'schema:agent — name of the software agent (model) that acted, for example muse-glimmer-30b; NULL for people.';
COMMENT ON COLUMN fulfillment.claim_event.instrument IS
  'schema:instrument — tool name or person-only endpoint that caused the event.';
COMMENT ON COLUMN fulfillment.claim_event.from_state IS
  'acom:fromState — state before the event; NULL for the creating event. values: claim_status';
COMMENT ON COLUMN fulfillment.claim_event.to_state IS
  'acom:toState — state after the event; NULL when the state did not change. values: claim_status';
COMMENT ON COLUMN fulfillment.claim_event.result IS
  'schema:result — what the action produced, as JSON.';
COMMENT ON COLUMN fulfillment.claim_event.error IS
  'schema:error — failure as {code, class, message}; NULL when it succeeded.';
COMMENT ON COLUMN fulfillment.claim_event.start_time IS
  'schema:startTime — when the action started.';
COMMENT ON COLUMN fulfillment.claim_event.end_time IS
  'schema:endTime — when the action ended; NULL while it runs.';
COMMENT ON COLUMN fulfillment.claim_event.request_id IS
  '- request trace id (req_) from the gateway; joins agent turns and the action log.';
COMMENT ON COLUMN fulfillment.claim_event.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN fulfillment.claim_event.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN fulfillment.claim_event.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE fulfillment.inspection IS
  'schema:CheckAction — the condition check of returned items; it drives claim status, cost and stock restore. id prefix ins_.';
COMMENT ON COLUMN fulfillment.inspection.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN fulfillment.inspection.id IS
  '- id with prefix ins_; the same value the API, logs and events use.';
COMMENT ON COLUMN fulfillment.inspection.claim_id IS
  'acom:claimId — claim whose items were checked (clm_ id).';
COMMENT ON COLUMN fulfillment.inspection.inspection_condition IS
  'acom:inspectionCondition — condition found; only as_new restores stock. values: inspection_condition';
COMMENT ON COLUMN fulfillment.inspection.liability IS
  'acom:liability — fault found at inspection; NULL keeps the claim''s. values: liability';
COMMENT ON COLUMN fulfillment.inspection.description IS
  'schema:description — inspector''s note.';
COMMENT ON COLUMN fulfillment.inspection.agent_id IS
  'schema:agent — who inspected (adm_ or sys_ id).';
COMMENT ON COLUMN fulfillment.inspection.end_time IS
  'schema:endTime — when the inspection was recorded.';
COMMENT ON COLUMN fulfillment.inspection.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN fulfillment.inspection.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN fulfillment.inspection.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE fulfillment.refund IS
  'schema:MoneyTransfer/acom:Refund — money returned for a claim to the original payment method; cannot be undone once sent. id prefix rfd_.';
COMMENT ON COLUMN fulfillment.refund.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN fulfillment.refund.id IS
  '- id with prefix rfd_; the same value the API, logs and events use.';
COMMENT ON COLUMN fulfillment.refund.claim_id IS
  'acom:claimId — claim refunded (clm_ id); one refund per claim.';
COMMENT ON COLUMN fulfillment.refund.order_id IS
  'acom:orderId — order the money came from (ord_ id).';
COMMENT ON COLUMN fulfillment.refund.payment_id IS
  'acom:paymentId — payment the money goes back to (pay_ id); NULL for store credit.';
COMMENT ON COLUMN fulfillment.refund.amount_value IS
  'schema:amount — money returned, after coupon deductions, in major units of the currency.';
COMMENT ON COLUMN fulfillment.refund.amount_currency IS
  'schema:amount — ISO 4217 code of amount_value.';
COMMENT ON COLUMN fulfillment.refund.refund_method IS
  'acom:refundMethod — where the money goes. values: refund_method';
COMMENT ON COLUMN fulfillment.refund.refund_type IS
  'schema:refundType — kind of refund. values: refund_type';
COMMENT ON COLUMN fulfillment.refund.refund_status IS
  'acom:refundStatus — confirmed by payment events; never fails silently. values: refund_status';
COMMENT ON COLUMN fulfillment.refund.external_id IS
  'acom:externalId — the payment provider''s refund reference.';
COMMENT ON COLUMN fulfillment.refund.preview_id IS
  'acom:previewId — refund_return preview that was run (pv_ id).';
COMMENT ON COLUMN fulfillment.refund.change_set_id IS
  'acom:changeSetId — change set that approved the refund.';
COMMENT ON COLUMN fulfillment.refund.start_time IS
  'schema:startTime — when the refund was sent to the provider.';
COMMENT ON COLUMN fulfillment.refund.end_time IS
  'schema:endTime — when the provider confirmed or failed it.';
COMMENT ON COLUMN fulfillment.refund.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN fulfillment.refund.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN fulfillment.refund.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE fulfillment.parcel_delivery ENABLE ROW LEVEL SECURITY;
ALTER TABLE fulfillment.parcel_delivery FORCE ROW LEVEL SECURITY;
CREATE POLICY parcel_delivery_tenant_isolation ON fulfillment.parcel_delivery
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE fulfillment.claim ENABLE ROW LEVEL SECURITY;
ALTER TABLE fulfillment.claim FORCE ROW LEVEL SECURITY;
CREATE POLICY claim_tenant_isolation ON fulfillment.claim
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE fulfillment.claim_item ENABLE ROW LEVEL SECURITY;
ALTER TABLE fulfillment.claim_item FORCE ROW LEVEL SECURITY;
CREATE POLICY claim_item_tenant_isolation ON fulfillment.claim_item
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE fulfillment.claim_event ENABLE ROW LEVEL SECURITY;
ALTER TABLE fulfillment.claim_event FORCE ROW LEVEL SECURITY;
CREATE POLICY claim_event_tenant_isolation ON fulfillment.claim_event
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE fulfillment.inspection ENABLE ROW LEVEL SECURITY;
ALTER TABLE fulfillment.inspection FORCE ROW LEVEL SECURITY;
CREATE POLICY inspection_tenant_isolation ON fulfillment.inspection
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE fulfillment.refund ENABLE ROW LEVEL SECURITY;
ALTER TABLE fulfillment.refund FORCE ROW LEVEL SECURITY;
CREATE POLICY refund_tenant_isolation ON fulfillment.refund
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
