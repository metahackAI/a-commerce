# 컬럼 사전 (생성됨)

> `cargo xtask check-schema --write`가 `migrations/*.sql`에서 만든 파일입니다. 손으로 고치지 말고 마이그레이션의 주석을 고치세요. 규칙: [database.md](database.md).

## `agent`

### `agent.conversation`

`schema:Conversation` — one chat between a person and the agent runtime, on the back office (bo) or the storefront (fo). id prefix conv_. · id `conv_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix conv_; the same value the API, logs and events use. |
| `surface` | text |  | `acom:surface` | bo for operators, fo for shoppers. values: chat_surface Values: `bo` · `fo`. |
| `customer_id` | text | ✓ | `schema:customer` | shopper in the conversation (cus_ id), NULL on bo and for guests; plain text because the agent service owns this schema. |
| `admin_id` | text | ✓ | `acom:adminId` | operator in the conversation (adm_ id), NULL on fo; plain text like customer_id. |
| `session_id` | text | ✓ | `acom:sessionId` | gateway session the conversation runs in (ses_ id). |
| `name` | text | ✓ | `schema:name` | short title for history lists. |
| `agent_profile` | text | ✓ | `acom:agentProfile` | specialized agent last routed to, for example promotion or shopper. |
| `started_at` | timestamptz |  | `acom:startedAt` | first turn. |
| `last_turn_at` | timestamptz | ✓ | `acom:lastTurnAt` | latest turn. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `agent.model_call`

`schema:Action` — one call to a language model, with tokens, latency and cost for evaluation.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `turn_id` | text |  | `schema:isPartOf` | the turn (turn_ id). → `agent.turn` |
| `position` | integer |  | `schema:position` | call order in the turn. |
| `step_id` | text | ✓ | `acom:stepId` | step that made the call (step_ id); NULL for calls outside a step. → `agent.step` |
| `model` | text |  | `acom:model` | model name, for example muse-glimmer-30b. |
| `purpose` | text |  | `acom:purpose` | why it was called, for example route, plan, arguments or reply. |
| `input_tokens` | integer |  | `acom:inputTokens` | tokens sent. |
| `output_tokens` | integer |  | `acom:outputTokens` | tokens received. |
| `latency_ms` | integer |  | `acom:latency` | time to the full response, in milliseconds. |
| `cost_value` | numeric(12,2) | ✓ | `acom:cost` | price of the call from agents/models.yml, in major units of the currency. |
| `cost_currency` | char(3) | ✓ | `acom:cost` | ISO 4217 code of cost_value. |
| `succeeded` | boolean |  | `acom:succeeded` | false when the call errored or its output failed validation. |
| `error` | jsonb | ✓ | `schema:error` | {code, class, message} when it failed. |
| `started_at` | timestamptz |  | `acom:startedAt` | when the call was sent. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `agent.step`

`schema:Action` — one step of a turn: routing, tool search, planning, a tool call or an intent check. id prefix step_. · id `step_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix step_; the same value the API, logs and events use. |
| `turn_id` | text |  | `schema:isPartOf` | the turn (turn_ id). → `agent.turn` |
| `position` | integer |  | `schema:position` | step order in the turn. |
| `kind` | text |  | `acom:stepKind` | what the step does. values: step_kind Values: `route` · `search_tools` · `plan` · `call_tool` · `check`. |
| `instrument` | text | ✓ | `schema:instrument` | tool called, for call_tool steps. |
| `action_status` | text |  | `schema:actionStatus` | state of the step. values: action_status Values: `potential` · `active` · `completed` · `failed`. |
| `summary` | text | ✓ | `acom:summary` | one-line description for traces and evaluation. |
| `input` | jsonb | ✓ | `acom:input` | step input, for example corrected tool arguments. |
| `output` | jsonb | ✓ | `acom:output` | step output, for example the tool result or the plan. |
| `preview_id` | text | ✓ | `acom:previewId` | preview the step created (pv_ id). |
| `change_set_id` | text | ✓ | `acom:changeSetId` | change set the step attached to (cs_ id). |
| `error` | jsonb | ✓ | `schema:error` | {code, class, message} when the step failed. |
| `started_at` | timestamptz |  | `acom:startedAt` | when the step started. |
| `ended_at` | timestamptz | ✓ | `acom:endedAt` | when it ended. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `agent.turn`

`schema:Message` — one request and its answer inside a conversation. id prefix turn_. · id `turn_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix turn_; the same value the API, logs and events use. |
| `conversation_id` | text |  | `acom:conversationId` | the conversation (conv_ id). → `agent.conversation` |
| `position` | integer |  | `schema:position` | turn number in the conversation. |
| `text` | text |  | `schema:text` | what the person said, or the intent a UI button sent. |
| `context` | jsonb |  | `acom:context` | screen elements the person pointed at: [{element_ref, ids}]. |
| `agent_profile` | text |  | `acom:agentProfile` | specialized agent that handled the turn. |
| `reply_text` | text | ✓ | `acom:replyText` | short text answer shown with the blocks. |
| `blocks` | jsonb |  | `acom:blocks` | UI blocks returned: [{block, schema, data}]. |
| `usage` | jsonb |  | `acom:usage` | {model_calls, tool_calls, input_tokens, output_tokens, latency_ms, cost: {value, currency}}. |
| `request_id` | text | ✓ | `-` | request trace id (req_); joins the commerce action log. |
| `error` | jsonb | ✓ | `schema:error` | why the turn stopped early: {code, class, message}. |
| `started_at` | timestamptz |  | `acom:startedAt` | when the request arrived. |
| `ended_at` | timestamptz | ✓ | `acom:endedAt` | when the answer finished streaming. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

## `analytics`

### `analytics.daily_sales` (view)

`schema:Observation/acom:DailySales` — revenue and order count per store and day, with the day taken in the store's time zone; orders still awaiting payment and cancelled orders are left out.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` |  | ✓ | `-` | store (tenant) the figures belong to. |
| `observation_date` |  | ✓ | `schema:observationDate` | calendar day in the store's time zone on which the orders were placed. |
| `revenue_value` |  | ✓ | `acom:revenue` | sum of total_payment_due of the day's orders (after discounts, including shipping), in major units. |
| `revenue_currency` |  | ✓ | `acom:revenue` | ISO 4217 code of revenue_value. |
| `order_count` |  | ✓ | `acom:orderCount` | number of orders placed that day that were paid and not cancelled. |

## `catalog`

### `catalog.brand`

`schema:Brand` — a brand products are sold under. id prefix brn_. · id `brn_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix brn_; the same value the API, logs and events use. |
| `name` | text |  | `schema:name` | brand name. |
| `logo` | text | ✓ | `schema:logo` | URL of the logo image. |
| `description` | text | ✓ | `schema:description` | short brand description. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `catalog.measurement`

`schema:QuantitativeValue` — one garment measurement of one size (hasMeasurement), for fit checks.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `product_group_id` | text |  | `acom:productGroupId` | product measured (prd_ id). → `catalog.product_group` |
| `size` | text |  | `schema:size` | size the measurement is for, a value of the product's size axis. |
| `name` | text |  | `schema:name` | what was measured. values: measurement_name Values: `chest` · `shoulder` · `sleeve` · `length` · `waist` · `hip` · `inseam` · `rise` · `foot_length`. |
| `value` | numeric(6,1) |  | `schema:value` | the measurement, one decimal place. |
| `unit_text` | text |  | `schema:unitText` | unit of value, for example cm. |
| `measurement_technique` | text | ✓ | `schema:measurementTechnique` | how it was measured, for example garment laid flat, chest armpit to armpit. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `catalog.merchant_return_policy`

`schema:MerchantReturnPolicy` — return terms products point to; the window comes from fulfillment.yml return_window_days. id prefix mrp_. · id `mrp_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix mrp_; the same value the API, logs and events use. |
| `name` | text |  | `schema:name` | policy name for operators, for example Standard 30 days. |
| `merchant_return_days` | integer |  | `schema:merchantReturnDays` | days after delivery a return can be requested. |
| `return_fees` | text |  | `schema:returnFees` | who pays return shipping for a change of mind. values: return_fees Values: `free_return` · `return_fees_customer_responsibility` · `return_shipping_fees`. |
| `return_method` | text[] |  | `schema:returnMethod` | ways to send items back; words from fulfillment.yml return.send_back_methods. |
| `refund_type` | text |  | `schema:refundType` | what the shopper gets back. values: refund_type Values: `full_refund` · `exchange_refund` · `store_credit_refund`. |
| `size_exchange_days` | integer | ✓ | `acom:sizeExchangeDays` | days a free size exchange is offered; NULL when not offered. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `catalog.offer`

