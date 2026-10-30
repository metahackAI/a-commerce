-- 0006_merchandising.sql
-- Merchandising: categories (DefinedTerm), product placement in categories, display slots,
-- banners and slot bookings.
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security.


CREATE TABLE merchandising.defined_term (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  term_code text NOT NULL,
  in_defined_term_set text NOT NULL,
  broader_id text,
  description text,
  position integer NOT NULL DEFAULT 1,
  is_active boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT defined_term_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT defined_term_id_check CHECK (id ~ '^cat_[a-z0-9_]+$'),
  CONSTRAINT defined_term_term_code_key UNIQUE (tenant_id, term_code),
  CONSTRAINT defined_term_broader_id_fkey FOREIGN KEY (tenant_id, broader_id) REFERENCES merchandising.defined_term (tenant_id, id),
  CONSTRAINT defined_term_term_code_check CHECK (term_code ~ '^[a-z][a-z0-9_]*$'),
  CONSTRAINT defined_term_in_defined_term_set_check CHECK (in_defined_term_set IN ('standard', 'display')),
  CONSTRAINT defined_term_position_check CHECK (position >= 1)
);

CREATE TABLE merchandising.product_group_category (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  product_group_id text NOT NULL,
  category_id text NOT NULL,
  is_primary boolean NOT NULL DEFAULT false,
  position integer,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT product_group_category_pkey PRIMARY KEY (tenant_id, product_group_id, category_id),
  CONSTRAINT product_group_category_product_group_id_fkey FOREIGN KEY (tenant_id, product_group_id) REFERENCES catalog.product_group (tenant_id, id),
  CONSTRAINT product_group_category_category_id_fkey FOREIGN KEY (tenant_id, category_id) REFERENCES merchandising.defined_term (tenant_id, id),
  CONSTRAINT product_group_category_position_check CHECK (position >= 1)
);

CREATE TABLE merchandising.display_slot (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  page text NOT NULL,
  slot_type text NOT NULL,
  position integer NOT NULL DEFAULT 1,
  item_list_element jsonb NOT NULL DEFAULT '[]',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT display_slot_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT display_slot_id_check CHECK (id ~ '^slot_[a-z0-9_]+$'),
  CONSTRAINT display_slot_slot_type_check CHECK (slot_type IN ('hero', 'banner_board', 'product_grid', 'brand_strip', 'collection_feed')),
  CONSTRAINT display_slot_position_check CHECK (position >= 1)
);

CREATE TABLE merchandising.banner (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  eyebrow text,
  headline text NOT NULL,
  alternative_headline text,
  cta_label text,
  url text NOT NULL,
  image text,
  image_product_group_id text,
  image_color text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT banner_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT banner_id_check CHECK (id ~ '^bnr_[a-z0-9_]+$'),
  CONSTRAINT banner_image_product_group_id_fkey FOREIGN KEY (tenant_id, image_product_group_id) REFERENCES catalog.product_group (tenant_id, id),
  CONSTRAINT banner_eyebrow_check CHECK (char_length(eyebrow) <= 24),
  CONSTRAINT banner_headline_check CHECK (char_length(headline) <= 60),
  CONSTRAINT banner_cta_label_check CHECK (char_length(cta_label) <= 24),
  CONSTRAINT banner_url_check CHECK (url ~ '^/'),
  CONSTRAINT banner_image_source_check CHECK (image IS NOT NULL OR image_product_group_id IS NOT NULL)
);

CREATE TABLE merchandising.display_booking (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  display_slot_id text NOT NULL,
  banner_id text,
  valid_for_member_tier text,
  valid_from timestamptz NOT NULL,
  valid_through timestamptz,
  display_status text NOT NULL DEFAULT 'scheduled',
  promotion_id text,
  change_set_id text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT display_booking_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT display_booking_id_check CHECK (id ~ '^dsp_[a-z0-9_]+$'),
  CONSTRAINT display_booking_display_slot_id_fkey FOREIGN KEY (tenant_id, display_slot_id) REFERENCES merchandising.display_slot (tenant_id, id),
  CONSTRAINT display_booking_banner_id_fkey FOREIGN KEY (tenant_id, banner_id) REFERENCES merchandising.banner (tenant_id, id),
  CONSTRAINT display_booking_valid_for_member_tier_fkey FOREIGN KEY (tenant_id, valid_for_member_tier) REFERENCES customer.member_program_tier (tenant_id, id),
  CONSTRAINT display_booking_promotion_id_fkey FOREIGN KEY (tenant_id, promotion_id) REFERENCES promotion.promotion (tenant_id, id),
  CONSTRAINT display_booking_change_set_id_fkey FOREIGN KEY (tenant_id, change_set_id) REFERENCES platform.change_set (tenant_id, id),
  CONSTRAINT display_booking_display_status_check CHECK (display_status IN ('draft', 'scheduled', 'active', 'ended')),
  CONSTRAINT display_booking_valid_period_check CHECK (valid_through IS NULL OR valid_through > valid_from)
);

