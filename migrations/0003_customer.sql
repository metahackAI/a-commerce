-- 0003_customer.sql
-- Customer: people (members), saved addresses, member programs and tiers, memberships,
-- and remembered shopper preferences.
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security.


CREATE TABLE customer.person (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  given_name text,
  family_name text,
  email text,
  telephone text,
  birth_date date,
  member_state text NOT NULL DEFAULT 'active',
  member_type text NOT NULL DEFAULT 'individual',
  sizes jsonb NOT NULL DEFAULT '{}',
  card_last4 text,
  registered_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT person_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT person_id_check CHECK (id ~ '^cus_[a-z0-9_]+$'),
  CONSTRAINT person_email_key UNIQUE (tenant_id, email),
  CONSTRAINT person_member_state_check CHECK (member_state IN ('active', 'dormant', 'blocked', 'pending_withdrawal', 'withdrawn')),
  CONSTRAINT person_member_type_check CHECK (member_type IN ('individual', 'business')),
  CONSTRAINT person_card_last4_check CHECK (card_last4 ~ '^[0-9]{4}$')
);

CREATE TABLE customer.postal_address (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  customer_id text NOT NULL,
  label text,
  name text,
  street_address text NOT NULL,
  address_locality text NOT NULL,
  address_region text,
  postal_code text NOT NULL,
  address_country char(2) NOT NULL,
  telephone text,
  address_usage text[] NOT NULL DEFAULT '{shipping}',
  is_primary boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT postal_address_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT postal_address_id_check CHECK (id ~ '^adr_[a-z0-9_]+$'),
  CONSTRAINT postal_address_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT postal_address_address_country_check CHECK (address_country ~ '^[A-Z]{2}$'),
  CONSTRAINT postal_address_address_usage_check CHECK (address_usage <@ ARRAY['shipping', 'billing', 'return']::text[])
);

CREATE TABLE customer.member_program (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  description text,
  evaluation jsonb NOT NULL DEFAULT '{}',
  movement jsonb NOT NULL DEFAULT '{}',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT member_program_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT member_program_id_check CHECK (id ~ '^prg_[a-z0-9_]+$')
);

CREATE TABLE customer.member_program_tier (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  member_program_id text NOT NULL,
  name text NOT NULL,
  rank integer NOT NULL,
  has_tier_requirement jsonb NOT NULL DEFAULT '{}',
  has_tier_benefit jsonb NOT NULL DEFAULT '[]',
  membership_points_earned_value numeric(6,2),
  membership_points_earned_unit_text text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT member_program_tier_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT member_program_tier_id_check CHECK (id ~ '^[a-z][a-z0-9_]*$'),
  CONSTRAINT member_program_tier_id_not_everyone_check CHECK (id <> 'everyone'),
  CONSTRAINT member_program_tier_member_program_id_rank_key UNIQUE (tenant_id, member_program_id, rank),
  CONSTRAINT member_program_tier_member_program_id_fkey FOREIGN KEY (tenant_id, member_program_id) REFERENCES customer.member_program (tenant_id, id),
  CONSTRAINT member_program_tier_rank_check CHECK (rank >= 1),
  CONSTRAINT member_program_tier_membership_points_earned_value_check CHECK (membership_points_earned_value >= 0)
);

CREATE TABLE customer.program_membership (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  customer_id text NOT NULL,
  member_program_id text NOT NULL,
  tier_id text NOT NULL,
  membership_number text NOT NULL,
  tier_since timestamptz NOT NULL,
  tier_override jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT program_membership_pkey PRIMARY KEY (tenant_id, customer_id, member_program_id),
  CONSTRAINT program_membership_membership_number_key UNIQUE (tenant_id, membership_number),
  CONSTRAINT program_membership_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT program_membership_member_program_id_fkey FOREIGN KEY (tenant_id, member_program_id) REFERENCES customer.member_program (tenant_id, id),
  CONSTRAINT program_membership_tier_id_fkey FOREIGN KEY (tenant_id, tier_id) REFERENCES customer.member_program_tier (tenant_id, id)
);

CREATE TABLE customer.preference (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  customer_id text NOT NULL,
  preference_key text NOT NULL,
  value text NOT NULL,
  description text NOT NULL,
  source text NOT NULL DEFAULT 'conversation',
  conversation_id text,
  learned_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT preference_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT preference_id_check CHECK (id ~ '^mem_[a-z0-9_]+$'),
  CONSTRAINT preference_customer_id_preference_key_value_key UNIQUE (tenant_id, customer_id, preference_key, value),
  CONSTRAINT preference_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT preference_preference_key_check CHECK (preference_key IN ('size', 'fit_note', 'fabric_preference', 'compares_specs', 'layout_preference', 'travel_priority', 'other')),
  CONSTRAINT preference_value_check CHECK (char_length(value) BETWEEN 1 AND 200)
);

