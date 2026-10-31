-- 0021_customer_members_and_service.sql
-- Customer: what members, tier programs and customer service need beyond 0003 — sign-in and
-- account facts on the person, identity checks and account recovery, consent history, tier
-- history, tier evaluations and program versions, employees, customer inquiries and their
-- threads, remedies, FAQs and notices, and withdrawals.
-- Rules: docs/backend/database.md. Order: tables, indexes, comments, row-level security, grants.
-- Business times (requested_at, recorded_at, ...) come from the store clock, which the demo can
-- move; created_at and updated_at stay technical.


ALTER TABLE customer.person ADD COLUMN login_id text;
ALTER TABLE customer.person ADD COLUMN legal_name text;
ALTER TABLE customer.person ADD COLUMN tax_id text;
ALTER TABLE customer.person ADD COLUMN business_verification text NOT NULL DEFAULT 'unverified';
ALTER TABLE customer.person ADD COLUMN email_verified_at timestamptz;
ALTER TABLE customer.person ADD COLUMN telephone_verified_at timestamptz;
ALTER TABLE customer.person ADD COLUMN password_hash text;
ALTER TABLE customer.person ADD COLUMN password_history text[] NOT NULL DEFAULT '{}';
ALTER TABLE customer.person ADD COLUMN password_changed_at timestamptz;
ALTER TABLE customer.person ADD COLUMN failed_password_attempts integer NOT NULL DEFAULT 0;
ALTER TABLE customer.person ADD COLUMN first_failed_password_at timestamptz;
ALTER TABLE customer.person ADD COLUMN locked_until timestamptz;
ALTER TABLE customer.person ADD COLUMN block_reason text;
ALTER TABLE customer.person ADD COLUMN blocked_until timestamptz;
ALTER TABLE customer.person ADD COLUMN withdrawn_at timestamptz;
ALTER TABLE customer.person ADD CONSTRAINT person_login_id_key UNIQUE (tenant_id, login_id);
ALTER TABLE customer.person ADD CONSTRAINT person_business_verification_check CHECK (business_verification IN ('unverified', 'verified'));
ALTER TABLE customer.person ADD CONSTRAINT person_failed_password_attempts_check CHECK (failed_password_attempts >= 0);

CREATE TABLE customer.identity_verification (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  purpose text NOT NULL,
  method text NOT NULL,
  destination text,
  customer_id text,
  subject jsonb,
  code_hash text,
  attempts integer NOT NULL DEFAULT 0,
  action_status text NOT NULL DEFAULT 'potential',
  requested_at timestamptz NOT NULL,
  expires timestamptz NOT NULL,
  verified_at timestamptz,
  token_hash text,
  token_expires timestamptz,
  token_used_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT identity_verification_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT identity_verification_id_check CHECK (id ~ '^ivf_[a-z0-9_]+$'),
  CONSTRAINT identity_verification_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT identity_verification_token_hash_key UNIQUE (tenant_id, token_hash),
  CONSTRAINT identity_verification_action_status_check CHECK (action_status IN ('potential', 'active', 'completed', 'failed')),
  CONSTRAINT identity_verification_attempts_check CHECK (attempts >= 0)
);

CREATE TABLE customer.recovery_request (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  recovery_kind text NOT NULL,
  customer_id text,
  identifier_fingerprint text NOT NULL,
  session_id text,
  method text,
  message_channel text,
  token_hash text,
  code_hash text,
  attempts integer NOT NULL DEFAULT 0,
  agent_id text NOT NULL,
  reason text,
  requested_at timestamptz NOT NULL,
  expires timestamptz NOT NULL,
  used_at timestamptz,
  invalidated_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT recovery_request_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT recovery_request_id_check CHECK (id ~ '^rcv_[a-z0-9_]+$'),
  CONSTRAINT recovery_request_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT recovery_request_token_hash_key UNIQUE (tenant_id, token_hash),
  CONSTRAINT recovery_request_recovery_kind_check CHECK (recovery_kind IN ('password_reset', 'login_id')),
  CONSTRAINT recovery_request_message_channel_check CHECK (message_channel IN ('email', 'sms', 'push', 'in_app')),
  CONSTRAINT recovery_request_attempts_check CHECK (attempts >= 0)
);

CREATE TABLE customer.consent_record (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  customer_id text NOT NULL,
  consent_type text NOT NULL,
  granted boolean NOT NULL,
  recorded_at timestamptz NOT NULL,
  agent_id text NOT NULL,
  instrument text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT consent_record_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT consent_record_id_check CHECK (id ~ '^cns_[a-z0-9_]+$'),
  CONSTRAINT consent_record_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT consent_record_consent_type_check CHECK (consent_type IN ('terms', 'privacy', 'marketing_email', 'marketing_sms', 'marketing_push', 'marketing_in_app', 'entry_terms', 'winner_contact'))
);

CREATE TABLE customer.tier_evaluation (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  member_program_id text NOT NULL,
  as_of timestamptz NOT NULL,
  scope jsonb NOT NULL DEFAULT '{"scope": "all"}',
  action_status text NOT NULL DEFAULT 'active',
  moves jsonb NOT NULL DEFAULT '[]',
  summary jsonb NOT NULL DEFAULT '{}',
  agent_id text NOT NULL,
  start_time timestamptz NOT NULL,
  end_time timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT tier_evaluation_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT tier_evaluation_id_check CHECK (id ~ '^tev_[a-z0-9_]+$'),
  CONSTRAINT tier_evaluation_member_program_id_fkey FOREIGN KEY (tenant_id, member_program_id) REFERENCES customer.member_program (tenant_id, id),
  CONSTRAINT tier_evaluation_action_status_check CHECK (action_status IN ('potential', 'active', 'completed', 'failed'))
);

