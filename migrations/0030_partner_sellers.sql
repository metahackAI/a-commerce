-- 0030_partner_sellers.sql
-- Partner: marketplace sellers that list in the store (schema.org Organization), the products
-- each one sells, their status history, contracts kept as numbered versions, and grades
-- (docs/api/partner.md, policies/partner.yml).
-- Rules: docs/backend/database.md. Order: schema, tables, indexes, comments, row-level security.

CREATE SCHEMA IF NOT EXISTS partner;

COMMENT ON SCHEMA partner IS
  'Marketplace sellers (Organization), the products they sell, their status history, contracts with numbered versions, and grades.';

CREATE TABLE partner.seller (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  legal_name text NOT NULL,
  tax_id text NOT NULL,
  seller_status text NOT NULL DEFAULT 'applied',
  seller_type text NOT NULL,
  address jsonb NOT NULL,
  contact_point jsonb NOT NULL DEFAULT '[]',
  business_category text[] NOT NULL DEFAULT '{}',
  documents jsonb NOT NULL DEFAULT '[]',
  organization_id text,
  applied_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT seller_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT seller_id_check CHECK (id ~ '^slr_[a-z0-9_]+$'),
  CONSTRAINT seller_legal_name_check CHECK (char_length(legal_name) BETWEEN 1 AND 200),
  CONSTRAINT seller_tax_id_check CHECK (char_length(tax_id) BETWEEN 1 AND 40),
  CONSTRAINT seller_seller_status_check CHECK (seller_status IN ('applied', 'under_review', 'approved', 'active', 'suspended', 'terminated')),
  CONSTRAINT seller_seller_type_check CHECK (seller_type IN ('marketplace', 'consignment', 'direct_supply')),
  CONSTRAINT seller_organization_id_fkey FOREIGN KEY (tenant_id, organization_id) REFERENCES platform.organization (tenant_id, id),
  CONSTRAINT seller_organization_id_key UNIQUE (tenant_id, organization_id)
);

CREATE TABLE partner.seller_event (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  seller_id text NOT NULL,
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
  CONSTRAINT seller_event_pkey PRIMARY KEY (tenant_id, seller_id, position),
  CONSTRAINT seller_event_seller_id_fkey FOREIGN KEY (tenant_id, seller_id) REFERENCES partner.seller (tenant_id, id),
  CONSTRAINT seller_event_position_check CHECK (position >= 1),
  CONSTRAINT seller_event_action_status_check CHECK (action_status IN ('potential', 'active', 'completed', 'failed')),
  CONSTRAINT seller_event_from_state_check CHECK (from_state IN ('applied', 'under_review', 'approved', 'active', 'suspended', 'terminated')),
  CONSTRAINT seller_event_to_state_check CHECK (to_state IN ('applied', 'under_review', 'approved', 'active', 'suspended', 'terminated'))
);

CREATE TABLE partner.seller_product_group (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  product_group_id text NOT NULL,
  seller_id text NOT NULL,
  hidden_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT seller_product_group_pkey PRIMARY KEY (tenant_id, product_group_id),
  CONSTRAINT seller_product_group_product_group_id_fkey FOREIGN KEY (tenant_id, product_group_id) REFERENCES catalog.product_group (tenant_id, id),
  CONSTRAINT seller_product_group_seller_id_fkey FOREIGN KEY (tenant_id, seller_id) REFERENCES partner.seller (tenant_id, id)
);

CREATE TABLE partner.seller_contract (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  seller_id text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT seller_contract_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT seller_contract_id_check CHECK (id ~ '^ctr_[a-z0-9_]+$'),
  CONSTRAINT seller_contract_seller_id_fkey FOREIGN KEY (tenant_id, seller_id) REFERENCES partner.seller (tenant_id, id),
  CONSTRAINT seller_contract_seller_id_key UNIQUE (tenant_id, seller_id)
);

CREATE TABLE partner.seller_contract_version (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  seller_contract_id text NOT NULL,
  version_number integer NOT NULL,
  default_commission_rate numeric(5,4) NOT NULL,
  commission_rate jsonb NOT NULL DEFAULT '[]',
  settlement_cycle text NOT NULL,
  settlement_method text NOT NULL,
  payout_account_ref text NOT NULL,
  valid_from timestamptz NOT NULL,
  valid_through timestamptz NOT NULL,
  agent_id text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT seller_contract_version_pkey PRIMARY KEY (tenant_id, seller_contract_id, version_number),
  CONSTRAINT seller_contract_version_seller_contract_id_fkey FOREIGN KEY (tenant_id, seller_contract_id) REFERENCES partner.seller_contract (tenant_id, id),
  CONSTRAINT seller_contract_version_version_number_check CHECK (version_number >= 1),
  CONSTRAINT seller_contract_version_default_commission_rate_check CHECK (default_commission_rate BETWEEN 0 AND 1),
  CONSTRAINT seller_contract_version_settlement_cycle_check CHECK (settlement_cycle IN ('weekly', 'biweekly', 'monthly')),
  CONSTRAINT seller_contract_version_settlement_method_check CHECK (settlement_method IN ('on_delivery', 'on_confirmation')),
  CONSTRAINT seller_contract_version_payout_account_ref_check CHECK (char_length(payout_account_ref) BETWEEN 4 AND 80),
  CONSTRAINT seller_contract_version_period_check CHECK (valid_through > valid_from)
);

