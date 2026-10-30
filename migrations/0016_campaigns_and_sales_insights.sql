-- 0016_campaigns_and_sales_insights.sql
-- Messaging campaigns, the campaign an order came from, and the analytics read models behind
-- explain_sales_change, get_sales_trend, get_sales_overview and get_customer_overview
-- (docs/api/analytics.md). Rules: docs/backend/database.md. Order: tables, indexes, comments,
-- row-level security, then the views.
--
-- The views run with the owner's rights, and every base table forces row-level security, so a
-- reader still sees only the store in app.tenant_id (as in 0011_analytics.sql). They hold no
-- personal data: no names, contacts, addresses or customer ids.


CREATE TABLE messaging.campaign (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  message_channel text NOT NULL,
  template_id text,
  valid_for_member_tier text,
  campaign_status text NOT NULL DEFAULT 'draft',
  scheduled_time timestamptz,
  date_sent timestamptz,
  recipient_count integer,
  error jsonb,
  change_set_id text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT campaign_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT campaign_id_check CHECK (id ~ '^cmp_[a-z0-9_]+$'),
  CONSTRAINT campaign_template_id_fkey FOREIGN KEY (tenant_id, template_id) REFERENCES messaging.message_template (tenant_id, id),
  CONSTRAINT campaign_valid_for_member_tier_fkey FOREIGN KEY (tenant_id, valid_for_member_tier) REFERENCES customer.member_program_tier (tenant_id, id),
  CONSTRAINT campaign_change_set_id_fkey FOREIGN KEY (tenant_id, change_set_id) REFERENCES platform.change_set (tenant_id, id),
  CONSTRAINT campaign_message_channel_check CHECK (message_channel IN ('email', 'sms', 'push', 'in_app')),
  CONSTRAINT campaign_campaign_status_check CHECK (campaign_status IN ('draft', 'scheduled', 'sending', 'sent', 'failed', 'cancelled')),
  CONSTRAINT campaign_recipient_count_check CHECK (recipient_count >= 0)
);

CREATE INDEX campaign_template_id_idx ON messaging.campaign (tenant_id, template_id);
CREATE INDEX campaign_scheduled_time_idx ON messaging.campaign (tenant_id, scheduled_time);

COMMENT ON TABLE messaging.campaign IS
  'schema:CreativeWork/acom:Campaign — a marketing message sent to many shoppers at a scheduled time from one template; sending it reaches customers. id prefix cmp_.';
COMMENT ON COLUMN messaging.campaign.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN messaging.campaign.id IS
  '- id with prefix cmp_; the same value the API, logs and events use.';
COMMENT ON COLUMN messaging.campaign.name IS
  'schema:name — name operators know the campaign by, for example Weekly email.';
COMMENT ON COLUMN messaging.campaign.message_channel IS
  'acom:messageChannel — channel it is sent on; the template must be written for the same channel. values: message_channel';
COMMENT ON COLUMN messaging.campaign.template_id IS
  'acom:templateId — template the message is made from (tpl_ id); campaigns of one template are the same recurring send.';
COMMENT ON COLUMN messaging.campaign.valid_for_member_tier IS
  'schema:validForMemberTier — tier the campaign goes to; NULL means every member who agreed to marketing messages.';
COMMENT ON COLUMN messaging.campaign.campaign_status IS
  'acom:campaignStatus — where the campaign is; failed means the send stopped before any message went out. values: campaign_status';
COMMENT ON COLUMN messaging.campaign.scheduled_time IS
  'acom:scheduledTime — when the send is due, on the store clock; NULL while a draft has no time.';
COMMENT ON COLUMN messaging.campaign.date_sent IS
  'schema:dateSent — when the messages went out; NULL while not sent, and for failed and cancelled campaigns.';
COMMENT ON COLUMN messaging.campaign.recipient_count IS
  'acom:recipientCount — shoppers the campaign was addressed to after consent checks; NULL until the list is built.';
COMMENT ON COLUMN messaging.campaign.error IS
  'schema:error — why the send failed, as {code, message}; NULL unless campaign_status is failed.';
COMMENT ON COLUMN messaging.campaign.change_set_id IS
  'acom:changeSetId — change set that approved the campaign; NULL for sends set up before change sets.';