CREATE TABLE customer.program_membership_event (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  customer_id text NOT NULL,
  member_program_id text NOT NULL,
  position integer NOT NULL,
  from_tier_id text,
  to_tier_id text,
  cause text NOT NULL,
  evaluation_id text,
  snapshot jsonb,
  agent_id text NOT NULL,
  reason text,
  expires timestamptz,
  changed_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT program_membership_event_pkey PRIMARY KEY (tenant_id, customer_id, member_program_id, position),
  CONSTRAINT program_membership_event_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT program_membership_event_member_program_id_fkey FOREIGN KEY (tenant_id, member_program_id) REFERENCES customer.member_program (tenant_id, id),
  CONSTRAINT program_membership_event_evaluation_id_fkey FOREIGN KEY (tenant_id, evaluation_id) REFERENCES customer.tier_evaluation (tenant_id, id),
  CONSTRAINT program_membership_event_cause_check CHECK (cause IN ('entry', 'evaluation', 'event', 'manual_override', 'override_expired', 'grace_started', 'grace_ended', 'program_change', 'withdrawal')),
  CONSTRAINT program_membership_event_position_check CHECK (position >= 1)
);

CREATE TABLE customer.member_program_version (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  member_program_id text NOT NULL,
  program_version integer NOT NULL,
  definition jsonb NOT NULL,
  agent_id text NOT NULL,
  defined_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT member_program_version_pkey PRIMARY KEY (tenant_id, member_program_id, program_version),
  CONSTRAINT member_program_version_member_program_id_fkey FOREIGN KEY (tenant_id, member_program_id) REFERENCES customer.member_program (tenant_id, id),
  CONSTRAINT member_program_version_program_version_check CHECK (program_version >= 1)
);

