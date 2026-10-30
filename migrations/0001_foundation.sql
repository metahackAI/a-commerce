-- 0001_foundation.sql
-- Extensions and schemas. One schema per domain (docs/backend/database.md §2); tables follow
-- in 0002-0012. Roles are created by deploy/postgres/init.sql, never by migrations.

CREATE EXTENSION IF NOT EXISTS vector;

CREATE SCHEMA IF NOT EXISTS platform;
CREATE SCHEMA IF NOT EXISTS catalog;
CREATE SCHEMA IF NOT EXISTS merchandising;
CREATE SCHEMA IF NOT EXISTS search;
CREATE SCHEMA IF NOT EXISTS promotion;
CREATE SCHEMA IF NOT EXISTS customer;
CREATE SCHEMA IF NOT EXISTS sales;
CREATE SCHEMA IF NOT EXISTS fulfillment;
CREATE SCHEMA IF NOT EXISTS messaging;
CREATE SCHEMA IF NOT EXISTS analytics;
CREATE SCHEMA IF NOT EXISTS agent;

COMMENT ON SCHEMA platform IS
  'Stores (tenants), operators, roles, sessions, policy overrides, previews, change sets, action log, tasks, job queue, outbox, idempotency keys, demo clock.';
COMMENT ON SCHEMA catalog IS
  'Brands, products (ProductGroup), variants (Product), offers, measurements, return policies, reviews.';
COMMENT ON SCHEMA merchandising IS
  'Categories (DefinedTerm), category placement, display slots, banners, slot bookings.';
COMMENT ON SCHEMA search IS
  'Search documents: full text and embeddings per product.';
COMMENT ON SCHEMA promotion IS
  'Promotions, member and sale prices (UnitPriceSpecification), coupons, member coupons.';
COMMENT ON SCHEMA customer IS
  'People (Person), addresses, member programs, tiers, memberships, remembered preferences.';
COMMENT ON SCHEMA sales IS
  'Carts, orders (Order), order items (OrderItem), payments, order events.';
COMMENT ON SCHEMA fulfillment IS
  'Parcel deliveries, claims (cancel, return, exchange), inspections, refunds.';
COMMENT ON SCHEMA messaging IS
  'Transactional messages to shoppers.';
COMMENT ON SCHEMA analytics IS
  'Read-only views for insight queries; the only place read-only roles query.';
COMMENT ON SCHEMA agent IS
  'Agent runtime records: conversations, turns, steps, model calls (owned by the agent service).';
