// Response types for the storefront, written by hand from docs/api/*.md (catalog, search,
// merchandising, promotion, sales, customer, fulfillment). They follow the specs' example
// outputs; optional fields are the ones a spec shows only sometimes. When the generated contract
// lands in src/contract/, these move there.

/** `{"value": 71.20, "currency": "USD"}` (docs/api/conventions.md §3). */
export interface Money {
  value: number;
  currency: string;
}

export type Availability = 'in_stock' | 'out_of_stock' | 'pre_order' | 'back_order' | 'discontinued';

/** The member or limited-time price that applies to the caller now (`UnitPriceSpecification`). */
export interface MemberPrice {
  price: number;
  price_currency?: string;
  name: string;
  valid_for_member_tier?: string | null;
  promotion_id?: string | null;
  valid_through?: string | null;
}

/** schema.org `ImageObject`, or a plain URL in seed-shaped responses. */
export interface ImageObject {
  content_url?: string;
  url?: string;
  caption?: string | null;
  color?: string | null;
}
export type ImageRef = string | ImageObject | Array<string | ImageObject> | null | undefined;

export interface Variant {
  variant_id: string;
  sku?: string;
  size?: string | null;
  color?: string | null;
  inventory_level?: number;
  availability: Availability;
  image?: ImageRef;
}

export interface AdditionalProperty {
  name: string;
  value: unknown;
  unit_code?: string | null;
  unit_text?: string | null;
  size?: string;
}

/** One `search_products` item (search.md). */
export interface ProductSummary {
  id: string;
  name: string;
  category_id?: string;
  brand_id?: string;
  price: number;
  price_currency: string;
  member_price: MemberPrice | null;
  inventory_level?: number;
  availability?: Availability;
  has_variant?: Variant[];
  image?: ImageRef;
  description?: string | null;
}

/** `get_product` (catalog.md): the sales view of one product. */
export interface Product extends ProductSummary {
  '@type'?: string;
  approval_status?: string;
  varies_by?: string[];
  additional_property?: AdditionalProperty[];
}

/** `get_product_group` (catalog.md): the catalog view. */
export interface ProductGroup {
  id: string;
  name: string;
  description?: string | null;
  image?: ImageRef;
  brand_id?: string;
  category_id?: string;
  approval_status?: string;
  varies_by?: string[];
  has_variant?: Variant[];
}

/** `get_price_breakdown` (catalog.md). */
export interface PriceBreakdown {
  variant_id: string;
  list_price: number;
  member_price: number | null;
  price_currency: string;
  applied: {promotion_id: string; name: string; valid_for_member_tier?: string | null}[];
}

/** `get_product_measurements` (catalog.md). */
export interface Measurements {
  product_id: string;
  unit: string;
  measurements: string[];
  sizes: Array<{size: string} & Record<string, number | string | null>>;
  how_measured?: string | null;
}

export type FitVote = 'true_to_size' | 'runs_small' | 'runs_large';

/** `get_review_summary` (catalog.md). */
export interface ReviewSummary {
  product_id: string;
  filter?: {height_min_cm?: number | null; height_max_cm?: number | null; size?: string | null};
  review_count: number;
  rating_value: number | null;
  fit_votes: {fit: FitVote; label: string; count: number}[];
  mentions: {text: string; count: number}[];
  sample: ReviewSample[];
}

export interface ReviewSample {
  review_id: string;
  review_rating: number;
  size_bought?: string | null;
  height_cm?: number | null;
  excerpt: string;
  date_published?: string | null;
}

/** One `list_reviews` item (catalog.md: `review_rating`, `author`, `review_body`). */
export interface Review {
  id: string;
  product_id?: string;
  review_rating: number;
  author?: string | {name?: string} | null;
  review_body?: string | null;
  name?: string | null;
  date_published?: string | null;
  size_bought?: string | null;
  fit?: FitVote | null;
}

