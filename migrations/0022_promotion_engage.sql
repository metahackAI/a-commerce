-- 0022_promotion_engage.sql
-- Promotion, the parts that engage members beyond prices and coupons: promotions of a type
-- (create_promotion), the store-point ledger, conditional gifts, customer segments, and events
-- with their entries, draw records and rewards.
-- Rules: docs/backend/database.md. Order: columns added, tables, indexes, comments, row-level
-- security, grants.


ALTER TABLE promotion.promotion ADD COLUMN promotion_type text;
ALTER TABLE promotion.promotion ADD COLUMN promotion_terms jsonb NOT NULL DEFAULT '{}';
ALTER TABLE promotion.promotion ADD COLUMN sales_channels text[];
ALTER TABLE promotion.promotion ADD CONSTRAINT promotion_promotion_type_check CHECK (promotion_type IN ('bogo', 'bundle', 'threshold', 'amount_off'));
ALTER TABLE promotion.promotion ADD CONSTRAINT promotion_sales_channels_check CHECK (sales_channels <@ ARRAY['web', 'mobile_web', 'app', 'agent']::text[]);

CREATE TABLE promotion.loyalty_transaction (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  customer_id text NOT NULL,
  transaction_type text NOT NULL,
  amount_value numeric(12,2) NOT NULL,
  amount_currency char(3) NOT NULL,
  order_id text,
  source_transaction_id text,
  expires date,
  description text,
  agent_id text,
  start_time timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT loyalty_transaction_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT loyalty_transaction_id_check CHECK (id ~ '^lyt_[a-z0-9_]+$'),
  CONSTRAINT loyalty_transaction_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT loyalty_transaction_source_transaction_id_fkey FOREIGN KEY (tenant_id, source_transaction_id) REFERENCES promotion.loyalty_transaction (tenant_id, id),
  CONSTRAINT loyalty_transaction_transaction_type_check CHECK (transaction_type IN ('accrual', 'spend', 'expire', 'grant', 'restore', 'reversal')),
  CONSTRAINT loyalty_transaction_amount_value_check CHECK (amount_value > 0),
  CONSTRAINT loyalty_transaction_amount_currency_check CHECK (amount_currency ~ '^[A-Z]{3}$')
);

CREATE TABLE promotion.conditional_gift (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  gift_tiers jsonb NOT NULL,
  valid_for_member_tier text,
  sales_channels text[],
  valid_from timestamptz NOT NULL,
  valid_through timestamptz NOT NULL,
  change_set_id text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT conditional_gift_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT conditional_gift_id_check CHECK (id ~ '^cgf_[a-z0-9_]+$'),
  CONSTRAINT conditional_gift_valid_for_member_tier_fkey FOREIGN KEY (tenant_id, valid_for_member_tier) REFERENCES customer.member_program_tier (tenant_id, id),
  CONSTRAINT conditional_gift_sales_channels_check CHECK (sales_channels <@ ARRAY['web', 'mobile_web', 'app', 'agent']::text[]),
  CONSTRAINT conditional_gift_valid_period_check CHECK (valid_through > valid_from)
);

CREATE TABLE promotion.segment (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  condition_rules jsonb NOT NULL,
  estimated_size integer NOT NULL DEFAULT 0,
  estimated_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT segment_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT segment_id_check CHECK (id ~ '^seg_[a-z0-9_]+$'),
  CONSTRAINT segment_estimated_size_check CHECK (estimated_size >= 0)
);

CREATE TABLE promotion.event (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  event_status text NOT NULL DEFAULT 'draft',
  start_date timestamptz NOT NULL,
  end_date timestamptz NOT NULL,
  display_valid_from timestamptz,
  display_valid_through timestamptz,
  entry_mechanic text NOT NULL,
  selection_method text NOT NULL,
  entry_payload_schema jsonb,
  reward_tiers jsonb NOT NULL,
  eligibility jsonb NOT NULL DEFAULT '{}',
  valid_for_member_tier text,
  entry_limit jsonb NOT NULL,
  draw_at timestamptz,
  announce_at timestamptz,
  dispatch_at timestamptz,
  compliance_jurisdictions text[] NOT NULL DEFAULT '{}',
  seed_commitment text NOT NULL,
  draw_seed text NOT NULL,
  change_set_id text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT event_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT event_id_check CHECK (id ~ '^evt_[a-z0-9_]+$'),
  CONSTRAINT event_valid_for_member_tier_fkey FOREIGN KEY (tenant_id, valid_for_member_tier) REFERENCES customer.member_program_tier (tenant_id, id),
  CONSTRAINT event_event_status_check CHECK (event_status IN ('draft', 'scheduled', 'running', 'paused', 'closed', 'drawing', 'announced', 'fulfilling', 'completed', 'cancelled')),
  CONSTRAINT event_entry_mechanic_check CHECK (entry_mechanic ~ '^[a-z][a-z0-9_]*$'),
  CONSTRAINT event_selection_method_check CHECK (selection_method ~ '^[a-z][a-z0-9_]*$'),
  CONSTRAINT event_valid_period_check CHECK (end_date > start_date),
  CONSTRAINT event_display_period_check CHECK (display_valid_through IS NULL OR display_valid_from IS NULL OR display_valid_through > display_valid_from)
);