CREATE INDEX person_member_state_idx ON customer.person (tenant_id, member_state);
CREATE INDEX postal_address_customer_id_idx ON customer.postal_address (tenant_id, customer_id);
CREATE INDEX program_membership_member_program_id_idx ON customer.program_membership (tenant_id, member_program_id);
CREATE INDEX program_membership_tier_id_idx ON customer.program_membership (tenant_id, tier_id);

COMMENT ON TABLE customer.person IS
  'schema:Person — a shopper with an account (member). id prefix cus_. API: customer_id.';
COMMENT ON COLUMN customer.person.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.person.id IS
  '- id with prefix cus_; the same value the API, logs and events use.';
COMMENT ON COLUMN customer.person.name IS
  'schema:name — display name shown to the shopper, for example Jamie.';
COMMENT ON COLUMN customer.person.given_name IS
  'schema:givenName — first name.';
COMMENT ON COLUMN customer.person.family_name IS
  'schema:familyName — last name.';
COMMENT ON COLUMN customer.person.email IS
  'schema:email — contact email; masked for operators by default.';
COMMENT ON COLUMN customer.person.telephone IS
  'schema:telephone — contact phone in E.164 form; masked for operators by default.';
COMMENT ON COLUMN customer.person.birth_date IS
  'schema:birthDate — date of birth, used for the birthday coupon; NULL when not given.';
COMMENT ON COLUMN customer.person.member_state IS
  'acom:memberState — account lifecycle. values: member_state';
COMMENT ON COLUMN customer.person.member_type IS
  'acom:memberType — individual or business member. values: member_type';
COMMENT ON COLUMN customer.person.sizes IS
  'acom:sizes — known sizes: {tops, bottoms, shoes}, each a size word or null.';
COMMENT ON COLUMN customer.person.card_last4 IS
  'acom:cardLast4 — last four digits of the saved card, the only card data kept; NULL when no card is saved.';
COMMENT ON COLUMN customer.person.registered_at IS
  'acom:registeredAt — when the shopper created the account.';
COMMENT ON COLUMN customer.person.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.person.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.person.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE customer.postal_address IS
  'schema:PostalAddress — a saved address of a shopper. id prefix adr_.';
COMMENT ON COLUMN customer.postal_address.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.postal_address.id IS
  '- id with prefix adr_; the same value the API, logs and events use.';
COMMENT ON COLUMN customer.postal_address.customer_id IS
  'schema:customer — shopper who saved the address (cus_ id).';
COMMENT ON COLUMN customer.postal_address.label IS
  'acom:label — the shopper''s name for the address, for example Home.';
COMMENT ON COLUMN customer.postal_address.name IS
  'schema:name — recipient name printed on parcels.';
COMMENT ON COLUMN customer.postal_address.street_address IS
  'schema:streetAddress — street, number and unit, for example 12 Harbor St, Apt 3.';
COMMENT ON COLUMN customer.postal_address.address_locality IS
  'schema:addressLocality — city or town.';
COMMENT ON COLUMN customer.postal_address.address_region IS
  'schema:addressRegion — state or region code, for example NY.';
COMMENT ON COLUMN customer.postal_address.postal_code IS
  'schema:postalCode — postal or ZIP code.';
COMMENT ON COLUMN customer.postal_address.address_country IS
  'schema:addressCountry — ISO 3166-1 alpha-2 country code.';
COMMENT ON COLUMN customer.postal_address.telephone IS
  'schema:telephone — phone for the carrier.';
COMMENT ON COLUMN customer.postal_address.address_usage IS
  'acom:usage — what the address is used for; every element is a word from the list. values: address_usage';
COMMENT ON COLUMN customer.postal_address.is_primary IS
  'acom:isPrimary — the shopper''s main address; at most one per shopper.';
COMMENT ON COLUMN customer.postal_address.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.postal_address.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.postal_address.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE customer.member_program IS
  'schema:MemberProgram — a store''s tier program; tiers, requirements and benefits are data, not code. id prefix prg_.';
COMMENT ON COLUMN customer.member_program.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.member_program.id IS
  '- id with prefix prg_; the same value the API, logs and events use.';
COMMENT ON COLUMN customer.member_program.name IS
  'schema:name — program name, for example Northwind Rewards.';
COMMENT ON COLUMN customer.member_program.description IS
  'schema:description — what members get.';
COMMENT ON COLUMN customer.member_program.evaluation IS
  'acom:evaluation — when tiers are evaluated: {schedule: {every, on_day, at}, on_event[], settle_days}.';
COMMENT ON COLUMN customer.member_program.movement IS
  'acom:movement — how members move: {upgrade, downgrade, max_steps_down, grace_periods}.';
COMMENT ON COLUMN customer.member_program.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.member_program.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.member_program.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE customer.member_program_tier IS
  'schema:MemberProgramTier — one tier of a member program; the id is a word the store chooses (vip, gold, member) and everyone is reserved. API: tier_id.';
