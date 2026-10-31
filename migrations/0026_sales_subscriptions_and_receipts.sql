-- 0026_sales_subscriptions_and_receipts.sql
-- Sales after the order: standing orders a shopper subscribes to (prepare_subscription, the
-- person-only POST /v1/sales/subscriptions, pause/resume/cancel_subscription, update_schedule,
-- list_subscriptions), the receipts issued for an order (issue_receipt, get_receipt), and the
-- tries to open an order without signing in by its number and contact (verify_guest).
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security.


CREATE TABLE sales.subscription (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  customer_id text NOT NULL,
  subscription_status text NOT NULL DEFAULT 'active',
  billing_cycle text NOT NULL,
  next_charge_date date NOT NULL,
  ordered_item jsonb NOT NULL,
  order_delivery jsonb NOT NULL,
  payment_method text NOT NULL DEFAULT 'credit_card',
  card_last4 text,
  total_payment_due_value numeric(12,2) NOT NULL,
  total_payment_due_currency char(3) NOT NULL,
  preview_id text,
  paused_at timestamptz,
  cancelled_at timestamptz,
  cancel_reason_text text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT subscription_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT subscription_id_check CHECK (id ~ '^sub_[a-z0-9_]+$'),
  CONSTRAINT subscription_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT subscription_preview_id_fkey FOREIGN KEY (tenant_id, preview_id) REFERENCES platform.preview (tenant_id, id),
  CONSTRAINT subscription_subscription_status_check CHECK (subscription_status IN ('active', 'paused', 'cancelled')),
  CONSTRAINT subscription_billing_cycle_check CHECK (billing_cycle IN ('every_week', 'every_2_weeks', 'every_month')),
  CONSTRAINT subscription_payment_method_check CHECK (payment_method IN ('credit_card', 'bank_transfer', 'virtual_account', 'loyalty_points', 'store_credit')),
  CONSTRAINT subscription_card_last4_check CHECK (card_last4 ~ '^[0-9]{4}$'),
  CONSTRAINT subscription_total_payment_due_value_check CHECK (total_payment_due_value >= 0),
  CONSTRAINT subscription_total_payment_due_currency_check CHECK (total_payment_due_currency ~ '^[A-Z]{3}$')
);

CREATE TABLE sales.receipt (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  order_id text NOT NULL,
  receipt_type text NOT NULL,
  tax_id text,
  total_payment_due_value numeric(12,2) NOT NULL,
  total_payment_due_currency char(3) NOT NULL,
  issued_at timestamptz NOT NULL,
  cancelled_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT receipt_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT receipt_id_check CHECK (id ~ '^rct_[a-z0-9_]+$'),
  CONSTRAINT receipt_order_id_fkey FOREIGN KEY (tenant_id, order_id) REFERENCES sales.sales_order (tenant_id, id),
  CONSTRAINT receipt_receipt_type_check CHECK (receipt_type IN ('deduction', 'proof', 'tax_invoice')),
  CONSTRAINT receipt_tax_id_check CHECK (receipt_type <> 'tax_invoice' OR tax_id IS NOT NULL),
  CONSTRAINT receipt_total_payment_due_value_check CHECK (total_payment_due_value >= 0),
  CONSTRAINT receipt_total_payment_due_currency_check CHECK (total_payment_due_currency ~ '^[A-Z]{3}$')
);

CREATE TABLE sales.guest_lookup (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  order_number text NOT NULL,
  failed_attempt_count integer NOT NULL DEFAULT 0,
  first_failed_at timestamptz,
  locked_until timestamptz,
  order_id text,
  token_hash text,
  token_expires_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT guest_lookup_pkey PRIMARY KEY (tenant_id, order_number),
  CONSTRAINT guest_lookup_order_number_check CHECK (order_number ~ '^[0-9]{1,12}$'),
  CONSTRAINT guest_lookup_order_id_fkey FOREIGN KEY (tenant_id, order_id) REFERENCES sales.sales_order (tenant_id, id),
  CONSTRAINT guest_lookup_failed_attempt_count_check CHECK (failed_attempt_count >= 0)
);