COMMENT ON COLUMN messaging.campaign.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN messaging.campaign.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN messaging.campaign.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE messaging.campaign ENABLE ROW LEVEL SECURITY;
ALTER TABLE messaging.campaign FORCE ROW LEVEL SECURITY;
CREATE POLICY campaign_tenant_isolation ON messaging.campaign
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));


-- The campaign an order came from: the shopper followed a link in that campaign's message.
ALTER TABLE sales.sales_order ADD COLUMN campaign_id text;
ALTER TABLE sales.sales_order ADD CONSTRAINT sales_order_campaign_id_fkey FOREIGN KEY (tenant_id, campaign_id) REFERENCES messaging.campaign (tenant_id, id);

CREATE INDEX sales_order_campaign_id_idx ON sales.sales_order (tenant_id, campaign_id);

COMMENT ON COLUMN sales.sales_order.campaign_id IS
  'acom:campaignId — campaign whose message the shopper followed to place the order (cmp_ id); NULL when the order came from anywhere else.';


-- One row per order that was paid for (cancelled ones included, so rates can be taken).
CREATE VIEW analytics.order_summary AS
SELECT
  o.tenant_id,
  o.id AS order_id,
  (o.order_date AT TIME ZONE s.time_zone)::date AS observation_date,
  o.order_status,
  m.tier_id AS member_tier,
  o.campaign_id,
  o.total_payment_due_value AS revenue_value,
  o.total_payment_due_currency AS revenue_currency,
  EXISTS (
    SELECT 1 FROM fulfillment.claim AS c
    WHERE c.tenant_id = o.tenant_id AND c.order_id = o.id AND c.claim_type = 'return'
      AND c.claim_status NOT IN ('declined', 'withdrawn')
  ) AS has_return_request
FROM sales.sales_order AS o
JOIN platform.online_store AS s ON s.id = o.tenant_id
LEFT JOIN LATERAL (
  SELECT pm.tier_id FROM customer.program_membership AS pm
  WHERE pm.tenant_id = o.tenant_id AND pm.customer_id = o.customer_id
  ORDER BY pm.tier_since DESC, pm.member_program_id
  LIMIT 1
) AS m ON true
WHERE o.order_status <> 'payment_due';

COMMENT ON VIEW analytics.order_summary IS
  'schema:Order/acom:OrderSummary — one row per paid order with only what aggregates need (day, state, tier, campaign, amount, return request), never who placed it; orders still awaiting payment are left out.';
COMMENT ON COLUMN analytics.order_summary.tenant_id IS
  '- store (tenant) the order belongs to.';
COMMENT ON COLUMN analytics.order_summary.order_id IS
  'acom:orderId — the order (ord_ id).';
COMMENT ON COLUMN analytics.order_summary.observation_date IS
  'schema:observationDate — calendar day in the store''s time zone on which the order was placed.';
COMMENT ON COLUMN analytics.order_summary.order_status IS
  'schema:orderStatus — where the order is now; cancelled orders count for the cancellation rate only. values: order_status';
COMMENT ON COLUMN analytics.order_summary.member_tier IS
  'acom:tierId — the tier the shopper is in now; NULL for guest orders.';
COMMENT ON COLUMN analytics.order_summary.campaign_id IS
  'acom:campaignId — campaign the order came from (cmp_ id); NULL when it came from anywhere else.';
COMMENT ON COLUMN analytics.order_summary.revenue_value IS
  'acom:revenue — total_payment_due of the order (after discounts, with shipping and gift wrap), in major units.';
COMMENT ON COLUMN analytics.order_summary.revenue_currency IS
  'acom:revenue — ISO 4217 code of revenue_value.';
COMMENT ON COLUMN analytics.order_summary.has_return_request IS
  'acom:hasReturnRequest — true when a return was asked for on the order and not declined or withdrawn.';


-- One row per order line of the orders that count as sales.
CREATE VIEW analytics.order_line_summary AS
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
  i.price_currency AS revenue_currency
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
WHERE o.order_status NOT IN ('payment_due', 'cancelled');