`schema:Offer` — list price, availability and stock of one variant; cost is for operators only. id prefix off_. · id `off_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix off_; the same value the API, logs and events use. |
| `product_id` | text |  | `schema:itemOffered` | the variant offered (var_ id; API variant_id). → `catalog.product` |
| `price` | numeric(12,2) |  | `schema:price` | list price per unit; member and sale prices are promotion.unit_price_specification rows. |
| `price_currency` | char(3) |  | `schema:priceCurrency` | ISO 4217 code of price. |
| `availability` | text |  | `schema:availability` | whether the variant can be bought now. values: availability Values: `in_stock` · `out_of_stock` · `pre_order` · `back_order` · `discontinued`. |
| `inventory_level` | integer |  | `schema:inventoryLevel` | units that can be sold now, across all locations. |
| `safety_stock` | integer |  | `acom:safetyStock` | buffer level that set_safety_stock alerts at; catalog.md says it does not change availability. |
| `availability_starts` | timestamptz | ✓ | `schema:availabilityStarts` | when sold-out stock is expected back, for example Nov 12; NULL when unknown. |
| `sold_out_at` | timestamptz | ✓ | `acom:soldOutAt` | when the variant last ran out; NULL while in stock. |
| `valid_from` | timestamptz | ✓ | `schema:validFrom` | first moment the offer is valid; NULL means already valid. |
| `valid_through` | timestamptz | ✓ | `schema:validThrough` | last moment the offer is valid (inclusive); NULL means open-ended. |
| `cost_value` | numeric(12,2) | ✓ | `acom:inventoryCost` | what one unit costs the store; operators only, never shown to shoppers; the price_floor base, in major units of the currency. |
| `cost_currency` | char(3) | ✓ | `acom:inventoryCost` | ISO 4217 code of cost_value. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `catalog.product`

`schema:Product` — one sellable variant (a size and color of a product); cart lines, stock and order items use it. id prefix var_. API: variant_id. · id `var_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix var_; the same value the API, logs and events use. |
| `product_group_id` | text |  | `schema:isVariantOf` | product this variant belongs to (prd_ id). → `catalog.product_group` |
| `sku` | text |  | `schema:sku` | stock keeping unit, unique in the store, for example wb-slate-m. |
| `gtin` | text | ✓ | `schema:gtin` | barcode number when the product has one. |
| `name` | text |  | `schema:name` | variant name, for example Packable windbreaker, Slate, M. |
| `color` | text | ✓ | `schema:color` | color name, for example Slate; NULL when the product has one color. |
| `size` | text | ✓ | `schema:size` | size word, for example M, 32 or 9; NULL for one-size products. |
| `weight_value` | numeric(8,1) | ✓ | `schema:weight` | weight of this variant. |
| `weight_unit_text` | text | ✓ | `schema:weight` | unit of weight_value, for example g. |
| `image` | text | ✓ | `schema:image` | URL of the variant image. |
| `sale_type` | text |  | `acom:saleType` | how the variant is sold. values: sale_type Values: `always` · `pre_order` · `subscription` · `gift_card`. |
| `is_managed` | boolean |  | `acom:managed` | true when stock is tracked and can run out. |
| `additional_property` | jsonb |  | `schema:additionalProperty` | variant-only attributes: [{name, value, unit_text}]. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `catalog.product_group`

`schema:ProductGroup` — a product as people talk about it; its variants are catalog.product rows. id prefix prd_. API: product_id. · id `prd_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix prd_; the same value the API, logs and events use. |
| `name` | text |  | `schema:name` | product name, for example Packable windbreaker. |
| `description` | text | ✓ | `schema:description` | product description in the store's tone. |
| `brand_id` | text | ✓ | `schema:brand` | brand (brn_ id). → `catalog.brand` |
| `approval_status` | text |  | `acom:approvalStatus` | catalog review state; only active products are shown to shoppers. values: approval_status Values: `draft` · `pending_review` · `active` · `rejected` · `discontinued`. |
| `varies_by` | text[] |  | `schema:variesBy` | option axes of the variants, for example {color,size}. |
| `material` | text | ✓ | `schema:material` | main material shown to shoppers, for example Recycled nylon. |
| `keywords` | text[] |  | `schema:keywords` | product tags used by search filters, for example {packable,bestseller}. |
| `additional_property` | jsonb |  | `schema:additionalProperty` | comparison attributes: [{name, value, unit_text}], names from search.yml compare.attributes (weight, packed_size, packing, warmth, rain). |
| `image` | text | ✓ | `schema:image` | URL of the main image. |
| `has_merchant_return_policy_id` | text | ✓ | `schema:hasMerchantReturnPolicy` | return policy (mrp_ id); NULL means the store default. → `catalog.merchant_return_policy` |
| `release_date` | date | ✓ | `schema:releaseDate` | first day on sale; sorts newest first. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `catalog.review`

`schema:Review` — a shopper review of a product with a fit vote; hidden reviews are left out of summaries. id prefix rev_. · id `rev_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix rev_; the same value the API, logs and events use. |
| `product_group_id` | text |  | `schema:itemReviewed` | product reviewed (prd_ id). → `catalog.product_group` |
| `product_id` | text | ✓ | `acom:variantBought` | variant the reviewer bought (var_ id); gives the size bought. → `catalog.product` |
| `author_id` | text | ✓ | `schema:author` | reviewer (cus_ id); NULL for imported reviews. → `customer.person` |
| `author_name` | text |  | `schema:author` | name shown with the review, for example Chris T. |
| `order_id` | text | ✓ | `acom:orderId` | order the review was written for (ord_ id); checked by review.verified_purchase_only. |
| `rating_value` | smallint |  | `schema:ratingValue` | the reviewRating of this review, from 1 (worst) to 5 (best). |
| `review_body` | text |  | `schema:reviewBody` | review text. |
| `fit_vote` | text | ✓ | `acom:fitVote` | how the size fit; NULL when the reviewer did not say. values: fit_vote Values: `true_to_size` · `runs_small` · `runs_large`. |
| `author_height_value` | numeric(5,1) | ✓ | `acom:authorHeight` | reviewer height. |
| `author_height_unit_text` | text | ✓ | `acom:authorHeight` | unit of author_height_value, cm. |
| `keywords` | text[] |  | `schema:keywords` | repeated phrases found in the review, counted as mentions, for example size up for a hoodie. |
| `creative_work_status` | text |  | `schema:creativeWorkStatus` | moderation state. values: review_status Values: `visible` · `hidden` · `reported`. |
| `date_published` | date |  | `schema:datePublished` | date the review was posted. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

## `customer`

### `customer.member_program`

`schema:MemberProgram` — a store's tier program; tiers, requirements and benefits are data, not code. id prefix prg_. · id `prg_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix prg_; the same value the API, logs and events use. |
| `name` | text |  | `schema:name` | program name, for example Northwind Rewards. |
| `description` | text | ✓ | `schema:description` | what members get. |
| `evaluation` | jsonb |  | `acom:evaluation` | when tiers are evaluated: {schedule: {every, on_day, at}, on_event[], settle_days}. |
| `movement` | jsonb |  | `acom:movement` | how members move: {upgrade, downgrade, max_steps_down, grace_periods}. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `customer.member_program_tier`

`schema:MemberProgramTier` — one tier of a member program; the id is a word the store chooses (vip, gold, member) and everyone is reserved. API: tier_id.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | tier id, a lowercase word the store chooses (for example vip); never everyone; referenced by valid_for_member_tier. |
| `member_program_id` | text |  | `schema:isTierOf` | program the tier belongs to (prg_ id). → `customer.member_program` |
| `name` | text |  | `schema:name` | tier name people see, for example VIP. |
| `rank` | integer |  | `acom:rank` | order of the tier, 1 is the entry tier and higher ranks are above. |
| `has_tier_requirement` | jsonb |  | `schema:hasTierRequirement` | {combine: any or all, requirements: [{metric, threshold, retain_threshold, window, counts}]}; empty for the entry tier. |
| `has_tier_benefit` | jsonb |  | `schema:hasTierBenefit` | standing benefits: [{type, coupon_id, every, value}], type from the tier benefit list in enums.md. |
| `membership_points_earned_value` | numeric(6,2) | ✓ | `schema:membershipPointsEarned` | points multiplier for the tier; NULL when the tier has none. |
| `membership_points_earned_unit_text` | text | ✓ | `schema:membershipPointsEarned` | unit of membership_points_earned_value, for example points per USD. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `customer.person`

`schema:Person` — a shopper with an account (member). id prefix cus_. API: customer_id. · id `cus_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix cus_; the same value the API, logs and events use. |
| `name` | text |  | `schema:name` | display name shown to the shopper, for example Jamie. |
| `given_name` | text | ✓ | `schema:givenName` | first name. |
| `family_name` | text | ✓ | `schema:familyName` | last name. |
| `email` | text | ✓ | `schema:email` | contact email; masked for operators by default. |
| `telephone` | text | ✓ | `schema:telephone` | contact phone in E.164 form; masked for operators by default. |
| `birth_date` | date | ✓ | `schema:birthDate` | date of birth, used for the birthday coupon; NULL when not given. |
| `member_state` | text |  | `acom:memberState` | account lifecycle. values: member_state Values: `active` · `dormant` · `blocked` · `pending_withdrawal` · `withdrawn`. |
| `member_type` | text |  | `acom:memberType` | individual or business member. values: member_type Values: `individual` · `business`. |
| `sizes` | jsonb |  | `acom:sizes` | known sizes: {tops, bottoms, shoes}, each a size word or null. |
| `card_last4` | text | ✓ | `acom:cardLast4` | last four digits of the saved card, the only card data kept; NULL when no card is saved. |
| `registered_at` | timestamptz |  | `acom:registeredAt` | when the shopper created the account. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `customer.postal_address`

`schema:PostalAddress` — a saved address of a shopper. id prefix adr_. · id `adr_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix adr_; the same value the API, logs and events use. |
| `customer_id` | text |  | `schema:customer` | shopper who saved the address (cus_ id). → `customer.person` |
| `label` | text | ✓ | `acom:label` | the shopper's name for the address, for example Home. |
| `name` | text | ✓ | `schema:name` | recipient name printed on parcels. |
| `street_address` | text |  | `schema:streetAddress` | street, number and unit, for example 12 Harbor St, Apt 3. |
| `address_locality` | text |  | `schema:addressLocality` | city or town. |
| `address_region` | text | ✓ | `schema:addressRegion` | state or region code, for example NY. |
| `postal_code` | text |  | `schema:postalCode` | postal or ZIP code. |
| `address_country` | char(2) |  | `schema:addressCountry` | ISO 3166-1 alpha-2 country code. |
| `telephone` | text | ✓ | `schema:telephone` | phone for the carrier. |
| `address_usage` | text[] |  | `acom:usage` | what the address is used for; every element is a word from the list. values: address_usage |
| `is_primary` | boolean |  | `acom:isPrimary` | the shopper's main address; at most one per shopper. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `customer.preference`