CREATE INDEX subscription_customer_id_idx ON sales.subscription (tenant_id, customer_id);
CREATE INDEX subscription_subscription_status_next_charge_date_idx ON sales.subscription (tenant_id, subscription_status, next_charge_date);
CREATE INDEX subscription_preview_id_idx ON sales.subscription (tenant_id, preview_id);
CREATE INDEX receipt_order_id_idx ON sales.receipt (tenant_id, order_id);
CREATE UNIQUE INDEX receipt_order_id_receipt_type_key ON sales.receipt (tenant_id, order_id, receipt_type) WHERE cancelled_at IS NULL;
CREATE INDEX guest_lookup_order_id_idx ON sales.guest_lookup (tenant_id, order_id);

COMMENT ON TABLE sales.subscription IS
  'schema:Order/acom:Subscription — a standing order: the same lines ordered and charged to the saved card every billing cycle. id prefix sub_. API: subscription_id.';
COMMENT ON COLUMN sales.subscription.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN sales.subscription.id IS
  '- id with prefix sub_; the same value the API, logs and events use.';
COMMENT ON COLUMN sales.subscription.customer_id IS
  'schema:customer — shopper who subscribed (cus_ id).';
COMMENT ON COLUMN sales.subscription.subscription_status IS
  'acom:subscriptionStatus — active orders every cycle, paused skips cycles until resumed, cancelled is final. values: subscription_status';
COMMENT ON COLUMN sales.subscription.billing_cycle IS
  'acom:billingCycle — how often an order is placed; the length of each cycle is order.yml subscription.cycle_length. values: billing_cycle';
COMMENT ON COLUMN sales.subscription.next_charge_date IS
  'acom:nextChargeDate — store-time day the next order is placed and charged.';
COMMENT ON COLUMN sales.subscription.ordered_item IS
  'schema:orderedItem — the lines every order repeats: [{variant_id, product_id, name, size, color, quantity, price: {value, currency}, discount: {value, currency}}], price per unit and discount per line.';
COMMENT ON COLUMN sales.subscription.order_delivery IS
  'schema:orderDelivery — where every order goes: {address_id (NULL for an address given for this subscription only), delivery_address: {name, street_address, address_locality, address_region, postal_code, address_country}, address_label}.';
COMMENT ON COLUMN sales.subscription.payment_method IS
  'schema:paymentMethod — how every order is paid; a saved card (order.yml payment.subscription_card_only). values: payment_method';
COMMENT ON COLUMN sales.subscription.card_last4 IS
  'acom:cardLast4 — last four digits of the card charged; NULL for other methods.';
COMMENT ON COLUMN sales.subscription.total_payment_due_value IS
  'schema:totalPaymentDue — what each order comes to: subtotal − subscription discounts + shipping, in major units of the currency.';
COMMENT ON COLUMN sales.subscription.total_payment_due_currency IS
  'schema:totalPaymentDue — ISO 4217 code of total_payment_due_value.';
COMMENT ON COLUMN sales.subscription.preview_id IS
  'acom:previewId — prepare_subscription preview the shopper confirmed (pv_ id); NULL for subscriptions made before previews.';
COMMENT ON COLUMN sales.subscription.paused_at IS
  'acom:pausedAt — when it was last paused; NULL when it never was.';
COMMENT ON COLUMN sales.subscription.cancelled_at IS
  'acom:cancelledAt — when it was cancelled; NULL while it runs or is paused.';
COMMENT ON COLUMN sales.subscription.cancel_reason_text IS
  'acom:cancelReasonText — why the shopper cancelled, in their words; NULL when not given.';
COMMENT ON COLUMN sales.subscription.created_at IS
  '- when the subscription started.';