CREATE TABLE promotion.event_entry (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  event_id text NOT NULL,
  customer_id text NOT NULL,
  entry_status text NOT NULL DEFAULT 'entered',
  entry_payload jsonb NOT NULL DEFAULT '{}',
  entry_consent jsonb NOT NULL DEFAULT '{}',
  order_id text,
  period_bucket text NOT NULL,
  entry_number integer NOT NULL DEFAULT 1,
  rank integer,
  waitlist_position integer,
  draw_record_id text,
  start_time timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT event_entry_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT event_entry_id_check CHECK (id ~ '^ent_[a-z0-9_]+$'),
  CONSTRAINT event_entry_event_id_fkey FOREIGN KEY (tenant_id, event_id) REFERENCES promotion.event (tenant_id, id),
  CONSTRAINT event_entry_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT event_entry_bucket_key UNIQUE (tenant_id, event_id, customer_id, period_bucket, entry_number),
  CONSTRAINT event_entry_entry_status_check CHECK (entry_status IN ('entered', 'invalid', 'won', 'lost', 'waitlist', 'cancelled')),
  CONSTRAINT event_entry_entry_number_check CHECK (entry_number >= 1),
  CONSTRAINT event_entry_rank_check CHECK (rank >= 1),
  CONSTRAINT event_entry_waitlist_position_check CHECK (waitlist_position >= 1)
);

CREATE TABLE promotion.draw_record (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  event_id text NOT NULL,
  draw_position integer NOT NULL,
  draw_mode text NOT NULL,
  rank integer,
  method text NOT NULL,
  seed_commitment text NOT NULL,
  seed text NOT NULL,
  entrant_snapshot_digest text NOT NULL,
  entrant_count integer NOT NULL,
  excluded jsonb NOT NULL DEFAULT '{}',
  result jsonb NOT NULL,
  reason text,
  agent_id text NOT NULL,
  change_set_id text,
  start_time timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT draw_record_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT draw_record_id_check CHECK (id ~ '^drw_[a-z0-9_]+$'),
  CONSTRAINT draw_record_event_id_fkey FOREIGN KEY (tenant_id, event_id) REFERENCES promotion.event (tenant_id, id),
  CONSTRAINT draw_record_position_key UNIQUE (tenant_id, event_id, draw_position),
  CONSTRAINT draw_record_draw_mode_check CHECK (draw_mode ~ '^[a-z][a-z0-9_]*$'),
  CONSTRAINT draw_record_draw_position_check CHECK (draw_position >= 1),
  CONSTRAINT draw_record_rank_check CHECK (rank >= 1),
  CONSTRAINT draw_record_entrant_count_check CHECK (entrant_count >= 0)
);

