-- 0007_sales.sql
-- Sales: carts and cart lines, orders, order items, payments and order events.
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security.


CREATE TABLE sales.cart (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  customer_id text,
  session_id text,
  delivery_address_id text,
  payment_method text,
  sales_channel text NOT NULL DEFAULT 'web',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT cart_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT cart_id_check CHECK (id ~ '^cart_[a-z0-9_]+$'),
  CONSTRAINT cart_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT cart_delivery_address_id_fkey FOREIGN KEY (tenant_id, delivery_address_id) REFERENCES customer.postal_address (tenant_id, id),
  CONSTRAINT cart_payment_method_check CHECK (payment_method IN ('credit_card', 'bank_transfer', 'virtual_account', 'loyalty_points', 'store_credit')),
  CONSTRAINT cart_sales_channel_check CHECK (sales_channel IN ('web', 'mobile_web', 'app', 'agent')),
  CONSTRAINT cart_owner_check CHECK (customer_id IS NOT NULL OR session_id IS NOT NULL)
);

CREATE TABLE sales.cart_line (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  cart_id text NOT NULL,
  product_id text NOT NULL,
  order_quantity integer NOT NULL,
  position integer NOT NULL,
  description text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT cart_line_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT cart_line_id_check CHECK (id ~ '^cln_[a-z0-9_]+$'),
  CONSTRAINT cart_line_cart_id_product_id_key UNIQUE (tenant_id, cart_id, product_id),
  CONSTRAINT cart_line_cart_id_fkey FOREIGN KEY (tenant_id, cart_id) REFERENCES sales.cart (tenant_id, id),
  CONSTRAINT cart_line_product_id_fkey FOREIGN KEY (tenant_id, product_id) REFERENCES catalog.product (tenant_id, id),
  CONSTRAINT cart_line_order_quantity_check CHECK (order_quantity >= 1),
  CONSTRAINT cart_line_position_check CHECK (position >= 1)
);