COMMENT ON VIEW analytics.order_line_summary IS
  'schema:OrderItem/acom:OrderLineSummary — one row per line of the orders that count as sales (paid, not cancelled): day, variant, product, category, campaign, units and net amount, never who bought it.';
COMMENT ON COLUMN analytics.order_line_summary.tenant_id IS
  '- store (tenant) the line belongs to.';
COMMENT ON COLUMN analytics.order_line_summary.order_id IS
  'acom:orderId — the order the line is in (ord_ id).';
COMMENT ON COLUMN analytics.order_line_summary.observation_date IS
  'schema:observationDate — calendar day in the store''s time zone on which the order was placed.';
COMMENT ON COLUMN analytics.order_line_summary.product_id IS
  'schema:orderedItem — the variant sold (var_ id; API variant_id).';
COMMENT ON COLUMN analytics.order_line_summary.product_group_id IS
  'schema:isVariantOf — the product the variant belongs to (prd_ id; API product_id).';
COMMENT ON COLUMN analytics.order_line_summary.category_id IS
  'schema:category — the product''s primary standard category (cat_ id); NULL when it has none.';
COMMENT ON COLUMN analytics.order_line_summary.campaign_id IS
  'acom:campaignId — campaign the order came from (cmp_ id); NULL when it came from anywhere else.';
COMMENT ON COLUMN analytics.order_line_summary.order_quantity IS
  'schema:orderQuantity — units sold on the line.';
COMMENT ON COLUMN analytics.order_line_summary.revenue_value IS
  'acom:revenue — price × quantity − line discount, in major units; shipping is not on lines.';
COMMENT ON COLUMN analytics.order_line_summary.revenue_currency IS
  'acom:revenue — ISO 4217 code of revenue_value.';


-- Variants that are out of stock now, with when they ran out.
CREATE VIEW analytics.variant_stock_out AS
SELECT
  f.tenant_id,
  f.product_id,
  p.product_group_id,
  g.name AS product_name,
  p.size,
  p.color,
  ('bestseller' = ANY (g.keywords)) AS is_bestseller,
  f.sold_out_at,
  f.availability_starts
FROM catalog.offer AS f
JOIN catalog.product AS p ON p.tenant_id = f.tenant_id AND p.id = f.product_id
JOIN catalog.product_group AS g ON g.tenant_id = p.tenant_id AND g.id = p.product_group_id
WHERE f.availability = 'out_of_stock' AND f.sold_out_at IS NOT NULL;

COMMENT ON VIEW analytics.variant_stock_out IS
  'schema:Offer/acom:StockOut — one row per variant that is out of stock now, with when it ran out and when it is expected back.';
COMMENT ON COLUMN analytics.variant_stock_out.tenant_id IS
  '- store (tenant) the variant belongs to.';
COMMENT ON COLUMN analytics.variant_stock_out.product_id IS
  'schema:itemOffered — the variant that ran out (var_ id; API variant_id).';
COMMENT ON COLUMN analytics.variant_stock_out.product_group_id IS
  'schema:isVariantOf — the product the variant belongs to (prd_ id; API product_id).';
COMMENT ON COLUMN analytics.variant_stock_out.product_name IS
  'schema:name — name of the product, for example Cotton chore coat.';
COMMENT ON COLUMN analytics.variant_stock_out.size IS
  'schema:size — size word of the variant; NULL for one-size products.';
COMMENT ON COLUMN analytics.variant_stock_out.color IS
  'schema:color — color name of the variant; NULL when the product has one color.';
COMMENT ON COLUMN analytics.variant_stock_out.is_bestseller IS
  'acom:isBestseller — true when the store tags the product bestseller (catalog keywords).';
COMMENT ON COLUMN analytics.variant_stock_out.sold_out_at IS
  'acom:soldOutAt — when the variant ran out.';
COMMENT ON COLUMN analytics.variant_stock_out.availability_starts IS
  'schema:availabilityStarts — when stock is expected back; NULL when unknown.';


-- Campaign sends as analytics reads them: when each was due, whether it went out, and why not.
CREATE VIEW analytics.campaign_send AS
SELECT
  c.tenant_id,
  c.id AS campaign_id,
  c.name,
  c.template_id,
  c.message_channel,
  c.campaign_status,
  c.scheduled_time,
  c.date_sent,
  c.recipient_count,
  c.error ->> 'message' AS error_message