CREATE TABLE promotion.event_reward (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  event_id text NOT NULL,
  entry_id text NOT NULL,
  customer_id text NOT NULL,
  rank integer NOT NULL,
  reward_type text NOT NULL,
  reward_ref text,
  reward_value numeric(12,2),
  reward_currency char(3),
  fulfillment text NOT NULL,
  reward_status text NOT NULL DEFAULT 'pending',
  compliance_requirements text[] NOT NULL DEFAULT '{}',
  fulfillment_ref text,
  reward_batch_id text NOT NULL,
  dispatched_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT event_reward_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT event_reward_id_check CHECK (id ~ '^rwd_[a-z0-9_]+$'),
  CONSTRAINT event_reward_event_id_fkey FOREIGN KEY (tenant_id, event_id) REFERENCES promotion.event (tenant_id, id),
  CONSTRAINT event_reward_entry_id_fkey FOREIGN KEY (tenant_id, entry_id) REFERENCES promotion.event_entry (tenant_id, id),
  CONSTRAINT event_reward_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT event_reward_entry_rank_key UNIQUE (tenant_id, entry_id, rank),
  CONSTRAINT event_reward_reward_status_check CHECK (reward_status IN ('pending', 'compliance_pending', 'ready', 'dispatched', 'delivered', 'failed', 'forfeited')),
  CONSTRAINT event_reward_reward_type_check CHECK (reward_type ~ '^[a-z][a-z0-9_]*$'),
  CONSTRAINT event_reward_fulfillment_check CHECK (fulfillment ~ '^[a-z][a-z0-9_]*$'),
  CONSTRAINT event_reward_reward_batch_id_check CHECK (reward_batch_id ~ '^rwb_[a-z0-9_]+$'),
  CONSTRAINT event_reward_rank_check CHECK (rank >= 1),
  CONSTRAINT event_reward_reward_value_check CHECK (reward_value >= 0),
  CONSTRAINT event_reward_reward_currency_check CHECK (reward_currency ~ '^[A-Z]{3}$')
);

CREATE INDEX loyalty_transaction_customer_id_idx ON promotion.loyalty_transaction (tenant_id, customer_id, start_time);
CREATE INDEX loyalty_transaction_expires_idx ON promotion.loyalty_transaction (tenant_id, expires);
CREATE UNIQUE INDEX loyalty_transaction_spend_order_key ON promotion.loyalty_transaction (tenant_id, customer_id, order_id) WHERE transaction_type = 'spend';
CREATE UNIQUE INDEX loyalty_transaction_source_key ON promotion.loyalty_transaction (tenant_id, source_transaction_id, transaction_type) WHERE transaction_type IN ('expire', 'reversal');
CREATE INDEX conditional_gift_valid_from_valid_through_idx ON promotion.conditional_gift (tenant_id, valid_from, valid_through);
CREATE INDEX event_event_status_idx ON promotion.event (tenant_id, event_status);
CREATE INDEX event_start_date_end_date_idx ON promotion.event (tenant_id, start_date, end_date);
CREATE INDEX event_entry_event_id_idx ON promotion.event_entry (tenant_id, event_id, entry_status);
CREATE INDEX event_entry_customer_id_idx ON promotion.event_entry (tenant_id, customer_id);
CREATE INDEX draw_record_event_id_idx ON promotion.draw_record (tenant_id, event_id);
CREATE INDEX event_reward_event_id_idx ON promotion.event_reward (tenant_id, event_id, reward_status);

COMMENT ON COLUMN promotion.promotion.promotion_type IS
  'acom:promotionType — kind of benefit a promotion made by create_promotion gives; NULL for member and sale prices (set_sale_prices). values: promotion_type';
COMMENT ON COLUMN promotion.promotion.promotion_terms IS
  'acom:promotionTerms — parameters of the type: {threshold_benefit: amount_off or free_shipping, buy_quantity, get_quantity, bundle_price: {value, currency}, cost_share: {store, seller} rates 0-1}.';
COMMENT ON COLUMN promotion.promotion.sales_channels IS
  'acom:salesChannel — channels where the promotion applies; NULL means every channel. values: sales_channel';

COMMENT ON TABLE promotion.loyalty_transaction IS
  'schema:MoneyTransfer/acom:LoyaltyTransaction — one entry of a member''s store-point ledger; entries are only added and the balance is their sum. id prefix lyt_.';
COMMENT ON COLUMN promotion.loyalty_transaction.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN promotion.loyalty_transaction.id IS
  '- id with prefix lyt_; the same value the API, logs and events use.';
COMMENT ON COLUMN promotion.loyalty_transaction.customer_id IS
  'schema:customer — member whose points these are (cus_ id).';
COMMENT ON COLUMN promotion.loyalty_transaction.transaction_type IS
  'acom:loyaltyTransactionType — accrual, grant and restore add points; spend, expire and reversal take them away. values: loyalty_transaction_type';
COMMENT ON COLUMN promotion.loyalty_transaction.amount_value IS
  'schema:amount — points as money, always positive (the type says which way), in major units of the currency.';
COMMENT ON COLUMN promotion.loyalty_transaction.amount_currency IS
  'schema:amount — ISO 4217 code of amount_value.';