/** `get_return_policy` (catalog.md). The spec writes the extension keys with their `acom:` prefix. */
export interface ReturnPolicy {
  product_id: string;
  merchant_return_days: number;
  return_fees: string;
  'acom:non_returnable_when'?: string[];
  non_returnable_when?: string[];
  'acom:size_exchange_days'?: number | null;
  size_exchange_days?: number | null;
}

/** One `get_delivery_dates` item (fulfillment.md). */
export interface DeliveryDate {
  product_id: string;
  variant_id?: string | null;
  method: string;
  ships_on: string;
  earliest_arrival: string;
  shipping: Money;
  shipping_note?: string | null;
  remote_area_surcharge?: Money | null;
  by?: string | null;
  arrives_by_requested_date?: boolean | null;
}

/** One `get_facets` item (search.md). */
export interface Facet {
  name: 'category' | 'brand' | 'price_range' | 'color' | 'size' | 'availability' | string;
  value: string;
  count: number;
  label?: string;
}

/** One `autocomplete` item (search.md). */
export interface Suggestion {
  text: string;
  suggest_type: 'keyword' | 'product' | 'brand' | 'category' | string;
  score?: number;
  id?: string;
}

/** `compare_products` (search.md). */
export interface CompareValue {
  attribute: string;
  value: unknown;
  unit: string | null;
  display: string;
}
export interface CompareItem {
  product_id: string;
  name: string;
  color?: string | null;
  material?: string | null;
  price: number;
  member_price: number | null;
  price_currency: string;
  values: CompareValue[];
}

/** `list_categories` (merchandising.md). */
export interface Category {
  id: string;
  name: string;
  term_code?: string;
  term_set?: 'standard' | 'display';
  parent_id?: string | null;
  display_order?: number;
  active?: boolean;
}

/** `get_category` (merchandising.md). */
export interface CategoryDetail extends Category {
  mappings?: {id: string; display_category_id?: string; is_primary?: boolean}[];
  product_count?: number;
}

/** A booking on a display slot (merchandising.md). */
export interface DisplayBooking {
  display_booking_id: string;
  audience: string;
  status?: 'draft' | 'scheduled' | 'active' | 'ended' | string;
  valid_from?: string | null;
  valid_through?: string | null;
  banner_id?: string | null;
  eyebrow?: string | null;
  headline?: string | null;
  alternative_headline?: string | null;
  cta_label?: string | null;
  url?: string | null;
  content?: BannerContent | null;
  /** The banner this booking shows (`get_display_slot`), with its image. */
  banner?: (BannerContent & {id: string}) | null;
}

export interface BannerContent {
  eyebrow?: string | null;
  headline?: string | null;
  alternative_headline?: string | null;
  cta_label?: string | null;
  url?: string | null;
  image_source?: 'product' | 'image' | string;
  image_product_id?: string | null;
  image_color?: string | null;
  image_id?: string | null;
  content_url?: string | null;
}

/** One `list_display_slots` item. */
export interface DisplaySlotSummary {
  id: string;
  page: string;
  name?: string;
  slot_type: 'hero' | 'banner_board' | 'product_grid' | 'brand_strip' | 'collection_feed' | string;
  position?: number;
  bookings?: DisplayBooking[];
}

/** An entry of a slot's `item_list_element`: an id, or `{position, item_id}`. */
export type SlotItem = string | {position?: number; item_id?: string; product_id?: string; id?: string};

/** `get_display_slot`. */
export interface DisplaySlot {
  id: string;
  name?: string;
  slot_type: DisplaySlotSummary['slot_type'];
  item_list_element: SlotItem[];
  bookings: DisplayBooking[];
  live_now?: boolean;
  hidden_items?: {product_id: string; reason: string}[];
}

/** One `list_banners` item. */
export interface Banner extends BannerContent {
  id: string;
  name?: string | null;
  channels?: string[];
  priority?: number;
  bookings?: {display_booking_id: string; audience: string; valid_from?: string | null; valid_through?: string | null}[];
  image_product_group_id?: string | null;
}

/** One `list_brands` item. */
export interface Brand {
  id: string;
  name: string;
}

