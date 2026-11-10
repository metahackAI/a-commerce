// Response types the back office reads, hand-written from the specs in docs/api/*.md. Nothing
// generates them; keep them equal to the specs when a response changes.
// Fields the spec marks optional or that a tool may leave out are optional, so a page renders
// whatever the API returns without guessing.

import type {Page, Preview} from '@/lib/api';

export type {Page, Preview};

/** schema.org MonetaryAmount (docs/api/conventions.md §3). */
export interface Money {
  value: number;
  currency: string;
}

/** One rule check of a preview (docs/api/README.md §3.3). */
export interface Check {
  rule: string;
  passed: boolean;
  detail: string | null;
  fix_tool: string | null;
}

/** One change of a preview: `op` on `entity` `id`, `from` → `to`. */
export interface Change {
  op: string;
  entity: string;
  id: string | null;
  from?: unknown;
  to?: unknown;
  note?: string;
}

// ─── platform ───────────────────────────────────────────────────────────────

export type ChangeSetState =
  | 'drafting'
  | 'waiting_for_approval'
  | 'running'
  | 'verifying'
  | 'live'
  | 'failed'
  | 'rejected'
  | 'undone';

export interface Interpretation {
  slot: 'who' | 'products' | 'benefit' | 'when' | 'where' | 'other' | string;
  text: string;
  value?: unknown;
}

export interface ChangeSetItem {
  position: number;
  preview_id: string;
  tool: string;
  address?: string;
  depends_on?: string[];
  action_status?: string;
  needs_approval: boolean;
  approval_reason: string | null;
  changes: Change[];
  changes_total?: number;
  checks: Check[];
  /** The tool input the preview was made from. */
  input?: Record<string, unknown>;
}

export interface LeftOut {
  entity: string;
  id: string;
  name?: string;
  rule?: string;
  reason?: string;
}

export interface Agent {
  '@type'?: string;
  name?: string;
  id?: string;
}

/** `get_change_set` (docs/api/platform.md). */
export interface ChangeSet {
  id: string;
  number: number;
  version: number;
  state: ChangeSetState;
  title: string;
  request?: {text: string[]; conversation_id?: string | null; requested_by?: string | null};
  interpretation?: Interpretation[];
  items?: ChangeSetItem[];
  left_out?: LeftOut[];
  checks_summary?: {passed: number; failed: number; total: number};
  needs_approval?: boolean;
  undo_until: string | null;
  created_by?: string;
  agent?: Agent | null;
  created_at?: string;
  submitted_at?: string | null;
  approved_by?: string | null;
  approved_at?: string | null;
  verification?: {state: string; matched: number; total: number} | null;
  /** State changes, oldest first, with who made each (`agent_id`, `agent_name`). */
  history?: {agent_id?: string | null; agent_name?: string | null; to?: string | null; time?: string | null}[];
}

/** One row of `search_change_sets`. */
export interface ChangeSetSummary {
  id: string;
  number: number;
  title: string;
  state: ChangeSetState;
  version: number;
  item_count: number;
  needs_approval: boolean;
  created_by: string;
  created_at: string;
  undo_until: string | null;
}

export interface VerificationItem {
  preview_id: string;
  tool: string;
  result_id: string | null;
  matched: boolean;
  checked: number;
  mismatches: {entity: string; id: string; field: string; expected: unknown; actual: unknown}[];
}

/** `verify_change_set`. */
export interface Verification {
  items: VerificationItem[];
  total: number;
  change_set_id: string;
  version: number;
  state: string;
  verified_at: string;
}

export type TaskState = 'needs_you' | 'working' | 'done' | 'dismissed';
export type TaskOrigin = 'you_asked' | 'agent_noticed' | 'automation';

/** One row of `list_tasks`. */
export interface TaskSummary {
  id: string;
  title: string;
  state: TaskState;
  origin: TaskOrigin;
  at: string;
  summary: string | null;
  tag: string | null;
  change_set_id: string | null;
  watcher: string | null;
  conversation_id?: string | null;
  /** When it last changed, including a watcher writing what it observed again. */
  updated_at?: string;
}

/** `get_task`. */
export interface Task {
  id: string;
  title: string;
  state: TaskState;
  origin: TaskOrigin;
  tag: string | null;
  summary: string | null;
  request: string | null;
  watcher: {
    key: string;
    threshold?: {policy_key: string; value: unknown};
    observed?: Record<string, unknown>;
  } | null;
  related: {
    change_set_id: string | null;
    conversation_id: string | null;
    entities: {entity: string; id: string}[];
  } | null;
  history: {at: string; state: string; note?: string | null}[];
  created_at: string;
  updated_at: string;
}