CREATE INDEX defined_term_broader_id_idx ON merchandising.defined_term (tenant_id, broader_id);
CREATE INDEX product_group_category_category_id_idx ON merchandising.product_group_category (tenant_id, category_id);
CREATE INDEX display_slot_page_idx ON merchandising.display_slot (tenant_id, page);
CREATE INDEX banner_image_product_group_id_idx ON merchandising.banner (tenant_id, image_product_group_id);
CREATE INDEX display_booking_display_slot_id_valid_from_idx ON merchandising.display_booking (tenant_id, display_slot_id, valid_from);
CREATE INDEX display_booking_banner_id_idx ON merchandising.display_booking (tenant_id, banner_id);
CREATE INDEX display_booking_valid_for_member_tier_idx ON merchandising.display_booking (tenant_id, valid_for_member_tier);
CREATE INDEX display_booking_promotion_id_idx ON merchandising.display_booking (tenant_id, promotion_id);
CREATE INDEX display_booking_display_status_idx ON merchandising.display_booking (tenant_id, display_status);
CREATE INDEX display_booking_change_set_id_idx ON merchandising.display_booking (tenant_id, change_set_id);

COMMENT ON TABLE merchandising.defined_term IS
  'schema:DefinedTerm — a category in the standard set (internal classification) or the display set (shopper menu). id prefix cat_. API: category_id.';
COMMENT ON COLUMN merchandising.defined_term.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN merchandising.defined_term.id IS
  '- id with prefix cat_; the same value the API, logs and events use.';
COMMENT ON COLUMN merchandising.defined_term.name IS
  'schema:name — category name shown to people, for example Summer outerwear.';
COMMENT ON COLUMN merchandising.defined_term.term_code IS
  'schema:termCode — meaningful English word for the category, unique in the store, for example summer_outerwear.';
COMMENT ON COLUMN merchandising.defined_term.in_defined_term_set IS
  'schema:inDefinedTermSet — which category set the term belongs to. values: term_set';
COMMENT ON COLUMN merchandising.defined_term.broader_id IS
  'acom:broader — parent category (cat_ id); NULL for a top-level category.';
COMMENT ON COLUMN merchandising.defined_term.description IS
  'schema:description — what belongs in the category.';
COMMENT ON COLUMN merchandising.defined_term.position IS
  'schema:position — display order among siblings.';
COMMENT ON COLUMN merchandising.defined_term.is_active IS
  'acom:isActive — false hides the category and its children.';
COMMENT ON COLUMN merchandising.defined_term.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN merchandising.defined_term.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN merchandising.defined_term.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE merchandising.product_group_category IS
  'acom:CategoryPlacement — a product placed in a standard or display category; one standard category per product is primary.';
COMMENT ON COLUMN merchandising.product_group_category.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN merchandising.product_group_category.product_group_id IS
  'acom:productGroupId — the product (prd_ id; API product_id).';
COMMENT ON COLUMN merchandising.product_group_category.category_id IS
  'schema:category — the category (cat_ id).';
COMMENT ON COLUMN merchandising.product_group_category.is_primary IS
  'acom:isPrimary — true for the product''s main standard category (API category_id on the product).';
COMMENT ON COLUMN merchandising.product_group_category.position IS
  'schema:position — order of the product inside the category; NULL sorts by search ranking.';
COMMENT ON COLUMN merchandising.product_group_category.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN merchandising.product_group_category.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN merchandising.product_group_category.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE merchandising.display_slot IS
  'schema:WebPageElement — a place on a store page that holds products or banners, booked per audience. id prefix slot_.';
COMMENT ON COLUMN merchandising.display_slot.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN merchandising.display_slot.id IS
  '- id with prefix slot_; the same value the API, logs and events use.';
COMMENT ON COLUMN merchandising.display_slot.name IS
  'schema:name — slot name for operators, for example Home hero.';
COMMENT ON COLUMN merchandising.display_slot.page IS
  'acom:page — page the slot is on: main, category, product or search (list_display_slots page values).';
COMMENT ON COLUMN merchandising.display_slot.slot_type IS
  'acom:slotType — what kind of content the slot shows. values: slot_type';
COMMENT ON COLUMN merchandising.display_slot.position IS
  'schema:position — order of the slot on its page, top first.';
COMMENT ON COLUMN merchandising.display_slot.item_list_element IS
  'schema:itemListElement — default content: [{position, item_id}] with prd_, bnr_ or col_ ids.';
