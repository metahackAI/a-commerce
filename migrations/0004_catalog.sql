-- 0004_catalog.sql
-- Catalog: brands, return policies, products (ProductGroup), variants (Product), offers,
-- garment measurements and reviews.
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security.


CREATE TABLE catalog.brand (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  logo text,
  description text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT brand_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT brand_id_check CHECK (id ~ '^brn_[a-z0-9_]+$'),
  CONSTRAINT brand_name_key UNIQUE (tenant_id, name)
);

CREATE TABLE catalog.merchant_return_policy (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  merchant_return_days integer NOT NULL,
  return_fees text NOT NULL,
  return_method text[] NOT NULL DEFAULT '{pickup,drop_off}',
  refund_type text NOT NULL DEFAULT 'full_refund',
  size_exchange_days integer,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT merchant_return_policy_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT merchant_return_policy_id_check CHECK (id ~ '^mrp_[a-z0-9_]+$'),
  CONSTRAINT merchant_return_policy_merchant_return_days_check CHECK (merchant_return_days >= 0),
  CONSTRAINT merchant_return_policy_return_fees_check CHECK (return_fees IN ('free_return', 'return_fees_customer_responsibility', 'return_shipping_fees')),
  CONSTRAINT merchant_return_policy_refund_type_check CHECK (refund_type IN ('full_refund', 'exchange_refund', 'store_credit_refund')),
  CONSTRAINT merchant_return_policy_size_exchange_days_check CHECK (size_exchange_days >= 0)
);

CREATE TABLE catalog.product_group (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  description text,
  brand_id text,
  approval_status text NOT NULL DEFAULT 'draft',
  varies_by text[] NOT NULL DEFAULT '{}',
  material text,
  keywords text[] NOT NULL DEFAULT '{}',
  additional_property jsonb NOT NULL DEFAULT '[]',
  image text,
  has_merchant_return_policy_id text,
  release_date date,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT product_group_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT product_group_id_check CHECK (id ~ '^prd_[a-z0-9_]+$'),
  CONSTRAINT product_group_brand_id_fkey FOREIGN KEY (tenant_id, brand_id) REFERENCES catalog.brand (tenant_id, id),
  CONSTRAINT product_group_has_merchant_return_policy_id_fkey FOREIGN KEY (tenant_id, has_merchant_return_policy_id) REFERENCES catalog.merchant_return_policy (tenant_id, id),
  CONSTRAINT product_group_approval_status_check CHECK (approval_status IN ('draft', 'pending_review', 'active', 'rejected', 'discontinued'))
);

CREATE TABLE catalog.product (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  product_group_id text NOT NULL,
  sku text NOT NULL,
  gtin text,
  name text NOT NULL,
  color text,
  size text,
  weight_value numeric(8,1),
  weight_unit_text text,
  image text,
  sale_type text NOT NULL DEFAULT 'always',
  is_managed boolean NOT NULL DEFAULT true,
  additional_property jsonb NOT NULL DEFAULT '[]',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT product_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT product_id_check CHECK (id ~ '^var_[a-z0-9_]+$'),
  CONSTRAINT product_sku_key UNIQUE (tenant_id, sku),
  CONSTRAINT product_product_group_id_fkey FOREIGN KEY (tenant_id, product_group_id) REFERENCES catalog.product_group (tenant_id, id),
  CONSTRAINT product_gtin_check CHECK (gtin ~ '^[0-9]{8,14}$'),
  CONSTRAINT product_weight_value_check CHECK (weight_value >= 0),
  CONSTRAINT product_sale_type_check CHECK (sale_type IN ('always', 'pre_order', 'subscription', 'gift_card'))
);