FROM messaging.campaign AS c;

COMMENT ON VIEW analytics.campaign_send IS
  'schema:CreativeWork/acom:CampaignSend — one row per campaign: when it was due, whether and when it went out, and why it failed.';
COMMENT ON COLUMN analytics.campaign_send.tenant_id IS
  '- store (tenant) the campaign belongs to.';
COMMENT ON COLUMN analytics.campaign_send.campaign_id IS
  'acom:campaignId — the campaign (cmp_ id).';
COMMENT ON COLUMN analytics.campaign_send.name IS
  'schema:name — campaign name, for example Weekly email.';
COMMENT ON COLUMN analytics.campaign_send.template_id IS
  'acom:templateId — template it is made from (tpl_ id); the sends of one template are one recurring campaign.';
COMMENT ON COLUMN analytics.campaign_send.message_channel IS
  'acom:messageChannel — channel it goes out on. values: message_channel';
COMMENT ON COLUMN analytics.campaign_send.campaign_status IS
  'acom:campaignStatus — where the campaign is. values: campaign_status';
COMMENT ON COLUMN analytics.campaign_send.scheduled_time IS
  'acom:scheduledTime — when the send was due.';
COMMENT ON COLUMN analytics.campaign_send.date_sent IS
  'schema:dateSent — when the messages went out; NULL when they did not.';
COMMENT ON COLUMN analytics.campaign_send.recipient_count IS
  'acom:recipientCount — shoppers it was addressed to.';
COMMENT ON COLUMN analytics.campaign_send.error_message IS
  'schema:error — why the send failed, in a few words, for example List import failed; NULL unless it failed.';


-- Members per tier now (the count list_tiers shows).
CREATE VIEW analytics.member_tier_count AS
SELECT
  m.tenant_id,
  m.tier_id AS member_tier,
  count(*) AS member_count
FROM customer.program_membership AS m
JOIN customer.person AS p ON p.tenant_id = m.tenant_id AND p.id = m.customer_id
WHERE p.member_state NOT IN ('blocked', 'withdrawn')
GROUP BY m.tenant_id, m.tier_id;

COMMENT ON VIEW analytics.member_tier_count IS
  'schema:Observation/acom:TierCount — members per tier now, leaving out blocked and withdrawn accounts.';
COMMENT ON COLUMN analytics.member_tier_count.tenant_id IS
  '- store (tenant) the members belong to.';
COMMENT ON COLUMN analytics.member_tier_count.member_tier IS
  'acom:tierId — the tier.';
COMMENT ON COLUMN analytics.member_tier_count.member_count IS
  'acom:memberCount — members in the tier now.';


-- New accounts per day, by the tier the members are in now.
CREATE VIEW analytics.daily_new_members AS
SELECT
  p.tenant_id,
  (p.registered_at AT TIME ZONE s.time_zone)::date AS observation_date,
  m.tier_id AS member_tier,
  count(*) AS member_count
FROM customer.person AS p
JOIN platform.online_store AS s ON s.id = p.tenant_id
LEFT JOIN LATERAL (
  SELECT pm.tier_id FROM customer.program_membership AS pm
  WHERE pm.tenant_id = p.tenant_id AND pm.customer_id = p.id
  ORDER BY pm.tier_since DESC, pm.member_program_id
  LIMIT 1
) AS m ON true
GROUP BY p.tenant_id, (p.registered_at AT TIME ZONE s.time_zone)::date, m.tier_id;

COMMENT ON VIEW analytics.daily_new_members IS
  'schema:Observation/acom:NewMembers — accounts created per store day, by the tier the members are in now.';
COMMENT ON COLUMN analytics.daily_new_members.tenant_id IS
  '- store (tenant) the members belong to.';
COMMENT ON COLUMN analytics.daily_new_members.observation_date IS
  'schema:observationDate — calendar day in the store''s time zone on which the accounts were created.';
COMMENT ON COLUMN analytics.daily_new_members.member_tier IS
  'acom:tierId — tier the members are in now; NULL for accounts outside every program.';
COMMENT ON COLUMN analytics.daily_new_members.member_count IS
  'acom:memberCount — accounts created that day.';