COMMENT ON COLUMN merchandising.display_slot.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN merchandising.display_slot.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN merchandising.display_slot.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE merchandising.banner IS
  'schema:CreativeWork/acom:Banner — banner content (small header, headline, button, link, image) that bookings show. id prefix bnr_.';
COMMENT ON COLUMN merchandising.banner.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN merchandising.banner.id IS
  '- id with prefix bnr_; the same value the API, logs and events use.';
COMMENT ON COLUMN merchandising.banner.name IS
  'schema:name — banner name for operators.';
COMMENT ON COLUMN merchandising.banner.eyebrow IS
  'acom:eyebrow — small header above the headline, up to 24 characters.';
COMMENT ON COLUMN merchandising.banner.headline IS
  'schema:headline — main text, up to 60 characters.';
COMMENT ON COLUMN merchandising.banner.alternative_headline IS
  'schema:alternativeHeadline — second line under the headline.';
COMMENT ON COLUMN merchandising.banner.cta_label IS
  'acom:ctaLabel — button text, up to 24 characters.';
COMMENT ON COLUMN merchandising.banner.url IS
  'schema:url — store path the button opens, starting with /.';
COMMENT ON COLUMN merchandising.banner.image IS
  'schema:image — URL of an uploaded or generated image; NULL when a product image is used.';
COMMENT ON COLUMN merchandising.banner.image_product_group_id IS
  'acom:imageProduct — product whose main image the banner shows (prd_ id).';
COMMENT ON COLUMN merchandising.banner.image_color IS
  'acom:imageColor — color variant of image_product_group_id to photograph, for example Sand.';
COMMENT ON COLUMN merchandising.banner.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN merchandising.banner.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN merchandising.banner.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE merchandising.display_booking IS
  'schema:Schedule/acom:DisplayBooking — a slot booked for one audience over a period, showing one banner or the slot''s items. id prefix dsp_.';
COMMENT ON COLUMN merchandising.display_booking.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN merchandising.display_booking.id IS
  '- id with prefix dsp_; the same value the API, logs and events use.';
COMMENT ON COLUMN merchandising.display_booking.display_slot_id IS
  'acom:slotId — the slot booked (slot_ id; API slot_id).';
COMMENT ON COLUMN merchandising.display_booking.banner_id IS
  'acom:bannerId — banner shown (bnr_ id); NULL when the booking shows the slot''s items.';
COMMENT ON COLUMN merchandising.display_booking.valid_for_member_tier IS
  'schema:validForMemberTier — tier that sees the booking; other audiences keep seeing their own bookings; NULL means everyone (the reserved word everyone is never stored).';
COMMENT ON COLUMN merchandising.display_booking.valid_from IS
  'schema:validFrom — first moment the booking shows.';
COMMENT ON COLUMN merchandising.display_booking.valid_through IS
  'schema:validThrough — last moment it shows (inclusive); NULL when open-ended.';
COMMENT ON COLUMN merchandising.display_booking.display_status IS
  'acom:displayStatus — computed from the period and the store clock. values: display_status';
COMMENT ON COLUMN merchandising.display_booking.promotion_id IS
  'acom:promotionId — promotion the banner announces; the booking must lie inside its period.';
COMMENT ON COLUMN merchandising.display_booking.change_set_id IS
  'acom:changeSetId — change set that created the booking.';
COMMENT ON COLUMN merchandising.display_booking.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN merchandising.display_booking.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN merchandising.display_booking.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE merchandising.defined_term ENABLE ROW LEVEL SECURITY;
ALTER TABLE merchandising.defined_term FORCE ROW LEVEL SECURITY;
CREATE POLICY defined_term_tenant_isolation ON merchandising.defined_term
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE merchandising.product_group_category ENABLE ROW LEVEL SECURITY;
ALTER TABLE merchandising.product_group_category FORCE ROW LEVEL SECURITY;
CREATE POLICY product_group_category_tenant_isolation ON merchandising.product_group_category
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE merchandising.display_slot ENABLE ROW LEVEL SECURITY;
ALTER TABLE merchandising.display_slot FORCE ROW LEVEL SECURITY;
CREATE POLICY display_slot_tenant_isolation ON merchandising.display_slot
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE merchandising.banner ENABLE ROW LEVEL SECURITY;
ALTER TABLE merchandising.banner FORCE ROW LEVEL SECURITY;
CREATE POLICY banner_tenant_isolation ON merchandising.banner
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE merchandising.display_booking ENABLE ROW LEVEL SECURITY;
ALTER TABLE merchandising.display_booking FORCE ROW LEVEL SECURITY;
CREATE POLICY display_booking_tenant_isolation ON merchandising.display_booking
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