CREATE TABLE sales.sales_order (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  order_number text NOT NULL,
  order_date timestamptz NOT NULL,
  order_status text NOT NULL DEFAULT 'payment_due',
  customer_id text,
  sales_channel text NOT NULL DEFAULT 'web',
  is_gift boolean NOT NULL DEFAULT false,
  gift_options jsonb,
  discount_code text,
  coupon_id text,
  subtotal_value numeric(12,2) NOT NULL,
  subtotal_currency char(3) NOT NULL,
  discount_value numeric(12,2) NOT NULL,
  discount_currency char(3) NOT NULL,
  shipping_value numeric(12,2) NOT NULL,
  shipping_currency char(3) NOT NULL,
  gift_wrap_value numeric(12,2),
  gift_wrap_currency char(3),
  total_payment_due_value numeric(12,2) NOT NULL,
  total_payment_due_currency char(3) NOT NULL,
  preview_id text,
  confirmed_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT sales_order_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT sales_order_id_check CHECK (id ~ '^ord_[a-z0-9_]+$'),
  CONSTRAINT sales_order_order_number_key UNIQUE (tenant_id, order_number),
  CONSTRAINT sales_order_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT sales_order_coupon_id_fkey FOREIGN KEY (tenant_id, coupon_id) REFERENCES promotion.coupon (tenant_id, id),
  CONSTRAINT sales_order_preview_id_fkey FOREIGN KEY (tenant_id, preview_id) REFERENCES platform.preview (tenant_id, id),
  CONSTRAINT sales_order_order_number_check CHECK (order_number ~ '^[0-9]+$'),
  CONSTRAINT sales_order_order_status_check CHECK (order_status IN ('payment_due', 'processing', 'in_transit', 'delivered', 'returned', 'cancelled', 'problem')),
  CONSTRAINT sales_order_sales_channel_check CHECK (sales_channel IN ('web', 'mobile_web', 'app', 'agent')),
  CONSTRAINT sales_order_subtotal_value_check CHECK (subtotal_value >= 0),
  CONSTRAINT sales_order_subtotal_currency_check CHECK (subtotal_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT sales_order_discount_value_check CHECK (discount_value >= 0),
  CONSTRAINT sales_order_discount_currency_check CHECK (discount_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT sales_order_shipping_value_check CHECK (shipping_value >= 0),
  CONSTRAINT sales_order_shipping_currency_check CHECK (shipping_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT sales_order_gift_wrap_value_check CHECK (gift_wrap_value >= 0),
  CONSTRAINT sales_order_gift_wrap_currency_check CHECK (gift_wrap_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT sales_order_total_payment_due_value_check CHECK (total_payment_due_value >= 0),
  CONSTRAINT sales_order_total_payment_due_currency_check CHECK (total_payment_due_currency ~ '^[A-Z]{3}$')
);

CREATE TABLE sales.order_item (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  order_id text NOT NULL,
  position integer NOT NULL,
  product_id text NOT NULL,
  name text NOT NULL,
  order_quantity integer NOT NULL,
  order_item_status text NOT NULL DEFAULT 'payment_due',
  fulfillment_stage text,
  price numeric(12,2) NOT NULL,
  price_currency char(3) NOT NULL,
  discount_value numeric(12,2) NOT NULL,
  discount_currency char(3) NOT NULL,
  promotion_id text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT order_item_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT order_item_id_check CHECK (id ~ '^oln_[a-z0-9_]+$'),
  CONSTRAINT order_item_order_id_position_key UNIQUE (tenant_id, order_id, position),
  CONSTRAINT order_item_order_id_fkey FOREIGN KEY (tenant_id, order_id) REFERENCES sales.sales_order (tenant_id, id),
  CONSTRAINT order_item_product_id_fkey FOREIGN KEY (tenant_id, product_id) REFERENCES catalog.product (tenant_id, id),
  CONSTRAINT order_item_promotion_id_fkey FOREIGN KEY (tenant_id, promotion_id) REFERENCES promotion.promotion (tenant_id, id),
  CONSTRAINT order_item_position_check CHECK (position >= 1),
  CONSTRAINT order_item_order_quantity_check CHECK (order_quantity >= 1),
  CONSTRAINT order_item_order_item_status_check CHECK (order_item_status IN ('payment_due', 'processing', 'in_transit', 'delivered', 'returned', 'cancelled', 'problem')),
  CONSTRAINT order_item_fulfillment_stage_check CHECK (fulfillment_stage IN ('awaiting_stock', 'ship_instructed', 'packed', 'shipped')),
  CONSTRAINT order_item_price_check CHECK (price >= 0),
  CONSTRAINT order_item_price_currency_check CHECK (price_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT order_item_discount_value_check CHECK (discount_value >= 0),
  CONSTRAINT order_item_discount_currency_check CHECK (discount_currency ~ '^[A-Z]{3}$')
);

CREATE TABLE sales.payment (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  order_id text NOT NULL,
  payment_method text NOT NULL,
  payment_status text NOT NULL DEFAULT 'payment_due',
  amount_value numeric(12,2) NOT NULL,
  amount_currency char(3) NOT NULL,
  card_last4 text,
  installment_months integer NOT NULL DEFAULT 0,
  external_id text,
  paid_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT payment_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT payment_id_check CHECK (id ~ '^pay_[a-z0-9_]+$'),
  CONSTRAINT payment_order_id_fkey FOREIGN KEY (tenant_id, order_id) REFERENCES sales.sales_order (tenant_id, id),
  CONSTRAINT payment_payment_method_check CHECK (payment_method IN ('credit_card', 'bank_transfer', 'virtual_account', 'loyalty_points', 'store_credit')),
  CONSTRAINT payment_payment_status_check CHECK (payment_status IN ('payment_due', 'payment_complete', 'payment_declined', 'payment_past_due', 'payment_automatically_applied')),
  CONSTRAINT payment_amount_value_check CHECK (amount_value >= 0),
  CONSTRAINT payment_amount_currency_check CHECK (amount_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT payment_card_last4_check CHECK (card_last4 ~ '^[0-9]{4}$'),
  CONSTRAINT payment_installment_months_check CHECK (installment_months >= 0)
);

CREATE TABLE sales.order_event (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  order_id text NOT NULL,
  position integer NOT NULL,
  action_type text NOT NULL,
  action_status text NOT NULL,
  agent_id text,
  agent_name text,
  instrument text,
  from_state text,
  to_state text,
  result jsonb,
  error jsonb,
  start_time timestamptz NOT NULL,
  end_time timestamptz,
  request_id text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT order_event_pkey PRIMARY KEY (tenant_id, order_id, position),
  CONSTRAINT order_event_order_id_fkey FOREIGN KEY (tenant_id, order_id) REFERENCES sales.sales_order (tenant_id, id),
  CONSTRAINT order_event_position_check CHECK (position >= 1),
  CONSTRAINT order_event_action_status_check CHECK (action_status IN ('potential', 'active', 'completed', 'failed')),
  CONSTRAINT order_event_from_state_check CHECK (from_state IN ('payment_due', 'processing', 'in_transit', 'delivered', 'returned', 'cancelled', 'problem')),
  CONSTRAINT order_event_to_state_check CHECK (to_state IN ('payment_due', 'processing', 'in_transit', 'delivered', 'returned', 'cancelled', 'problem'))
);

CREATE INDEX cart_customer_id_idx ON sales.cart (tenant_id, customer_id);
CREATE INDEX cart_session_id_idx ON sales.cart (tenant_id, session_id);
CREATE INDEX cart_delivery_address_id_idx ON sales.cart (tenant_id, delivery_address_id);
CREATE INDEX cart_line_product_id_idx ON sales.cart_line (tenant_id, product_id);
CREATE INDEX sales_order_customer_id_order_date_idx ON sales.sales_order (tenant_id, customer_id, order_date);
CREATE INDEX sales_order_order_date_idx ON sales.sales_order (tenant_id, order_date);
CREATE INDEX sales_order_order_status_idx ON sales.sales_order (tenant_id, order_status);
CREATE INDEX sales_order_coupon_id_idx ON sales.sales_order (tenant_id, coupon_id);
CREATE INDEX sales_order_preview_id_idx ON sales.sales_order (tenant_id, preview_id);
CREATE INDEX order_item_product_id_idx ON sales.order_item (tenant_id, product_id);
CREATE INDEX order_item_promotion_id_idx ON sales.order_item (tenant_id, promotion_id);
CREATE INDEX order_item_order_item_status_idx ON sales.order_item (tenant_id, order_item_status);
CREATE INDEX payment_order_id_idx ON sales.payment (tenant_id, order_id);
CREATE INDEX payment_payment_status_idx ON sales.payment (tenant_id, payment_status);
CREATE INDEX order_event_start_time_idx ON sales.order_event (tenant_id, start_time);

COMMENT ON TABLE sales.cart IS
  'schema:Order/acom:Cart — an order before checkout; one per signed-in shopper or guest session. id prefix cart_.';
COMMENT ON COLUMN sales.cart.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN sales.cart.id IS
  '- id with prefix cart_; the same value the API, logs and events use.';
COMMENT ON COLUMN sales.cart.customer_id IS
  'schema:customer — shopper (cus_ id); NULL for a guest cart.';
COMMENT ON COLUMN sales.cart.session_id IS
  'acom:sessionId — guest session that owns the cart (ses_ id); NULL for a signed-in shopper.';
COMMENT ON COLUMN sales.cart.delivery_address_id IS
  'acom:deliveryAddressId — saved address used for delivery estimates (adr_ id).';
COMMENT ON COLUMN sales.cart.payment_method IS
  'schema:paymentMethod — payment method chosen so far. values: payment_method';
COMMENT ON COLUMN sales.cart.sales_channel IS
  'acom:salesChannel — where the cart was built. values: sales_channel';
COMMENT ON COLUMN sales.cart.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN sales.cart.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN sales.cart.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE sales.cart_line IS
  'schema:OrderItem/acom:CartLine — one variant and quantity in a cart. id prefix cln_.';
COMMENT ON COLUMN sales.cart_line.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN sales.cart_line.id IS
  '- id with prefix cln_; the same value the API, logs and events use.';
COMMENT ON COLUMN sales.cart_line.cart_id IS
  'acom:cartId — the cart (cart_ id).';
COMMENT ON COLUMN sales.cart_line.product_id IS
  'schema:orderedItem — the variant (var_ id; API variant_id).';
COMMENT ON COLUMN sales.cart_line.order_quantity IS
  'schema:orderQuantity — units, 1 up to cart.max_quantity_per_line.';
COMMENT ON COLUMN sales.cart_line.position IS
  'schema:position — line order in the cart.';
COMMENT ON COLUMN sales.cart_line.description IS
  'schema:description — the shopper''s note on the line.';
COMMENT ON COLUMN sales.cart_line.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN sales.cart_line.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN sales.cart_line.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE sales.sales_order IS
  'schema:Order — an order a shopper placed; order_number is what people see. id prefix ord_. API: order_id.';
COMMENT ON COLUMN sales.sales_order.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN sales.sales_order.id IS
  '- id with prefix ord_; the same value the API, logs and events use.';
COMMENT ON COLUMN sales.sales_order.order_number IS
  'schema:orderNumber — number people see (20402), unique in the store; same digits as the id suffix.';
COMMENT ON COLUMN sales.sales_order.order_date IS
  'schema:orderDate — when the order was placed.';
COMMENT ON COLUMN sales.sales_order.order_status IS
  'schema:orderStatus — where the order is as a whole; each item also has its own status. values: order_status';
COMMENT ON COLUMN sales.sales_order.customer_id IS
  'schema:customer — shopper who placed it (cus_ id); NULL for a guest order.';
COMMENT ON COLUMN sales.sales_order.sales_channel IS
  'acom:salesChannel — where the order was placed. values: sales_channel';
COMMENT ON COLUMN sales.sales_order.is_gift IS
  'schema:isGift — true for a gift order; recipient details live only on this order.';
COMMENT ON COLUMN sales.sales_order.gift_options IS
  'acom:giftOptions — {card_message, gift_receipt, exchange_until} for gift orders.';
COMMENT ON COLUMN sales.sales_order.discount_code IS
  'schema:discountCode — coupon code recorded on the order; NULL when none.';
COMMENT ON COLUMN sales.sales_order.coupon_id IS
  'acom:couponId — coupon recorded on the order (cpn_ id).';
COMMENT ON COLUMN sales.sales_order.subtotal_value IS
  'acom:subtotal — sum of line list prices × quantities before discounts, in major units of the currency.';
COMMENT ON COLUMN sales.sales_order.subtotal_currency IS
  'acom:subtotal — ISO 4217 code of subtotal_value.';
COMMENT ON COLUMN sales.sales_order.discount_value IS
  'schema:discount — total discount on the order as a positive amount, in major units of the currency.';
COMMENT ON COLUMN sales.sales_order.discount_currency IS
  'schema:discount — ISO 4217 code of discount_value.';
COMMENT ON COLUMN sales.sales_order.shipping_value IS
  'acom:shipping — shipping charged; 0.00 when free, in major units of the currency.';
COMMENT ON COLUMN sales.sales_order.shipping_currency IS
  'acom:shipping — ISO 4217 code of shipping_value.';
COMMENT ON COLUMN sales.sales_order.gift_wrap_value IS
  'acom:giftWrap — gift wrap fee; NULL when not wrapped, in major units of the currency.';
COMMENT ON COLUMN sales.sales_order.gift_wrap_currency IS
  'acom:giftWrap — ISO 4217 code of gift_wrap_value.';
COMMENT ON COLUMN sales.sales_order.total_payment_due_value IS
  'schema:totalPaymentDue — what the shopper pays: subtotal − discount + shipping + gift wrap, in major units of the currency.';
COMMENT ON COLUMN sales.sales_order.total_payment_due_currency IS
  'schema:totalPaymentDue — ISO 4217 code of total_payment_due_value.';
COMMENT ON COLUMN sales.sales_order.preview_id IS
  'acom:previewId — prepare_order preview the shopper confirmed (pv_ id).';
COMMENT ON COLUMN sales.sales_order.confirmed_at IS
  'acom:confirmedAt — when the purchase was confirmed after delivery (order.yml auto_confirm_days).';
COMMENT ON COLUMN sales.sales_order.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN sales.sales_order.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN sales.sales_order.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE sales.order_item IS
  'schema:OrderItem — one variant line of an order; each line moves through its own status. id prefix oln_.';
COMMENT ON COLUMN sales.order_item.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN sales.order_item.id IS
  '- id with prefix oln_; the same value the API, logs and events use.';
COMMENT ON COLUMN sales.order_item.order_id IS
  'acom:orderId — the order (ord_ id).';
COMMENT ON COLUMN sales.order_item.position IS
  'schema:position — line number in the order.';
COMMENT ON COLUMN sales.order_item.product_id IS
  'schema:orderedItem — the variant bought (var_ id; API variant_id).';
COMMENT ON COLUMN sales.order_item.name IS
  'schema:name — product name at the time of the order, as printed on receipts.';
COMMENT ON COLUMN sales.order_item.order_quantity IS
  'schema:orderQuantity — units ordered.';
COMMENT ON COLUMN sales.order_item.order_item_status IS
  'schema:orderItemStatus — where this line is. values: order_status';
COMMENT ON COLUMN sales.order_item.fulfillment_stage IS
  'acom:fulfillmentStage — warehouse step of the line; NULL before payment and after delivery. values: fulfillment_stage';
COMMENT ON COLUMN sales.order_item.price IS
  'schema:price — list price per unit when ordered.';
COMMENT ON COLUMN sales.order_item.price_currency IS
  'schema:priceCurrency — ISO 4217 code of price.';
COMMENT ON COLUMN sales.order_item.discount_value IS
  'schema:discount — discount on this line, as a positive amount, including its share of order discounts, in major units of the currency.';
COMMENT ON COLUMN sales.order_item.discount_currency IS
  'schema:discount — ISO 4217 code of discount_value.';
COMMENT ON COLUMN sales.order_item.promotion_id IS
  'acom:promotionId — promotion whose price or discount applied to the line.';
COMMENT ON COLUMN sales.order_item.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN sales.order_item.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN sales.order_item.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE sales.payment IS
  'schema:PayAction — payment of one order with one method; card numbers are never stored, only the last four digits. id prefix pay_.';
COMMENT ON COLUMN sales.payment.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN sales.payment.id IS
  '- id with prefix pay_; the same value the API, logs and events use.';
COMMENT ON COLUMN sales.payment.order_id IS
  'acom:orderId — order paid for (ord_ id).';
COMMENT ON COLUMN sales.payment.payment_method IS
  'schema:paymentMethod — how the shopper paid. values: payment_method';
COMMENT ON COLUMN sales.payment.payment_status IS
  'schema:paymentStatus — state of the payment. values: payment_status';
COMMENT ON COLUMN sales.payment.amount_value IS
  'schema:amount — amount charged, in major units of the currency.';
COMMENT ON COLUMN sales.payment.amount_currency IS
  'schema:amount — ISO 4217 code of amount_value.';
COMMENT ON COLUMN sales.payment.card_last4 IS
  'acom:cardLast4 — last four digits of the card; NULL for other methods.';
COMMENT ON COLUMN sales.payment.installment_months IS
  'acom:installmentMonths — card installments, 0 means paid at once.';
COMMENT ON COLUMN sales.payment.external_id IS
  'acom:externalId — the payment provider''s reference.';
COMMENT ON COLUMN sales.payment.paid_at IS
  'acom:paidAt — when the payment completed.';
COMMENT ON COLUMN sales.payment.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN sales.payment.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN sales.payment.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE sales.order_event IS
  'schema:Action — one state change of an order; rows are only added.';
COMMENT ON COLUMN sales.order_event.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN sales.order_event.order_id IS
  'acom:orderId — the order that changed (ord_ id), the object of the event.';
COMMENT ON COLUMN sales.order_event.position IS
  'schema:position — 1-based order of this event within its aggregate.';
COMMENT ON COLUMN sales.order_event.action_type IS
  'acom:actionType — the schema.org Action subtype (@type), for example UpdateAction or AuthorizeAction.';
COMMENT ON COLUMN sales.order_event.action_status IS
  'schema:actionStatus — lifecycle of the action. values: action_status';
COMMENT ON COLUMN sales.order_event.agent_id IS
  'schema:agent — person or system that acted (adm_, cus_ or sys_ id); NULL when only a model acted.';
COMMENT ON COLUMN sales.order_event.agent_name IS
  'schema:agent — name of the software agent (model) that acted, for example muse-glimmer-30b; NULL for people.';
COMMENT ON COLUMN sales.order_event.instrument IS
  'schema:instrument — tool name or person-only endpoint that caused the event.';
COMMENT ON COLUMN sales.order_event.from_state IS
  'acom:fromState — state before the event; NULL for the creating event. values: order_status';
COMMENT ON COLUMN sales.order_event.to_state IS
  'acom:toState — state after the event; NULL when the state did not change. values: order_status';
COMMENT ON COLUMN sales.order_event.result IS
  'schema:result — what the action produced, as JSON.';
COMMENT ON COLUMN sales.order_event.error IS
  'schema:error — failure as {code, class, message}; NULL when it succeeded.';
COMMENT ON COLUMN sales.order_event.start_time IS
  'schema:startTime — when the action started.';
COMMENT ON COLUMN sales.order_event.end_time IS
  'schema:endTime — when the action ended; NULL while it runs.';
COMMENT ON COLUMN sales.order_event.request_id IS
  '- request trace id (req_) from the gateway; joins agent turns and the action log.';
COMMENT ON COLUMN sales.order_event.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN sales.order_event.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN sales.order_event.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE sales.cart ENABLE ROW LEVEL SECURITY;
ALTER TABLE sales.cart FORCE ROW LEVEL SECURITY;
CREATE POLICY cart_tenant_isolation ON sales.cart
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE sales.cart_line ENABLE ROW LEVEL SECURITY;
ALTER TABLE sales.cart_line FORCE ROW LEVEL SECURITY;
CREATE POLICY cart_line_tenant_isolation ON sales.cart_line
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE sales.sales_order ENABLE ROW LEVEL SECURITY;
ALTER TABLE sales.sales_order FORCE ROW LEVEL SECURITY;
CREATE POLICY sales_order_tenant_isolation ON sales.sales_order
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE sales.order_item ENABLE ROW LEVEL SECURITY;
ALTER TABLE sales.order_item FORCE ROW LEVEL SECURITY;
CREATE POLICY order_item_tenant_isolation ON sales.order_item
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE sales.payment ENABLE ROW LEVEL SECURITY;
ALTER TABLE sales.payment FORCE ROW LEVEL SECURITY;
CREATE POLICY payment_tenant_isolation ON sales.payment
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE sales.order_event ENABLE ROW LEVEL SECURITY;
ALTER TABLE sales.order_event FORCE ROW LEVEL SECURITY;
CREATE POLICY order_event_tenant_isolation ON sales.order_event
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