CREATE TABLE catalog.offer (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  product_id text NOT NULL,
  price numeric(12,2) NOT NULL,
  price_currency char(3) NOT NULL,
  availability text NOT NULL DEFAULT 'in_stock',
  inventory_level integer NOT NULL DEFAULT 0,
  safety_stock integer NOT NULL DEFAULT 0,
  availability_starts timestamptz,
  sold_out_at timestamptz,
  valid_from timestamptz,
  valid_through timestamptz,
  cost_value numeric(12,2),
  cost_currency char(3),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT offer_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT offer_id_check CHECK (id ~ '^off_[a-z0-9_]+$'),
  CONSTRAINT offer_product_id_key UNIQUE (tenant_id, product_id),
  CONSTRAINT offer_product_id_fkey FOREIGN KEY (tenant_id, product_id) REFERENCES catalog.product (tenant_id, id),
  CONSTRAINT offer_price_check CHECK (price >= 0),
  CONSTRAINT offer_price_currency_check CHECK (price_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT offer_availability_check CHECK (availability IN ('in_stock', 'out_of_stock', 'pre_order', 'back_order', 'discontinued')),
  CONSTRAINT offer_inventory_level_check CHECK (inventory_level >= 0),
  CONSTRAINT offer_safety_stock_check CHECK (safety_stock >= 0),
  CONSTRAINT offer_cost_value_check CHECK (cost_value >= 0),
  CONSTRAINT offer_cost_currency_check CHECK (cost_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT offer_valid_period_check CHECK (valid_through IS NULL OR valid_from IS NULL OR valid_through > valid_from)
);

CREATE TABLE catalog.measurement (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  product_group_id text NOT NULL,
  size text NOT NULL,
  name text NOT NULL,
  value numeric(6,1) NOT NULL,
  unit_text text NOT NULL DEFAULT 'cm',
  measurement_technique text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT measurement_pkey PRIMARY KEY (tenant_id, product_group_id, size, name),
  CONSTRAINT measurement_product_group_id_fkey FOREIGN KEY (tenant_id, product_group_id) REFERENCES catalog.product_group (tenant_id, id),
  CONSTRAINT measurement_name_check CHECK (name IN ('chest', 'shoulder', 'sleeve', 'length', 'waist', 'hip', 'inseam', 'rise', 'foot_length')),
  CONSTRAINT measurement_value_check CHECK (value > 0)
);

CREATE TABLE catalog.review (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  product_group_id text NOT NULL,
  product_id text,
  author_id text,
  author_name text NOT NULL,
  order_id text,
  rating_value smallint NOT NULL,
  review_body text NOT NULL,
  fit_vote text,
  author_height_value numeric(5,1),
  author_height_unit_text text,
  keywords text[] NOT NULL DEFAULT '{}',
  creative_work_status text NOT NULL DEFAULT 'visible',
  date_published date NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT review_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT review_id_check CHECK (id ~ '^rev_[a-z0-9_]+$'),
  CONSTRAINT review_product_group_id_fkey FOREIGN KEY (tenant_id, product_group_id) REFERENCES catalog.product_group (tenant_id, id),
  CONSTRAINT review_product_id_fkey FOREIGN KEY (tenant_id, product_id) REFERENCES catalog.product (tenant_id, id),
  CONSTRAINT review_author_id_fkey FOREIGN KEY (tenant_id, author_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT rating_value_check CHECK (rating_value BETWEEN 1 AND 5),
  CONSTRAINT review_fit_vote_check CHECK (fit_vote IN ('true_to_size', 'runs_small', 'runs_large')),
  CONSTRAINT review_author_height_value_check CHECK (author_height_value BETWEEN 100 AND 230),
  CONSTRAINT review_creative_work_status_check CHECK (creative_work_status IN ('visible', 'hidden', 'reported'))
);

CREATE INDEX product_group_approval_status_idx ON catalog.product_group (tenant_id, approval_status);
CREATE INDEX product_group_brand_id_idx ON catalog.product_group (tenant_id, brand_id);
CREATE INDEX product_group_has_merchant_return_policy_id_idx ON catalog.product_group (tenant_id, has_merchant_return_policy_id);
CREATE INDEX product_product_group_id_idx ON catalog.product (tenant_id, product_group_id);
CREATE INDEX offer_availability_idx ON catalog.offer (tenant_id, availability);
CREATE INDEX review_product_id_idx ON catalog.review (tenant_id, product_id);
CREATE INDEX review_author_id_idx ON catalog.review (tenant_id, author_id);
CREATE INDEX review_creative_work_status_idx ON catalog.review (tenant_id, creative_work_status);
CREATE INDEX review_product_group_id_idx ON catalog.review (tenant_id, product_group_id);

COMMENT ON TABLE catalog.brand IS
  'schema:Brand — a brand products are sold under. id prefix brn_.';
COMMENT ON COLUMN catalog.brand.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.brand.id IS
  '- id with prefix brn_; the same value the API, logs and events use.';
COMMENT ON COLUMN catalog.brand.name IS
  'schema:name — brand name.';
COMMENT ON COLUMN catalog.brand.logo IS
  'schema:logo — URL of the logo image.';
COMMENT ON COLUMN catalog.brand.description IS
  'schema:description — short brand description.';
COMMENT ON COLUMN catalog.brand.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.brand.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.brand.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE catalog.merchant_return_policy IS
  'schema:MerchantReturnPolicy — return terms products point to; the window comes from fulfillment.yml return_window_days. id prefix mrp_.';
COMMENT ON COLUMN catalog.merchant_return_policy.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.merchant_return_policy.id IS
  '- id with prefix mrp_; the same value the API, logs and events use.';
COMMENT ON COLUMN catalog.merchant_return_policy.name IS
  'schema:name — policy name for operators, for example Standard 30 days.';
COMMENT ON COLUMN catalog.merchant_return_policy.merchant_return_days IS
  'schema:merchantReturnDays — days after delivery a return can be requested.';
COMMENT ON COLUMN catalog.merchant_return_policy.return_fees IS
  'schema:returnFees — who pays return shipping for a change of mind. values: return_fees';
COMMENT ON COLUMN catalog.merchant_return_policy.return_method IS
  'schema:returnMethod — ways to send items back; words from fulfillment.yml return.send_back_methods.';
COMMENT ON COLUMN catalog.merchant_return_policy.refund_type IS
  'schema:refundType — what the shopper gets back. values: refund_type';
COMMENT ON COLUMN catalog.merchant_return_policy.size_exchange_days IS
  'acom:sizeExchangeDays — days a free size exchange is offered; NULL when not offered.';
COMMENT ON COLUMN catalog.merchant_return_policy.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.merchant_return_policy.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.merchant_return_policy.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE catalog.product_group IS
  'schema:ProductGroup — a product as people talk about it; its variants are catalog.product rows. id prefix prd_. API: product_id.';
COMMENT ON COLUMN catalog.product_group.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.product_group.id IS
  '- id with prefix prd_; the same value the API, logs and events use.';
COMMENT ON COLUMN catalog.product_group.name IS
  'schema:name — product name, for example Packable windbreaker.';
COMMENT ON COLUMN catalog.product_group.description IS
  'schema:description — product description in the store''s tone.';
COMMENT ON COLUMN catalog.product_group.brand_id IS
  'schema:brand — brand (brn_ id).';
COMMENT ON COLUMN catalog.product_group.approval_status IS
  'acom:approvalStatus — catalog review state; only active products are shown to shoppers. values: approval_status';
COMMENT ON COLUMN catalog.product_group.varies_by IS
  'schema:variesBy — option axes of the variants, for example {color,size}.';
COMMENT ON COLUMN catalog.product_group.material IS
  'schema:material — main material shown to shoppers, for example Recycled nylon.';
COMMENT ON COLUMN catalog.product_group.keywords IS
  'schema:keywords — product tags used by search filters, for example {packable,bestseller}.';
COMMENT ON COLUMN catalog.product_group.additional_property IS
  'schema:additionalProperty — comparison attributes: [{name, value, unit_text}], names from search.yml compare.attributes (weight, packed_size, packing, warmth, rain).';
COMMENT ON COLUMN catalog.product_group.image IS
  'schema:image — URL of the main image.';
COMMENT ON COLUMN catalog.product_group.has_merchant_return_policy_id IS
  'schema:hasMerchantReturnPolicy — return policy (mrp_ id); NULL means the store default.';
COMMENT ON COLUMN catalog.product_group.release_date IS
  'schema:releaseDate — first day on sale; sorts newest first.';
COMMENT ON COLUMN catalog.product_group.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.product_group.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.product_group.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE catalog.product IS
  'schema:Product — one sellable variant (a size and color of a product); cart lines, stock and order items use it. id prefix var_. API: variant_id.';
COMMENT ON COLUMN catalog.product.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.product.id IS
  '- id with prefix var_; the same value the API, logs and events use.';
COMMENT ON COLUMN catalog.product.product_group_id IS
  'schema:isVariantOf — product this variant belongs to (prd_ id).';
COMMENT ON COLUMN catalog.product.sku IS
  'schema:sku — stock keeping unit, unique in the store, for example wb-slate-m.';
COMMENT ON COLUMN catalog.product.gtin IS
  'schema:gtin — barcode number when the product has one.';
COMMENT ON COLUMN catalog.product.name IS
  'schema:name — variant name, for example Packable windbreaker, Slate, M.';
COMMENT ON COLUMN catalog.product.color IS
  'schema:color — color name, for example Slate; NULL when the product has one color.';
COMMENT ON COLUMN catalog.product.size IS
  'schema:size — size word, for example M, 32 or 9; NULL for one-size products.';
COMMENT ON COLUMN catalog.product.weight_value IS
  'schema:weight — weight of this variant.';
COMMENT ON COLUMN catalog.product.weight_unit_text IS
  'schema:weight — unit of weight_value, for example g.';
COMMENT ON COLUMN catalog.product.image IS
  'schema:image — URL of the variant image.';
COMMENT ON COLUMN catalog.product.sale_type IS
  'acom:saleType — how the variant is sold. values: sale_type';
COMMENT ON COLUMN catalog.product.is_managed IS
  'acom:managed — true when stock is tracked and can run out.';
COMMENT ON COLUMN catalog.product.additional_property IS
  'schema:additionalProperty — variant-only attributes: [{name, value, unit_text}].';
COMMENT ON COLUMN catalog.product.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.product.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.product.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE catalog.offer IS
  'schema:Offer — list price, availability and stock of one variant; cost is for operators only. id prefix off_.';
COMMENT ON COLUMN catalog.offer.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.offer.id IS
  '- id with prefix off_; the same value the API, logs and events use.';
COMMENT ON COLUMN catalog.offer.product_id IS
  'schema:itemOffered — the variant offered (var_ id; API variant_id).';
COMMENT ON COLUMN catalog.offer.price IS
  'schema:price — list price per unit; member and sale prices are promotion.unit_price_specification rows.';
COMMENT ON COLUMN catalog.offer.price_currency IS
  'schema:priceCurrency — ISO 4217 code of price.';
COMMENT ON COLUMN catalog.offer.availability IS
  'schema:availability — whether the variant can be bought now. values: availability';
COMMENT ON COLUMN catalog.offer.inventory_level IS
  'schema:inventoryLevel — units that can be sold now, across all locations.';
COMMENT ON COLUMN catalog.offer.safety_stock IS
  'acom:safetyStock — buffer level that set_safety_stock alerts at; catalog.md says it does not change availability.';
COMMENT ON COLUMN catalog.offer.availability_starts IS
  'schema:availabilityStarts — when sold-out stock is expected back, for example Nov 12; NULL when unknown.';
COMMENT ON COLUMN catalog.offer.sold_out_at IS
  'acom:soldOutAt — when the variant last ran out; NULL while in stock.';
COMMENT ON COLUMN catalog.offer.valid_from IS
  'schema:validFrom — first moment the offer is valid; NULL means already valid.';
COMMENT ON COLUMN catalog.offer.valid_through IS
  'schema:validThrough — last moment the offer is valid (inclusive); NULL means open-ended.';
COMMENT ON COLUMN catalog.offer.cost_value IS
  'acom:inventoryCost — what one unit costs the store; operators only, never shown to shoppers; the price_floor base, in major units of the currency.';
COMMENT ON COLUMN catalog.offer.cost_currency IS
  'acom:inventoryCost — ISO 4217 code of cost_value.';
COMMENT ON COLUMN catalog.offer.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.offer.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.offer.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE catalog.measurement IS
  'schema:QuantitativeValue — one garment measurement of one size (hasMeasurement), for fit checks.';
COMMENT ON COLUMN catalog.measurement.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.measurement.product_group_id IS
  'acom:productGroupId — product measured (prd_ id).';
COMMENT ON COLUMN catalog.measurement.size IS
  'schema:size — size the measurement is for, a value of the product''s size axis.';
COMMENT ON COLUMN catalog.measurement.name IS
  'schema:name — what was measured. values: measurement_name';
COMMENT ON COLUMN catalog.measurement.value IS
  'schema:value — the measurement, one decimal place.';
COMMENT ON COLUMN catalog.measurement.unit_text IS
  'schema:unitText — unit of value, for example cm.';
COMMENT ON COLUMN catalog.measurement.measurement_technique IS
  'schema:measurementTechnique — how it was measured, for example garment laid flat, chest armpit to armpit.';
COMMENT ON COLUMN catalog.measurement.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.measurement.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.measurement.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE catalog.review IS
  'schema:Review — a shopper review of a product with a fit vote; hidden reviews are left out of summaries. id prefix rev_.';
COMMENT ON COLUMN catalog.review.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.review.id IS
  '- id with prefix rev_; the same value the API, logs and events use.';
COMMENT ON COLUMN catalog.review.product_group_id IS
  'schema:itemReviewed — product reviewed (prd_ id).';
COMMENT ON COLUMN catalog.review.product_id IS
  'acom:variantBought — variant the reviewer bought (var_ id); gives the size bought.';
COMMENT ON COLUMN catalog.review.author_id IS
  'schema:author — reviewer (cus_ id); NULL for imported reviews.';
COMMENT ON COLUMN catalog.review.author_name IS
  'schema:author — name shown with the review, for example Chris T.';
COMMENT ON COLUMN catalog.review.order_id IS
  'acom:orderId — order the review was written for (ord_ id); checked by review.verified_purchase_only.';
COMMENT ON COLUMN catalog.review.rating_value IS
  'schema:ratingValue — the reviewRating of this review, from 1 (worst) to 5 (best).';
COMMENT ON COLUMN catalog.review.review_body IS
  'schema:reviewBody — review text.';
COMMENT ON COLUMN catalog.review.fit_vote IS
  'acom:fitVote — how the size fit; NULL when the reviewer did not say. values: fit_vote';
COMMENT ON COLUMN catalog.review.author_height_value IS
  'acom:authorHeight — reviewer height.';
COMMENT ON COLUMN catalog.review.author_height_unit_text IS
  'acom:authorHeight — unit of author_height_value, cm.';
COMMENT ON COLUMN catalog.review.keywords IS
  'schema:keywords — repeated phrases found in the review, counted as mentions, for example size up for a hoodie.';
COMMENT ON COLUMN catalog.review.creative_work_status IS
  'schema:creativeWorkStatus — moderation state. values: review_status';
COMMENT ON COLUMN catalog.review.date_published IS
  'schema:datePublished — date the review was posted.';
COMMENT ON COLUMN catalog.review.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.review.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.review.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE catalog.brand ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.brand FORCE ROW LEVEL SECURITY;
CREATE POLICY brand_tenant_isolation ON catalog.brand
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE catalog.merchant_return_policy ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.merchant_return_policy FORCE ROW LEVEL SECURITY;
CREATE POLICY merchant_return_policy_tenant_isolation ON catalog.merchant_return_policy
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE catalog.product_group ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.product_group FORCE ROW LEVEL SECURITY;
CREATE POLICY product_group_tenant_isolation ON catalog.product_group
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE catalog.product ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.product FORCE ROW LEVEL SECURITY;
CREATE POLICY product_tenant_isolation ON catalog.product
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE catalog.offer ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.offer FORCE ROW LEVEL SECURITY;
CREATE POLICY offer_tenant_isolation ON catalog.offer
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE catalog.measurement ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.measurement FORCE ROW LEVEL SECURITY;
CREATE POLICY measurement_tenant_isolation ON catalog.measurement
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE catalog.review ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.review FORCE ROW LEVEL SECURITY;
CREATE POLICY review_tenant_isolation ON catalog.review
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
