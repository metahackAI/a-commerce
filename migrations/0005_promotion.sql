-- 0005_promotion.sql
-- Promotion: promotions, member and sale prices (UnitPriceSpecification), coupons and
-- coupons issued to members.
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security.


CREATE TABLE promotion.promotion (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  description text,
  promotion_status text NOT NULL DEFAULT 'draft',
  discount_layer text NOT NULL,
  valid_for_member_tier text,
  valid_from timestamptz NOT NULL,
  valid_through timestamptz,
  percent_off numeric(5,2),
  amount_off_value numeric(12,2),
  amount_off_currency char(3),
  eligible_transaction_volume_value numeric(12,2),
  eligible_transaction_volume_currency char(3),
  applies_to jsonb NOT NULL DEFAULT '{}',
  change_set_id text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT promotion_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT promotion_id_check CHECK (id ~ '^promo_[a-z0-9_]+$'),
  CONSTRAINT promotion_valid_for_member_tier_fkey FOREIGN KEY (tenant_id, valid_for_member_tier) REFERENCES customer.member_program_tier (tenant_id, id),
  CONSTRAINT promotion_change_set_id_fkey FOREIGN KEY (tenant_id, change_set_id) REFERENCES platform.change_set (tenant_id, id),
  CONSTRAINT promotion_promotion_status_check CHECK (promotion_status IN ('draft', 'scheduled', 'running', 'paused', 'ended')),
  CONSTRAINT promotion_discount_layer_check CHECK (discount_layer IN ('item_price', 'order_discount', 'add_on')),
  CONSTRAINT promotion_percent_off_check CHECK (percent_off > 0 AND percent_off < 100),
  CONSTRAINT promotion_amount_off_value_check CHECK (amount_off_value >= 0),
  CONSTRAINT promotion_amount_off_currency_check CHECK (amount_off_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT promotion_eligible_transaction_volume_value_check CHECK (eligible_transaction_volume_value >= 0),
  CONSTRAINT promotion_eligible_transaction_volume_currency_check CHECK (eligible_transaction_volume_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT promotion_valid_period_check CHECK (valid_through IS NULL OR valid_through > valid_from)
);

CREATE TABLE promotion.unit_price_specification (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  offer_id text NOT NULL,
  promotion_id text,
  price numeric(12,2) NOT NULL,
  price_currency char(3) NOT NULL,
  price_type text NOT NULL DEFAULT 'sale_price',
  valid_for_member_tier text,
  valid_from timestamptz NOT NULL,
  valid_through timestamptz,
  percent_off numeric(5,2),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT unit_price_specification_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT unit_price_specification_id_check CHECK (id ~ '^ups_[a-z0-9_]+$'),
  CONSTRAINT unit_price_specification_offer_id_fkey FOREIGN KEY (tenant_id, offer_id) REFERENCES catalog.offer (tenant_id, id),
  CONSTRAINT unit_price_specification_promotion_id_fkey FOREIGN KEY (tenant_id, promotion_id) REFERENCES promotion.promotion (tenant_id, id),
  CONSTRAINT unit_price_specification_valid_for_member_tier_fkey FOREIGN KEY (tenant_id, valid_for_member_tier) REFERENCES customer.member_program_tier (tenant_id, id),
  CONSTRAINT unit_price_specification_price_check CHECK (price >= 0),
  CONSTRAINT unit_price_specification_price_currency_check CHECK (price_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT unit_price_specification_price_type_check CHECK (price_type IN ('list_price', 'sale_price')),
  CONSTRAINT unit_price_specification_percent_off_check CHECK (percent_off > 0 AND percent_off < 100),
  CONSTRAINT unit_price_specification_valid_period_check CHECK (valid_through IS NULL OR valid_through > valid_from)
);

CREATE TABLE promotion.coupon (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text,
  discount_code text NOT NULL,
  promotion_id text NOT NULL,
  valid_for_member_tier text,
  application text NOT NULL,
  stacking text NOT NULL DEFAULT 'no_tier_coupons',
  is_benefit_record_only boolean NOT NULL DEFAULT true,
  valid_from timestamptz,
  valid_through timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT coupon_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT coupon_id_check CHECK (id ~ '^cpn_[a-z0-9_]+$'),
  CONSTRAINT coupon_discount_code_key UNIQUE (tenant_id, discount_code),
  CONSTRAINT coupon_promotion_id_fkey FOREIGN KEY (tenant_id, promotion_id) REFERENCES promotion.promotion (tenant_id, id),
  CONSTRAINT coupon_valid_for_member_tier_fkey FOREIGN KEY (tenant_id, valid_for_member_tier) REFERENCES customer.member_program_tier (tenant_id, id),
  CONSTRAINT coupon_discount_code_check CHECK (discount_code ~ '^[A-Z0-9]{4,20}$'),
  CONSTRAINT coupon_application_check CHECK (application IN ('automatic_at_checkout', 'code_entry', 'download')),
  CONSTRAINT coupon_stacking_check CHECK (stacking IN ('no_tier_coupons', 'with_tier_coupons', 'no_other_coupons')),
  CONSTRAINT coupon_valid_period_check CHECK (valid_through IS NULL OR valid_from IS NULL OR valid_through > valid_from)
);

CREATE TABLE promotion.member_coupon (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  coupon_id text NOT NULL,
  customer_id text NOT NULL,
  coupon_status text NOT NULL DEFAULT 'issued',
  owned_from timestamptz NOT NULL,
  owned_through timestamptz,
  order_id text,
  used_at timestamptz,
  benefit_recorded_value numeric(12,2),
  benefit_recorded_currency char(3),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT member_coupon_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT member_coupon_id_check CHECK (id ~ '^mcp_[a-z0-9_]+$'),
  CONSTRAINT member_coupon_coupon_id_fkey FOREIGN KEY (tenant_id, coupon_id) REFERENCES promotion.coupon (tenant_id, id),
  CONSTRAINT member_coupon_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT member_coupon_coupon_status_check CHECK (coupon_status IN ('issued', 'available', 'used', 'restored', 'expired', 'revoked')),
  CONSTRAINT member_coupon_benefit_recorded_value_check CHECK (benefit_recorded_value >= 0),
  CONSTRAINT member_coupon_benefit_recorded_currency_check CHECK (benefit_recorded_currency ~ '^[A-Z]{3}$')
);

CREATE INDEX promotion_promotion_status_idx ON promotion.promotion (tenant_id, promotion_status);
CREATE INDEX promotion_valid_from_valid_through_idx ON promotion.promotion (tenant_id, valid_from, valid_through);
CREATE INDEX promotion_valid_for_member_tier_idx ON promotion.promotion (tenant_id, valid_for_member_tier);
CREATE INDEX promotion_change_set_id_idx ON promotion.promotion (tenant_id, change_set_id);
CREATE INDEX unit_price_specification_offer_id_idx ON promotion.unit_price_specification (tenant_id, offer_id);
CREATE INDEX unit_price_specification_promotion_id_idx ON promotion.unit_price_specification (tenant_id, promotion_id);
CREATE INDEX unit_price_specification_valid_for_member_tier_idx ON promotion.unit_price_specification (tenant_id, valid_for_member_tier);
CREATE INDEX unit_price_specification_valid_from_valid_through_idx ON promotion.unit_price_specification (tenant_id, valid_from, valid_through);
CREATE INDEX coupon_promotion_id_idx ON promotion.coupon (tenant_id, promotion_id);
CREATE INDEX coupon_valid_for_member_tier_idx ON promotion.coupon (tenant_id, valid_for_member_tier);
CREATE INDEX member_coupon_coupon_id_idx ON promotion.member_coupon (tenant_id, coupon_id);
CREATE INDEX member_coupon_customer_id_idx ON promotion.member_coupon (tenant_id, customer_id);
CREATE INDEX member_coupon_coupon_status_idx ON promotion.member_coupon (tenant_id, coupon_status);
CREATE INDEX member_coupon_order_id_idx ON promotion.member_coupon (tenant_id, order_id);

COMMENT ON TABLE promotion.promotion IS
  'schema:OfferCatalog/acom:Promotion — a time-boxed benefit that groups member and sale prices or an order discount. id prefix promo_.';
COMMENT ON COLUMN promotion.promotion.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN promotion.promotion.id IS
  '- id with prefix promo_; the same value the API, logs and events use.';
COMMENT ON COLUMN promotion.promotion.name IS
  'schema:name — name shown as the cart discount label, for example VIP weekend.';
COMMENT ON COLUMN promotion.promotion.description IS
  'schema:description — what the promotion gives.';
COMMENT ON COLUMN promotion.promotion.promotion_status IS
  'acom:promotionStatus — computed from the period and the store clock unless paused. values: promotion_status';
COMMENT ON COLUMN promotion.promotion.discount_layer IS
  'acom:discountLayer — where the benefit applies when benefits are calculated. values: discount_layer';
COMMENT ON COLUMN promotion.promotion.valid_for_member_tier IS
  'schema:validForMemberTier — tier the promotion is for; NULL means everyone (the reserved word everyone is never stored).';
COMMENT ON COLUMN promotion.promotion.valid_from IS
  'schema:validFrom — first moment the promotion applies.';
COMMENT ON COLUMN promotion.promotion.valid_through IS
  'schema:validThrough — last moment it applies (inclusive, minute precision); NULL for a standing promotion.';
COMMENT ON COLUMN promotion.promotion.percent_off IS
  'acom:percentOff — requested discount, 0-100; NULL when the benefit is an amount.';
COMMENT ON COLUMN promotion.promotion.amount_off_value IS
  'acom:amountOff — fixed discount per order; NULL when the benefit is a percentage, in major units of the currency.';
COMMENT ON COLUMN promotion.promotion.amount_off_currency IS
  'acom:amountOff — ISO 4217 code of amount_off_value.';
COMMENT ON COLUMN promotion.promotion.eligible_transaction_volume_value IS
  'schema:eligibleTransactionVolume — minimum item subtotal before member benefits for the promotion to apply, for example 80.00 for free shipping, in major units of the currency.';
COMMENT ON COLUMN promotion.promotion.eligible_transaction_volume_currency IS
  'schema:eligibleTransactionVolume — ISO 4217 code of eligible_transaction_volume_value.';
COMMENT ON COLUMN promotion.promotion.applies_to IS
  'acom:appliesTo — scope: {scope: products, categories or order, product_ids[], category_ids[], exclude_product_ids[]}.';
COMMENT ON COLUMN promotion.promotion.change_set_id IS
  'acom:changeSetId — change set that created the promotion; NULL for seeded or imported ones.';
COMMENT ON COLUMN promotion.promotion.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN promotion.promotion.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN promotion.promotion.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE promotion.unit_price_specification IS
  'schema:UnitPriceSpecification — a member or time-limited price for one variant offer; list prices stay on the offer. id prefix ups_.';
COMMENT ON COLUMN promotion.unit_price_specification.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN promotion.unit_price_specification.id IS
  '- id with prefix ups_; the same value the API, logs and events use.';
COMMENT ON COLUMN promotion.unit_price_specification.offer_id IS
  'acom:offerId — offer the price applies to (off_ id); the Offer''s priceSpecification.';
COMMENT ON COLUMN promotion.unit_price_specification.promotion_id IS
  'acom:promotionId — promotion that created the price; NULL for a scheduled list price.';
COMMENT ON COLUMN promotion.unit_price_specification.price IS
  'schema:price — the price, list price × (1 − percent_off / 100) rounded to the cent (half up).';
COMMENT ON COLUMN promotion.unit_price_specification.price_currency IS
  'schema:priceCurrency — ISO 4217 code of price.';
COMMENT ON COLUMN promotion.unit_price_specification.price_type IS
  'schema:priceType — sale_price for member and sale prices, list_price for a scheduled list price change. values: price_type';
COMMENT ON COLUMN promotion.unit_price_specification.valid_for_member_tier IS
  'schema:validForMemberTier — tier allowed to pay this price; NULL means everyone (the reserved word everyone is never stored).';
COMMENT ON COLUMN promotion.unit_price_specification.valid_from IS
  'schema:validFrom — first moment the price applies.';
COMMENT ON COLUMN promotion.unit_price_specification.valid_through IS
  'schema:validThrough — last moment it applies (inclusive); NULL when open-ended.';
COMMENT ON COLUMN promotion.unit_price_specification.percent_off IS
  'acom:percentOff — the requested discount behind this price, 0-100.';
COMMENT ON COLUMN promotion.unit_price_specification.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN promotion.unit_price_specification.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN promotion.unit_price_specification.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE promotion.coupon IS
  'schema:Offer/acom:Coupon — a coupon tied to a promotion; it records the promotion''s benefit on the order and never adds a second discount. id prefix cpn_.';
COMMENT ON COLUMN promotion.coupon.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN promotion.coupon.id IS
  '- id with prefix cpn_; the same value the API, logs and events use.';
COMMENT ON COLUMN promotion.coupon.name IS
  'schema:name — coupon name shown in the member''s wallet.';
COMMENT ON COLUMN promotion.coupon.discount_code IS
  'schema:discountCode — code, unique in the store, recorded on orders, for example VIPWEEKEND20.';
COMMENT ON COLUMN promotion.coupon.promotion_id IS
  'acom:promotionId — promotion whose benefit the coupon records (promo_ id).';
COMMENT ON COLUMN promotion.coupon.valid_for_member_tier IS
  'schema:validForMemberTier — tier allowed to use the coupon, equal to the promotion''s tier; NULL means everyone (the reserved word everyone is never stored).';
COMMENT ON COLUMN promotion.coupon.application IS
  'acom:couponApplication — how the coupon reaches an order. values: coupon_application';
COMMENT ON COLUMN promotion.coupon.stacking IS
  'acom:stacking — which other coupons may be used in the same order. values: coupon_stacking';
COMMENT ON COLUMN promotion.coupon.is_benefit_record_only IS
  'acom:recordsBenefitOnly — true: the coupon records the promotion''s benefit and adds no discount of its own.';
COMMENT ON COLUMN promotion.coupon.valid_from IS
  'schema:validFrom — first moment the coupon can be used; NULL follows the promotion.';
COMMENT ON COLUMN promotion.coupon.valid_through IS
  'schema:validThrough — last moment it can be used; NULL follows the promotion.';
COMMENT ON COLUMN promotion.coupon.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN promotion.coupon.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN promotion.coupon.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE promotion.member_coupon IS
  'schema:OwnershipInfo/acom:MemberCoupon — a coupon issued to one member, or recorded when an automatic coupon is used. id prefix mcp_.';
COMMENT ON COLUMN promotion.member_coupon.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN promotion.member_coupon.id IS
  '- id with prefix mcp_; the same value the API, logs and events use.';
COMMENT ON COLUMN promotion.member_coupon.coupon_id IS
  'acom:couponId — the coupon issued (cpn_ id).';
COMMENT ON COLUMN promotion.member_coupon.customer_id IS
  'schema:customer — member who holds it (cus_ id).';
COMMENT ON COLUMN promotion.member_coupon.coupon_status IS
  'acom:couponStatus — state of this issued coupon. values: coupon_status';
COMMENT ON COLUMN promotion.member_coupon.owned_from IS
  'schema:ownedFrom — when it was issued.';
COMMENT ON COLUMN promotion.member_coupon.owned_through IS
  'schema:ownedThrough — when it expires; NULL when it does not expire.';
COMMENT ON COLUMN promotion.member_coupon.order_id IS
  'acom:orderId — order it was used on (ord_ id); NULL until used.';
COMMENT ON COLUMN promotion.member_coupon.used_at IS
  'acom:usedAt — when it was used.';
COMMENT ON COLUMN promotion.member_coupon.benefit_recorded_value IS
  'acom:benefitRecorded — discount recorded on the order when the coupon was used, in major units of the currency.';
COMMENT ON COLUMN promotion.member_coupon.benefit_recorded_currency IS
  'acom:benefitRecorded — ISO 4217 code of benefit_recorded_value.';
COMMENT ON COLUMN promotion.member_coupon.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN promotion.member_coupon.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN promotion.member_coupon.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE promotion.promotion ENABLE ROW LEVEL SECURITY;
ALTER TABLE promotion.promotion FORCE ROW LEVEL SECURITY;
CREATE POLICY promotion_tenant_isolation ON promotion.promotion
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE promotion.unit_price_specification ENABLE ROW LEVEL SECURITY;
ALTER TABLE promotion.unit_price_specification FORCE ROW LEVEL SECURITY;
CREATE POLICY unit_price_specification_tenant_isolation ON promotion.unit_price_specification
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE promotion.coupon ENABLE ROW LEVEL SECURITY;
ALTER TABLE promotion.coupon FORCE ROW LEVEL SECURITY;
CREATE POLICY coupon_tenant_isolation ON promotion.coupon
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE promotion.member_coupon ENABLE ROW LEVEL SECURITY;
ALTER TABLE promotion.member_coupon FORCE ROW LEVEL SECURITY;
CREATE POLICY member_coupon_tenant_isolation ON promotion.member_coupon
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