`schema:PropertyValue/acom:Preference` — something the agent remembered about a shopper; visible to the shopper and undoable. id prefix mem_. API: memory_id. · id `mem_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix mem_; the same value the API, logs and events use. |
| `customer_id` | text |  | `schema:customer` | shopper the preference is about (cus_ id); never a gift recipient. → `customer.person` |
| `preference_key` | text |  | `acom:preferenceKey` | what the preference is about (API key). values: preference_key Values: `size` · `fit_note` · `fabric_preference` · `compares_specs` · `layout_preference` · `travel_priority` · `other`. |
| `value` | text |  | `schema:value` | the value in the shopper's words, 1 to 200 characters. |
| `description` | text |  | `schema:description` | the one line shown to the shopper, for example Natural fabrics. |
| `source` | text |  | `acom:source` | how it was learned; words from customer.yml preference.sources. |
| `conversation_id` | text | ✓ | `acom:conversationId` | conversation it was learned in (conv_ id). |
| `learned_at` | timestamptz |  | `acom:learnedAt` | when it was remembered. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `customer.program_membership`

`schema:ProgramMembership` — a shopper's membership in a program and their current tier; one tier per program.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `customer_id` | text |  | `schema:member` | the member (cus_ id). → `customer.person` |
| `member_program_id` | text |  | `schema:program` | the program (prg_ id). → `customer.member_program` |
| `tier_id` | text |  | `acom:tierId` | current tier (member_program_tier id). → `customer.member_program_tier` |
| `membership_number` | text |  | `schema:membershipNumber` | number printed for the member, unique in the store. |
| `tier_since` | timestamptz |  | `acom:tierSince` | when the member entered the current tier. |
| `tier_override` | jsonb | ✓ | `acom:tierOverride` | manual tier set by an operator: {reason, expires_at, after_expiry, by}; NULL when the tier comes from evaluation. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

## `fulfillment`

### `fulfillment.claim`

`schema:ReturnAction/acom:Claim` — a cancel, return or exchange request; @type is ReturnAction for returns and exchanges and CancelAction for cancels (claim_type decides). id prefix clm_. · id `clm_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix clm_; the same value the API, logs and events use. |
| `order_id` | text |  | `acom:orderId` | order the claim is about (ord_ id). → `sales.sales_order` |
| `customer_id` | text |  | `schema:customer` | shopper who sent the request (cus_ id), the agent of the action. → `customer.person` |
| `claim_type` | text |  | `acom:claimType` | cancel, return or exchange. values: claim_type Values: `cancel` · `return` · `exchange`. |
| `claim_status` | text |  | `acom:claimStatus` | where the claim is. values: claim_status Values: `charge_due` · `requested` · `photos_requested` · `approved` · `pickup_instructed` · `picked_up` · `inspected` · `reship_requested` · `completed` · `declined` · `withdrawn` · `converted`. |
| `claim_path` | text |  | `acom:claimPath` | who decides the claim. values: claim_path Values: `self_service` · `store_review` · `not_allowed`. |
| `claim_reason` | text | ✓ | `acom:claimReason` | reason for a return or exchange; NULL for cancels. values: claim_reason Values: `defect` · `damaged_in_transit` · `not_as_described` · `wrong_item_sent` · `wrong_option_sent` · `missing_parts` · `wrong_size` · `different_option_wanted` · `changed_mind`. |
| `cancel_reason` | text | ✓ | `acom:cancelReason` | reason for a cancel; NULL for returns and exchanges. values: cancel_reason Values: `changed_mind` · `wrong_option_ordered` · `found_lower_price` · `discount_changed` · `payment_method_change` · `duplicate_order` · `payment_not_completed` · `shipping_delay` · `out_of_stock` · `free_gift_unavailable` · `product_info_inaccurate` · `seller_cancelled` · `store_cancelled`. |
| `reason_text` | text | ✓ | `acom:reasonText` | the shopper's own words, up to 500 characters. |
| `liability` | text |  | `acom:liability` | who is at fault; decides who pays shipping. values: liability Values: `store_fault` · `seller_fault` · `customer_fault` · `carrier_fault` · `none`. |
| `return_fees` | text | ✓ | `schema:returnFees` | who pays return shipping for this claim. values: return_fees Values: `free_return` · `return_fees_customer_responsibility` · `return_shipping_fees`. |
| `send_back_method` | text | ✓ | `acom:sendBackMethod` | pickup or drop_off, from fulfillment.yml return.send_back_methods. |
| `pickup_window` | jsonb | ✓ | `acom:pickupWindow` | carrier pickup window {start, end}; NULL for drop-off. |
| `evidence` | jsonb |  | `acom:evidence` | photos and documents attached: [{id, evidence_type, content_url}], evd_ ids. |
| `amount_value` | numeric(12,2) |  | `schema:amount` | value of the claimed lines (price paid after line discounts), in major units of the currency. |
| `amount_currency` | char(3) |  | `schema:amount` | ISO 4217 code of amount_value. |
| `refund_estimate_value` | numeric(12,2) | ✓ | `acom:refundEstimate` | refund the shopper confirmed when sending the request; NULL for exchanges, in major units of the currency. |
| `refund_estimate_currency` | char(3) | ✓ | `acom:refundEstimate` | ISO 4217 code of refund_estimate_value. |
| `start_time` | timestamptz |  | `schema:startTime` | when the shopper sent the request (API requested_at). |
| `end_time` | timestamptz | ✓ | `schema:endTime` | when the claim reached completed, declined, withdrawn or converted. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `fulfillment.claim_event`

`schema:Action` — one state change of a claim; rows are only added.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `claim_id` | text |  | `acom:claimId` | the claim that changed (clm_ id), the object of the event. → `fulfillment.claim` |
| `position` | integer |  | `schema:position` | 1-based order of this event within its aggregate. |
| `action_type` | text |  | `acom:actionType` | the schema.org Action subtype (@type), for example UpdateAction or AuthorizeAction. |
| `action_status` | text |  | `schema:actionStatus` | lifecycle of the action. values: action_status Values: `potential` · `active` · `completed` · `failed`. |
| `agent_id` | text | ✓ | `schema:agent` | person or system that acted (adm_, cus_ or sys_ id); NULL when only a model acted. |
| `agent_name` | text | ✓ | `schema:agent` | name of the software agent (model) that acted, for example muse-glimmer-30b; NULL for people. |
| `instrument` | text | ✓ | `schema:instrument` | tool name or person-only endpoint that caused the event. |
| `from_state` | text | ✓ | `acom:fromState` | state before the event; NULL for the creating event. values: claim_status Values: `charge_due` · `requested` · `photos_requested` · `approved` · `pickup_instructed` · `picked_up` · `inspected` · `reship_requested` · `completed` · `declined` · `withdrawn` · `converted`. |
| `to_state` | text | ✓ | `acom:toState` | state after the event; NULL when the state did not change. values: claim_status Values: `charge_due` · `requested` · `photos_requested` · `approved` · `pickup_instructed` · `picked_up` · `inspected` · `reship_requested` · `completed` · `declined` · `withdrawn` · `converted`. |
| `result` | jsonb | ✓ | `schema:result` | what the action produced, as JSON. |
| `error` | jsonb | ✓ | `schema:error` | failure as {code, class, message}; NULL when it succeeded. |
| `start_time` | timestamptz |  | `schema:startTime` | when the action started. |
| `end_time` | timestamptz | ✓ | `schema:endTime` | when the action ended; NULL while it runs. |
| `request_id` | text | ✓ | `-` | request trace id (req_) from the gateway; joins agent turns and the action log. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `fulfillment.claim_item`

`schema:OrderItem/acom:ClaimItem` — one order line and quantity inside a claim.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `claim_id` | text |  | `acom:claimId` | the claim (clm_ id). → `fulfillment.claim` |
| `order_item_id` | text |  | `acom:orderItemId` | order line claimed (oln_ id). → `sales.order_item` |
| `order_quantity` | integer |  | `schema:orderQuantity` | units claimed from the line. |
| `replacement_product_id` | text | ✓ | `acom:replacement` | variant sent instead for an exchange (var_ id); NULL otherwise. → `catalog.product` |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `fulfillment.inspection`