// ── Sales ──────────────────────────────────────────────────────────────────────

export interface CartLine {
  cart_line_id: string;
  product_id: string;
  variant_id: string;
  name: string;
  color?: string | null;
  size?: string | null;
  quantity: number;
  price: Money;
  note?: string | null;
  image?: ImageRef;
}

export interface Discount {
  label: string;
  amount: Money;
  promotion_id?: string | null;
  discount_layer?: 'item_price' | 'order_discount' | 'add_on' | string;
}

export interface RecordedCoupon {
  coupon_id: string;
  code?: string | null;
  promotion_id?: string | null;
}

export interface DeliverySummary {
  address_id?: string | null;
  address_label?: string | null;
  arrives?: string | null;
  address_saved?: boolean;
}

export interface PaymentSummary {
  payment_method?: string | null;
  label?: string | null;
}

/** `get_cart` (sales.md). */
export interface Cart {
  id: string | null;
  customer_id?: string | null;
  lines: CartLine[];
  subtotal: Money;
  discounts: Discount[];
  shipping: Money;
  total: Money;
  delivery?: DeliverySummary | null;
  payment?: PaymentSummary | null;
  coupons_recorded?: RecordedCoupon[];
}

/** A cart write run (`add_to_cart`, `update_cart_item`, `remove_cart_item`, `clear_cart`). */
export interface CartWriteResult {
  ok: boolean;
  id: string;
  cart?: Cart;
  cart_line_id?: string;
  variant_id?: string;
  undo: null;
}

/** One `get_payment_methods` item. */
export interface PaymentMethod {
  payment_method: string;
  label: string;
  saved?: boolean;
  default?: boolean;
  installment_months?: number[];
  loyalty_usable?: boolean;
}

/** `changes[0].to` of a `prepare_order` preview: what the shopper will pay. */
export interface OrderDraft {
  lines: Array<Omit<CartLine, 'product_id' | 'cart_line_id'> & {cart_line_id?: string; product_id?: string}>;
  subtotal: Money;
  discounts: Discount[];
  shipping: Money;
  total: Money;
  delivery?: DeliverySummary | null;
  payment?: PaymentSummary | null;
  coupons_recorded?: RecordedCoupon[];
  gift_wrap?: Money | null;
}

/** `changes[0].to` of a `prepare_gift_order` preview (docs/api/sales.md §gift_order). */
export interface GiftDraft extends Omit<OrderDraft, 'delivery'> {
  order_type?: 'gift';
  /** The cart's total before the gift's own charges. */
  goods_total?: Money;
  delivery?: (DeliverySummary & {to?: string | null; recipient_contact?: string | null}) | null;
  gift_options?: {
    wrap?: boolean;
    wrap_fee?: Money | null;
    card_message?: string | null;
    gift_receipt?: boolean;
    receipt_hides_prices?: boolean;
    exchange_until?: string | null;
  };
  recipient?: {name: string; mode: 'address_known' | 'recipient_accepts'};
}

/** `POST /v1/sales/orders` (person-only). */
export interface PlacedOrder {
  ok: boolean;
  id: string;
  order_number: string;
  order_status: OrderStatus;
  undo: null;
  /** A gift its recipient accepts by link: where they accept it. */
  gift_order_id?: string;
  accept_url?: string;
}

/** One way the recipient may take a gift: same product, same price (get_gift_order). */
export interface GiftOption {
  size: string | null;
  color: string | null;
  available: boolean;
}

export interface GiftViewItem {
  name: string;
  image: string | null;
  size: string | null;
  color: string | null;
  quantity: number;
  /** Empty when the recipient can only take it as chosen. */
  options: GiftOption[];
}

