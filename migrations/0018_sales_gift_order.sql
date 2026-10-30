-- 0018_sales_gift_order.sql
-- Sales: gifts whose recipient accepts them through a link (prepare_gift_order with
-- recipient_mode recipient_accepts). The order itself is a sales.sales_order with is_gift; this
-- row holds what only such a gift has: who may accept it, by when, and whether they did.
-- Gifts the sender addresses themselves need no row here.
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security.


CREATE TABLE sales.gift_order (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  order_id text NOT NULL,
  recipient_name text NOT NULL,
  recipient_contact text NOT NULL,
  action_status text NOT NULL DEFAULT 'potential',
  expires timestamptz NOT NULL,
  accepted_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT gift_order_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT gift_order_id_check CHECK (id ~ '^gift_[a-z0-9_]+$'),
  CONSTRAINT gift_order_order_id_key UNIQUE (tenant_id, order_id),
  CONSTRAINT gift_order_order_id_fkey FOREIGN KEY (tenant_id, order_id) REFERENCES sales.sales_order (tenant_id, id),
  CONSTRAINT gift_order_action_status_check CHECK (action_status IN ('potential', 'active', 'completed', 'failed'))
);

COMMENT ON TABLE sales.gift_order IS
  'schema:AcceptAction/acom:GiftOrder — a gift waiting for its recipient to accept it and give an address; id prefix gift_. API: gift_order.';
COMMENT ON COLUMN sales.gift_order.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN sales.gift_order.id IS
  '- gift id (gift_ prefix), the part of the accept link that names the gift (API gift_order_id).';
COMMENT ON COLUMN sales.gift_order.order_id IS
  'schema:object — the gift order the recipient accepts (ord_ id).';
COMMENT ON COLUMN sales.gift_order.recipient_name IS
  'schema:recipient — the name the sender gave for the recipient, printed on the card and the parcel; used for this gift only.';
COMMENT ON COLUMN sales.gift_order.recipient_contact IS
  'acom:recipientContact — the email or phone the accept link goes to; used for this gift only and never saved to anyone''s profile.';
COMMENT ON COLUMN sales.gift_order.action_status IS
  'schema:actionStatus — the recipient''s acceptance: potential until they accept, completed once they did, failed when it lapsed. values: action_status';
COMMENT ON COLUMN sales.gift_order.expires IS
  'schema:expires — the last moment the recipient can accept (order time + gift.auto_cancel_if_no_address_days); payment waits until then.';
COMMENT ON COLUMN sales.gift_order.accepted_at IS
  'acom:acceptedAt — when the recipient accepted and gave the address; NULL until then.';
COMMENT ON COLUMN sales.gift_order.created_at IS
  '- when the gift was ordered.';
COMMENT ON COLUMN sales.gift_order.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN sales.gift_order.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE sales.gift_order ENABLE ROW LEVEL SECURITY;
ALTER TABLE sales.gift_order FORCE ROW LEVEL SECURITY;
CREATE POLICY gift_order_tenant_isolation ON sales.gift_order
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
