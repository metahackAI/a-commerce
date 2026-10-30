-- 0017_catalog_drafts.sql
-- Product drafts from supplier sheets (scene B4): the uploaded sheet, each import run and the
-- sheet row every draft came from; scheduled list prices; standard-to-display category mappings;
-- and the deletes that undoing an import, a new product or a mapping needs.
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security, grants.


CREATE TABLE catalog.data_feed (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  encoding_format text NOT NULL,
  provider_name text,
  data_feed_element jsonb NOT NULL DEFAULT '[]',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT data_feed_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT data_feed_id_check CHECK (id ~ '^upl_[a-z0-9_]+$'),
  CONSTRAINT data_feed_data_feed_element_check CHECK (jsonb_typeof(data_feed_element) = 'array')
);

CREATE TABLE catalog.product_import (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  data_feed_id text NOT NULL,
  import_mode text NOT NULL,
  action_status text NOT NULL DEFAULT 'completed',
  agent_id text NOT NULL,
  change_set_id text,
  start_time timestamptz NOT NULL,
  end_time timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT product_import_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT product_import_id_check CHECK (id ~ '^imp_[a-z0-9_]+$'),
  CONSTRAINT product_import_data_feed_id_fkey FOREIGN KEY (tenant_id, data_feed_id) REFERENCES catalog.data_feed (tenant_id, id),
  CONSTRAINT product_import_change_set_id_fkey FOREIGN KEY (tenant_id, change_set_id) REFERENCES platform.change_set (tenant_id, id),
  CONSTRAINT product_import_import_mode_check CHECK (import_mode IN ('create', 'upsert')),
  CONSTRAINT product_import_action_status_check CHECK (action_status IN ('potential', 'active', 'completed', 'failed')),
  CONSTRAINT product_import_period_check CHECK (end_time IS NULL OR end_time >= start_time)
);

CREATE TABLE catalog.data_feed_item (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  product_import_id text NOT NULL,
  position integer NOT NULL,
  product_group_id text NOT NULL,
  supplier_code text NOT NULL,
  supplier_text text,
  draft_status text NOT NULL DEFAULT 'ready',
  open_fields text[] NOT NULL DEFAULT '{}',
  draft_note text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT data_feed_item_pkey PRIMARY KEY (tenant_id, product_import_id, position),
  CONSTRAINT data_feed_item_product_import_id_fkey FOREIGN KEY (tenant_id, product_import_id) REFERENCES catalog.product_import (tenant_id, id),
  CONSTRAINT data_feed_item_product_group_id_fkey FOREIGN KEY (tenant_id, product_group_id) REFERENCES catalog.product_group (tenant_id, id),
  CONSTRAINT data_feed_item_product_group_id_key UNIQUE (tenant_id, product_group_id),
  CONSTRAINT data_feed_item_position_check CHECK (position >= 1),
  CONSTRAINT data_feed_item_draft_status_check CHECK (draft_status IN ('ready', 'needs_you', 'working'))
);