/** A schema.org Action from the action log. */
export interface ActionEntry {
  '@type'?: string;
  id: string;
  action_status: 'potential' | 'active' | 'completed' | 'failed' | string;
  agent?: Agent | null;
  instrument?: string | null;
  object?: ({'@type'?: string; id?: string} & Record<string, unknown>) | null;
  result?: Record<string, unknown> | null;
  error?: unknown;
  start_time?: string | null;
  end_time?: string | null;
  'acom:change_set_id'?: string | null;
  'acom:preview_id'?: string | null;
  'acom:request_id'?: string | null;
  'acom:tokens'?: {input?: number; output?: number} | null;
  'acom:latency_ms'?: number | null;
  'acom:policy_version'?: string | null;
  'acom:approved_by'?: string | null;
  [key: string]: unknown;
}

/** One value of `get_policy`. */
export interface PolicyValue {
  key: string;
  value: unknown;
  display: string;
  file: string;
  overridden_by_store: boolean;
}

/** One file of `list_policies`. */
export interface PolicyFile {
  file: string;
  version: number | string;
  domain: string;
  summary: string;
  overridden_keys: number;
}

export interface PolicyIndex extends Page<PolicyFile> {
  policy_version?: string;
}

/** One table of `describe_data`. */
export interface DataTable {
  table: string;
  term: string;
  id_prefix?: string | null;
  api_name?: string | null;
  description: string;
  columns: {
    name: string;
    type: string;
    nullable: boolean;
    term: string;
    description: string;
    values?: string[] | null;
    references?: string | null;
    api_name?: string | null;
  }[];
}

// ─── catalog ────────────────────────────────────────────────────────────────

export interface Variant {
  variant_id: string;
  sku?: string;
  size?: string | null;
  color?: string | null;
  inventory_level?: number;
  availability?: string;
}

/** Where a draft made from a supplier sheet came from, and what is still open on it (`acom:draft`). */
export interface DraftInfo {
  import_id: string;
  row: number;
  supplier_code: string;
  draft_status: 'ready' | 'needs_you' | 'working' | string;
  open_fields: string[];
  note: string | null;
  supplier_text: string | null;
}

/** One row of `search_product_groups` (a `ProductGroup`). */
export interface ProductGroupSummary {
  id: string;
  name: string;
  brand_id?: string | null;
  category_id?: string | null;
  approval_status?: string;
  availability?: string;
  price?: number | null;
  price_currency?: string | null;
  cost?: number | null;
  inventory_level?: number | null;
  varies_by?: string[];
  variant_count?: number;
  image?: string | null;
  'acom:draft'?: DraftInfo;
}

/** A variant as `get_product_group` lists it (operators also see cost and scheduled prices). */
export interface GroupVariant extends Variant {
  name?: string;
  offer_id?: string | null;
  price?: number | null;
  price_currency?: string | null;
  cost?: number | null;
  'acom:safety_stock'?: number | null;
  'acom:managed'?: boolean;
  'acom:sale_type'?: string;
  scheduled_prices?: {unit_price_specification_id: string; price: number; valid_from: string; valid_through: string | null}[];
}

/** `get_product_group` (catalog view). */
export interface ProductGroup extends Omit<ProductGroupSummary, 'availability'> {
  description?: string | null;
  material?: string | null;
  release_date?: string | null;
  has_variant?: GroupVariant[];
  pending_changes?: unknown[];
}

/** `list_brands` / `get_brand`. */
export interface Brand {
  id: string;
  name: string;
  logo?: string | null;
  description?: string | null;
}

/** `get_product` (sales view; `cost` only for operators). */
export interface Product {
  id: string;
  name: string;
  brand_id?: string | null;
  category_id?: string | null;
  approval_status?: string;
  varies_by?: string[];
  price: number;
  price_currency: string;
  member_price: {
    price: number;
    name?: string;
    valid_for_member_tier?: string;
    promotion_id?: string;
    valid_through?: string | null;
  } | null;
  cost?: number | null;
  inventory_level?: number;
  availability?: string;
  additional_property?: {name: string; value: unknown; unit_code?: string; size?: string}[];
  has_variant?: Variant[];
}