COMMENT ON COLUMN promotion.loyalty_transaction.order_id IS
  'acom:orderId — order that earned, spent or gave back the points (ord_ id); NULL for grants and expiries.';
COMMENT ON COLUMN promotion.loyalty_transaction.source_transaction_id IS
  'acom:sourceTransaction — earlier entry this one expires or reverses (lyt_ id); NULL otherwise.';
COMMENT ON COLUMN promotion.loyalty_transaction.expires IS
  'schema:expires — last day (store time) the points of an accrual, grant or restore can be spent; NULL for entries that take points away.';
COMMENT ON COLUMN promotion.loyalty_transaction.description IS
  'schema:description — why, in a sentence, for example the reason a person gave for a grant.';
COMMENT ON COLUMN promotion.loyalty_transaction.agent_id IS
  'schema:agent — who recorded it: an operator (adm_), a shopper (cus_) or the system (sys_).';
COMMENT ON COLUMN promotion.loyalty_transaction.start_time IS
  'schema:startTime — when the entry happened on the store clock.';
COMMENT ON COLUMN promotion.loyalty_transaction.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN promotion.loyalty_transaction.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN promotion.loyalty_transaction.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE promotion.conditional_gift IS
  'schema:Offer/acom:ConditionalGift — a gift added to an order whose subtotal before member benefits reaches a threshold, with one gift per threshold. id prefix cgf_.';
COMMENT ON COLUMN promotion.conditional_gift.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN promotion.conditional_gift.id IS
  '- id with prefix cgf_; the same value the API, logs and events use.';
COMMENT ON COLUMN promotion.conditional_gift.name IS
  'schema:name — name people read, for example Holiday gift over $120.';
COMMENT ON COLUMN promotion.conditional_gift.gift_tiers IS
  'acom:giftTiers — thresholds in ascending order, each with its gift: [{trigger_threshold: {value, currency}, gift_product_id}].';
COMMENT ON COLUMN promotion.conditional_gift.valid_for_member_tier IS
  'schema:validForMemberTier — tier the gift is for; NULL means everyone (the reserved word everyone is never stored).';
COMMENT ON COLUMN promotion.conditional_gift.sales_channels IS
  'acom:salesChannel — channels where the gift applies; NULL means every channel. values: sales_channel';
COMMENT ON COLUMN promotion.conditional_gift.valid_from IS
  'schema:validFrom — first moment the gift applies.';
COMMENT ON COLUMN promotion.conditional_gift.valid_through IS
  'schema:validThrough — last moment it applies (inclusive, minute precision).';
COMMENT ON COLUMN promotion.conditional_gift.change_set_id IS
  'acom:changeSetId — change set that created it; NULL for seeded ones.';
COMMENT ON COLUMN promotion.conditional_gift.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN promotion.conditional_gift.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN promotion.conditional_gift.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE promotion.segment IS
  'schema:Audience/acom:Segment — a group of customers defined by conditions, used to issue coupons, send messages and limit events; the row version is the segment version. id prefix seg_.';
COMMENT ON COLUMN promotion.segment.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN promotion.segment.id IS
  '- id with prefix seg_; the same value the API, logs and events use.';
COMMENT ON COLUMN promotion.segment.name IS
  'schema:name — name people read, for example Lapsed knit buyers.';
COMMENT ON COLUMN promotion.segment.condition_rules IS
  'acom:conditions — who is in it: {member_tiers[], purchased_category_ids[], last_order_within_days, min_spend: {value, currency}, exclude_segment_ids[]}; every given condition must hold.';
COMMENT ON COLUMN promotion.segment.estimated_size IS
  'acom:estimatedSize — how many active members matched when the segment was last defined.';
COMMENT ON COLUMN promotion.segment.estimated_at IS
  'acom:estimatedAt — store-clock time estimated_size was counted.';
COMMENT ON COLUMN promotion.segment.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN promotion.segment.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN promotion.segment.version IS
  '- row version and segment version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE promotion.event IS
  'schema:Event — a promotion event: shoppers enter by an entry mechanic and winners are chosen by a selection method, with rewards by rank. id prefix evt_.';
COMMENT ON COLUMN promotion.event.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN promotion.event.id IS
  '- id with prefix evt_; the same value the API, logs and events use.';
COMMENT ON COLUMN promotion.event.name IS
  'schema:name — name people read, for example Fall check-in week.';