`schema:CheckAction` — the condition check of returned items; it drives claim status, cost and stock restore. id prefix ins_. · id `ins_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix ins_; the same value the API, logs and events use. |
| `claim_id` | text |  | `acom:claimId` | claim whose items were checked (clm_ id). → `fulfillment.claim` |
| `inspection_condition` | text |  | `acom:inspectionCondition` | condition found; only as_new restores stock. values: inspection_condition Values: `as_new` · `damaged` · `missing_parts`. |
| `liability` | text | ✓ | `acom:liability` | fault found at inspection; NULL keeps the claim's. values: liability Values: `store_fault` · `seller_fault` · `customer_fault` · `carrier_fault` · `none`. |
| `description` | text | ✓ | `schema:description` | inspector's note. |
| `agent_id` | text |  | `schema:agent` | who inspected (adm_ or sys_ id). |
| `end_time` | timestamptz |  | `schema:endTime` | when the inspection was recorded. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `fulfillment.parcel_delivery`

`schema:ParcelDelivery` — one shipment of order items to an address, with its carrier progress. id prefix dlv_. · id `dlv_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix dlv_; the same value the API, logs and events use. |
| `order_id` | text |  | `schema:partOfOrder` | order shipped (ord_ id). → `sales.sales_order` |
| `item_shipped` | jsonb | ✓ | `schema:itemShipped` | lines in this parcel: [{order_item_id, order_quantity}]; NULL means every line of the order. |
| `delivery_state` | text |  | `acom:deliveryState` | warehouse step or carrier event the parcel is at. values: delivery_state Values: `awaiting_stock` · `ship_instructed` · `packed` · `label_created` · `picked_up` · `in_transit` · `delivered` · `delivery_failed` · `cancelled`. |
| `delivery_address_id` | text | ✓ | `acom:deliveryAddressId` | saved address used (adr_ id); NULL for a one-time address such as a gift recipient's. → `customer.postal_address` |
| `delivery_address` | jsonb |  | `schema:deliveryAddress` | address as printed on the label: {name, street_address, address_locality, address_region, postal_code, address_country}. |
| `has_delivery_method` | text |  | `schema:hasDeliveryMethod` | delivery method word from fulfillment.yml delivery.methods. |
| `carrier` | text | ✓ | `schema:carrier` | carrier name. |
| `tracking_number` | text | ✓ | `schema:trackingNumber` | carrier tracking number. |
| `tracking_url` | text | ✓ | `schema:trackingUrl` | carrier tracking page. |
| `expected_arrival_from` | timestamptz | ✓ | `schema:expectedArrivalFrom` | earliest expected arrival. |
| `expected_arrival_until` | timestamptz | ✓ | `schema:expectedArrivalUntil` | latest expected arrival. |
| `shipped_at` | timestamptz | ✓ | `acom:shippedAt` | when the carrier took the parcel. |
| `delivered_at` | timestamptz | ✓ | `acom:deliveredAt` | when the carrier reported delivery; return windows count from it. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `fulfillment.refund`

`schema:MoneyTransfer/acom:Refund` — money returned for a claim to the original payment method; cannot be undone once sent. id prefix rfd_. · id `rfd_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix rfd_; the same value the API, logs and events use. |
| `claim_id` | text |  | `acom:claimId` | claim refunded (clm_ id); one refund per claim. → `fulfillment.claim` |
| `order_id` | text |  | `acom:orderId` | order the money came from (ord_ id). → `sales.sales_order` |
| `payment_id` | text | ✓ | `acom:paymentId` | payment the money goes back to (pay_ id); NULL for store credit. → `sales.payment` |
| `amount_value` | numeric(12,2) |  | `schema:amount` | money returned, after coupon deductions, in major units of the currency. |
| `amount_currency` | char(3) |  | `schema:amount` | ISO 4217 code of amount_value. |
| `refund_method` | text |  | `acom:refundMethod` | where the money goes. values: refund_method Values: `original_payment` · `bank_account` · `store_credit`. |
| `refund_type` | text |  | `schema:refundType` | kind of refund. values: refund_type Values: `full_refund` · `exchange_refund` · `store_credit_refund`. |
| `refund_status` | text |  | `acom:refundStatus` | confirmed by payment events; never fails silently. values: refund_status Values: `requested` · `completed` · `failed`. |
| `external_id` | text | ✓ | `acom:externalId` | the payment provider's refund reference. |
| `preview_id` | text | ✓ | `acom:previewId` | refund_return preview that was run (pv_ id). → `platform.preview` |
| `change_set_id` | text | ✓ | `acom:changeSetId` | change set that approved the refund. → `platform.change_set` |
| `start_time` | timestamptz |  | `schema:startTime` | when the refund was sent to the provider. |
| `end_time` | timestamptz | ✓ | `schema:endTime` | when the provider confirmed or failed it. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

## `merchandising`

### `merchandising.banner`

`schema:CreativeWork/acom:Banner` — banner content (small header, headline, button, link, image) that bookings show. id prefix bnr_. · id `bnr_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix bnr_; the same value the API, logs and events use. |
| `name` | text |  | `schema:name` | banner name for operators. |
| `eyebrow` | text | ✓ | `acom:eyebrow` | small header above the headline, up to 24 characters. |
| `headline` | text |  | `schema:headline` | main text, up to 60 characters. |
| `alternative_headline` | text | ✓ | `schema:alternativeHeadline` | second line under the headline. |
| `cta_label` | text | ✓ | `acom:ctaLabel` | button text, up to 24 characters. |
| `url` | text |  | `schema:url` | store path the button opens, starting with /. |
| `image` | text | ✓ | `schema:image` | URL of an uploaded or generated image; NULL when a product image is used. |
| `image_product_group_id` | text | ✓ | `acom:imageProduct` | product whose main image the banner shows (prd_ id). → `catalog.product_group` |
| `image_color` | text | ✓ | `acom:imageColor` | color variant of image_product_group_id to photograph, for example Sand. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `merchandising.defined_term`

`schema:DefinedTerm` — a category in the standard set (internal classification) or the display set (shopper menu). id prefix cat_. API: category_id. · id `cat_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix cat_; the same value the API, logs and events use. |
| `name` | text |  | `schema:name` | category name shown to people, for example Summer outerwear. |
| `term_code` | text |  | `schema:termCode` | meaningful English word for the category, unique in the store, for example summer_outerwear. |
| `in_defined_term_set` | text |  | `schema:inDefinedTermSet` | which category set the term belongs to. values: term_set Values: `standard` · `display`. |
| `broader_id` | text | ✓ | `acom:broader` | parent category (cat_ id); NULL for a top-level category. → `merchandising.defined_term` |
| `description` | text | ✓ | `schema:description` | what belongs in the category. |
| `position` | integer |  | `schema:position` | display order among siblings. |
| `is_active` | boolean |  | `acom:isActive` | false hides the category and its children. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `merchandising.display_booking`

`schema:Schedule/acom:DisplayBooking` — a slot booked for one audience over a period, showing one banner or the slot's items. id prefix dsp_. · id `dsp_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix dsp_; the same value the API, logs and events use. |
| `display_slot_id` | text |  | `acom:slotId` | the slot booked (slot_ id; API slot_id). → `merchandising.display_slot` |
| `banner_id` | text | ✓ | `acom:bannerId` | banner shown (bnr_ id); NULL when the booking shows the slot's items. → `merchandising.banner` |
| `valid_for_member_tier` | text | ✓ | `schema:validForMemberTier` | tier that sees the booking; other audiences keep seeing their own bookings; NULL means everyone (the reserved word everyone is never stored). → `customer.member_program_tier` |
| `valid_from` | timestamptz |  | `schema:validFrom` | first moment the booking shows. |
| `valid_through` | timestamptz | ✓ | `schema:validThrough` | last moment it shows (inclusive); NULL when open-ended. |
| `display_status` | text |  | `acom:displayStatus` | computed from the period and the store clock. values: display_status Values: `draft` · `scheduled` · `active` · `ended`. |
| `promotion_id` | text | ✓ | `acom:promotionId` | promotion the banner announces; the booking must lie inside its period. → `promotion.promotion` |
| `change_set_id` | text | ✓ | `acom:changeSetId` | change set that created the booking. → `platform.change_set` |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `merchandising.display_slot`

