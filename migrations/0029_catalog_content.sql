-- 0029_catalog_content.sql
-- Catalog: what product content needs. A size chart can give the underbust (bras, bikini tops,
-- bralettes), and additional_property holds the product's facts, not only the comparison
-- attributes.
-- Rules: docs/backend/database.md. Additive only: the CHECK gains a word, rows stay valid.

ALTER TABLE catalog.measurement DROP CONSTRAINT measurement_name_check;
ALTER TABLE catalog.measurement ADD CONSTRAINT measurement_name_check CHECK (name IN ('chest', 'underbust', 'shoulder', 'sleeve', 'length', 'waist', 'hip', 'inseam', 'rise', 'foot_length'));

COMMENT ON COLUMN catalog.product_group.additional_property IS
  'schema:additionalProperty — facts about the product: [{name, value, unit_text, size?}]; the comparison attributes from search.yml compare.attributes plus product facts such as fiber_content, fit, care and product_type (docs/api/catalog.md).';