COMMENT ON COLUMN promotion.event.event_status IS
  'acom:eventStatus — stored status; scheduled and running follow the entry period and the store clock, closed after it ends. values: event_status';
COMMENT ON COLUMN promotion.event.start_date IS
  'schema:startDate — first moment entries are taken.';
COMMENT ON COLUMN promotion.event.end_date IS
  'schema:endDate — last moment entries are taken (inclusive, minute precision).';
COMMENT ON COLUMN promotion.event.display_valid_from IS
  'acom:displayValidFrom — when shoppers start seeing the event; NULL means from start_date.';
COMMENT ON COLUMN promotion.event.display_valid_through IS
  'acom:displayValidThrough — when shoppers stop seeing it; NULL means until end_date.';
COMMENT ON COLUMN promotion.event.entry_mechanic IS
  'acom:entryMechanic — how shoppers enter, an id of the store registry promotion.yml event.entry_mechanics, for example check_in.';
COMMENT ON COLUMN promotion.event.selection_method IS
  'acom:selectionMethod — how winners are chosen, an id of the store registry promotion.yml event.selection_methods, for example random_draw.';
COMMENT ON COLUMN promotion.event.entry_payload_schema IS
  'acom:entryPayloadSchema — fields an entry must send, {field: type}; NULL when an entry needs no data.';
COMMENT ON COLUMN promotion.event.reward_tiers IS
  'acom:rewardTiers — rewards by rank: [{rank, reward_type, reward_ref, reward_value: {value, currency}, winner_count, waitlist_count, odds, round}].';
COMMENT ON COLUMN promotion.event.eligibility IS
  'acom:eligibility — who may enter: {valid_for_member_tier, segment_id, member_types[], purchase_condition: {min_subtotal, window, scope}, channels[]}.';
COMMENT ON COLUMN promotion.event.valid_for_member_tier IS
  'schema:validForMemberTier — tier allowed to enter, the same as eligibility.valid_for_member_tier; NULL means everyone.';
COMMENT ON COLUMN promotion.event.entry_limit IS
  'acom:entryLimit — entries one member may make: {count, period: day, week, event or unlimited}.';
COMMENT ON COLUMN promotion.event.draw_at IS
  'acom:drawAt — when the draw is planned; NULL when not planned.';
COMMENT ON COLUMN promotion.event.announce_at IS
  'acom:announceAt — when winners are announced; NULL when not planned.';
COMMENT ON COLUMN promotion.event.dispatch_at IS
  'acom:dispatchAt — when rewards are sent; NULL when not planned.';
COMMENT ON COLUMN promotion.event.compliance_jurisdictions IS
  'acom:complianceJurisdictions — store-defined jurisdiction ids whose rules apply; empty means chosen by promotion.yml event.compliance.jurisdiction_from.';
COMMENT ON COLUMN promotion.event.seed_commitment IS
  'acom:seedCommitment — sha256 of the draw seed, published when the event is created so the draw can be checked afterwards.';
COMMENT ON COLUMN promotion.event.draw_seed IS
  'acom:seed — the secret draw seed; shown only in draw records once a draw ran.';
COMMENT ON COLUMN promotion.event.change_set_id IS
  'acom:changeSetId — change set that created the event; NULL for seeded ones.';
COMMENT ON COLUMN promotion.event.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN promotion.event.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN promotion.event.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE promotion.event_entry IS
  'schema:JoinAction/acom:EventEntry — one entry of a member in an event, and whether it won. id prefix ent_.';
COMMENT ON COLUMN promotion.event_entry.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN promotion.event_entry.id IS
  '- id with prefix ent_; the same value the API, logs and events use.';
COMMENT ON COLUMN promotion.event_entry.event_id IS
  'schema:object — event entered (evt_ id).';
COMMENT ON COLUMN promotion.event_entry.customer_id IS
  'schema:agent — member who entered (cus_ id).';
COMMENT ON COLUMN promotion.event_entry.entry_status IS
  'acom:entryStatus — entered until a draw or an instant result decides it. values: entry_status';
COMMENT ON COLUMN promotion.event_entry.entry_payload IS
  'acom:entryPayload — what the member sent, following the event''s entry_payload_schema (answers, a caption, a code).';
COMMENT ON COLUMN promotion.event_entry.entry_consent IS
  'acom:entryConsent — consents given when entering, {consent type: true}.';