CREATE TABLE partner.seller_grade (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  seller_id text NOT NULL,
  position integer NOT NULL,
  grade_level text NOT NULL,
  computed_grade_level text,
  adjustment_reason text,
  metrics jsonb NOT NULL DEFAULT '{}',
  valid_from timestamptz NOT NULL,
  valid_through timestamptz NOT NULL,
  agent_id text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT seller_grade_pkey PRIMARY KEY (tenant_id, seller_id, position),
  CONSTRAINT seller_grade_seller_id_fkey FOREIGN KEY (tenant_id, seller_id) REFERENCES partner.seller (tenant_id, id),
  CONSTRAINT seller_grade_position_check CHECK (position >= 1),
  CONSTRAINT seller_grade_grade_level_check CHECK (grade_level IN ('platinum', 'gold', 'silver', 'bronze', 'watch')),
  CONSTRAINT seller_grade_computed_grade_level_check CHECK (computed_grade_level IN ('platinum', 'gold', 'silver', 'bronze', 'watch')),
  CONSTRAINT seller_grade_period_check CHECK (valid_through > valid_from)
);

CREATE INDEX seller_seller_status_idx ON partner.seller (tenant_id, seller_status);
CREATE INDEX seller_tax_id_idx ON partner.seller (tenant_id, tax_id);
CREATE INDEX seller_product_group_seller_id_idx ON partner.seller_product_group (tenant_id, seller_id);
CREATE INDEX seller_grade_valid_from_idx ON partner.seller_grade (tenant_id, seller_id, valid_from);

COMMENT ON TABLE partner.seller IS
  'schema:Organization/acom:Seller — a business that sells in the store under a contract, from application to termination; products and orders name it by the same id. id prefix slr_. API: seller.';
COMMENT ON COLUMN partner.seller.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN partner.seller.id IS
  '- seller id (slr_), API seller_id.';
COMMENT ON COLUMN partner.seller.legal_name IS
  'schema:legalName — the registered name of the business, for example Harbor Goods LLC.';
COMMENT ON COLUMN partner.seller.tax_id IS
  'schema:taxID — business registration (tax) number as the seller gave it; sensitive, so operators see it masked (partner.yml privacy).';
COMMENT ON COLUMN partner.seller.seller_status IS
  'acom:sellerStatus — where the seller is between application and termination; transitions follow partner.yml lifecycle.allowed_transitions. values: seller_status';
COMMENT ON COLUMN partner.seller.seller_type IS
  'acom:sellerType — how the seller sells in the store. values: seller_type';
COMMENT ON COLUMN partner.seller.address IS
  'schema:address — business address: {street_address, address_locality, address_region, postal_code, address_country}.';
COMMENT ON COLUMN partner.seller.contact_point IS
  'schema:contactPoint — people to reach: [{contact_type (settlement, customer_service, product), name, email, telephone}].';
COMMENT ON COLUMN partner.seller.business_category IS
  'acom:businessCategory — standard categories (cat_ ids) the seller sells in; checked against partner.yml onboarding.prohibited_categories.';
COMMENT ON COLUMN partner.seller.documents IS
  'acom:documents — papers attached to the application: [{document_type, upload_id, name}]; document_type is a seller_document_type word.';
COMMENT ON COLUMN partner.seller.organization_id IS
  'acom:vendorOrganization — the vendor organization (org_ id) whose operator accounts act for this seller; NULL when the seller has no back-office accounts.';
COMMENT ON COLUMN partner.seller.applied_at IS
  'acom:appliedAt — when the seller applied to sell in the store.';
COMMENT ON COLUMN partner.seller.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN partner.seller.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN partner.seller.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE partner.seller_event IS
  'schema:Action — one state change of a seller, with the reason in result; a change booked for later waits as potential until its time. Rows are only added.';
COMMENT ON COLUMN partner.seller_event.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN partner.seller_event.seller_id IS
  'acom:sellerId — the seller that changed (slr_ id), the object of the event.';
