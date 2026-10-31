-- 0027_merchandising_curations_popups.sql
-- Merchandising: curated pages (CollectionPage) with their sections, store popups, the record of
-- display cache clears, and what a slot booking of the slot's own items needs (the booked items
-- and the sales channels it shows on).
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security.


CREATE TABLE merchandising.collection_page (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  has_part jsonb NOT NULL DEFAULT '[]',
  valid_for_member_tier text,
  valid_from timestamptz,
  valid_through timestamptz,
  display_status text NOT NULL DEFAULT 'draft',
  sales_channel text[],
  banner_id text[] NOT NULL DEFAULT '{}',
  keywords text[] NOT NULL DEFAULT '{}',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT collection_page_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT collection_page_id_check CHECK (id ~ '^col_[a-z0-9_]+$'),
  CONSTRAINT collection_page_valid_for_member_tier_fkey FOREIGN KEY (tenant_id, valid_for_member_tier) REFERENCES customer.member_program_tier (tenant_id, id),
  CONSTRAINT collection_page_display_status_check CHECK (display_status IN ('draft', 'scheduled', 'active', 'ended')),
  CONSTRAINT collection_page_valid_period_check CHECK (valid_through IS NULL OR valid_from IS NULL OR valid_through > valid_from),
  CONSTRAINT collection_page_published_check CHECK (display_status = 'draft' OR valid_from IS NOT NULL),
  CONSTRAINT collection_page_sales_channel_check CHECK (sales_channel <@ ARRAY['web', 'mobile_web', 'app', 'agent']::text[])
);

CREATE TABLE merchandising.popup (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  popup_type text NOT NULL,
  text text,
  image text,
  url text,
  page text[] NOT NULL,
  priority integer NOT NULL DEFAULT 1,
  suppress_days integer,
  valid_from timestamptz NOT NULL,
  valid_through timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT popup_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT popup_id_check CHECK (id ~ '^pop_[a-z0-9_]+$'),
  CONSTRAINT popup_popup_type_check CHECK (popup_type IN ('image', 'notice', 'event', 'consent')),
  CONSTRAINT popup_content_check CHECK (text IS NOT NULL OR image IS NOT NULL),
  CONSTRAINT popup_url_check CHECK (url ~ '^/'),
  CONSTRAINT popup_page_check CHECK (cardinality(page) >= 1),
  CONSTRAINT popup_priority_check CHECK (priority >= 1),
  CONSTRAINT popup_suppress_days_check CHECK (suppress_days >= 0),
  CONSTRAINT popup_valid_period_check CHECK (valid_through > valid_from)
);

CREATE TABLE merchandising.cache_invalidation (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  resource_type text NOT NULL,
  resource_id text,
  page text[] NOT NULL DEFAULT '{}',
  cache_key text[] NOT NULL DEFAULT '{}',
  reason text,
  start_time timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT cache_invalidation_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT cache_invalidation_id_check CHECK (id ~ '^cin_[a-z0-9_]+$'),
  CONSTRAINT cache_invalidation_resource_type_check CHECK (resource_type IN ('category', 'display_slot', 'banner', 'popup', 'collection_page', 'all'))
);

ALTER TABLE merchandising.display_booking ADD COLUMN item_list_element jsonb;
ALTER TABLE merchandising.display_booking ADD COLUMN sales_channel text[];
ALTER TABLE merchandising.display_booking ADD CONSTRAINT display_booking_sales_channel_check CHECK (sales_channel <@ ARRAY['web', 'mobile_web', 'app', 'agent']::text[]);

CREATE INDEX collection_page_display_status_idx ON merchandising.collection_page (tenant_id, display_status);
CREATE INDEX collection_page_valid_for_member_tier_idx ON merchandising.collection_page (tenant_id, valid_for_member_tier);
CREATE INDEX popup_valid_from_idx ON merchandising.popup (tenant_id, valid_from);
CREATE INDEX cache_invalidation_resource_type_start_time_idx ON merchandising.cache_invalidation (tenant_id, resource_type, start_time);

COMMENT ON TABLE merchandising.collection_page IS
  'schema:CollectionPage — a curated page of products in sections, published for one audience over a period. id prefix col_. API: collection_page_id.';
COMMENT ON COLUMN merchandising.collection_page.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN merchandising.collection_page.id IS
  '- id with prefix col_; the same value the API, logs and events use.';
COMMENT ON COLUMN merchandising.collection_page.name IS
  'schema:name — page title shown to shoppers, for example Winter outerwear edit.';
COMMENT ON COLUMN merchandising.collection_page.has_part IS
  'schema:hasPart — the sections in order, each an ItemList: [{name, item_list_element: [prd_ ids]}].';
COMMENT ON COLUMN merchandising.collection_page.valid_for_member_tier IS
  'schema:validForMemberTier — tier that sees the page; NULL means everyone (the reserved word everyone is never stored).';
COMMENT ON COLUMN merchandising.collection_page.valid_from IS
  'schema:validFrom — first moment the page shows; set for good when it is published, tentative on a draft.';
COMMENT ON COLUMN merchandising.collection_page.valid_through IS
  'schema:validThrough — last moment it shows (inclusive); NULL when open-ended or not set yet.';
COMMENT ON COLUMN merchandising.collection_page.display_status IS
  'acom:displayStatus — draft until published; after that computed from the period and the store clock, ended when taken down early. values: display_status';