COMMENT ON COLUMN promotion.event_entry.order_id IS
  'acom:orderId — order the entry is based on for purchase_triggered events (ord_ id); NULL otherwise.';
COMMENT ON COLUMN promotion.event_entry.period_bucket IS
  'acom:periodBucket — the entry-limit period the entry counts in: a store date for day, an ISO week for week, event or unlimited.';
COMMENT ON COLUMN promotion.event_entry.entry_number IS
  'acom:entryNumber — which entry of the member in that period it is, from 1.';
COMMENT ON COLUMN promotion.event_entry.rank IS
  'acom:rank — reward rank won or waited for; NULL until decided.';
COMMENT ON COLUMN promotion.event_entry.waitlist_position IS
  'acom:waitlistPosition — place on the waitlist of its rank, from 1; NULL when not waiting.';
COMMENT ON COLUMN promotion.event_entry.draw_record_id IS
  'acom:drawRecordId — draw that last decided the entry (drw_ id); NULL for instant results.';
COMMENT ON COLUMN promotion.event_entry.start_time IS
  'schema:startTime — when the member entered, on the store clock.';
COMMENT ON COLUMN promotion.event_entry.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN promotion.event_entry.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN promotion.event_entry.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE promotion.draw_record IS
  'schema:ChooseAction/acom:DrawRecord — the audit record of one draw run: method, seed, entrant snapshot and result, so anyone can repeat it. Rows are only added. id prefix drw_.';
COMMENT ON COLUMN promotion.draw_record.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN promotion.draw_record.id IS
  '- id with prefix drw_; the same value the API, logs and events use.';
COMMENT ON COLUMN promotion.draw_record.event_id IS
  'schema:object — event drawn (evt_ id).';
COMMENT ON COLUMN promotion.draw_record.draw_position IS
  'acom:drawPosition — which run of the event''s draws this is, from 1; part of the shuffle key.';
COMMENT ON COLUMN promotion.draw_record.draw_mode IS
  'acom:drawMode — what the run did, one of promotion.yml event.draw.modes: draw, redraw, cancel_winner, promote_waitlist, add_waitlist or manual_select.';
COMMENT ON COLUMN promotion.draw_record.rank IS
  'acom:rank — the reward rank drawn; NULL for every rank.';
COMMENT ON COLUMN promotion.draw_record.method IS
  'acom:method — how winners were chosen, from promotion.yml event.draw.fairness.method, for example seeded_shuffle.';
COMMENT ON COLUMN promotion.draw_record.seed_commitment IS
  'acom:seedCommitment — sha256 of the seed, as published before entries closed.';
COMMENT ON COLUMN promotion.draw_record.seed IS
  'acom:seed — the seed the shuffle used, revealed by the draw.';
COMMENT ON COLUMN promotion.draw_record.entrant_snapshot_digest IS
  'acom:entrantSnapshotDigest — sha256 of the entry ids the draw chose from, in order.';
COMMENT ON COLUMN promotion.draw_record.entrant_count IS
  'acom:entrantCount — how many entries the draw chose from.';
COMMENT ON COLUMN promotion.draw_record.excluded IS
  'acom:excluded — entries left out and why: {previous_winners, invalid_entries}.';
COMMENT ON COLUMN promotion.draw_record.result IS
  'schema:result — [{rank, winners[], waitlist[], cancelled[]}] with entry ids.';
COMMENT ON COLUMN promotion.draw_record.reason IS
  'acom:reason — why a person redrew, cancelled a winner or chose by hand.';
COMMENT ON COLUMN promotion.draw_record.agent_id IS
  'schema:agent — who asked for the draw (adm_ or sys_ id).';
COMMENT ON COLUMN promotion.draw_record.change_set_id IS
  'acom:changeSetId — change set whose approval ran the draw; its approver is the person who approved it.';
COMMENT ON COLUMN promotion.draw_record.start_time IS
  'schema:startTime — when the draw ran, on the store clock.';
COMMENT ON COLUMN promotion.draw_record.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN promotion.draw_record.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN promotion.draw_record.version IS
  '- row version; draw records are never changed.';

COMMENT ON TABLE promotion.event_reward IS
  'schema:Offer/acom:EventReward — the reward of one winning entry and how it is sent. id prefix rwd_.';
COMMENT ON COLUMN promotion.event_reward.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN promotion.event_reward.id IS
  '- id with prefix rwd_; the same value the API, logs and events use.';
