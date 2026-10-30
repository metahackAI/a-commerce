-- 0010_search.sql
-- Search: one search document per product (full text + embedding + filter columns).
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security.


CREATE TABLE search.search_document (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  product_group_id text NOT NULL,
  search_vector tsvector NOT NULL,
  embedding vector(1024),
  embedding_model text,
  brand_id text,
  category_ids text[] NOT NULL DEFAULT '{}',
  keywords text[] NOT NULL DEFAULT '{}',
  available_sizes text[] NOT NULL DEFAULT '{}',
  available_colors text[] NOT NULL DEFAULT '{}',
  price_min numeric(12,2),
  price_max numeric(12,2),
  price_currency char(3),
  approval_status text NOT NULL,
  source_version integer NOT NULL,
  indexed_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT search_document_pkey PRIMARY KEY (tenant_id, product_group_id),
  CONSTRAINT search_document_product_group_id_fkey FOREIGN KEY (tenant_id, product_group_id) REFERENCES catalog.product_group (tenant_id, id),
  CONSTRAINT search_document_price_min_check CHECK (price_min >= 0),
  CONSTRAINT search_document_price_max_check CHECK (price_max >= 0),
  CONSTRAINT search_document_price_currency_check CHECK (price_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT search_document_approval_status_check CHECK (approval_status IN ('draft', 'pending_review', 'active', 'rejected', 'discontinued')),
  CONSTRAINT search_document_source_version_check CHECK (source_version >= 1)
);

CREATE INDEX search_document_search_vector_idx ON search.search_document USING gin (search_vector);
CREATE INDEX search_document_embedding_idx ON search.search_document USING hnsw (embedding vector_cosine_ops);
CREATE INDEX search_document_category_ids_idx ON search.search_document USING gin (category_ids);
CREATE INDEX search_document_available_sizes_idx ON search.search_document USING gin (available_sizes);
CREATE INDEX search_document_keywords_idx ON search.search_document USING gin (keywords);
CREATE INDEX search_document_brand_id_idx ON search.search_document (tenant_id, brand_id);
CREATE INDEX search_document_price_min_price_max_idx ON search.search_document (tenant_id, price_min, price_max);

COMMENT ON TABLE search.search_document IS
  '- one searchable document per product: weighted full-text vector, embedding and filter columns, rebuilt from catalog through the outbox.';
COMMENT ON COLUMN search.search_document.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN search.search_document.product_group_id IS
  '- product the document indexes (prd_ id; API product_id).';
COMMENT ON COLUMN search.search_document.search_vector IS
  '- weighted text: name A, brand and categories B, material C, description D.';
COMMENT ON COLUMN search.search_document.embedding IS
  '- embedding of the product text from the configured embedding model; NULL until computed.';
COMMENT ON COLUMN search.search_document.embedding_model IS
  '- model that produced embedding.';
COMMENT ON COLUMN search.search_document.brand_id IS
  '- brand filter (brn_ id).';
COMMENT ON COLUMN search.search_document.category_ids IS
  '- standard and display categories including ancestors (cat_ ids).';
COMMENT ON COLUMN search.search_document.keywords IS
  '- product tags for the tags filter.';
COMMENT ON COLUMN search.search_document.available_sizes IS
  '- sizes of variants that are in stock.';
COMMENT ON COLUMN search.search_document.available_colors IS
  '- colors of variants that are in stock.';
COMMENT ON COLUMN search.search_document.price_min IS
  '- lowest list price among the variants.';
COMMENT ON COLUMN search.search_document.price_max IS
  '- highest list price among the variants.';
COMMENT ON COLUMN search.search_document.price_currency IS
  '- ISO 4217 code of price_min and price_max.';
COMMENT ON COLUMN search.search_document.approval_status IS
  '- approval status copied from the product; shoppers only see active. values: approval_status';
COMMENT ON COLUMN search.search_document.source_version IS
  '- product_group.version the document was built from.';
COMMENT ON COLUMN search.search_document.indexed_at IS
  '- when the document was last rebuilt.';
COMMENT ON COLUMN search.search_document.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN search.search_document.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN search.search_document.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE search.search_document ENABLE ROW LEVEL SECURITY;
ALTER TABLE search.search_document FORCE ROW LEVEL SECURITY;
CREATE POLICY search_document_tenant_isolation ON search.search_document
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