CREATE TABLE customer.employee_role (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  customer_id text NOT NULL,
  employee_number text NOT NULL,
  discount_limit_value numeric(12,2) NOT NULL,
  discount_limit_currency char(3) NOT NULL,
  discount_limit_period text NOT NULL DEFAULT 'year',
  start_date date NOT NULL,
  end_date date,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT employee_role_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT employee_role_id_check CHECK (id ~ '^emp_[a-z0-9_]+$'),
  CONSTRAINT employee_role_customer_id_key UNIQUE (tenant_id, customer_id),
  CONSTRAINT employee_role_employee_number_key UNIQUE (tenant_id, employee_number),
  CONSTRAINT employee_role_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT employee_role_discount_limit_value_check CHECK (discount_limit_value >= 0),
  CONSTRAINT employee_role_discount_limit_currency_check CHECK (discount_limit_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT employee_role_discount_limit_period_check CHECK (discount_limit_period IN ('month', 'year'))
);

CREATE TABLE customer.customer_inquiry (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  customer_id text NOT NULL,
  inquiry_type text NOT NULL,
  inquiry_status text NOT NULL DEFAULT 'received',
  order_id text,
  admin_user_id text,
  text text NOT NULL,
  resolution text,
  date_received timestamptz NOT NULL,
  assigned_at timestamptz,
  resolved_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT customer_inquiry_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT customer_inquiry_id_check CHECK (id ~ '^inq_[a-z0-9_]+$'),
  CONSTRAINT customer_inquiry_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT customer_inquiry_admin_user_id_fkey FOREIGN KEY (tenant_id, admin_user_id) REFERENCES platform.admin_user (tenant_id, id),
  CONSTRAINT customer_inquiry_inquiry_type_check CHECK (inquiry_type IN ('order_status', 'return', 'product_question', 'account', 'callback', 'other')),
  CONSTRAINT customer_inquiry_inquiry_status_check CHECK (inquiry_status IN ('received', 'in_progress', 'resolved')),
  CONSTRAINT customer_inquiry_order_id_check CHECK (order_id ~ '^ord_[a-z0-9_]+$')
);

CREATE TABLE customer.comment (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  inquiry_id text NOT NULL,
  author_type text NOT NULL,
  agent_id text NOT NULL,
  text text NOT NULL,
  message_channel text,
  date_created timestamptz NOT NULL,
  date_sent timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT comment_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT comment_id_check CHECK (id ~ '^cmt_[a-z0-9_]+$'),
  CONSTRAINT comment_inquiry_id_fkey FOREIGN KEY (tenant_id, inquiry_id) REFERENCES customer.customer_inquiry (tenant_id, id),
  CONSTRAINT comment_author_type_check CHECK (author_type IN ('customer', 'staff', 'agent')),
  CONSTRAINT comment_message_channel_check CHECK (message_channel IN ('email', 'sms', 'push', 'in_app'))
);

CREATE TABLE customer.remedy (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  customer_id text,
  inquiry_id text,
  order_id text,
  remedy_type text NOT NULL,
  liability text NOT NULL,
  amount_value numeric(12,2) NOT NULL,
  amount_currency char(3) NOT NULL,
  reason text NOT NULL,
  remedy_status text NOT NULL DEFAULT 'requested',
  requested_by_id text NOT NULL,
  requested_at timestamptz NOT NULL,
  decided_by_id text,
  decided_at timestamptz,
  decision_reason text,
  paid_at timestamptz,
  payout jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT remedy_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT remedy_id_check CHECK (id ~ '^rmd_[a-z0-9_]+$'),
  CONSTRAINT remedy_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT remedy_inquiry_id_fkey FOREIGN KEY (tenant_id, inquiry_id) REFERENCES customer.customer_inquiry (tenant_id, id),
  CONSTRAINT remedy_order_id_check CHECK (order_id ~ '^ord_[a-z0-9_]+$'),
  CONSTRAINT remedy_target_check CHECK (inquiry_id IS NOT NULL OR order_id IS NOT NULL),
  CONSTRAINT remedy_remedy_type_check CHECK (remedy_type IN ('loyalty_points', 'coupon', 'cash')),
  CONSTRAINT remedy_liability_check CHECK (liability IN ('store_fault', 'seller_fault', 'customer_fault', 'carrier_fault', 'none')),
  CONSTRAINT remedy_remedy_status_check CHECK (remedy_status IN ('requested', 'approved', 'rejected', 'payout_pending', 'paid', 'failed')),
  CONSTRAINT remedy_amount_value_check CHECK (amount_value > 0),
  CONSTRAINT remedy_amount_currency_check CHECK (amount_currency ~ '^[A-Z]{3}$')
);

CREATE TABLE customer.question (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  accepted_answer text NOT NULL,
  category text NOT NULL,
  position integer NOT NULL DEFAULT 1,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT question_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT question_id_check CHECK (id ~ '^faq_[a-z0-9_]+$'),
  CONSTRAINT question_category_check CHECK (category ~ '^[a-z][a-z0-9_]*$')
);

CREATE TABLE customer.news_article (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  headline text NOT NULL,
  article_body text NOT NULL,
  category text NOT NULL,
  notice_status text NOT NULL DEFAULT 'draft',
  date_published timestamptz,
  is_pinned boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT news_article_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT news_article_id_check CHECK (id ~ '^ntc_[a-z0-9_]+$'),
  CONSTRAINT news_article_category_check CHECK (category ~ '^[a-z][a-z0-9_]*$'),
  CONSTRAINT news_article_notice_status_check CHECK (notice_status IN ('draft', 'published'))
);

CREATE TABLE customer.withdrawal (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  customer_id text NOT NULL,
  reasons text[] NOT NULL,
  reason_text text,
  effective_at timestamptz NOT NULL,
  identifier_fingerprints text[] NOT NULL DEFAULT '{}',
  rejoin_after timestamptz NOT NULL,
  identifier_reusable_after timestamptz,
  settled jsonb NOT NULL DEFAULT '[]',
  data_retention jsonb NOT NULL DEFAULT '[]',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT withdrawal_pkey PRIMARY KEY (tenant_id, customer_id),
  CONSTRAINT withdrawal_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT withdrawal_reasons_check CHECK (reasons <@ ARRAY['not_using', 'too_many_messages', 'privacy_concern', 'poor_service', 'found_alternative', 'other']::text[])
);

CREATE INDEX identity_verification_destination_idx ON customer.identity_verification (tenant_id, destination, requested_at);
CREATE INDEX recovery_request_identifier_fingerprint_idx ON customer.recovery_request (tenant_id, identifier_fingerprint, requested_at);
CREATE INDEX recovery_request_customer_id_idx ON customer.recovery_request (tenant_id, customer_id);
CREATE INDEX consent_record_customer_id_idx ON customer.consent_record (tenant_id, customer_id, consent_type, recorded_at);
CREATE INDEX tier_evaluation_member_program_id_idx ON customer.tier_evaluation (tenant_id, member_program_id, start_time);
CREATE INDEX customer_inquiry_customer_id_idx ON customer.customer_inquiry (tenant_id, customer_id);
CREATE INDEX customer_inquiry_inquiry_status_idx ON customer.customer_inquiry (tenant_id, inquiry_status, date_received);
CREATE INDEX comment_inquiry_id_idx ON customer.comment (tenant_id, inquiry_id, date_created);
CREATE INDEX remedy_customer_id_idx ON customer.remedy (tenant_id, customer_id);
CREATE INDEX remedy_inquiry_id_idx ON customer.remedy (tenant_id, inquiry_id);
CREATE INDEX withdrawal_identifier_fingerprints_idx ON customer.withdrawal USING gin (identifier_fingerprints);

COMMENT ON COLUMN customer.person.login_id IS
  'acom:loginId — the id the member signs in with besides the email; NULL when they sign in with the email only.';
COMMENT ON COLUMN customer.person.legal_name IS
  'schema:legalName — registered name of a business member; NULL for individuals.';
COMMENT ON COLUMN customer.person.tax_id IS
  'schema:taxID — business registration number of a business member; masked for operators; NULL for individuals.';
COMMENT ON COLUMN customer.person.business_verification IS
  'acom:businessVerification — whether the business details were checked (verify_business); turns on business pricing and tax receipts. values: business_verification';
COMMENT ON COLUMN customer.person.email_verified_at IS
  'acom:emailVerifiedAt — when the member proved they own the email; NULL when it was never verified. Recovery links go only to verified contacts.';
COMMENT ON COLUMN customer.person.telephone_verified_at IS
  'acom:telephoneVerifiedAt — when the member proved they own the phone number; NULL when it was never verified.';
COMMENT ON COLUMN customer.person.password_hash IS
  '- adaptive (argon2id) hash of the member''s password; NULL for accounts that sign in with a linked login only. Never returned or logged.';
COMMENT ON COLUMN customer.person.password_history IS
  '- adaptive hashes of the member''s earlier passwords, newest first, kept for password.history_block.';
COMMENT ON COLUMN customer.person.password_changed_at IS
  'acom:passwordChangedAt — when the password was last set; NULL when it never was.';
COMMENT ON COLUMN customer.person.failed_password_attempts IS
  'acom:failedPasswordAttempts — wrong passwords since first_failed_password_at, counted for password.lockout.';
COMMENT ON COLUMN customer.person.first_failed_password_at IS
  'acom:firstFailedPasswordAt — the first wrong password of the current lockout window; NULL when there is none.';
COMMENT ON COLUMN customer.person.locked_until IS
  'acom:lockedUntil — password sign-in and re-authentication are refused until this time (password.lockout); NULL when not locked.';
COMMENT ON COLUMN customer.person.block_reason IS
  'acom:blockReason — why an operator blocked the member (member.block_reason_required); NULL when not blocked.';
COMMENT ON COLUMN customer.person.blocked_until IS
  'acom:blockedUntil — when a block ends; NULL for a block without end or when not blocked.';
COMMENT ON COLUMN customer.person.withdrawn_at IS
  'acom:withdrawnAt — when the member withdrew; NULL for members who did not.';

COMMENT ON TABLE customer.identity_verification IS
  'schema:CheckAction/acom:IdentityVerification — one identity check by a method from customer.yml identity_verification (one-time code, link, registry lookup). id prefix ivf_.';
COMMENT ON COLUMN customer.identity_verification.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.identity_verification.id IS
  '- id with prefix ivf_; the same value the API, logs and events use.';
COMMENT ON COLUMN customer.identity_verification.purpose IS
  'acom:purpose — what the check is for: a key of identity_verification.required_for (signup, contact_change, business_verification, password_reset, login_id_recovery, withdrawal_reauth).';
COMMENT ON COLUMN customer.identity_verification.method IS
  'schema:instrument — the method id from identity_verification.methods, for example email_code.';
COMMENT ON COLUMN customer.identity_verification.destination IS
  'acom:destination — the email or phone number the code or link goes to; NULL for registry lookups.';
COMMENT ON COLUMN customer.identity_verification.customer_id IS
  'schema:agent — the member being checked (cus_ id); NULL before sign-up.';
COMMENT ON COLUMN customer.identity_verification.subject IS
  'schema:object — what a registry lookup checks: {tax_id, legal_name}; NULL for code and link methods.';
COMMENT ON COLUMN customer.identity_verification.code_hash IS
  '- SHA-256 of the one-time code; the code itself is never stored.';
COMMENT ON COLUMN customer.identity_verification.attempts IS
  'acom:attempts — wrong codes entered; at identity_verification.code.max_attempts the check fails.';
COMMENT ON COLUMN customer.identity_verification.action_status IS
  'schema:actionStatus — potential while waiting for the code, completed once verified, failed after too many attempts. values: action_status';
COMMENT ON COLUMN customer.identity_verification.requested_at IS
  'acom:requestedAt — when the code was sent (store clock); resend limits count from here.';
COMMENT ON COLUMN customer.identity_verification.expires IS
  'schema:expires — the last moment the code or link can be used.';
COMMENT ON COLUMN customer.identity_verification.verified_at IS
  'acom:verifiedAt — when the check succeeded; NULL until then.';
COMMENT ON COLUMN customer.identity_verification.token_hash IS
  '- SHA-256 of the one-time verification_token handed out on success; the token itself is never stored.';
COMMENT ON COLUMN customer.identity_verification.token_expires IS
  'acom:tokenExpires — the last moment the verification_token can be used (identity_verification.token_ttl_seconds).';
COMMENT ON COLUMN customer.identity_verification.token_used_at IS
  'acom:tokenUsedAt — when a sign-up or contact change used the token; a token works once.';
COMMENT ON COLUMN customer.identity_verification.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.identity_verification.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.identity_verification.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE customer.recovery_request IS
  'schema:Action/acom:RecoveryRequest — a password reset or sign-in id request; made the same way whether an account matches or not. id prefix rcv_.';
COMMENT ON COLUMN customer.recovery_request.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.recovery_request.id IS
  '- id with prefix rcv_; the same value the API, logs and events use (API: request_id).';
COMMENT ON COLUMN customer.recovery_request.recovery_kind IS
  'acom:recoveryKind — a password reset or a sign-in id reminder. values: recovery_kind';
COMMENT ON COLUMN customer.recovery_request.customer_id IS
  'schema:object — the account that matched (cus_ id); NULL when none did.';
COMMENT ON COLUMN customer.recovery_request.identifier_fingerprint IS
  '- SHA-256 of the identifier as typed (lowercased email, sign-in id or phone), for the per-identifier rate limit; the identifier itself is not kept.';
COMMENT ON COLUMN customer.recovery_request.session_id IS
  '- the browser session that asked (ses_ id), for the per-client rate limit.';
COMMENT ON COLUMN customer.recovery_request.method IS
  'schema:instrument — the method id from identity_verification.required_for.password_reset, for example email_link.';
COMMENT ON COLUMN customer.recovery_request.message_channel IS
  'acom:messageChannel — where the link or code went (the account''s verified contact); NULL when no account matched. values: message_channel';
COMMENT ON COLUMN customer.recovery_request.token_hash IS
  '- SHA-256 of the reset link token; the token itself is never stored.';
COMMENT ON COLUMN customer.recovery_request.code_hash IS
  '- SHA-256 of the reset code; the code itself is never stored.';
COMMENT ON COLUMN customer.recovery_request.attempts IS
  'acom:attempts — wrong codes entered; past account_recovery.confirm_max_attempts the request stops working.';
COMMENT ON COLUMN customer.recovery_request.agent_id IS
  'schema:agent — who asked: the session (ses_), the member (cus_) or an operator (adm_) through send_password_reset.';
COMMENT ON COLUMN customer.recovery_request.reason IS
  'acom:reason — why an operator sent the link; NULL for requests people made themselves.';
COMMENT ON COLUMN customer.recovery_request.requested_at IS
  'acom:requestedAt — when it was asked for (store clock); rate limits count from here.';
COMMENT ON COLUMN customer.recovery_request.expires IS
  'schema:expires — the last moment the link or code works.';
COMMENT ON COLUMN customer.recovery_request.used_at IS
  'acom:usedAt — when the new password was set with it; links and codes work once.';
COMMENT ON COLUMN customer.recovery_request.invalidated_at IS
  'acom:invalidatedAt — when a newer request or too many wrong codes made it stop working.';
COMMENT ON COLUMN customer.recovery_request.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.recovery_request.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.recovery_request.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE customer.consent_record IS
  'schema:Action/acom:ConsentRecord — one consent given or withdrawn; records are only added, never changed. id prefix cns_. API: consent.';
COMMENT ON COLUMN customer.consent_record.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.consent_record.id IS
  '- id with prefix cns_; the same value the API, logs and events use.';
COMMENT ON COLUMN customer.consent_record.customer_id IS
  'schema:agent — the member who consented (cus_ id).';
COMMENT ON COLUMN customer.consent_record.consent_type IS
  'acom:consentType — what was agreed to. values: consent_type';
COMMENT ON COLUMN customer.consent_record.granted IS
  'acom:granted — true when consent was given, false when it was withdrawn.';
COMMENT ON COLUMN customer.consent_record.recorded_at IS
  'acom:recordedAt — when the member gave or withdrew it (store clock; API at).';
COMMENT ON COLUMN customer.consent_record.agent_id IS
  'schema:agent — who recorded it: the member (cus_), an operator (adm_) or the system (sys_).';
COMMENT ON COLUMN customer.consent_record.instrument IS
  'schema:instrument — the tool or endpoint that recorded it, for example set_consent.';
COMMENT ON COLUMN customer.consent_record.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.consent_record.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.consent_record.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE customer.tier_evaluation IS
  'schema:AssessAction/acom:TierEvaluation — one run of a program''s tier evaluation and the moves it made. id prefix tev_. API: evaluation_id.';
COMMENT ON COLUMN customer.tier_evaluation.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.tier_evaluation.id IS
  '- id with prefix tev_; the same value the API, logs and events use.';
COMMENT ON COLUMN customer.tier_evaluation.member_program_id IS
  'schema:object — the program evaluated (prg_ id).';
COMMENT ON COLUMN customer.tier_evaluation.as_of IS
  'acom:asOf — the moment the members'' results were measured at.';
COMMENT ON COLUMN customer.tier_evaluation.scope IS
  'acom:scope — who was evaluated: {scope: all | segment | customers, segment_id?, customer_ids?}.';
COMMENT ON COLUMN customer.tier_evaluation.action_status IS
  'schema:actionStatus — active while it runs, completed when done, failed when undone or stopped. values: action_status';
COMMENT ON COLUMN customer.tier_evaluation.moves IS
  'acom:moves — the tier moves made: [{customer_id, from_tier_id, to_tier_id, direction, reason, from_since}], kept so the run can be undone.';
COMMENT ON COLUMN customer.tier_evaluation.summary IS
  'acom:summary — counts of the run: {up, down, grace, unchanged, coupons, notices}.';
COMMENT ON COLUMN customer.tier_evaluation.agent_id IS
  'schema:agent — who ran it: the change-set runner (sys_runner) or the scheduler (sys_scheduler).';
COMMENT ON COLUMN customer.tier_evaluation.start_time IS
  'schema:startTime — when the run started (store clock).';
COMMENT ON COLUMN customer.tier_evaluation.end_time IS
  'schema:endTime — when the run finished; NULL while it runs.';
COMMENT ON COLUMN customer.tier_evaluation.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.tier_evaluation.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.tier_evaluation.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE customer.program_membership_event IS
  'schema:UpdateAction/acom:TierChange — one change of a member''s tier, only ever added (list_tier_history). Keyed by member, program and position.';
COMMENT ON COLUMN customer.program_membership_event.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.program_membership_event.customer_id IS
  'schema:object — the member whose tier changed (cus_ id).';
COMMENT ON COLUMN customer.program_membership_event.member_program_id IS
  'schema:instrument — the program the tier belongs to (prg_ id).';
COMMENT ON COLUMN customer.program_membership_event.position IS
  '- 1, 2, 3 ... per member and program, in the order the changes happened.';
COMMENT ON COLUMN customer.program_membership_event.from_tier_id IS
  'acom:fromTierId — the tier before; NULL when the member entered the program. Not a foreign key: history outlives deleted tiers.';
COMMENT ON COLUMN customer.program_membership_event.to_tier_id IS
  'acom:toTierId — the tier after; NULL when the member left (withdrawal).';
COMMENT ON COLUMN customer.program_membership_event.cause IS
  'acom:cause — why the tier changed. values: tier_change_cause';
COMMENT ON COLUMN customer.program_membership_event.evaluation_id IS
  'acom:evaluationId — the evaluation run that made the change (tev_ id); NULL for other causes.';
COMMENT ON COLUMN customer.program_membership_event.snapshot IS
  'acom:snapshot — the result the change was based on: {metric, value}; NULL when none applies.';
COMMENT ON COLUMN customer.program_membership_event.agent_id IS
  'schema:agent — who made the change: an operator (adm_), the scheduler (sys_scheduler) or the runner (sys_runner).';
COMMENT ON COLUMN customer.program_membership_event.reason IS
  'acom:reason — the operator''s reason for a manual tier; NULL otherwise.';
COMMENT ON COLUMN customer.program_membership_event.expires IS
  'schema:expires — when a manual tier ends; NULL otherwise.';
COMMENT ON COLUMN customer.program_membership_event.changed_at IS
  'acom:changedAt — when the tier changed (store clock; API at).';
COMMENT ON COLUMN customer.program_membership_event.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.program_membership_event.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.program_membership_event.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE customer.member_program_version IS
  'schema:MemberProgram/acom:ProgramVersion — one saved definition of a tier program (define_tier_program); earlier versions stay so a change can be undone.';
COMMENT ON COLUMN customer.member_program_version.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.member_program_version.member_program_id IS
  'schema:isVariantOf — the program this is a version of (prg_ id).';
COMMENT ON COLUMN customer.member_program_version.program_version IS
  'schema:version — the program version number, 1 for the first definition.';
COMMENT ON COLUMN customer.member_program_version.definition IS
  'acom:definition — the whole definition: {name, evaluation, movement, has_tiers[]}.';
COMMENT ON COLUMN customer.member_program_version.agent_id IS
  'schema:agent — who saved it: the change-set runner (sys_runner) or an operator (adm_).';
COMMENT ON COLUMN customer.member_program_version.defined_at IS
  'acom:definedAt — when this version took effect (store clock).';
COMMENT ON COLUMN customer.member_program_version.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.member_program_version.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.member_program_version.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE customer.employee_role IS
  'schema:EmployeeRole — a member who is a store employee, with their employee discount limit. id prefix emp_. API: employee.';
COMMENT ON COLUMN customer.employee_role.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.employee_role.id IS
  '- id with prefix emp_; the same value the API, logs and events use (API employee_id).';
COMMENT ON COLUMN customer.employee_role.customer_id IS
  'schema:member — the member who is the employee (cus_ id); one employee record per member.';
COMMENT ON COLUMN customer.employee_role.employee_number IS
  'acom:employeeNumber — the store''s own number for the employee, unique in the store.';
COMMENT ON COLUMN customer.employee_role.discount_limit_value IS
  'acom:discountLimit — the most employee discount the member can use per period, in discount_limit_currency.';
COMMENT ON COLUMN customer.employee_role.discount_limit_currency IS
  'acom:discountLimit — ISO 4217 currency of discount_limit_value.';
COMMENT ON COLUMN customer.employee_role.discount_limit_period IS
  'acom:discountLimitPeriod — the period the limit renews over. values: discount_limit_period';
COMMENT ON COLUMN customer.employee_role.start_date IS
  'schema:startDate — first day the employee discount applies.';
COMMENT ON COLUMN customer.employee_role.end_date IS
  'schema:endDate — last day it applies (set when the member withdraws); NULL while it lasts.';
COMMENT ON COLUMN customer.employee_role.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.employee_role.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.employee_role.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE customer.customer_inquiry IS
  'schema:Message/acom:CustomerInquiry — a customer service inquiry and its state; the thread is in customer.comment. id prefix inq_. API: inquiry.';
COMMENT ON COLUMN customer.customer_inquiry.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.customer_inquiry.id IS
  '- id with prefix inq_; the same value the API, logs and events use.';
COMMENT ON COLUMN customer.customer_inquiry.customer_id IS
  'schema:sender — the customer who asked (cus_ id).';
COMMENT ON COLUMN customer.customer_inquiry.inquiry_type IS
  'acom:inquiryType — what the inquiry is about. values: inquiry_type';
COMMENT ON COLUMN customer.customer_inquiry.inquiry_status IS
  'acom:inquiryStatus — received, in progress with an operator, or resolved. values: inquiry_status';
COMMENT ON COLUMN customer.customer_inquiry.order_id IS
  'schema:about — the order it is about (ord_ id); NULL when none. Not a foreign key: orders belong to sales.';
COMMENT ON COLUMN customer.customer_inquiry.admin_user_id IS
  'schema:recipient — the operator handling it (adm_ id); NULL until assigned.';
COMMENT ON COLUMN customer.customer_inquiry.text IS
  'schema:text — the customer''s first message.';
COMMENT ON COLUMN customer.customer_inquiry.resolution IS
  'acom:resolution — how it was resolved, in a sentence; NULL until resolved.';
COMMENT ON COLUMN customer.customer_inquiry.date_received IS
  'schema:dateReceived — when the store received it (store clock; API created_at).';
COMMENT ON COLUMN customer.customer_inquiry.assigned_at IS
  'acom:assignedAt — when an operator was assigned; NULL until then.';
COMMENT ON COLUMN customer.customer_inquiry.resolved_at IS
  'acom:resolvedAt — when it was closed as resolved; NULL until then.';
COMMENT ON COLUMN customer.customer_inquiry.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.customer_inquiry.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.customer_inquiry.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE customer.comment IS
  'schema:Comment — one message in an inquiry thread, from the customer, an operator or the agent. id prefix cmt_.';
COMMENT ON COLUMN customer.comment.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.comment.id IS
  '- id with prefix cmt_; the same value the API, logs and events use.';
COMMENT ON COLUMN customer.comment.inquiry_id IS
  'schema:parentItem — the inquiry the message belongs to (inq_ id).';
COMMENT ON COLUMN customer.comment.author_type IS
  'acom:authorType — who wrote it. values: author_type';
COMMENT ON COLUMN customer.comment.agent_id IS
  'schema:author — who wrote it: the customer (cus_), an operator (adm_) or the system (sys_).';
COMMENT ON COLUMN customer.comment.text IS
  'schema:text — the message.';
COMMENT ON COLUMN customer.comment.message_channel IS
  'acom:messageChannel — how a reply goes to the customer (email or in_app); NULL for the customer''s own messages. values: message_channel';
COMMENT ON COLUMN customer.comment.date_created IS
  'schema:dateCreated — when it was written (store clock; API at).';
COMMENT ON COLUMN customer.comment.date_sent IS
  'schema:dateSent — when a reply reached the customer; NULL while delivery is not connected (replies are recorded, not delivered) or for messages that are not sent.';
COMMENT ON COLUMN customer.comment.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.comment.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.comment.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE customer.remedy IS
  'schema:Action/acom:Remedy — compensation for a customer (points, coupon or cash) with its liability and step-by-step approval. id prefix rmd_.';
COMMENT ON COLUMN customer.remedy.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.remedy.id IS
  '- id with prefix rmd_; the same value the API, logs and events use.';
COMMENT ON COLUMN customer.remedy.customer_id IS
  'schema:recipient — the customer compensated (cus_ id); NULL when an order-only request could not name them.';
COMMENT ON COLUMN customer.remedy.inquiry_id IS
  'schema:object — the inquiry it answers (inq_ id); NULL for an order-only request.';
COMMENT ON COLUMN customer.remedy.order_id IS
  'schema:about — the order it is about (ord_ id); NULL when none. Not a foreign key: orders belong to sales.';
COMMENT ON COLUMN customer.remedy.remedy_type IS
  'acom:remedyType — what the customer gets. values: remedy_type';
COMMENT ON COLUMN customer.remedy.liability IS
  'acom:liability — whose fault led to it. values: liability';
COMMENT ON COLUMN customer.remedy.amount_value IS
  'acom:amount — how much the remedy is worth, in amount_currency.';
COMMENT ON COLUMN customer.remedy.amount_currency IS
  'acom:amount — ISO 4217 currency of amount_value.';
COMMENT ON COLUMN customer.remedy.reason IS
  'acom:reason — why it was requested, in a sentence.';
COMMENT ON COLUMN customer.remedy.remedy_status IS
  'acom:remedyStatus — where it is in approval and payment. values: remedy_status';
COMMENT ON COLUMN customer.remedy.requested_by_id IS
  'schema:agent — who asked for it (adm_ or sys_ id).';
COMMENT ON COLUMN customer.remedy.requested_at IS
  'acom:requestedAt — when it was requested (store clock).';
COMMENT ON COLUMN customer.remedy.decided_by_id IS
  'acom:decidedBy — who approved or rejected it (the approving person''s id); NULL until decided.';
COMMENT ON COLUMN customer.remedy.decided_at IS
  'acom:decidedAt — when it was approved or rejected; NULL until decided.';
COMMENT ON COLUMN customer.remedy.decision_reason IS
  'acom:decisionReason — why it was rejected; NULL otherwise.';
COMMENT ON COLUMN customer.remedy.paid_at IS
  'acom:paidAt — when it was paid; NULL until paid.';
COMMENT ON COLUMN customer.remedy.payout IS
  'acom:payout — how it was paid: {method, to, reference, delivered}; NULL until paid. Payments to outside accounts are a stub here.';
COMMENT ON COLUMN customer.remedy.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.remedy.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.remedy.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE customer.question IS
  'schema:Question — one FAQ entry with its accepted answer, part of the store''s FAQPage. id prefix faq_. API: faq.';
COMMENT ON COLUMN customer.question.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.question.id IS
  '- id with prefix faq_; the same value the API, logs and events use (API faq_id).';
COMMENT ON COLUMN customer.question.name IS
  'schema:name — the question as shoppers ask it.';
COMMENT ON COLUMN customer.question.accepted_answer IS
  'schema:acceptedAnswer — the store''s answer.';
COMMENT ON COLUMN customer.question.category IS
  'acom:category — the FAQ section, a lowercase word the store chooses, for example returns.';
COMMENT ON COLUMN customer.question.position IS
  'schema:position — order within the section, 1 first.';
COMMENT ON COLUMN customer.question.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.question.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.question.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE customer.news_article IS
  'schema:NewsArticle — a store notice shown to shoppers once published. id prefix ntc_. API: notice.';
COMMENT ON COLUMN customer.news_article.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.news_article.id IS
  '- id with prefix ntc_; the same value the API, logs and events use (API notice_id).';
COMMENT ON COLUMN customer.news_article.headline IS
  'schema:headline — the title shoppers see.';
COMMENT ON COLUMN customer.news_article.article_body IS
  'schema:articleBody — the notice text.';
COMMENT ON COLUMN customer.news_article.category IS
  'acom:category — the notice section, a lowercase word the store chooses, for example shipping.';
COMMENT ON COLUMN customer.news_article.notice_status IS
  'acom:noticeStatus — draft until published. values: notice_status';
COMMENT ON COLUMN customer.news_article.date_published IS
  'schema:datePublished — when it was (or will be shown as) published; NULL for drafts.';
COMMENT ON COLUMN customer.news_article.is_pinned IS
  'acom:isPinned — shown above the other notices.';
COMMENT ON COLUMN customer.news_article.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.news_article.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.news_article.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE customer.withdrawal IS
  'schema:Action/acom:Withdrawal — a member leaving the store: why, what was settled, what is kept and until when, and the one-way fingerprints that block rejoining and id reuse for a while.';
COMMENT ON COLUMN customer.withdrawal.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN customer.withdrawal.customer_id IS
  'schema:agent — the member who withdrew (cus_ id); one withdrawal per member.';
COMMENT ON COLUMN customer.withdrawal.reasons IS
  'acom:reasons — the reasons the member picked; every element is a word from the list. values: withdrawal_reason';
COMMENT ON COLUMN customer.withdrawal.reason_text IS
  'acom:reasonText — what the member wrote, up to 500 characters; NULL when nothing.';
COMMENT ON COLUMN customer.withdrawal.effective_at IS
  'acom:effectiveAt — when the withdrawal took effect (store clock).';
COMMENT ON COLUMN customer.withdrawal.identifier_fingerprints IS
  '- SHA-256 of the email, phone and sign-in id the member used; the only contact data kept, for withdrawal.rejoin and withdrawal.identifier_reuse.';
COMMENT ON COLUMN customer.withdrawal.rejoin_after IS
  'acom:rejoinAfter — the same person can sign up again from this time (withdrawal.rejoin.block_days).';
COMMENT ON COLUMN customer.withdrawal.identifier_reusable_after IS
  'acom:identifierReusableAfter — the email and sign-in id can be used again from this time; NULL when never (withdrawal.identifier_reuse).';
COMMENT ON COLUMN customer.withdrawal.settled IS
  'acom:settled — what was settled: [{type, action, amount?, count?}].';
COMMENT ON COLUMN customer.withdrawal.data_retention IS
  'acom:dataRetention — what is kept and until when: [{data, until, why}].';
COMMENT ON COLUMN customer.withdrawal.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN customer.withdrawal.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN customer.withdrawal.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE customer.identity_verification ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.identity_verification FORCE ROW LEVEL SECURITY;
CREATE POLICY identity_verification_tenant_isolation ON customer.identity_verification
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE customer.recovery_request ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.recovery_request FORCE ROW LEVEL SECURITY;
CREATE POLICY recovery_request_tenant_isolation ON customer.recovery_request
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE customer.consent_record ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.consent_record FORCE ROW LEVEL SECURITY;
CREATE POLICY consent_record_tenant_isolation ON customer.consent_record
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE customer.tier_evaluation ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.tier_evaluation FORCE ROW LEVEL SECURITY;
CREATE POLICY tier_evaluation_tenant_isolation ON customer.tier_evaluation
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE customer.program_membership_event ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.program_membership_event FORCE ROW LEVEL SECURITY;
CREATE POLICY program_membership_event_tenant_isolation ON customer.program_membership_event
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE customer.member_program_version ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.member_program_version FORCE ROW LEVEL SECURITY;
CREATE POLICY member_program_version_tenant_isolation ON customer.member_program_version
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE customer.employee_role ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.employee_role FORCE ROW LEVEL SECURITY;
CREATE POLICY employee_role_tenant_isolation ON customer.employee_role
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE customer.customer_inquiry ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.customer_inquiry FORCE ROW LEVEL SECURITY;
CREATE POLICY customer_inquiry_tenant_isolation ON customer.customer_inquiry
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE customer.comment ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.comment FORCE ROW LEVEL SECURITY;
CREATE POLICY comment_tenant_isolation ON customer.comment
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE customer.remedy ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.remedy FORCE ROW LEVEL SECURITY;
CREATE POLICY remedy_tenant_isolation ON customer.remedy
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE customer.question ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.question FORCE ROW LEVEL SECURITY;
CREATE POLICY question_tenant_isolation ON customer.question
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE customer.news_article ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.news_article FORCE ROW LEVEL SECURITY;
CREATE POLICY news_article_tenant_isolation ON customer.news_article
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE customer.withdrawal ENABLE ROW LEVEL SECURITY;
ALTER TABLE customer.withdrawal FORCE ROW LEVEL SECURITY;
CREATE POLICY withdrawal_tenant_isolation ON customer.withdrawal
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- DELETE where removing a row is part of the design (see 0014_app_delete_grants.sql):
--   customer.member_program_tier   define_tier_program removes tiers; its undo removes added ones
--   customer.program_membership    a withdrawn member leaves the program (withdrawal.on_withdraw.tier)
--   customer.employee_role         undoing register_employee
--   customer.remedy                undoing request_remedy before anyone decided it
-- Consent records, tier history, inquiries and their threads are only ever added to.
GRANT DELETE ON
  customer.member_program_tier,
  customer.program_membership,
  customer.employee_role,
  customer.remedy
TO acom_app;
