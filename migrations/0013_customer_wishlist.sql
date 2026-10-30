-- 0013_customer_wishlist.sql
-- Customer: the products a shopper keeps for later (wishlist).
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security.


CREATE TABLE customer.wishlist_item (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  customer_id text NOT NULL,
  product_group_id text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT wishlist_item_pkey PRIMARY KEY (tenant_id, customer_id, product_group_id),
  CONSTRAINT wishlist_item_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT wishlist_item_product_group_id_fkey FOREIGN KEY (tenant_id, product_group_id) REFERENCES catalog.product_group (tenant_id, id)
);

CREATE INDEX wishlist_item_product_group_id_idx ON customer.wishlist_item (tenant_id, product_group_id);

COMMENT ON TABLE customer.wishlist_item IS
  'schema:ListItem/acom:WishlistItem — one product a shopper keeps for later; the product id names the item, there is no id of its own.';
COMMENT ON COLUMN customer.wishlist_item.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.wishlist_item.customer_id IS
  'schema:customer — shopper whose wishlist this is (cus_ id).';
COMMENT ON COLUMN customer.wishlist_item.product_group_id IS
  'schema:item — the product kept (prd_ id; API product_id).';
COMMENT ON COLUMN customer.wishlist_item.created_at IS
  '- when the shopper added the product (API added_at).';
COMMENT ON COLUMN customer.wishlist_item.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.wishlist_item.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE customer.wishlist_item ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.wishlist_item FORCE ROW LEVEL SECURITY;
CREATE POLICY wishlist_item_tenant_isolation ON customer.wishlist_item
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