COMMENT ON COLUMN sales.subscription.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN sales.subscription.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE sales.receipt IS
  'schema:Invoice/acom:Receipt — a receipt or tax document issued for a paid order; one live receipt per order and type. id prefix rct_. API: receipt_id.';
COMMENT ON COLUMN sales.receipt.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN sales.receipt.id IS
  '- id with prefix rct_; the same value the API, logs and events use.';
COMMENT ON COLUMN sales.receipt.order_id IS
  'schema:referencesOrder — the order the receipt is for (ord_ id).';
COMMENT ON COLUMN sales.receipt.receipt_type IS
  'acom:receiptType — what kind of document: deduction (income deduction), proof (proof of expense) or tax_invoice (needs the buyer''s tax id). values: receipt_type';
COMMENT ON COLUMN sales.receipt.tax_id IS
  'schema:taxID — the buyer''s business tax id a tax invoice is made out to; NULL for other types.';
COMMENT ON COLUMN sales.receipt.total_payment_due_value IS
  'schema:totalPaymentDue — the amount the receipt states, the order''s total when issued, in major units of the currency.';
COMMENT ON COLUMN sales.receipt.total_payment_due_currency IS
  'schema:totalPaymentDue — ISO 4217 code of total_payment_due_value.';
COMMENT ON COLUMN sales.receipt.issued_at IS
  'acom:issuedAt — when the receipt was issued.';
COMMENT ON COLUMN sales.receipt.cancelled_at IS
  'acom:cancelledAt — when it was cancelled to be issued again for a changed amount (order.yml receipt.after_claim); NULL while it stands.';
COMMENT ON COLUMN sales.receipt.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN sales.receipt.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN sales.receipt.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE sales.guest_lookup IS
  '- tries to open an order without signing in, by order number (verify_guest): wrong tries counted toward a lock, and the short-lived token the last right try was given, kept only as a hash. Unknown numbers are counted like known ones.';
COMMENT ON COLUMN sales.guest_lookup.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN sales.guest_lookup.order_number IS
  '- order number as tried (digits, without #), whether or not such an order exists.';
COMMENT ON COLUMN sales.guest_lookup.failed_attempt_count IS
  '- wrong tries since the last right one or since the lock lifted (order.yml guest_lookup.max_attempts).';
COMMENT ON COLUMN sales.guest_lookup.first_failed_at IS
  '- when the first of the counted wrong tries happened; NULL when none is counted.';
COMMENT ON COLUMN sales.guest_lookup.locked_until IS
  '- no try is checked before this time (order.yml guest_lookup.lock_minutes); NULL when not locked.';
COMMENT ON COLUMN sales.guest_lookup.order_id IS
  '- the order the last token opens (ord_ id); NULL before a right try.';
COMMENT ON COLUMN sales.guest_lookup.token_hash IS
  '- SHA-256 of the last token handed out; the token itself is shown once and never stored.';
COMMENT ON COLUMN sales.guest_lookup.token_expires_at IS
  '- when the last token stops opening the order (order.yml guest_lookup.token_ttl_seconds).';
COMMENT ON COLUMN sales.guest_lookup.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN sales.guest_lookup.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN sales.guest_lookup.version IS
  '- row version, +1 on every change.';

ALTER TABLE sales.subscription ENABLE ROW LEVEL SECURITY;
ALTER TABLE sales.subscription FORCE ROW LEVEL SECURITY;
CREATE POLICY subscription_tenant_isolation ON sales.subscription
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE sales.receipt ENABLE ROW LEVEL SECURITY;
ALTER TABLE sales.receipt FORCE ROW LEVEL SECURITY;
CREATE POLICY receipt_tenant_isolation ON sales.receipt
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE sales.guest_lookup ENABLE ROW LEVEL SECURITY;
ALTER TABLE sales.guest_lookup FORCE ROW LEVEL SECURITY;
CREATE POLICY guest_lookup_tenant_isolation ON sales.guest_lookup
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