COMMENT ON COLUMN merchandising.collection_page.sales_channel IS
  'acom:salesChannel — channels the page shows on, every element a word from the list; NULL means every channel. values: sales_channel';
COMMENT ON COLUMN merchandising.collection_page.banner_id IS
  'acom:bannerId — banners linked to the page (bnr_ ids), in order; empty when none.';
COMMENT ON COLUMN merchandising.collection_page.keywords IS
  'schema:keywords — search words that lead to the page, for example winter coat.';
COMMENT ON COLUMN merchandising.collection_page.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN merchandising.collection_page.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN merchandising.collection_page.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE merchandising.popup IS
  'schema:WebPageElement/acom:Popup — a window that opens over store pages for a period, with a do-not-show-again choice. id prefix pop_.';
COMMENT ON COLUMN merchandising.popup.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN merchandising.popup.id IS
  '- id with prefix pop_; the same value the API, logs and events use.';
COMMENT ON COLUMN merchandising.popup.name IS
  'schema:name — popup name for operators, for example Holiday hours.';
COMMENT ON COLUMN merchandising.popup.popup_type IS
  'acom:popupType — what the popup is for. values: popup_type';
COMMENT ON COLUMN merchandising.popup.text IS
  'schema:text — the words the popup shows; NULL for a picture-only popup.';
COMMENT ON COLUMN merchandising.popup.image IS
  'schema:image — URL of the picture the popup shows; NULL for a text-only popup.';
COMMENT ON COLUMN merchandising.popup.url IS
  'schema:url — store path the popup opens when pressed, starting with /; NULL when it opens nothing.';
COMMENT ON COLUMN merchandising.popup.page IS
  'acom:page — pages the popup opens on, each main, category, product or search (list_display_slots page values).';
COMMENT ON COLUMN merchandising.popup.priority IS
  'acom:priority — order among the popups of a page, 1 first; two popups that share a page and a moment never share a priority.';
COMMENT ON COLUMN merchandising.popup.suppress_days IS
  'acom:suppressDays — days a shopper who chose do-not-show-again does not see it; NULL offers no such choice.';
COMMENT ON COLUMN merchandising.popup.valid_from IS
  'schema:validFrom — first moment the popup opens.';
COMMENT ON COLUMN merchandising.popup.valid_through IS
  'schema:validThrough — last moment it opens (inclusive).';
COMMENT ON COLUMN merchandising.popup.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN merchandising.popup.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN merchandising.popup.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE merchandising.cache_invalidation IS
  'schema:Action/acom:CacheInvalidation — a request to clear cached store pages after a display change; the store runs no display cache yet, so the request is recorded and clears nothing. id prefix cin_.';
COMMENT ON COLUMN merchandising.cache_invalidation.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN merchandising.cache_invalidation.id IS
  '- id with prefix cin_; the same value the API, logs and events use.';
COMMENT ON COLUMN merchandising.cache_invalidation.resource_type IS
  'acom:resourceType — what kind of display content changed; all clears every page. values: cache_resource_type';
COMMENT ON COLUMN merchandising.cache_invalidation.resource_id IS
  'acom:resourceId — the one item that changed (cat_, slot_, bnr_, pop_ or col_ id); NULL for every item of the kind.';
COMMENT ON COLUMN merchandising.cache_invalidation.page IS
  'acom:page — store pages named in the request (main, category, product, search); empty when none was named.';
COMMENT ON COLUMN merchandising.cache_invalidation.cache_key IS
  'acom:cacheKey — the cache keys the request covers, for example display_slot:slot_home_hero and page:main.';
COMMENT ON COLUMN merchandising.cache_invalidation.reason IS
  'acom:reason — why it was cleared, as the caller said.';
COMMENT ON COLUMN merchandising.cache_invalidation.start_time IS
  'schema:startTime — when the clear ran, by the store clock; the hourly limit on all counts these.';
COMMENT ON COLUMN merchandising.cache_invalidation.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN merchandising.cache_invalidation.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN merchandising.cache_invalidation.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON COLUMN merchandising.display_booking.item_list_element IS
  'schema:itemListElement — the slot items this booking shows, copied from the slot when booked: [{position, item_id}]; NULL when the booking shows a banner.';
COMMENT ON COLUMN merchandising.display_booking.sales_channel IS
  'acom:salesChannel — channels the booking shows on, every element a word from the list; NULL means every channel. values: sales_channel';

ALTER TABLE merchandising.collection_page ENABLE ROW LEVEL SECURITY;
ALTER TABLE merchandising.collection_page FORCE ROW LEVEL SECURITY;
CREATE POLICY collection_page_tenant_isolation ON merchandising.collection_page
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE merchandising.popup ENABLE ROW LEVEL SECURITY;
ALTER TABLE merchandising.popup FORCE ROW LEVEL SECURITY;
CREATE POLICY popup_tenant_isolation ON merchandising.popup
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE merchandising.cache_invalidation ENABLE ROW LEVEL SECURITY;
ALTER TABLE merchandising.cache_invalidation FORCE ROW LEVEL SECURITY;
CREATE POLICY cache_invalidation_tenant_isolation ON merchandising.cache_invalidation
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- Undoing a curated page or a popup created by a change set removes it before anyone saw it.
GRANT DELETE ON merchandising.collection_page, merchandising.popup TO acom_app;
