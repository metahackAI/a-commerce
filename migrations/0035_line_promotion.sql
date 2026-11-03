-- Sales lines carry the promotion that priced them, so a promotion's results (what it sold, what
-- it gave away) are read like any other split of sales. Columns are only added at the end, so the
-- view keeps its grants and every existing reader.

CREATE OR REPLACE VIEW analytics.order_line_summary AS
SELECT
  i.tenant_id,
  i.order_id,
  (o.order_date AT TIME ZONE s.time_zone)::date AS observation_date,
  i.product_id,
  p.product_group_id,
  c.category_id,
  o.campaign_id,
  i.order_quantity,
  i.price * i.order_quantity - i.discount_value AS revenue_value,
  i.price_currency AS revenue_currency,
  i.promotion_id,
  pr.name AS promotion_name,
  i.discount_value
FROM sales.order_item AS i
JOIN sales.sales_order AS o ON o.tenant_id = i.tenant_id AND o.id = i.order_id
JOIN platform.online_store AS s ON s.id = i.tenant_id
JOIN catalog.product AS p ON p.tenant_id = i.tenant_id AND p.id = i.product_id
LEFT JOIN LATERAL (
  SELECT pc.category_id FROM merchandising.product_group_category AS pc
  WHERE pc.tenant_id = p.tenant_id AND pc.product_group_id = p.product_group_id AND pc.is_primary
  ORDER BY pc.category_id
  LIMIT 1
) AS c ON true
LEFT JOIN promotion.promotion AS pr ON pr.tenant_id = i.tenant_id AND pr.id = i.promotion_id
WHERE o.order_status NOT IN ('payment_due', 'cancelled');

COMMENT ON COLUMN analytics.order_line_summary.promotion_id IS
  'acom:promotionId — promotion whose price or discount applied to the line (promo_ id); NULL when none did.';
COMMENT ON COLUMN analytics.order_line_summary.promotion_name IS
  'schema:name — that promotion''s name; NULL when none applied.';
COMMENT ON COLUMN analytics.order_line_summary.discount_value IS
  'schema:discount — what the line was given off its price, as a positive amount, in major units.';
