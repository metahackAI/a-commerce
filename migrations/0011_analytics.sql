-- 0011_analytics.sql
-- Analytics: read-only views. Views run with the owner's rights, and every base table forces
-- row-level security, so a reader still sees only the store in app.tenant_id.

CREATE VIEW analytics.daily_sales AS
SELECT
  o.tenant_id,
  (o.order_date AT TIME ZONE s.time_zone)::date AS observation_date,
  sum(o.total_payment_due_value) AS revenue_value,
  o.total_payment_due_currency AS revenue_currency,
  count(*) AS order_count
FROM sales.sales_order AS o
JOIN platform.online_store AS s ON s.id = o.tenant_id
WHERE o.order_status NOT IN ('payment_due', 'cancelled')
GROUP BY o.tenant_id, (o.order_date AT TIME ZONE s.time_zone)::date, o.total_payment_due_currency;

COMMENT ON VIEW analytics.daily_sales IS
  'schema:Observation/acom:DailySales — revenue and order count per store and day, with the day taken in the store''s time zone; orders still awaiting payment and cancelled orders are left out.';
COMMENT ON COLUMN analytics.daily_sales.tenant_id IS
  '- store (tenant) the figures belong to.';
COMMENT ON COLUMN analytics.daily_sales.observation_date IS
  'schema:observationDate — calendar day in the store''s time zone on which the orders were placed.';
COMMENT ON COLUMN analytics.daily_sales.revenue_value IS
  'acom:revenue — sum of total_payment_due of the day''s orders (after discounts, including shipping), in major units.';
COMMENT ON COLUMN analytics.daily_sales.revenue_currency IS
  'acom:revenue — ISO 4217 code of revenue_value.';
COMMENT ON COLUMN analytics.daily_sales.order_count IS
  'acom:orderCount — number of orders placed that day that were paid and not cancelled.';