COMMENT ON COLUMN customer.member_program_tier.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.member_program_tier.id IS
  '- tier id, a lowercase word the store chooses (for example vip); never everyone; referenced by valid_for_member_tier.';
COMMENT ON COLUMN customer.member_program_tier.member_program_id IS
  'schema:isTierOf — program the tier belongs to (prg_ id).';
COMMENT ON COLUMN customer.member_program_tier.name IS
  'schema:name — tier name people see, for example VIP.';
COMMENT ON COLUMN customer.member_program_tier.rank IS
  'acom:rank — order of the tier, 1 is the entry tier and higher ranks are above.';
COMMENT ON COLUMN customer.member_program_tier.has_tier_requirement IS
  'schema:hasTierRequirement — {combine: any or all, requirements: [{metric, threshold, retain_threshold, window, counts}]}; empty for the entry tier.';
COMMENT ON COLUMN customer.member_program_tier.has_tier_benefit IS
  'schema:hasTierBenefit — standing benefits: [{type, coupon_id, every, value}], type from the tier benefit list in enums.md.';
COMMENT ON COLUMN customer.member_program_tier.membership_points_earned_value IS
  'schema:membershipPointsEarned — points multiplier for the tier; NULL when the tier has none.';
COMMENT ON COLUMN customer.member_program_tier.membership_points_earned_unit_text IS
  'schema:membershipPointsEarned — unit of membership_points_earned_value, for example points per USD.';
COMMENT ON COLUMN customer.member_program_tier.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.member_program_tier.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.member_program_tier.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE customer.program_membership IS
  'schema:ProgramMembership — a shopper''s membership in a program and their current tier; one tier per program.';
COMMENT ON COLUMN customer.program_membership.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.program_membership.customer_id IS
  'schema:member — the member (cus_ id).';
COMMENT ON COLUMN customer.program_membership.member_program_id IS
  'schema:program — the program (prg_ id).';
COMMENT ON COLUMN customer.program_membership.tier_id IS
  'acom:tierId — current tier (member_program_tier id).';
COMMENT ON COLUMN customer.program_membership.membership_number IS
  'schema:membershipNumber — number printed for the member, unique in the store.';
COMMENT ON COLUMN customer.program_membership.tier_since IS
  'acom:tierSince — when the member entered the current tier.';
COMMENT ON COLUMN customer.program_membership.tier_override IS
  'acom:tierOverride — manual tier set by an operator: {reason, expires_at, after_expiry, by}; NULL when the tier comes from evaluation.';
COMMENT ON COLUMN customer.program_membership.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.program_membership.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.program_membership.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE customer.preference IS
  'schema:PropertyValue/acom:Preference — something the agent remembered about a shopper; visible to the shopper and undoable. id prefix mem_. API: memory_id.';
COMMENT ON COLUMN customer.preference.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.preference.id IS
  '- id with prefix mem_; the same value the API, logs and events use.';
COMMENT ON COLUMN customer.preference.customer_id IS
  'schema:customer — shopper the preference is about (cus_ id); never a gift recipient.';
COMMENT ON COLUMN customer.preference.preference_key IS
  'acom:preferenceKey — what the preference is about (API key). values: preference_key';
COMMENT ON COLUMN customer.preference.value IS
  'schema:value — the value in the shopper''s words, 1 to 200 characters.';
COMMENT ON COLUMN customer.preference.description IS
  'schema:description — the one line shown to the shopper, for example Natural fabrics.';
COMMENT ON COLUMN customer.preference.source IS
  'acom:source — how it was learned; words from customer.yml preference.sources.';
COMMENT ON COLUMN customer.preference.conversation_id IS
  'acom:conversationId — conversation it was learned in (conv_ id).';
COMMENT ON COLUMN customer.preference.learned_at IS
  'acom:learnedAt — when it was remembered.';
COMMENT ON COLUMN customer.preference.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.preference.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.preference.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE customer.person ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.person FORCE ROW LEVEL SECURITY;
CREATE POLICY person_tenant_isolation ON customer.person
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE customer.postal_address ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.postal_address FORCE ROW LEVEL SECURITY;
CREATE POLICY postal_address_tenant_isolation ON customer.postal_address
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE customer.member_program ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.member_program FORCE ROW LEVEL SECURITY;
CREATE POLICY member_program_tenant_isolation ON customer.member_program
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE customer.member_program_tier ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.member_program_tier FORCE ROW LEVEL SECURITY;
CREATE POLICY member_program_tier_tenant_isolation ON customer.member_program_tier
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE customer.program_membership ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.program_membership FORCE ROW LEVEL SECURITY;
CREATE POLICY program_membership_tenant_isolation ON customer.program_membership
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE customer.preference ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.preference FORCE ROW LEVEL SECURITY;
CREATE POLICY preference_tenant_isolation ON customer.preference
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