COMMENT ON COLUMN partner.seller_event.position IS
  'schema:position — 1-based order of this event within its seller.';
COMMENT ON COLUMN partner.seller_event.action_type IS
  'acom:actionType — the schema.org Action subtype (@type), for example CreateAction or UpdateAction.';
COMMENT ON COLUMN partner.seller_event.action_status IS
  'schema:actionStatus — lifecycle of the action; potential while a booked change waits for its time. values: action_status';
COMMENT ON COLUMN partner.seller_event.agent_id IS
  'schema:agent — person or system that acted (adm_ or sys_ id); NULL when only a model acted.';
COMMENT ON COLUMN partner.seller_event.agent_name IS
  'schema:agent — name of the software agent (model) that acted; NULL for people.';
COMMENT ON COLUMN partner.seller_event.instrument IS
  'schema:instrument — tool name or scheduled job that caused the event.';
COMMENT ON COLUMN partner.seller_event.from_state IS
  'acom:fromState — status before the event; NULL for the creating event. values: seller_status';
COMMENT ON COLUMN partner.seller_event.to_state IS
  'acom:toState — status after the event. values: seller_status';
COMMENT ON COLUMN partner.seller_event.result IS
  'schema:result — what the action recorded, as JSON: {reason, products_hidden[], products_restored[]}.';
COMMENT ON COLUMN partner.seller_event.error IS
  'schema:error — why a booked change did not happen, as {code, class, message}; NULL otherwise.';
COMMENT ON COLUMN partner.seller_event.start_time IS
  'schema:startTime — when the change takes (or took) effect.';
COMMENT ON COLUMN partner.seller_event.end_time IS
  'schema:endTime — when the change was applied; NULL while it waits.';
COMMENT ON COLUMN partner.seller_event.request_id IS
  '- request trace id (req_) from the gateway; joins agent turns and the action log.';
COMMENT ON COLUMN partner.seller_event.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN partner.seller_event.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN partner.seller_event.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE partner.seller_product_group IS
  'acom:SellerProduct — a product (ProductGroup) that a seller sells in the store (schema:offeredBy); one seller per product.';
COMMENT ON COLUMN partner.seller_product_group.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN partner.seller_product_group.product_group_id IS
  'acom:productGroupId — the product (prd_ id; API product_id).';
COMMENT ON COLUMN partner.seller_product_group.seller_id IS
  'schema:offeredBy — the seller (slr_ id) that sells the product.';
COMMENT ON COLUMN partner.seller_product_group.hidden_at IS
  'acom:hiddenAt — when the seller''s suspension took the product off sale; NULL while it is not hidden for the seller.';
COMMENT ON COLUMN partner.seller_product_group.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN partner.seller_product_group.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN partner.seller_product_group.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE partner.seller_contract IS
  'schema:Intangible/acom:SellerContract — the agreement between the store and one seller; its terms are numbered versions in seller_contract_version, which are only added. id prefix ctr_. API: contract.';
COMMENT ON COLUMN partner.seller_contract.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN partner.seller_contract.id IS
  '- contract id (ctr_), API contract_id; it stays the same across versions.';
COMMENT ON COLUMN partner.seller_contract.seller_id IS
  'acom:sellerId — the seller (slr_ id) the contract is with; one contract per seller.';
COMMENT ON COLUMN partner.seller_contract.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN partner.seller_contract.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN partner.seller_contract.version IS
  '- row version, +1 on every change (each new contract version); previews record it in basis and execution compares it.';

COMMENT ON TABLE partner.seller_contract_version IS
  'schema:Intangible/acom:SellerContractTerms — one numbered version of a seller contract: commission, settlement and payout terms for a period; versions of one contract never overlap (contract.no_period_overlap).';
COMMENT ON COLUMN partner.seller_contract_version.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN partner.seller_contract_version.seller_contract_id IS
  'acom:sellerContractId — the contract (ctr_ id) this version belongs to.';
COMMENT ON COLUMN partner.seller_contract_version.version_number IS
  'schema:version — 1-based number of this version within its contract (API version).';
COMMENT ON COLUMN partner.seller_contract_version.default_commission_rate IS
  'acom:defaultCommissionRate — the store''s commission on a sale in a category without its own rate, 0-1 within partner.yml commission.';
COMMENT ON COLUMN partner.seller_contract_version.commission_rate IS
  'acom:commissionRate — commission rates by category: [{category_id, rate}], rate 0-1; categories not listed use the default rate.';
COMMENT ON COLUMN partner.seller_contract_version.settlement_cycle IS
  'acom:settlementCycle — how often the seller is paid out. values: settlement_cycle';
