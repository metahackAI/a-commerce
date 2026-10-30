-- 0019_demo_seed_fingerprint.sql
-- Platform: which demo seed a store was loaded from. `acom-commerce migrate` with ACOM_SEED=demo
-- compares it with the seed it would load and loads the store again when they differ, so a
-- deployed demo store follows changes to seed/demo (a store not loaded from a seed keeps NULL).
-- Rules: docs/backend/database.md.

ALTER TABLE platform.online_store ADD COLUMN seed_fingerprint text;

COMMENT ON COLUMN platform.online_store.seed_fingerprint IS
  '- fingerprint (SHA-256 of the rows) of the demo seed this store was loaded from; NULL for a store not loaded from a seed.';