/** `get_inventory`. */
export interface Inventory {
  product_id: string;
  inventory_level: number;
  /** `on_order`: units in reorders not received yet; not sellable until they are. */
  variants: (Variant & {'acom:safety_stock'?: number; 'acom:managed'?: boolean; on_order?: number})[];
  /** Reorders not received yet, soonest first. */
  reorders?: {id: string; order_status: string; total_units: number; expected_arrival_until: string}[];
}

/** `get_price_breakdown`. */
export interface PriceBreakdown {
  variant_id: string;
  list_price: number;
  member_price: number | null;
  price_currency: string;
  applied: {promotion_id: string; name: string; valid_for_member_tier?: string}[];
  cost?: number | null;
  price_floor?: number | null;
}

/** `get_review_summary`. */
export interface ReviewSummary {
  product_id: string;
  review_count: number;
  rating_value: number | null;
  fit_votes: {fit: string; label: string; count: number}[];
  mentions: {text: string; count: number}[];
  sample: {
    review_id: string;
    review_rating: number;
    size_bought?: string | null;
    height_cm?: number | null;
    excerpt: string;
    date_published: string;
  }[];
}

/** One entry of `get_change_history`. */
export interface HistoryEntry {
  action_id: string;
  agent?: Agent | string | null;
  change_type: 'content' | 'structural' | string;
  from?: unknown;
  to?: unknown;
  end_time: string;
}

// ─── promotion ──────────────────────────────────────────────────────────────

export interface AppliesTo {
  scope: 'products' | 'categories' | 'order' | string;
  category_ids?: string[];
  product_count?: number | null;
}

/** One row of `search_promotions` / `list_active_promotions`. */
export interface PromotionSummary {
  id: string;
  name: string;
  status: string;
  discount_layer: string;
  valid_for_member_tier: string;
  valid_from: string;
  valid_through: string | null;
  applies_to?: AppliesTo;
}

/** `get_promotion`. */
export interface Promotion extends PromotionSummary {
  price_specification_count?: number;
  coupon_ids?: string[];
  change_set_id?: string | null;
}

/** One row of `search_coupons`. */
export interface CouponSummary {
  id: string;
  code: string;
  promotion_id: string | null;
  valid_for_member_tier: string;
  application: string;
  stacking: string;
  valid_from: string | null;
  valid_through: string | null;
}

/** `get_coupon`. */
export interface Coupon extends CouponSummary {
  records_benefit_only?: boolean;
  counts?: Record<string, number>;
}

// ─── merchandising ──────────────────────────────────────────────────────────

export interface Booking {
  display_booking_id: string;
  audience: string;
  headline?: string | null;
  valid_from: string;
  valid_through: string | null;
  status?: string;
}

/** One row of `list_display_slots`. */
export interface DisplaySlotSummary {
  id: string;
  page: string;
  name: string;
  slot_type: string;
  bookings: Booking[];
  availability: {audience: string; free_from: string; free_until: string}[];
}

/** `get_display_slot`. */
export interface DisplaySlot {
  id: string;
  name?: string;
  page?: string;
  slot_type: string;
  item_list_element?: unknown[];
  bookings?: Booking[];
  live_now?: boolean;
  hidden_items?: {product_id: string; reason: string}[];
}

/** One row of `list_banners`. */
export interface Banner {
  id: string;
  content_url?: string | null;
  url?: string | null;
  channels?: string[];
  priority?: number;
  bookings?: Booking[];
}

/** One row of `list_categories`. */
export interface Category {
  id: string;
  name: string;
  term_code?: string;
  term_set: string;
  parent_id: string | null;
  display_order?: number;
  active: boolean;
}

/** A standard → display category mapping (`get_category` `mappings`). */
export interface CategoryMapping {
  id: string;
  standard_category_id: string;
  display_category_id: string;
  is_primary: boolean;
}

/** `get_category`. */
export interface CategoryDetail extends Category {
  description?: string | null;
  ancestor_ids?: string[];
  mappings: CategoryMapping[];
  product_count: number;
}

// ─── sales ──────────────────────────────────────────────────────────────────

/** One row of `search_orders`. */
export interface OrderSummary {
  id: string;
  order_number: string;
  order_status: string;
  customer_id: string;
  order_date: string;
  total: Money;
  /** What was ordered (search_orders `ordered_item`): each line's product and its picture. */
  ordered_item?: {id: string; product_id: string; variant_id: string; name: string; size?: string | null; color?: string | null; image?: string | null; order_quantity: number}[];
}