CREATE TABLE catalog.unit_price_specification (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  offer_id text NOT NULL,
  price numeric(12,2) NOT NULL,
  price_currency char(3) NOT NULL,
  price_type text NOT NULL DEFAULT 'list_price',
  valid_from timestamptz NOT NULL,
  valid_through timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT unit_price_specification_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT unit_price_specification_id_check CHECK (id ~ '^ups_[a-z0-9_]+$'),
  CONSTRAINT unit_price_specification_offer_id_fkey FOREIGN KEY (tenant_id, offer_id) REFERENCES catalog.offer (tenant_id, id),
  CONSTRAINT unit_price_specification_price_check CHECK (price >= 0),
  CONSTRAINT unit_price_specification_price_currency_check CHECK (price_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT unit_price_specification_price_type_check CHECK (price_type IN ('list_price', 'sale_price')),
  CONSTRAINT unit_price_specification_valid_period_check CHECK (valid_through IS NULL OR valid_through > valid_from)
);

CREATE TABLE merchandising.category_mapping (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  standard_category_id text NOT NULL,
  display_category_id text NOT NULL,
  is_primary boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT category_mapping_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT category_mapping_id_check CHECK (id ~ '^map_[a-z0-9_]+$'),
  CONSTRAINT category_mapping_standard_category_id_fkey FOREIGN KEY (tenant_id, standard_category_id) REFERENCES merchandising.defined_term (tenant_id, id),
  CONSTRAINT category_mapping_display_category_id_fkey FOREIGN KEY (tenant_id, display_category_id) REFERENCES merchandising.defined_term (tenant_id, id),
  CONSTRAINT category_mapping_pair_key UNIQUE (tenant_id, standard_category_id, display_category_id),
  CONSTRAINT category_mapping_pair_check CHECK (standard_category_id <> display_category_id)
);

CREATE INDEX product_import_data_feed_id_idx ON catalog.product_import (tenant_id, data_feed_id);
CREATE INDEX product_import_change_set_id_idx ON catalog.product_import (tenant_id, change_set_id);
CREATE INDEX data_feed_item_draft_status_idx ON catalog.data_feed_item (tenant_id, draft_status);
CREATE INDEX unit_price_specification_offer_id_valid_from_idx ON catalog.unit_price_specification (tenant_id, offer_id, valid_from);
CREATE INDEX category_mapping_display_category_id_idx ON merchandising.category_mapping (tenant_id, display_category_id);
CREATE UNIQUE INDEX category_mapping_primary_key ON merchandising.category_mapping (tenant_id, standard_category_id) WHERE is_primary;

COMMENT ON TABLE catalog.data_feed IS
  'schema:DataFeed — a supplier sheet as uploaded, one element per row, read by batch_import. id prefix upl_. API: upload_id.';
COMMENT ON COLUMN catalog.data_feed.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.data_feed.id IS
  '- id with prefix upl_; the same value the API, logs and events use.';
COMMENT ON COLUMN catalog.data_feed.name IS
  'schema:name — file name as uploaded, for example fw26_supplier.xlsx.';
COMMENT ON COLUMN catalog.data_feed.encoding_format IS
  'schema:encodingFormat — media type of the uploaded file, for example text/csv.';
COMMENT ON COLUMN catalog.data_feed.provider_name IS
  'schema:provider — name of the supplier that sent the sheet; NULL when unknown.';
COMMENT ON COLUMN catalog.data_feed.data_feed_element IS
  'schema:dataFeedElement — the rows in sheet order, each a map from column name to cell text: [{supplier_code, style_name, color, category, brand, sizes, cost, units_per_size, material, image, supplier_text}].';
COMMENT ON COLUMN catalog.data_feed.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.data_feed.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.data_feed.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE catalog.product_import IS
  'schema:CreateAction/acom:ProductImport — one batch_import run that turned a supplier sheet into product drafts. id prefix imp_. API: import_id.';
COMMENT ON COLUMN catalog.product_import.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.product_import.id IS
  '- id with prefix imp_; the same value the API, logs and events use.';
COMMENT ON COLUMN catalog.product_import.data_feed_id IS
  'schema:object — the supplier sheet imported (upl_ id).';
COMMENT ON COLUMN catalog.product_import.import_mode IS
  'acom:importMode — create makes new drafts only; upsert also refreshes drafts an earlier import made. values: import_mode';
COMMENT ON COLUMN catalog.product_import.action_status IS
  'schema:actionStatus — whether the run finished. values: action_status';
COMMENT ON COLUMN catalog.product_import.agent_id IS
  'schema:agent — who ran the import: the change-set runner (sys_) or an operator (adm_).';
COMMENT ON COLUMN catalog.product_import.change_set_id IS
  'acom:changeSetId — the approved change set the import ran in (cs_ id); undoing the set removes its drafts.';
COMMENT ON COLUMN catalog.product_import.start_time IS
  'schema:startTime — when the run started.';
COMMENT ON COLUMN catalog.product_import.end_time IS
  'schema:endTime — when the run finished; NULL while it runs.';
COMMENT ON COLUMN catalog.product_import.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.product_import.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.product_import.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE catalog.data_feed_item IS
  'schema:DataFeedItem — one supplier sheet row and the draft product it became, with what is still open before the draft can be published.';
COMMENT ON COLUMN catalog.data_feed_item.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.data_feed_item.product_import_id IS
  'acom:productImportId — the import run that read the row (imp_ id).';
COMMENT ON COLUMN catalog.data_feed_item.position IS
  'schema:position — row number in the sheet, from 1.';
COMMENT ON COLUMN catalog.data_feed_item.product_group_id IS
  'schema:item — the draft product made from the row (prd_ id).';
COMMENT ON COLUMN catalog.data_feed_item.supplier_code IS
  'acom:supplierCode — the supplier''s own code for the row, for example FW26-OT-014.';
COMMENT ON COLUMN catalog.data_feed_item.supplier_text IS
  'acom:supplierText — the supplier''s description, kept so the store can write its own copy in its tone; NULL when the sheet had none.';
COMMENT ON COLUMN catalog.data_feed_item.draft_status IS
  'acom:draftStatus — whether the draft is complete or waits for a person. values: draft_status';
COMMENT ON COLUMN catalog.data_feed_item.open_fields IS
  'acom:openFields — required fields that were missing or unclear in the sheet, from catalog.yml import.required_fields, for example {material}; empty when none.';
COMMENT ON COLUMN catalog.data_feed_item.draft_note IS
  'acom:draftNote — one sentence for the person about what is open, for example Fabric mix is missing in the sheet; NULL when nothing is.';
COMMENT ON COLUMN catalog.data_feed_item.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.data_feed_item.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.data_feed_item.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE catalog.unit_price_specification IS
  'schema:UnitPriceSpecification — a list price scheduled for a period (schedule_price_change); it applies from valid_from, and the offer price applies again after valid_through. id prefix ups_.';
COMMENT ON COLUMN catalog.unit_price_specification.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.unit_price_specification.id IS
  '- id with prefix ups_; the same value the API, logs and events use.';
COMMENT ON COLUMN catalog.unit_price_specification.offer_id IS
  'acom:offerId — the offer whose list price this schedules (off_ id).';
COMMENT ON COLUMN catalog.unit_price_specification.price IS
  'schema:price — the list price per unit during the period.';
COMMENT ON COLUMN catalog.unit_price_specification.price_currency IS
  'schema:priceCurrency — ISO 4217 code of price.';
COMMENT ON COLUMN catalog.unit_price_specification.price_type IS
  'schema:priceType — always list_price here; member and sale prices live in promotion.unit_price_specification. values: price_type';
COMMENT ON COLUMN catalog.unit_price_specification.valid_from IS
  'schema:validFrom — first moment the scheduled price applies.';
COMMENT ON COLUMN catalog.unit_price_specification.valid_through IS
  'schema:validThrough — last moment it applies (inclusive); NULL means until changed.';
COMMENT ON COLUMN catalog.unit_price_specification.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.unit_price_specification.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.unit_price_specification.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE merchandising.category_mapping IS
  'schema:Intangible/acom:CategoryMapping — links a standard category to a display category, so the display tree shows the standard category''s products. id prefix map_.';
COMMENT ON COLUMN merchandising.category_mapping.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN merchandising.category_mapping.id IS
  '- id with prefix map_; the same value the API, logs and events use.';
COMMENT ON COLUMN merchandising.category_mapping.standard_category_id IS
  'acom:standardCategory — the internal category whose products are shown (cat_ id, term set standard).';
COMMENT ON COLUMN merchandising.category_mapping.display_category_id IS
  'acom:displayCategory — the shopper menu category that shows them (cat_ id, term set display).';
COMMENT ON COLUMN merchandising.category_mapping.is_primary IS
  'acom:isPrimary — true for the one display category a standard category''s product pages show as their path.';
COMMENT ON COLUMN merchandising.category_mapping.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN merchandising.category_mapping.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN merchandising.category_mapping.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE catalog.data_feed ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.data_feed FORCE ROW LEVEL SECURITY;
CREATE POLICY data_feed_tenant_isolation ON catalog.data_feed
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE catalog.product_import ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.product_import FORCE ROW LEVEL SECURITY;
CREATE POLICY product_import_tenant_isolation ON catalog.product_import
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE catalog.data_feed_item ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.data_feed_item FORCE ROW LEVEL SECURITY;
CREATE POLICY data_feed_item_tenant_isolation ON catalog.data_feed_item
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE catalog.unit_price_specification ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.unit_price_specification FORCE ROW LEVEL SECURITY;
CREATE POLICY unit_price_specification_tenant_isolation ON catalog.unit_price_specification
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE merchandising.category_mapping ENABLE ROW LEVEL SECURITY;
ALTER TABLE merchandising.category_mapping FORCE ROW LEVEL SECURITY;
CREATE POLICY category_mapping_tenant_isolation ON merchandising.category_mapping
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- DELETE where taking a write back removes what it made (see 0014_app_delete_grants.sql):
--   catalog.product_group, catalog.product, catalog.offer,
--   catalog.product_import, catalog.data_feed_item,
--   merchandising.product_group_category
--                                  undoing an import or a new product while it is still a draft
--   catalog.unit_price_specification
--                                  undoing a scheduled list price
--   merchandising.category_mapping unmap_category, and undoing map_category
--   catalog.brand, merchandising.defined_term
--                                  undoing a new brand or category nothing uses yet
GRANT DELETE ON
  catalog.brand,
  merchandising.defined_term,
  catalog.product_group,
  catalog.product,
  catalog.offer,
  catalog.product_import,
  catalog.data_feed_item,
  catalog.unit_price_specification,
  merchandising.product_group_category,
  merchandising.category_mapping
TO acom_app;
