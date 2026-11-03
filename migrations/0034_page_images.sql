-- Generated pictures for store pages that show no single product (generate_page_image): the top
-- of a curated page, a banner. The picture is made at preview so the person approving sees it;
-- the bytes are kept here and served by commerce, and only approved ones reach shoppers.

CREATE TABLE catalog.page_image (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  prompt text NOT NULL,
  caption text NOT NULL,
  encoding_format text NOT NULL,
  content bytea NOT NULL,
  generator text NOT NULL,
  is_ai_generated boolean NOT NULL DEFAULT true,
  date_created timestamptz NOT NULL,
  approved_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT page_image_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT page_image_id_check CHECK (id ~ '^img_[a-z0-9_]+$')
);

COMMENT ON TABLE catalog.page_image IS
  'schema:ImageObject — a generated picture for a store page (generate_page_image), not of one product; shoppers see it once approved and used. id prefix img_.';
COMMENT ON COLUMN catalog.page_image.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN catalog.page_image.id IS
  '- id with prefix img_page_, made from the request so the same request finds the same picture (API image_id).';
COMMENT ON COLUMN catalog.page_image.prompt IS
  'acom:prompt — what the operator asked the picture to show.';
COMMENT ON COLUMN catalog.page_image.caption IS
  'schema:caption — the alternative text shown with the picture.';
COMMENT ON COLUMN catalog.page_image.encoding_format IS
  'schema:encodingFormat — media type of the picture, for example image/webp.';
COMMENT ON COLUMN catalog.page_image.content IS
  '- the picture file itself, served at /v1/catalog/page-images/{image_id}/content.';
COMMENT ON COLUMN catalog.page_image.generator IS
  'acom:generator — the model that made it, or placeholder when no image model is connected.';
COMMENT ON COLUMN catalog.page_image.is_ai_generated IS
  'acom:aiGenerated — true for generated content (catalog.yml content.ai_content_disclosure).';
COMMENT ON COLUMN catalog.page_image.date_created IS
  'schema:dateCreated — when the picture was made (store clock).';
COMMENT ON COLUMN catalog.page_image.approved_at IS
  'acom:approvedAt — when a person approved it (store clock); NULL while it waits, and shoppers cannot load it.';
COMMENT ON COLUMN catalog.page_image.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN catalog.page_image.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN catalog.page_image.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE catalog.page_image ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.page_image FORCE ROW LEVEL SECURITY;
CREATE POLICY page_image_tenant_isolation ON catalog.page_image
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE merchandising.collection_page ADD COLUMN image_id text;

COMMENT ON COLUMN merchandising.collection_page.image_id IS
  'schema:image — the picture across the top of the page (img_ id of a page image); NULL for none.';