export interface PostalAddress {
  name?: string;
  street_address?: string;
  address_locality?: string;
  address_region?: string;
  postal_code?: string;
  address_country?: string;
}

/** `get_order`. */
export interface Order {
  id: string;
  order_number: string;
  order_status: string;
  order_date: string;
  customer_id: string;
  ordered_item: {
    id: string;
    variant_id: string;
    product_id: string;
    name: string;
    size?: string | null;
    color?: string | null;
    /** The ordered color's picture. */
    image?: string | null;
    order_quantity: number;
    price: Money;
    discount?: Money | null;
    order_item_status: string;
  }[];
  delivery: {
    id: string;
    delivery_address?: PostalAddress | string | null;
    expected_arrival_until?: string | null;
    delivery_state: string;
    delivered_at?: string | null;
  } | null;
  payment: {payment_method: string; card_last4?: string | null; payment_status: string} | null;
  totals: {
    subtotal: Money;
    discounts: {label?: string; amount?: Money; value?: number; currency?: string}[];
    shipping: Money;
    total: Money;
  };
  coupons_recorded: {coupon_id: string; code: string; promotion_id: string}[];
}

/** `get_payment`. */
export interface Payment {
  id: string;
  order_id: string;
  payment_status: string;
  payment_method: string;
  label: string;
  amount: Money;
  installment_months?: number;
}

// ─── fulfillment ────────────────────────────────────────────────────────────

/** `get_delivery`. */
export interface Delivery {
  id: string;
  order_id: string;
  delivery_address?: PostalAddress | string | null;
  tracking_number?: string | null;
  carrier?: string | null;
  delivery_state: string;
  expected_arrival_until?: string | null;
  events?: {delivery_state: string; at: string}[];
}

/** One event of `track_shipment`. */
export interface TrackingEvent {
  at: string;
  delivery_state: string;
  location?: string | null;
}

/** One row of `search_return_requests`. */
export interface ReturnRequest {
  claim_id: string;
  order_id: string;
  order_number: string;
  customer_id: string;
  order_item_id: string;
  item: string;
  quantity: number;
  amount: Money;
  reason: string;
  reason_text?: string | null;
  photos: number;
  delivered_at?: string | null;
  days_since_delivery?: number | null;
  requested_at: string;
  status: string;
}

/** One item of `evaluate_return_request`. */
export interface ReturnEvaluation {
  claim_id: string;
  order_number: string;
  suggested: 'refund' | 'ask_for_photos' | 'decline' | string;
  rule: string;
  rule_text: string;
  liability: string;
  refund_amount: Money | null;
  return_fees: string | null;
  needs_person: boolean;
  flags: {rule: string; detail: string}[];
}

/** `get_claim`. */
export interface Claim {
  id: string;
  claim_type: string;
  status: string;
  order_id: string;
  items: {order_item_id: string; quantity: number}[];
  reason: string;
  liability?: string | null;
  evidence?: {id: string; evidence_type: string}[];
  inspection?: unknown;
  refund_id: string | null;
  status_history?: {status: string; at: string}[];
}

/** `get_refund`. */
export interface Refund {
  id: string;
  claim_id: string;
  amount: Money;
  refund_method: string;
  to: string;
  refund_status: string;
  events?: {refund_status: string; at: string}[];
}

// ─── customer ───────────────────────────────────────────────────────────────

/** One row of `search_members`. */
export interface MemberSummary {
  id: string;
  name: string;
  email?: string | null;
  tier_id?: string | null;
  state: string;
}

/** `get_member`. */
export interface Member {
  id: string;
  name: string;
  type?: string;
  member_of?: {program_id: string; tier_id: string; membership_number?: string} | null;
  state: string;
  'acom:business_verification'?: string | null;
  consent?: Record<string, boolean>;
  email?: string | null;
}

/** `get_member_tier`. */
export interface MemberTier {
  customer_id: string;
  program_id: string;
  tier_id: string;
  rank: number;
  since: string;
  source: string;
  override: Record<string, unknown> | null;
  why: {rule: string; value: unknown; evaluation_id: string | null};
  progress: {
    to_tier_id: string;
    metric: string;
    current: Money | number;
    threshold: Money | number;
    remaining: Money | number;
    pending?: (Money & {detail?: string}) | null;
  }[];
  retention: {status: string; retain_threshold: unknown};
  next_evaluation: string | null;
  has_tier_benefit?: {type: string; coupon_id?: string; every?: string}[];
}

/** One row of `list_tiers`. */
export interface Tier {
  id: string;
  name: string;
  rank: number;
  program_id: string;
  member_count: number;
}