`schema:WebPageElement` — a place on a store page that holds products or banners, booked per audience. id prefix slot_. · id `slot_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix slot_; the same value the API, logs and events use. |
| `name` | text |  | `schema:name` | slot name for operators, for example Home hero. |
| `page` | text |  | `acom:page` | page the slot is on: main, category, product or search (list_display_slots page values). |
| `slot_type` | text |  | `acom:slotType` | what kind of content the slot shows. values: slot_type Values: `hero` · `banner_board` · `product_grid` · `brand_strip` · `collection_feed`. |
| `position` | integer |  | `schema:position` | order of the slot on its page, top first. |
| `item_list_element` | jsonb |  | `schema:itemListElement` | default content: [{position, item_id}] with prd_, bnr_ or col_ ids. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `merchandising.product_group_category`

`acom:CategoryPlacement` — a product placed in a standard or display category; one standard category per product is primary.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `product_group_id` | text |  | `acom:productGroupId` | the product (prd_ id; API product_id). → `catalog.product_group` |
| `category_id` | text |  | `schema:category` | the category (cat_ id). → `merchandising.defined_term` |
| `is_primary` | boolean |  | `acom:isPrimary` | true for the product's main standard category (API category_id on the product). |
| `position` | integer | ✓ | `schema:position` | order of the product inside the category; NULL sorts by search ranking. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

## `messaging`

### `messaging.message`

`schema:Message` — a transactional message to one shopper about an order or claim; a sent message cannot be taken back. id prefix msg_. · id `msg_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix msg_; the same value the API, logs and events use. |
| `recipient_id` | text |  | `schema:recipient` | shopper the message goes to (cus_ id). → `customer.person` |
| `about_id` | text | ✓ | `schema:about` | order or claim the message is about (ord_ or clm_ id). |
| `message_kind` | text |  | `acom:messageKind` | what the message does. values: message_kind Values: `ask_for_photos` · `decline` · `refund_done` · `custom`. |
| `message_channel` | text |  | `acom:messageChannel` | channel it is sent on. values: message_channel Values: `email` · `sms` · `push` · `in_app`. |
| `headline` | text | ✓ | `schema:headline` | subject line for email and push. |
| `text` | text |  | `schema:text` | body exactly as the shopper reads it, up to 2,000 characters. |
| `sender_id` | text | ✓ | `schema:sender` | operator or system that sent it (adm_ or sys_ id). |
| `preview_id` | text | ✓ | `acom:previewId` | send_customer_message preview that was run (pv_ id). → `platform.preview` |
| `change_set_id` | text | ✓ | `acom:changeSetId` | change set that approved the message. → `platform.change_set` |
| `date_sent` | timestamptz | ✓ | `schema:dateSent` | when it was sent; NULL while queued. |
| `date_read` | timestamptz | ✓ | `schema:dateRead` | when the shopper opened it, if the channel reports it. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

## `platform`

### `platform.action_log`

`schema:Action` — every tool call, preview, approval, run, verification and undo, in one shape; rows are only added. id prefix act_. · id `act_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix act_; the same value the API, logs and events use. |
| `action_type` | text |  | `acom:actionType` | the schema.org Action subtype (@type), for example UpdateAction. |
| `action_status` | text |  | `schema:actionStatus` | potential for previews, then active, completed or failed. values: action_status Values: `potential` · `active` · `completed` · `failed`. |
| `actor_kind` | text |  | `acom:actorKind` | kind of principal that acted. values: actor_kind Values: `operator` · `shopper` · `agent_on_behalf` · `external_agent` · `system`. |
| `agent_id` | text | ✓ | `schema:agent` | person or system that acted (adm_, cus_ or sys_ id). |
| `agent_name` | text | ✓ | `schema:agent` | model name when a model made the call, for example muse-glimmer-30b. |
| `instrument` | text |  | `schema:instrument` | tool name or person-only endpoint. |
| `object_type` | text | ✓ | `acom:objectType` | schema.org type of the object, for example Offer. |
| `object_id` | text | ✓ | `schema:object` | id of the entity acted on, for example off_windbreaker_m_slate. |
| `result` | jsonb | ✓ | `schema:result` | what the action produced. |
| `error` | jsonb | ✓ | `schema:error` | {code, class, message} when the action failed. |
| `start_time` | timestamptz |  | `schema:startTime` | when the action started. |
| `end_time` | timestamptz | ✓ | `schema:endTime` | when the action ended. |
| `change_set_id` | text | ✓ | `acom:changeSetId` | change set the action belongs to. → `platform.change_set` |
| `preview_id` | text | ✓ | `acom:previewId` | preview the action created or ran. → `platform.preview` |
| `policy_version` | text | ✓ | `acom:policyVersion` | policy snapshot hash used by checks. |
| `request_id` | text | ✓ | `-` | request trace id (req_). |
| `usage` | jsonb | ✓ | `acom:usage` | model usage for the call: {input_tokens, output_tokens}. |
| `latency_ms` | integer | ✓ | `acom:latency` | time the call took, in milliseconds. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `platform.admin_user`

`schema:Person` — an operator account that signs in to the back office. id prefix adm_. API: admin_id. · id `adm_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix adm_; the same value the API, logs and events use. |
| `name` | text |  | `schema:name` | full name shown in change sets and the action log. |
| `alternate_name` | text | ✓ | `schema:alternateName` | short name colleagues use, for example SJ. |
| `email` | text |  | `schema:email` | sign-in email address, unique in the store. |
| `job_title` | text | ✓ | `schema:jobTitle` | job title shown next to the name. |
| `admin_status` | text |  | `acom:adminStatus` | whether the account can sign in. values: admin_status Values: `active` · `suspended` · `locked`. |
| `login_id` | text | ✓ | `acom:loginId` | what the operator types to sign in (for example admin); NULL means the account cannot sign in with a password. |
| `password_hash` | text | ✓ | `-` | argon2 hash of the password; NULL until the person sets one (demo login does not use it). |
| `last_login_at` | timestamptz | ✓ | `acom:lastLoginAt` | last successful sign-in; NULL if never. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `platform.admin_user_role`

`-` — assignment of a role to an operator account.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `admin_user_id` | text |  | `-` | operator account (adm_ id; API admin_id). → `platform.admin_user` |
| `role_id` | text |  | `-` | role given to the account (rol_ id). → `platform.role` |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `platform.change_set`

`schema:ItemList/acom:ChangeSet` — previews from one request that a person approves, runs, verifies and can undo as a unit. id prefix cs_. · id `cs_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix cs_; the same value the API, logs and events use. |
| `number` | integer |  | `acom:number` | number people see (#1042), unique in the store; same as the id suffix in seed data. |
| `title` | text |  | `acom:title` | one-line title shown on the change set card. |
| `change_set_state` | text |  | `acom:changeSetState` | where the set is in its lifecycle. values: change_set_state Values: `drafting` · `waiting_for_approval` · `running` · `verifying` · `live` · `failed` · `rejected` · `undone`. |
| `request_text` | text[] |  | `acom:requestText` | the person's words, first request then each edit by conversation. |
| `interpretation` | jsonb |  | `acom:interpretation` | how the runtime understood the request: [{slot, text, value}], slot is who, products, benefit, when, where or other. |
| `conversation_id` | text | ✓ | `acom:conversationId` | agent conversation (conv_ id) the set came from; NULL for automation. |
| `created_by` | text |  | `schema:creator` | operator or system principal that asked for the set (adm_ or sys_ id). |
| `agent_name` | text | ✓ | `schema:agent` | model that drafted the set, for example muse-glimmer-30b; NULL when no model was involved. |
| `submitted_at` | timestamptz | ✓ | `acom:submittedAt` | when the latest version was submitted for approval. |
| `approved_by` | text | ✓ | `acom:approvedBy` | operator who approved (adm_ id); NULL before approval or under a standing approval. |
| `approved_at` | timestamptz | ✓ | `acom:approvedAt` | when the set was approved; for standing approvals, when the automation started it. |
| `rejected_at` | timestamptz | ✓ | `acom:rejectedAt` | when a person rejected the set. |
| `rejection_reason` | text | ✓ | `acom:rejectionReason` | the reason the person gave when rejecting. |
| `verified_at` | timestamptz | ✓ | `acom:verifiedAt` | when the executor finished re-reading the results. |
| `verification` | jsonb | ✓ | `acom:verification` | summary from the verifying step: {state, matched, total}. |
| `undo_until` | timestamptz | ✓ | `acom:undoUntil` | last moment a person can undo the set; NULL when an item cannot be undone. |
| `undone_at` | timestamptz | ✓ | `acom:undoneAt` | when a person undid the set. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `platform.change_set_event`

`schema:Action` — one state transition of a change set; rows are only added.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `change_set_id` | text |  | `acom:changeSetId` | the change set that changed (the object of the event). |
| `position` | integer |  | `schema:position` | 1-based order of this event within its aggregate. |
| `action_type` | text |  | `acom:actionType` | the schema.org Action subtype (@type), for example UpdateAction or AuthorizeAction. |
| `action_status` | text |  | `schema:actionStatus` | lifecycle of the action. values: action_status Values: `potential` · `active` · `completed` · `failed`. |
| `agent_id` | text | ✓ | `schema:agent` | person or system that acted (adm_, cus_ or sys_ id); NULL when only a model acted. |
| `agent_name` | text | ✓ | `schema:agent` | name of the software agent (model) that acted, for example muse-glimmer-30b; NULL for people. |
| `instrument` | text | ✓ | `schema:instrument` | tool name or person-only endpoint that caused the event. |
| `from_state` | text | ✓ | `acom:fromState` | state before the event; NULL for the creating event. values: change_set_state Values: `drafting` · `waiting_for_approval` · `running` · `verifying` · `live` · `failed` · `rejected` · `undone`. |
| `to_state` | text | ✓ | `acom:toState` | state after the event; NULL when the state did not change. values: change_set_state Values: `drafting` · `waiting_for_approval` · `running` · `verifying` · `live` · `failed` · `rejected` · `undone`. |
| `result` | jsonb | ✓ | `schema:result` | what the action produced, as JSON. |
| `error` | jsonb | ✓ | `schema:error` | failure as {code, class, message}; NULL when it succeeded. |
| `start_time` | timestamptz |  | `schema:startTime` | when the action started. |
| `end_time` | timestamptz | ✓ | `schema:endTime` | when the action ended; NULL while it runs. |
| `request_id` | text | ✓ | `-` | request trace id (req_) from the gateway; joins agent turns and the action log. |
| `change_set_version` | integer |  | `acom:changeSetVersion` | version of the set when the event happened. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `platform.change_set_item`

`schema:ListItem` — one preview inside a change set, in run order.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `change_set_id` | text |  | `acom:changeSetId` | the change set. |
| `position` | integer |  | `schema:position` | 1-based run order inside the set. |
| `preview_id` | text |  | `acom:previewId` | the preview this item runs (pv_ id). → `platform.preview` |
| `depends_on` | text[] |  | `acom:dependsOn` | preview ids whose results this item consumes, for example a promotion_id placeholder. |
| `result_id` | text | ✓ | `acom:resultId` | id the item produced when it ran, for example promo_1042. |
| `verification` | jsonb | ✓ | `acom:verification` | verify result for the item: {matched, checked, mismatches[]}. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `platform.demo_clock`

`-` — the store's demo time; in demo mode every Clock read and due job uses it.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `now` | timestamptz |  | `-` | current demo time for the store. |
| `is_running` | boolean |  | `-` | true when demo time advances with real time from now; false when it stays fixed. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `platform.idempotency_key`

`-` — first response to a person-only request, returned again when the same Idempotency-Key is retried.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `key` | text |  | `-` | the Idempotency-Key header value. |
| `endpoint` | text |  | `-` | method and path the key was used with. |
| `subject_id` | text | ✓ | `-` | person who sent the request (adm_ or cus_ id). |
| `request_hash` | text |  | `-` | hash of the request body; a retry with another body is refused. |
| `response_status` | integer | ✓ | `-` | HTTP status of the first response. |
| `response` | jsonb | ✓ | `-` | body of the first response. |
| `expires_at` | timestamptz |  | `-` | when the key may be forgotten. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `platform.job`

`-` — scheduled work run by the commerce job queue (FOR UPDATE SKIP LOCKED): watchers, banner and promotion start and end. id prefix job_. · id `job_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix job_; the same value the API, logs and events use. |
| `kind` | text |  | `-` | job kind the dispatcher routes on, for example promotion.start or watcher.price_below_cost. |
| `run_at` | timestamptz |  | `-` | when the job becomes due, on the store clock (the demo clock in demo mode). |
| `payload` | jsonb |  | `-` | job input, for example {promotion_id}. |
| `attempts` | integer |  | `-` | runs tried so far. |
| `locked_at` | timestamptz | ✓ | `-` | when a worker took the job; NULL when free. |
| `done_at` | timestamptz | ✓ | `-` | when the job finished successfully; NULL while pending. |
| `last_error` | text | ✓ | `-` | error of the last failed attempt. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `platform.online_store`