COMMENT ON COLUMN promotion.event_reward.event_id IS
  'acom:eventId — event the reward belongs to (evt_ id).';
COMMENT ON COLUMN promotion.event_reward.entry_id IS
  'acom:entryId — winning entry (ent_ id).';
COMMENT ON COLUMN promotion.event_reward.customer_id IS
  'schema:customer — member who gets the reward (cus_ id).';
COMMENT ON COLUMN promotion.event_reward.rank IS
  'acom:rank — reward rank won.';
COMMENT ON COLUMN promotion.event_reward.reward_type IS
  'acom:rewardType — kind of reward, an id of the store registry promotion.yml event.reward_types, for example product.';
COMMENT ON COLUMN promotion.event_reward.reward_ref IS
  'acom:rewardRef — what is given: a product (prd_), a coupon (cpn_) or free text; NULL when the type needs none.';
COMMENT ON COLUMN promotion.event_reward.reward_value IS
  'acom:rewardValue — value of the reward, in major units of the currency; NULL when the type has no value.';
COMMENT ON COLUMN promotion.event_reward.reward_currency IS
  'acom:rewardValue — ISO 4217 code of reward_value.';
COMMENT ON COLUMN promotion.event_reward.fulfillment IS
  'acom:fulfillment — how it is sent: ship, issue, grant, email or manual (promotion.yml event.reward_types.fulfillment_methods).';
COMMENT ON COLUMN promotion.event_reward.reward_status IS
  'acom:rewardStatus — compliance_pending while a store rule waits for the winner. values: reward_status';
COMMENT ON COLUMN promotion.event_reward.compliance_requirements IS
  'acom:complianceRequirements — what the winner still has to provide (legal_name, address, tax_id); empty when nothing.';
COMMENT ON COLUMN promotion.event_reward.fulfillment_ref IS
  'acom:fulfillmentRef — what sending made: a member coupon (mcp_), a ledger entry (lyt_), or the reference of a stubbed shipment or email.';
COMMENT ON COLUMN promotion.event_reward.reward_batch_id IS
  'acom:rewardBatchId — the dispatch run that made it (rwb_ id).';
COMMENT ON COLUMN promotion.event_reward.dispatched_at IS
  'acom:dispatchedAt — when it was sent; NULL while held or pending.';
COMMENT ON COLUMN promotion.event_reward.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN promotion.event_reward.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN promotion.event_reward.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE promotion.loyalty_transaction ENABLE ROW LEVEL SECURITY;
ALTER TABLE promotion.loyalty_transaction FORCE ROW LEVEL SECURITY;
CREATE POLICY loyalty_transaction_tenant_isolation ON promotion.loyalty_transaction
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE promotion.conditional_gift ENABLE ROW LEVEL SECURITY;
ALTER TABLE promotion.conditional_gift FORCE ROW LEVEL SECURITY;
CREATE POLICY conditional_gift_tenant_isolation ON promotion.conditional_gift
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE promotion.segment ENABLE ROW LEVEL SECURITY;
ALTER TABLE promotion.segment FORCE ROW LEVEL SECURITY;
CREATE POLICY segment_tenant_isolation ON promotion.segment
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE promotion.event ENABLE ROW LEVEL SECURITY;
ALTER TABLE promotion.event FORCE ROW LEVEL SECURITY;
CREATE POLICY event_tenant_isolation ON promotion.event
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE promotion.event_entry ENABLE ROW LEVEL SECURITY;
ALTER TABLE promotion.event_entry FORCE ROW LEVEL SECURITY;
CREATE POLICY event_entry_tenant_isolation ON promotion.event_entry
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE promotion.draw_record ENABLE ROW LEVEL SECURITY;
ALTER TABLE promotion.draw_record FORCE ROW LEVEL SECURITY;
CREATE POLICY draw_record_tenant_isolation ON promotion.draw_record
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE promotion.event_reward ENABLE ROW LEVEL SECURITY;
ALTER TABLE promotion.event_reward FORCE ROW LEVEL SECURITY;
CREATE POLICY event_reward_tenant_isolation ON promotion.event_reward
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- DELETE only where removing a row is the undo of creating it, before anything refers to it
-- (0014_app_delete_grants.sql): a conditional gift, a segment, an event without entries.
GRANT DELETE ON
  promotion.conditional_gift,
  promotion.segment,
  promotion.event
TO acom_app;