/** One claim of `get_return_history`. */
export interface ReturnHistoryEntry {
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

export interface ReturnHistory extends Page<ReturnHistoryEntry> {
  summary: {count: number; days: number; threshold: number; flagged: boolean};
}

/** The run result of a previewed write. */
export interface CommitResult {
  ok: boolean;
  id: string;
  undo?: {by: 'person'; target: string; until: string | null} | null;
}

/** `explain_sales_change` (docs/api/analytics.md): a period against a basis, split into causes. */
export interface SalesExplanation {
  items: {
    cause: string;
    detail: string;
    amount: Money | number;
    evidence: string[];
    expected?: Money | number;
    actual?: Money | number;
    since?: string | null;
    products?: {product_id: string; name: string | null; sizes: string[]; availability_starts: string | null}[];
    campaign?: {campaign_id: string; name: string; scheduled_time: string | null; error_message: string | null};
  }[];
  total: number;
  'acom:metric': string;
  period: {valid_from: string; valid_through: string; revenue: Money; order_count: number};
  compare_to: {basis: string; observation_dates: string[]; revenue: Money; order_count: number};
  gap: Money | number;
  change_rate: number | null;
  unexplained: Money | number;
}

/** One point of `get_sales_trend`. */
export interface SalesTrendPoint {
  observation_date: string;
  'acom:metric': 'revenue' | 'order_count' | 'average_order_value';
  value: Money | number | null;
  'acom:dimension'?: Record<string, string | null>;
}

/** `GET /v1/platform/store`: the store's currency, zone and clock (the demo clock in demo mode). */
export interface StoreClock {
  store_id: string;
  currency: string;
  time_zone: string;
  now: string;
}

/** `list_notifications`: a message the store sent one member (email, push, in-app). */
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

// Ailism Bot (docs/api/platform.md#bot).

export type BotMode = 'off' | 'observe' | 'propose';

export const BOT_AREAS = [
  'pricing',
  'inventory',
  'promotions',
  'merchandising',
  'catalog_content',
  'returns',
  'questions_reviews',
  'community',
] as const;

export interface BotSettings {
  mode: BotMode;
  acts_as: string | null;
  model: string;
  every_minutes: number;
  areas: Record<string, boolean>;
  max_proposals_per_cycle: number;
  max_open_decisions: number;
  cooldown_days: number;
  directives: string[];
}

export interface BotStatus {
  settings: BotSettings;
  /** The operator the drafts are made for (its own setting, else the task pickup's). */
  acts_as: string | null;
  next_run_at: string | null;
  last_cycle: {id: string; state: string; trigger: string; at: string; summary: string | null; error: string | null} | null;
  /** Decisions by state (`bot_decision_state`). */
  decisions: Record<string, number>;
  open_feedback: number;
}

export interface BotCycleSummary {
  id: string;
  trigger: string;
  state: string;
  mode: string;
  at: string;
  end_time: string | null;
  summary: string | null;
  proposals: number;
  opened: number;
  anomalies: number;
  error: string | null;
}

export interface BotEffect {
  metric?: string;
  direction?: 'up' | 'down';
  horizon_days?: number;
}

export interface BotProposal {
  area: string;
  title: string;
  request: string;
  reason?: string;
  evidence?: string[];
  expected_effect?: BotEffect;
  risk?: string;
  about?: {entity: string; id: string}[];
  outcome?: {opened?: string; skipped?: string};
}

export interface BotAnomaly {
  area: string;
  code: string;
  text: string;
  severity: string;
}

export interface BotCycle {
  id: string;
  trigger: string;
  requested_by: string | null;
  state: string;
  mode: string;
  at: string;
  end_time: string | null;
  snapshot: {at?: string; areas?: string[]; reads?: {area: string; tool: string; args?: unknown; result?: unknown; error?: string}[]};
  anomalies: BotAnomaly[];
  report: {summary?: string; findings?: {area: string; text: string; severity: string}[]; proposals?: BotProposal[]} | null;
  usage: {model?: string; model_calls?: number; input_tokens?: number; output_tokens?: number; reads?: number};
  decisions: string[];
  error: string | null;
}

export interface BotFeedback {
  id: string;
  kind: 'feedback' | 'directive' | 'correction';
  text: string;
  decision_id: string | null;
  state: 'open' | 'closed';
  by: string;
  at: string;
  closed_at: string | null;
}

export interface BotDecision {
  id: string;
  cycle_id: string;
  task_id: string;
  change_set_id: string | null;
  change_set_number: number | null;
  conversation_id: string | null;
  area: string;
  title: string;
  request: string;
  reason: string;
  evidence: string[];
  expected_effect: BotEffect;
  risk: string;
  about: {entity: string; id: string}[];
  revises: string | null;
  state: string;
  summary: string | null;
  /** A count in lists; the feedback itself in `get_bot_decision`. */
  feedback: number | BotFeedback[];
  at: string;
  updated_at: string;
}

// ---- Ailism Customer Bot (docs/api/platform.md#customer_bot) --------------------------------

export type CustomerBotMode = 'on' | 'off';

export interface CustomerBotSettings {
  mode: CustomerBotMode;
  every_minutes: number;
  workers: number;
  new_account_percent: number;
  max_actions: number;
  reviews_per_session: number;
  model: string;
  never_shop_as: string[];
}

/** What a run's sessions did together. */
export interface CustomerBotSummary {
  sessions?: number;
  done?: number;
  failed?: number;
  accounts_created?: number;
  personas_built?: number;
  chat_turns?: number;
  orders?: number;
  reviews?: number;
  memories?: number;
  cart_adds?: number;
  spent?: Money;
}

export interface CustomerBotRun {
  id: string;
  trigger: 'schedule' | 'manual';
  requested_by: string | null;
  state: 'queued' | 'running' | 'done' | 'failed';
  workers: number;
  /** Store time (the demo clock). */
  at: string;
  end_time: string | null;
  /** Real time the run started and ended. */
  started: string;
  finished: string | null;
  sessions: number;
  summary: CustomerBotSummary;
  error: string | null;
}

/** What one session did. */
export interface CustomerBotOutcome {
  chat_turns?: number;
  orders?: number;
  order_ids?: {order_id: string; total: string}[];
  spent?: {value: number | string | null; currency: string};
  reviews?: number;
  memories?: number;
  cart_adds?: number;
  tools?: string[];
  fallbacks?: number;
}

export interface CustomerBotSession {
  id: string;
  run_id: string;
  customer_id: string | null;
  account: 'existing' | 'created';
  state: 'running' | 'done' | 'failed';
  persona_built: boolean;
  /** The persona's one line. */
  persona: string | null;
  goal: string | null;
  outcome: CustomerBotOutcome;
  conversation_ids: string[];
  error: string | null;
  steps: number;
  at: string;
  end_time: string | null;
  started: string;
  finished: string | null;
}

export interface ShopperPersonaBody {
  age_range?: string;
  location?: string;
  occupation?: string;
  style?: string[];
  colors?: string[];
  sizes?: {top?: string; bottom?: string; shoes?: string};
  budget_per_order?: number;
  categories?: string[];
  shopping_style?: string;
  chattiness?: string;
  voice?: string;
  interests?: string[];
  [key: string]: unknown;
}

export interface ShopperPersona {
  id: string;
  customer_id: string;
  description: string;
  persona: ShopperPersonaBody;
  built_by_session_id: string | null;
  sessions: number;
  last_session: string | null;
  created: string;
}

export type CustomerBotStepKind = 'sign_up' | 'sign_in' | 'persona' | 'visit' | 'read' | 'think' | 'chat' | 'press' | 'review' | 'sign_out';

export interface CustomerBotStep {
  position: number;
  kind: CustomerBotStepKind;
  status: 'ok' | 'failed' | 'skipped';
  title: string;
  request: {method?: string; path?: string; body?: unknown} | null;
  response: {status?: number; body?: unknown} | null;
  detail: Record<string, unknown>;
  ms: number;
  at: string;
  recorded: string;
}

export interface CustomerBotSessionDetail extends Omit<CustomerBotSession, 'persona' | 'steps'> {
  persona: ShopperPersona | null;
  steps: CustomerBotStep[];
}

export interface CustomerBotRunDetail extends Omit<CustomerBotRun, 'sessions'> {
  sessions: CustomerBotSession[];
}

export interface CustomerBotStatus {
  settings: CustomerBotSettings;
  working: CustomerBotRun | null;
  last_run: CustomerBotRun | null;
  next_run_at: string | null;
  next_run_in_seconds: number | null;
  totals: {
    runs: number;
    sessions: number;
    failed_sessions: number;
    accounts_created: number;
    personas: number;
    chat_turns: number;
    orders: number;
    reviews: number;
    spent: Money;
  };
}