COMMENT ON COLUMN partner.seller_contract_version.settlement_method IS
  'acom:settlementMethod — when a sale becomes payable to the seller. values: settlement_method';
COMMENT ON COLUMN partner.seller_contract_version.payout_account_ref IS
  'acom:payoutAccountRef — the payout provider''s reference for the seller''s verified bank account, never the account number; operators see it masked.';
COMMENT ON COLUMN partner.seller_contract_version.valid_from IS
  'schema:validFrom — first moment the terms apply.';
COMMENT ON COLUMN partner.seller_contract_version.valid_through IS
  'schema:validThrough — last moment the terms apply.';
COMMENT ON COLUMN partner.seller_contract_version.agent_id IS
  'schema:agent — person or system that set the version (adm_ or sys_ id); NULL for seed rows.';
COMMENT ON COLUMN partner.seller_contract_version.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN partner.seller_contract_version.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN partner.seller_contract_version.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE partner.seller_grade IS
  'schema:Intangible/acom:SellerGrade — a seller grade a person confirmed or adjusted for a period, with the computed metrics it was based on; the latest one covering a moment is the grade then.';
COMMENT ON COLUMN partner.seller_grade.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN partner.seller_grade.seller_id IS
  'acom:sellerId — the graded seller (slr_ id).';
COMMENT ON COLUMN partner.seller_grade.position IS
  'schema:position — 1-based order of this grade within its seller.';
COMMENT ON COLUMN partner.seller_grade.grade_level IS
  'acom:gradeLevel — the grade that applies in the period. values: grade_level';
COMMENT ON COLUMN partner.seller_grade.computed_grade_level IS
  'acom:computedGradeLevel — the grade the metrics gave at the time; NULL when the review period had no orders. values: grade_level';
COMMENT ON COLUMN partner.seller_grade.adjustment_reason IS
  'acom:adjustmentReason — why the grade differs from the computed one (grade.override_requires_reason); NULL when it was confirmed as computed.';
COMMENT ON COLUMN partner.seller_grade.metrics IS
  'acom:gradeMetrics — the computed rates behind the grade: {shipping_compliance_rate, order_fulfillment_rate, claim_rate, service_score, review_period}, rates 0-1.';
COMMENT ON COLUMN partner.seller_grade.valid_from IS
  'schema:validFrom — first moment the grade applies.';
COMMENT ON COLUMN partner.seller_grade.valid_through IS
  'schema:validThrough — last moment the grade applies.';
COMMENT ON COLUMN partner.seller_grade.agent_id IS
  'schema:agent — person or system that set the grade (adm_ or sys_ id); NULL for seed rows.';
COMMENT ON COLUMN partner.seller_grade.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN partner.seller_grade.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN partner.seller_grade.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE partner.seller ENABLE ROW LEVEL SECURITY;
ALTER TABLE partner.seller FORCE ROW LEVEL SECURITY;
CREATE POLICY seller_tenant_isolation ON partner.seller
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE partner.seller_event ENABLE ROW LEVEL SECURITY;
ALTER TABLE partner.seller_event FORCE ROW LEVEL SECURITY;
CREATE POLICY seller_event_tenant_isolation ON partner.seller_event
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE partner.seller_product_group ENABLE ROW LEVEL SECURITY;
ALTER TABLE partner.seller_product_group FORCE ROW LEVEL SECURITY;
CREATE POLICY seller_product_group_tenant_isolation ON partner.seller_product_group
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE partner.seller_contract ENABLE ROW LEVEL SECURITY;
ALTER TABLE partner.seller_contract FORCE ROW LEVEL SECURITY;
CREATE POLICY seller_contract_tenant_isolation ON partner.seller_contract
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE partner.seller_contract_version ENABLE ROW LEVEL SECURITY;
ALTER TABLE partner.seller_contract_version FORCE ROW LEVEL SECURITY;
CREATE POLICY seller_contract_version_tenant_isolation ON partner.seller_contract_version
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE partner.seller_grade ENABLE ROW LEVEL SECURITY;
ALTER TABLE partner.seller_grade FORCE ROW LEVEL SECURITY;
CREATE POLICY seller_grade_tenant_isolation ON partner.seller_grade
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- The app role reads and writes these through the default privileges of infra/postgres/init.sql.
-- Undo removes what these writes created:
--   partner.seller, partner.seller_event   undoing onboard_seller (an application that never was)
--   partner.seller_contract(_version)      undoing set_contract (versions are otherwise only added)
--   partner.seller_grade                   undoing set_seller_grade
GRANT DELETE ON
  partner.seller,
  partner.seller_event,
  partner.seller_contract,
  partner.seller_contract_version,
  partner.seller_grade
TO acom_app;