`schema:OnlineStore` — one store; every other row belongs to exactly one store (tenant). id prefix store_. API: tenant. · id `store_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `id` | text |  | `-` | store id, prefix store_; the tenant claim in internal tokens and the tenant_id of every other row. |
| `name` | text |  | `schema:name` | store name shown to shoppers and operators. |
| `url` | text |  | `schema:url` | the storefront address, for example northwind.example. |
| `currency` | char(3) |  | `schema:currency` | ISO 4217 code of the store currency; all prices and amounts use it. |
| `time_zone` | text |  | `acom:timeZone` | IANA time zone that store times and dates are shown in, for example America/New_York. |
| `in_language` | text |  | `schema:inLanguage` | BCP 47 language of store text, for example en. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `platform.outbox`

`-` — domain events waiting to be published to other domains and to the event stream; at least once delivery.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `position` | bigint | ✓ | `-` | increasing event number; the SSE event id. |
| `topic` | text |  | `-` | event name, for example change_set.updated or order_delivered. |
| `object_id` | text | ✓ | `-` | id of the aggregate the event is about. |
| `payload` | jsonb |  | `-` | event body. |
| `published_at` | timestamptz | ✓ | `-` | when the dispatcher delivered the event; NULL while waiting. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `platform.policy_override`

`-` — store-specific values deep-merged over one policies/*.yml file (commerce.md §4).

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `file` | text |  | `-` | policy file name without .yml, for example promotion. |
| `partial` | jsonb |  | `-` | the partial YAML document as JSON, same key paths as the file, for example {max_discount: {by_tier: {vip: 0.30}}}. |
| `policy_version` | integer |  | `-` | version of the default policy file this override was written against. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `platform.preview`

`schema:Action` — a stored write preview (actionStatus potential); the operation decides the Action subtype. id prefix pv_. · id `pv_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix pv_; the same value the API, logs and events use. |
| `operation` | text |  | `acom:operation` | logical address of the write, for example promotion.offer.set_sale_prices. |
| `instrument` | text |  | `schema:instrument` | tool name of the write, for example set_sale_prices. |
| `input` | jsonb |  | `acom:input` | normalized operation input. |
| `input_hash` | text |  | `-` | hash of the normalized input; detects a repeated preview of the same input. |
| `plan` | jsonb |  | `acom:plan` | the WriteOp plan that commit applies unchanged. |
| `changes` | jsonb |  | `acom:changes` | changes[] as returned to the caller: [{op, entity, id, from, to}]. |
| `checks` | jsonb |  | `acom:checks` | policy checks as returned in the API field checks: [{rule, passed, detail, fix_tool}]. |
| `needs_approval` | boolean |  | `acom:needsApproval` | true when only an approved change set may run it. |
| `approval_reason` | text | ✓ | `acom:approvalReason` | why approval is needed, shown to the approver; NULL when not needed. |
| `basis` | jsonb |  | `acom:basis` | entities read to plan and their versions: [{entity, id, version}]; compared at execution. |
| `policy_version` | text |  | `acom:policyVersion` | hash of the policy snapshot the checks used. |
| `actor_kind` | text |  | `acom:actorKind` | kind of principal that asked for the preview. values: actor_kind Values: `operator` · `shopper` · `agent_on_behalf` · `external_agent` · `system`. |
| `agent_id` | text | ✓ | `schema:agent` | person or system behind the preview (adm_, cus_ or sys_ id). |
| `agent_name` | text | ✓ | `schema:agent` | model that produced the call, when a model did. |
| `request_id` | text | ✓ | `-` | request trace id (req_). |
| `change_set_id` | text | ✓ | `acom:changeSetId` | change set the preview belongs to; NULL for a stand-alone preview. → `platform.change_set` |
| `preview_state` | text |  | `acom:previewState` | open until committed, expired or superseded. values: preview_state Values: `open` · `committed` · `expired` · `superseded`. |
| `expires_at` | timestamptz | ✓ | `acom:expiresAt` | when a stand-alone preview stops being runnable; NULL for previews in a change set (rechecked at approval). |
| `superseded_by_id` | text | ✓ | `acom:supersededBy` | preview that replaced this one in its change set. → `platform.preview` |
| `committed_at` | timestamptz | ✓ | `acom:committedAt` | when the preview was executed. |
| `result` | jsonb | ✓ | `schema:result` | execution response {ok, id, undo}, returned again on a repeated execution. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `platform.role`

`schema:Role` — a named bundle of scopes that the gateway puts in operator tokens. id prefix rol_. · id `rol_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix rol_; the same value the API, logs and events use. |
| `name` | text |  | `schema:name` | role name shown to administrators, for example Merchandiser. |
| `description` | text | ✓ | `schema:description` | what people with this role do. |
| `approver_roles` | text[] |  | `acom:approverRoles` | approver_role values from approval.yml thresholds this role may approve, for example {merchandiser,operator}. |
| `is_system` | boolean |  | `acom:isSystem` | true for built-in roles that cannot be edited. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `platform.role_scope`

`-` — one scope granted by a role (gateway.md §2); the gateway reads these to build tokens.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `role_id` | text |  | `-` | role that grants the scope (rol_ id). → `platform.role` |
| `scope` | text |  | `-` | scope string, for example tools:read:promotion or human:approve. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `platform.session`

`-` — a signed-in browser session held by the gateway (operator, shopper or guest). id prefix ses_. · id `ses_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix ses_; the same value the API, logs and events use. |
| `actor_kind` | text |  | `-` | who holds the session. values: actor_kind Values: `operator` · `shopper` · `agent_on_behalf` · `external_agent` · `system`. |
| `subject_id` | text | ✓ | `-` | person behind the session (adm_ or cus_ id); NULL for an anonymous guest. |
| `token_hash` | text |  | `-` | hash of the session cookie value; the cookie itself is never stored. |
| `csrf_token_hash` | text |  | `-` | hash of the CSRF token paired with the session. |
| `expires_at` | timestamptz |  | `-` | when the session stops being accepted. |
| `last_seen_at` | timestamptz | ✓ | `-` | last request made with the session. |
| `revoked_at` | timestamptz | ✓ | `-` | when the session was signed out or revoked; NULL while active. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `platform.task`