/** get_gift_order: a gift as its recipient opens it from the link. It carries no prices. */
export interface GiftView {
  id: string;
  /** `potential` waits for the recipient, `completed` was accepted, `failed` lapsed. */
  action_status: 'potential' | 'active' | 'completed' | 'failed';
  expires: string;
  recipient_name: string;
  /** The sender's first name. */
  from: string | null;
  card_message: string | null;
  gift_wrap: boolean;
  exchange_until: string | null;
  items: GiftViewItem[];
  /** What the recipient may change (`gift.recipient_can_change`): acceptance, address, options. */
  can_change: string[];
  /** Countries the store ships to. */
  ships_to: string[];
}

export type OrderStatus = 'payment_due' | 'processing' | 'in_transit' | 'delivered' | 'returned' | 'cancelled' | 'problem';

export interface OrderItem {
  id: string;
  product_id: string;
  variant_id: string;
  name: string;
  size?: string | null;
  color?: string | null;
  order_quantity: number;
  price?: Money;
  discount?: Money;
  order_item_status: OrderStatus;
}

/** One `get_my_orders` item. */
export interface OrderSummary {
  id: string;
  order_number: string;
  order_status: OrderStatus;
  order_date: string;
  ordered_item: OrderItem[];
  delivered_at: string | null;
  total: Money;
}

export interface PostalAddress {
  name?: string | null;
  street_address: string;
  address_locality: string;
  address_region?: string | null;
  postal_code?: string | null;
  address_country: string;
}

/** `get_order`. */
export interface Order {
  id: string;
  order_number: string;
  order_status: OrderStatus;
  order_date: string;
  customer_id?: string;
  ordered_item: OrderItem[];
  delivery?: {
    id: string;
    delivery_address?: PostalAddress | null;
    expected_arrival_until?: string | null;
    delivery_state?: string | null;
    delivered_at?: string | null;
  } | null;
  payment?: {payment_method?: string; card_last4?: string | null; payment_status?: string} | null;
  totals: {subtotal: Money; discounts: Discount[]; shipping: Money; gift_wrap?: Money | null; total: Money};
  coupons_recorded?: RecordedCoupon[];
  /** Cancel, return and exchange requests on the order, oldest first. */
  claims?: OrderClaim[];
  /** Who a gift is for and how it reaches them; `null` for other orders. */
  gift?: OrderGift | null;
}

/** `get_order` `gift`. */
export interface OrderGift {
  recipient_name: string | null;
  recipient_mode: 'address_known' | 'recipient_accepts' | string | null;
  card_message: string | null;
  gift_receipt: boolean | null;
  exchange_until: string | null;
  /** A gift the recipient accepts by link: the link and where it stands. */
  acceptance: {gift_order_id: string; accept_url: string; action_status: string; expires: string} | null;
}

/** One request on an order (`get_order` `claims[]`). */
export interface OrderClaim {
  id: string;
  claim_type: 'cancel' | 'return' | 'exchange' | string;
  claim_status: string;
  order_item_ids: string[];
  requested_at: string;
}

// ── Fulfillment ────────────────────────────────────────────────────────────────

/** `get_delivery`. */
export interface Delivery {
  id: string;
  order_id: string;
  delivery_address?: string | PostalAddress | null;
  tracking_number?: string | null;
  carrier?: string | null;
  delivery_state: string;
  expected_arrival_until?: string | null;
  events?: {delivery_state: string; at: string}[];
}

/** One `track_shipment` item. */
export interface TrackingEvent {
  at: string;
  delivery_state: string;
  location?: string | null;
}

export interface CancelItem {
  order_item_id: string;
  name: string;
  ordered: number;
  cancellable_quantity: number;
  order_item_status: string;
  fulfillment_stage: string | null;
  path: 'self_service' | 'store_review' | 'not_allowed';
  unit_paid: Money;
  whole_quantity_only: boolean;
}

export interface CancelEstimate {
  scope: 'full' | 'partial';
  cancelled_amount: Money;
  extra_charge: {kind: string; amount: Money; why?: string}[];
  discount_recalculated: {label?: string; amount?: Money}[];
  refund_total: Money;
  refund_breakdown: {to: string; method?: string; amount: Money}[];
  money_back_business_days?: {min: number; max: number} | null;
  coupons_restored: {coupon_id: string; amount?: Money}[];
  points_pending_cancelled?: Money | null;
  charge_due: Money;
  needs_refund_destination: boolean;
  confirmed_by: 'store_automatic' | 'store_person' | string;
  revocable: boolean;
}

