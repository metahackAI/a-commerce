-- 0020_catalog_reorder.sql
-- Catalog: reorders, the stock a store orders from its supplier (create_reorder). A reorder is
-- an order the store places as the buyer; its items are the variants and how many of each, at
-- the variant's cost. Ordered stock is not sellable until it is received, so it lives here and
-- not in catalog.offer.inventory_level.
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security.


CREATE TABLE catalog.reorder (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  product_group_id text NOT NULL,
  order_status text NOT NULL DEFAULT 'processing',
  order_date timestamptz NOT NULL,
  expected_arrival_until timestamptz NOT NULL,
  total_payment_due_value numeric(12,2) NOT NULL,
  total_payment_due_currency char(3) NOT NULL,
  description text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT reorder_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT reorder_id_check CHECK (id ~ '^ro_[a-z0-9_]+$'),
  CONSTRAINT reorder_product_group_id_fkey FOREIGN KEY (tenant_id, product_group_id) REFERENCES catalog.product_group (tenant_id, id),
  CONSTRAINT reorder_order_status_check CHECK (order_status IN ('payment_due', 'processing', 'in_transit', 'delivered', 'returned', 'cancelled', 'problem')),
  CONSTRAINT reorder_total_payment_due_value_check CHECK (total_payment_due_value >= 0)
);

CREATE TABLE catalog.reorder_item (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  reorder_id text NOT NULL,
  position integer NOT NULL,
  product_id text NOT NULL,
  order_quantity integer NOT NULL,
  price numeric(12,2) NOT NULL,
  price_currency char(3) NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT reorder_item_pkey PRIMARY KEY (tenant_id, reorder_id, position),
  CONSTRAINT reorder_item_reorder_id_fkey FOREIGN KEY (tenant_id, reorder_id) REFERENCES catalog.reorder (tenant_id, id) ON DELETE CASCADE,
  CONSTRAINT reorder_item_product_id_fkey FOREIGN KEY (tenant_id, product_id) REFERENCES catalog.product (tenant_id, id),
  CONSTRAINT reorder_item_reorder_id_product_id_key UNIQUE (tenant_id, reorder_id, product_id),
  CONSTRAINT reorder_item_order_quantity_check CHECK (order_quantity > 0),
  CONSTRAINT reorder_item_price_check CHECK (price >= 0)
);

CREATE INDEX reorder_open_idx ON catalog.reorder (tenant_id, product_group_id) WHERE order_status IN ('processing', 'in_transit');
CREATE INDEX reorder_item_product_id_idx ON catalog.reorder_item (tenant_id, product_id);

COMMENT ON TABLE catalog.reorder IS
  'schema:Order/acom:Reorder — stock the store orders from its supplier for one product; id prefix ro_. API: reorder.';
COMMENT ON COLUMN catalog.reorder.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.reorder.id IS
  '- reorder id (ro_ prefix, API reorder_id).';
COMMENT ON COLUMN catalog.reorder.product_group_id IS
  'schema:orderedItem — the product the reorder is for (prd_ id, API product_id); its items name the variants.';
COMMENT ON COLUMN catalog.reorder.order_status IS
  'schema:orderStatus — processing once ordered, in_transit when the supplier sends it, delivered when received, cancelled when called off before that. values: order_status';
COMMENT ON COLUMN catalog.reorder.order_date IS
  'schema:orderDate — when the store placed the reorder (store clock).';
COMMENT ON COLUMN catalog.reorder.expected_arrival_until IS
  'schema:expectedArrivalUntil — the end of the day the stock is expected (order date + reorder.lead_time_days).';
COMMENT ON COLUMN catalog.reorder.total_payment_due_value IS
  'schema:totalPaymentDue — what the store owes the supplier: the sum of each item''s quantity times its unit cost.';
COMMENT ON COLUMN catalog.reorder.total_payment_due_currency IS
  'schema:priceCurrency — currency of total_payment_due_value (ISO 4217).';
COMMENT ON COLUMN catalog.reorder.description IS
  'schema:description — a note for the supplier or why it was ordered; NULL when none.';
COMMENT ON COLUMN catalog.reorder.created_at IS
  '- when the row was written.';
COMMENT ON COLUMN catalog.reorder.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.reorder.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE catalog.reorder_item IS
  'schema:OrderItem — one variant of a reorder and how many of it. API: reorder_item.';
COMMENT ON COLUMN catalog.reorder_item.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.reorder_item.reorder_id IS
  '- the reorder this item belongs to (ro_ id).';
COMMENT ON COLUMN catalog.reorder_item.position IS
  '- the item''s place in the reorder, from 1.';
COMMENT ON COLUMN catalog.reorder_item.product_id IS
  'schema:orderedItem — the variant ordered (var_ id, API variant_id).';
COMMENT ON COLUMN catalog.reorder_item.order_quantity IS
  'schema:orderQuantity — how many units of the variant are ordered.';
COMMENT ON COLUMN catalog.reorder_item.price IS
  'schema:price — what the store pays the supplier for one unit: the variant''s cost when it was ordered.';
COMMENT ON COLUMN catalog.reorder_item.price_currency IS
  'schema:priceCurrency — currency of price (ISO 4217).';
COMMENT ON COLUMN catalog.reorder_item.created_at IS
  '- when the row was written.';

ALTER TABLE catalog.reorder ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.reorder FORCE ROW LEVEL SECURITY;
CREATE POLICY reorder_tenant_isolation ON catalog.reorder
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE catalog.reorder_item ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.reorder_item FORCE ROW LEVEL SECURITY;
CREATE POLICY reorder_item_tenant_isolation ON catalog.reorder_item
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