`schema:Action/acom:Task` — one item on the operator home list, from a watcher, an automation or a request. id prefix task_. · id `task_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix task_; the same value the API, logs and events use. |
| `title` | text |  | `acom:title` | one line shown on the home list. |
| `task_state` | text |  | `acom:taskState` | whether the task waits for a person, is being worked, done or dismissed. values: task_state Values: `needs_you` · `working` · `done` · `dismissed`. |
| `task_origin` | text |  | `acom:taskOrigin` | how the task started. values: task_origin Values: `you_asked` · `agent_noticed` · `automation`. |
| `summary` | text | ✓ | `acom:summary` | current one-line status, for example 15 of 18 checked. |
| `tag` | text | ✓ | `acom:tag` | area label shown on the list, for example returns or pricing. |
| `request_text` | text | ✓ | `acom:requestText` | the sentence the agent starts a turn with when the task is opened. |
| `watcher` | text | ✓ | `acom:watcher` | platform.yml watchers key that created the task; NULL for requests. |
| `watcher_observation` | jsonb | ✓ | `acom:watcherObservation` | threshold and what was observed: {threshold, observed}. |
| `related_entities` | jsonb |  | `schema:object` | entities the task is about: [{entity, id}]. |
| `change_set_id` | text | ✓ | `acom:changeSetId` | change set holding the task's writes; NULL until one exists. → `platform.change_set` |
| `conversation_id` | text | ✓ | `acom:conversationId` | agent conversation working on the task (conv_ id). |
| `start_time` | timestamptz |  | `schema:startTime` | when the task appeared (API at). |
| `due_at` | timestamptz | ✓ | `acom:dueAt` | when it should be done, if a watcher sets one. |
| `end_time` | timestamptz | ✓ | `schema:endTime` | when the task became done or dismissed. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

## `promotion`

### `promotion.coupon`

`schema:Offer/acom:Coupon` — a coupon tied to a promotion; it records the promotion's benefit on the order and never adds a second discount. id prefix cpn_. · id `cpn_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix cpn_; the same value the API, logs and events use. |
| `name` | text | ✓ | `schema:name` | coupon name shown in the member's wallet. |
| `discount_code` | text |  | `schema:discountCode` | code, unique in the store, recorded on orders, for example VIPWEEKEND20. |
| `promotion_id` | text |  | `acom:promotionId` | promotion whose benefit the coupon records (promo_ id). → `promotion.promotion` |
| `valid_for_member_tier` | text | ✓ | `schema:validForMemberTier` | tier allowed to use the coupon, equal to the promotion's tier; NULL means everyone (the reserved word everyone is never stored). → `customer.member_program_tier` |
| `application` | text |  | `acom:couponApplication` | how the coupon reaches an order. values: coupon_application Values: `automatic_at_checkout` · `code_entry` · `download`. |
| `stacking` | text |  | `acom:stacking` | which other coupons may be used in the same order. values: coupon_stacking Values: `no_tier_coupons` · `with_tier_coupons` · `no_other_coupons`. |
| `is_benefit_record_only` | boolean |  | `acom:recordsBenefitOnly` | true: the coupon records the promotion's benefit and adds no discount of its own. |
| `valid_from` | timestamptz | ✓ | `schema:validFrom` | first moment the coupon can be used; NULL follows the promotion. |
| `valid_through` | timestamptz | ✓ | `schema:validThrough` | last moment it can be used; NULL follows the promotion. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `promotion.member_coupon`

`schema:OwnershipInfo/acom:MemberCoupon` — a coupon issued to one member, or recorded when an automatic coupon is used. id prefix mcp_. · id `mcp_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix mcp_; the same value the API, logs and events use. |
| `coupon_id` | text |  | `acom:couponId` | the coupon issued (cpn_ id). → `promotion.coupon` |
| `customer_id` | text |  | `schema:customer` | member who holds it (cus_ id). → `customer.person` |
| `coupon_status` | text |  | `acom:couponStatus` | state of this issued coupon. values: coupon_status Values: `issued` · `available` · `used` · `restored` · `expired` · `revoked`. |
| `owned_from` | timestamptz |  | `schema:ownedFrom` | when it was issued. |
| `owned_through` | timestamptz | ✓ | `schema:ownedThrough` | when it expires; NULL when it does not expire. |
| `order_id` | text | ✓ | `acom:orderId` | order it was used on (ord_ id); NULL until used. |
| `used_at` | timestamptz | ✓ | `acom:usedAt` | when it was used. |
| `benefit_recorded_value` | numeric(12,2) | ✓ | `acom:benefitRecorded` | discount recorded on the order when the coupon was used, in major units of the currency. |
| `benefit_recorded_currency` | char(3) | ✓ | `acom:benefitRecorded` | ISO 4217 code of benefit_recorded_value. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `promotion.promotion`

`schema:OfferCatalog/acom:Promotion` — a time-boxed benefit that groups member and sale prices or an order discount. id prefix promo_. · id `promo_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix promo_; the same value the API, logs and events use. |
| `name` | text |  | `schema:name` | name shown as the cart discount label, for example VIP weekend. |
| `description` | text | ✓ | `schema:description` | what the promotion gives. |
| `promotion_status` | text |  | `acom:promotionStatus` | computed from the period and the store clock unless paused. values: promotion_status Values: `draft` · `scheduled` · `running` · `paused` · `ended`. |
| `discount_layer` | text |  | `acom:discountLayer` | where the benefit applies when benefits are calculated. values: discount_layer Values: `item_price` · `order_discount` · `add_on`. |
| `valid_for_member_tier` | text | ✓ | `schema:validForMemberTier` | tier the promotion is for; NULL means everyone (the reserved word everyone is never stored). → `customer.member_program_tier` |
| `valid_from` | timestamptz |  | `schema:validFrom` | first moment the promotion applies. |
| `valid_through` | timestamptz | ✓ | `schema:validThrough` | last moment it applies (inclusive, minute precision); NULL for a standing promotion. |
| `percent_off` | numeric(5,2) | ✓ | `acom:percentOff` | requested discount, 0-100; NULL when the benefit is an amount. |
| `amount_off_value` | numeric(12,2) | ✓ | `acom:amountOff` | fixed discount per order; NULL when the benefit is a percentage, in major units of the currency. |
| `amount_off_currency` | char(3) | ✓ | `acom:amountOff` | ISO 4217 code of amount_off_value. |
| `eligible_transaction_volume_value` | numeric(12,2) | ✓ | `schema:eligibleTransactionVolume` | minimum item subtotal before member benefits for the promotion to apply, for example 80.00 for free shipping, in major units of the currency. |
| `eligible_transaction_volume_currency` | char(3) | ✓ | `schema:eligibleTransactionVolume` | ISO 4217 code of eligible_transaction_volume_value. |
| `applies_to` | jsonb |  | `acom:appliesTo` | scope: {scope: products, categories or order, product_ids[], category_ids[], exclude_product_ids[]}. |
| `change_set_id` | text | ✓ | `acom:changeSetId` | change set that created the promotion; NULL for seeded or imported ones. → `platform.change_set` |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `promotion.unit_price_specification`

`schema:UnitPriceSpecification` — a member or time-limited price for one variant offer; list prices stay on the offer. id prefix ups_. · id `ups_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix ups_; the same value the API, logs and events use. |
| `offer_id` | text |  | `acom:offerId` | offer the price applies to (off_ id); the Offer's priceSpecification. → `catalog.offer` |
| `promotion_id` | text | ✓ | `acom:promotionId` | promotion that created the price; NULL for a scheduled list price. → `promotion.promotion` |
| `price` | numeric(12,2) |  | `schema:price` | the price, list price × (1 − percent_off / 100) rounded to the cent (half up). |
| `price_currency` | char(3) |  | `schema:priceCurrency` | ISO 4217 code of price. |
| `price_type` | text |  | `schema:priceType` | sale_price for member and sale prices, list_price for a scheduled list price change. values: price_type Values: `list_price` · `sale_price`. |
| `valid_for_member_tier` | text | ✓ | `schema:validForMemberTier` | tier allowed to pay this price; NULL means everyone (the reserved word everyone is never stored). → `customer.member_program_tier` |
| `valid_from` | timestamptz |  | `schema:validFrom` | first moment the price applies. |
| `valid_through` | timestamptz | ✓ | `schema:validThrough` | last moment it applies (inclusive); NULL when open-ended. |
| `percent_off` | numeric(5,2) | ✓ | `acom:percentOff` | the requested discount behind this price, 0-100. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

## `sales`

### `sales.cart`

`schema:Order/acom:Cart` — an order before checkout; one per signed-in shopper or guest session. id prefix cart_. · id `cart_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix cart_; the same value the API, logs and events use. |
| `customer_id` | text | ✓ | `schema:customer` | shopper (cus_ id); NULL for a guest cart. → `customer.person` |
| `session_id` | text | ✓ | `acom:sessionId` | guest session that owns the cart (ses_ id); NULL for a signed-in shopper. |
| `delivery_address_id` | text | ✓ | `acom:deliveryAddressId` | saved address used for delivery estimates (adr_ id). → `customer.postal_address` |
| `payment_method` | text | ✓ | `schema:paymentMethod` | payment method chosen so far. values: payment_method Values: `credit_card` · `bank_transfer` · `virtual_account` · `loyalty_points` · `store_credit`. |
| `sales_channel` | text |  | `acom:salesChannel` | where the cart was built. values: sales_channel Values: `web` · `mobile_web` · `app` · `agent`. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `sales.cart_line`