/** `get_cancel_options`. */
export interface CancelOptions {
  items: CancelItem[];
  reasons: {reason: string; liability: string; needs_store_review?: boolean}[];
  estimate: CancelEstimate;
  not_cancellable: {order_item_id: string; why: string; next_tool?: string | null}[];
  total: number;
}

export interface PickupWindow {
  start: string;
  end: string;
}

/** One `get_return_options` item. */
export interface ReturnOption {
  kind: 'replacement' | 'refund';
  title: string;
  variant_id?: string;
  available_quantity?: number;
  ships_on?: string | null;
  arrives_by?: string | null;
  refund_amount?: Money;
  refund_to?: string | null;
  money_back_business_days?: {min: number; max: number} | null;
  you_pay: Money;
  charge_due?: Money | null;
  confirmed_by: 'store_automatic' | 'store_person' | string;
  send_back: {
    methods: ('pickup' | 'drop_off')[];
    return_fees?: string;
    cost?: Money;
    pickup_window?: PickupWindow | null;
  };
  facts: string[];
}

/** Response of a claim sent by the shopper (cancel or return). */
export interface ClaimResult {
  ok: boolean;
  id: string;
  claim_type: 'cancel' | 'return' | 'exchange';
  status: string;
  confirmed_by?: string;
  refund?: {total: Money; breakdown: {to: string; amount: Money}[]};
  refund_estimate?: {total: Money; breakdown: {to: string; amount: Money}[]};
}

/** One `get_return_history` item (customer.md). */
/** One of the shopper's own requests (`search_claims`, which a shopper only sees for themselves). */
export interface ClaimSummary {
  id: string;
  claim_type: string;
  status: string;
  order_id: string;
  reason: string | null;
  amount: Money;
  created_at: string;
}

export interface ReturnHistoryItem {
  id: string;
  claim_type: string;
  order_id: string;
  order_number: string;
  item: string;
  amount: Money;
  reason: string;
  status: string;
  requested_at: string;
}

// ── Customer and promotion ─────────────────────────────────────────────────────

export interface ProfilePreference {
  memory_id: string;
  key: string;
  value: string;
  text: string;
  source: string;
}

export interface Address extends PostalAddress {
  id: string;
  label?: string | null;
  usage?: string[];
  is_primary: boolean;
}

/** `get_my_profile`. */
export interface Profile {
  id: string;
  name: string;
  tier_id: string | null;
  sizes?: {tops?: string | null; bottoms?: string | null; shoes?: string | null};
  preferences?: ProfilePreference[];
  addresses?: Address[];
  card?: {label: string; last4: string} | null;
}

/** One `get_preferences` item. */
export interface Preference {
  id: string;
  key: string;
  value: string;
  text: string;
  source: string;
  created_at?: string;
}

/** One `list_wishlist` item. */
export interface WishlistItem {
  product_id: string;
  name: string;
  price: number;
  price_currency: string;
  availability?: Availability;
  added_at?: string;
}

/** One `list_member_coupons` item. */
export interface MemberCoupon {
  id: string;
  coupon_id: string;
  code?: string | null;
  name?: string | null;
  status: 'issued' | 'available' | 'used' | 'restored' | 'expired' | 'revoked' | string;
  order_id?: string | null;
  valid_through?: string | null;
}

/** `get_loyalty_balance`. */
export interface LoyaltyBalance {
  customer_id: string;
  balance: Money;
  expiring_soon: {amount: Money; expires: string}[];
}

/** `list_notifications`: a message the store sent the shopper (email, push, in-app). */
export interface StoreMessage {
  id: string;
  channel: string;
  headline: string;
  text: string;
  /** What it is about: an order, a return request, a campaign. */
  about: string | null;
  date_sent: string;
  read?: boolean;
}