`schema:OrderItem/acom:CartLine` — one variant and quantity in a cart. id prefix cln_. · id `cln_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix cln_; the same value the API, logs and events use. |
| `cart_id` | text |  | `acom:cartId` | the cart (cart_ id). → `sales.cart` |
| `product_id` | text |  | `schema:orderedItem` | the variant (var_ id; API variant_id). → `catalog.product` |
| `order_quantity` | integer |  | `schema:orderQuantity` | units, 1 up to cart.max_quantity_per_line. |
| `position` | integer |  | `schema:position` | line order in the cart. |
| `description` | text | ✓ | `schema:description` | the shopper's note on the line. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `sales.order_event`

`schema:Action` — one state change of an order; rows are only added.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `order_id` | text |  | `acom:orderId` | the order that changed (ord_ id), the object of the event. → `sales.sales_order` |
| `position` | integer |  | `schema:position` | 1-based order of this event within its aggregate. |
| `action_type` | text |  | `acom:actionType` | the schema.org Action subtype (@type), for example UpdateAction or AuthorizeAction. |
| `action_status` | text |  | `schema:actionStatus` | lifecycle of the action. values: action_status Values: `potential` · `active` · `completed` · `failed`. |
| `agent_id` | text | ✓ | `schema:agent` | person or system that acted (adm_, cus_ or sys_ id); NULL when only a model acted. |
| `agent_name` | text | ✓ | `schema:agent` | name of the software agent (model) that acted, for example muse-glimmer-30b; NULL for people. |
| `instrument` | text | ✓ | `schema:instrument` | tool name or person-only endpoint that caused the event. |
| `from_state` | text | ✓ | `acom:fromState` | state before the event; NULL for the creating event. values: order_status Values: `payment_due` · `processing` · `in_transit` · `delivered` · `returned` · `cancelled` · `problem`. |
| `to_state` | text | ✓ | `acom:toState` | state after the event; NULL when the state did not change. values: order_status Values: `payment_due` · `processing` · `in_transit` · `delivered` · `returned` · `cancelled` · `problem`. |
| `result` | jsonb | ✓ | `schema:result` | what the action produced, as JSON. |
| `error` | jsonb | ✓ | `schema:error` | failure as {code, class, message}; NULL when it succeeded. |
| `start_time` | timestamptz |  | `schema:startTime` | when the action started. |
| `end_time` | timestamptz | ✓ | `schema:endTime` | when the action ended; NULL while it runs. |
| `request_id` | text | ✓ | `-` | request trace id (req_) from the gateway; joins agent turns and the action log. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `sales.order_item`

`schema:OrderItem` — one variant line of an order; each line moves through its own status. id prefix oln_. · id `oln_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix oln_; the same value the API, logs and events use. |
| `order_id` | text |  | `acom:orderId` | the order (ord_ id). → `sales.sales_order` |
| `position` | integer |  | `schema:position` | line number in the order. |
| `product_id` | text |  | `schema:orderedItem` | the variant bought (var_ id; API variant_id). → `catalog.product` |
| `name` | text |  | `schema:name` | product name at the time of the order, as printed on receipts. |
| `order_quantity` | integer |  | `schema:orderQuantity` | units ordered. |
| `order_item_status` | text |  | `schema:orderItemStatus` | where this line is. values: order_status Values: `payment_due` · `processing` · `in_transit` · `delivered` · `returned` · `cancelled` · `problem`. |
| `fulfillment_stage` | text | ✓ | `acom:fulfillmentStage` | warehouse step of the line; NULL before payment and after delivery. values: fulfillment_stage Values: `awaiting_stock` · `ship_instructed` · `packed` · `shipped`. |
| `price` | numeric(12,2) |  | `schema:price` | list price per unit when ordered. |
| `price_currency` | char(3) |  | `schema:priceCurrency` | ISO 4217 code of price. |
| `discount_value` | numeric(12,2) |  | `schema:discount` | discount on this line, as a positive amount, including its share of order discounts, in major units of the currency. |
| `discount_currency` | char(3) |  | `schema:discount` | ISO 4217 code of discount_value. |
| `promotion_id` | text | ✓ | `acom:promotionId` | promotion whose price or discount applied to the line. → `promotion.promotion` |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `sales.payment`

`schema:PayAction` — payment of one order with one method; card numbers are never stored, only the last four digits. id prefix pay_. · id `pay_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix pay_; the same value the API, logs and events use. |
| `order_id` | text |  | `acom:orderId` | order paid for (ord_ id). → `sales.sales_order` |
| `payment_method` | text |  | `schema:paymentMethod` | how the shopper paid. values: payment_method Values: `credit_card` · `bank_transfer` · `virtual_account` · `loyalty_points` · `store_credit`. |
| `payment_status` | text |  | `schema:paymentStatus` | state of the payment. values: payment_status Values: `payment_due` · `payment_complete` · `payment_declined` · `payment_past_due` · `payment_automatically_applied`. |
| `amount_value` | numeric(12,2) |  | `schema:amount` | amount charged, in major units of the currency. |
| `amount_currency` | char(3) |  | `schema:amount` | ISO 4217 code of amount_value. |
| `card_last4` | text | ✓ | `acom:cardLast4` | last four digits of the card; NULL for other methods. |
| `installment_months` | integer |  | `acom:installmentMonths` | card installments, 0 means paid at once. |
| `external_id` | text | ✓ | `acom:externalId` | the payment provider's reference. |
| `paid_at` | timestamptz | ✓ | `acom:paidAt` | when the payment completed. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `sales.sales_order`

`schema:Order` — an order a shopper placed; order_number is what people see. id prefix ord_. API: order_id. · id `ord_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix ord_; the same value the API, logs and events use. |
| `order_number` | text |  | `schema:orderNumber` | number people see (20402), unique in the store; same digits as the id suffix. |
| `order_date` | timestamptz |  | `schema:orderDate` | when the order was placed. |
| `order_status` | text |  | `schema:orderStatus` | where the order is as a whole; each item also has its own status. values: order_status Values: `payment_due` · `processing` · `in_transit` · `delivered` · `returned` · `cancelled` · `problem`. |
| `customer_id` | text | ✓ | `schema:customer` | shopper who placed it (cus_ id); NULL for a guest order. → `customer.person` |
| `sales_channel` | text |  | `acom:salesChannel` | where the order was placed. values: sales_channel Values: `web` · `mobile_web` · `app` · `agent`. |
| `is_gift` | boolean |  | `schema:isGift` | true for a gift order; recipient details live only on this order. |
| `gift_options` | jsonb | ✓ | `acom:giftOptions` | {card_message, gift_receipt, exchange_until} for gift orders. |
| `discount_code` | text | ✓ | `schema:discountCode` | coupon code recorded on the order; NULL when none. |
| `coupon_id` | text | ✓ | `acom:couponId` | coupon recorded on the order (cpn_ id). → `promotion.coupon` |
| `subtotal_value` | numeric(12,2) |  | `acom:subtotal` | sum of line list prices × quantities before discounts, in major units of the currency. |
| `subtotal_currency` | char(3) |  | `acom:subtotal` | ISO 4217 code of subtotal_value. |
| `discount_value` | numeric(12,2) |  | `schema:discount` | total discount on the order as a positive amount, in major units of the currency. |
| `discount_currency` | char(3) |  | `schema:discount` | ISO 4217 code of discount_value. |
| `shipping_value` | numeric(12,2) |  | `acom:shipping` | shipping charged; 0.00 when free, in major units of the currency. |
| `shipping_currency` | char(3) |  | `acom:shipping` | ISO 4217 code of shipping_value. |
| `gift_wrap_value` | numeric(12,2) | ✓ | `acom:giftWrap` | gift wrap fee; NULL when not wrapped, in major units of the currency. |
| `gift_wrap_currency` | char(3) | ✓ | `acom:giftWrap` | ISO 4217 code of gift_wrap_value. |
| `total_payment_due_value` | numeric(12,2) |  | `schema:totalPaymentDue` | what the shopper pays: subtotal − discount + shipping + gift wrap, in major units of the currency. |
| `total_payment_due_currency` | char(3) |  | `schema:totalPaymentDue` | ISO 4217 code of total_payment_due_value. |
| `preview_id` | text | ✓ | `acom:previewId` | prepare_order preview the shopper confirmed (pv_ id). → `platform.preview` |
| `confirmed_at` | timestamptz | ✓ | `acom:confirmedAt` | when the purchase was confirmed after delivery (order.yml auto_confirm_days). |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

## `search`

### `search.search_document`

`-` — one searchable document per product: weighted full-text vector, embedding and filter columns, rebuilt from catalog through the outbox.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `product_group_id` | text |  | `-` | product the document indexes (prd_ id; API product_id). → `catalog.product_group` |
| `search_vector` | tsvector |  | `-` | weighted text: name A, brand and categories B, material C, description D. |
| `embedding` | vector(1024) | ✓ | `-` | embedding of the product text from the configured embedding model; NULL until computed. |
| `embedding_model` | text | ✓ | `-` | model that produced embedding. |
| `brand_id` | text | ✓ | `-` | brand filter (brn_ id). |
| `category_ids` | text[] |  | `-` | standard and display categories including ancestors (cat_ ids). |
| `keywords` | text[] |  | `-` | product tags for the tags filter. |
| `available_sizes` | text[] |  | `-` | sizes of variants that are in stock. |
| `available_colors` | text[] |  | `-` | colors of variants that are in stock. |
| `price_min` | numeric(12,2) | ✓ | `-` | lowest list price among the variants. |
| `price_max` | numeric(12,2) | ✓ | `-` | highest list price among the variants. |
| `price_currency` | char(3) | ✓ | `-` | ISO 4217 code of price_min and price_max. |
| `approval_status` | text |  | `-` | approval status copied from the product; shoppers only see active. values: approval_status Values: `draft` · `pending_review` · `active` · `rejected` · `discontinued`. |
| `source_version` | integer |  | `-` | product_group.version the document was built from. |
| `indexed_at` | timestamptz |  | `-` | when the document was last rebuilt. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |
