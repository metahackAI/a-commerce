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

### `analytics.campaign_send` (view)

`schema:CreativeWork/acom:CampaignSend` — one row per campaign: when it was due, whether and when it went out, and why it failed.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` |  | ✓ | `-` | store (tenant) the campaign belongs to. |
| `campaign_id` |  | ✓ | `acom:campaignId` | the campaign (cmp_ id). |
| `name` |  | ✓ | `schema:name` | campaign name, for example Weekly email. |
| `template_id` |  | ✓ | `acom:templateId` | template it is made from (tpl_ id); the sends of one template are one recurring campaign. |
| `message_channel` |  | ✓ | `acom:messageChannel` | channel it goes out on. values: message_channel |
| `campaign_status` |  | ✓ | `acom:campaignStatus` | where the campaign is. values: campaign_status |
| `scheduled_time` |  | ✓ | `acom:scheduledTime` | when the send was due. |
| `date_sent` |  | ✓ | `schema:dateSent` | when the messages went out; NULL when they did not. |
| `recipient_count` |  | ✓ | `acom:recipientCount` | shoppers it was addressed to. |
| `error_message` |  | ✓ | `schema:error` | why the send failed, in a few words, for example List import failed; NULL unless it failed. |

### `analytics.daily_new_members` (view)

`schema:Observation/acom:NewMembers` — accounts created per store day, by the tier the members are in now.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` |  | ✓ | `-` | store (tenant) the members belong to. |
| `observation_date` |  | ✓ | `schema:observationDate` | calendar day in the store's time zone on which the accounts were created. |
| `member_tier` |  | ✓ | `acom:tierId` | tier the members are in now; NULL for accounts outside every program. |
| `member_count` |  | ✓ | `acom:memberCount` | accounts created that day. |

### `analytics.daily_sales` (view)

`schema:Observation/acom:DailySales` — revenue and order count per store and day, with the day taken in the store's time zone; orders still awaiting payment and cancelled orders are left out.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` |  | ✓ | `-` | store (tenant) the figures belong to. |
| `observation_date` |  | ✓ | `schema:observationDate` | calendar day in the store's time zone on which the orders were placed. |
| `revenue_value` |  | ✓ | `acom:revenue` | sum of total_payment_due of the day's orders (after discounts, including shipping), in major units. |
| `revenue_currency` |  | ✓ | `acom:revenue` | ISO 4217 code of revenue_value. |
| `order_count` |  | ✓ | `acom:orderCount` | number of orders placed that day that were paid and not cancelled. |

### `analytics.member_tier_count` (view)

`schema:Observation/acom:TierCount` — members per tier now, leaving out blocked and withdrawn accounts.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` |  | ✓ | `-` | store (tenant) the members belong to. |
| `member_tier` |  | ✓ | `acom:tierId` | the tier. |
| `member_count` |  | ✓ | `acom:memberCount` | members in the tier now. |

### `analytics.order_line_summary` (view)

`schema:OrderItem/acom:OrderLineSummary` — one row per line of the orders that count as sales (paid, not cancelled): day, variant, product, category, campaign, units and net amount, never who bought it.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` |  | ✓ | `-` | store (tenant) the line belongs to. |
| `order_id` |  | ✓ | `acom:orderId` | the order the line is in (ord_ id). |
| `observation_date` |  | ✓ | `schema:observationDate` | calendar day in the store's time zone on which the order was placed. |
| `product_id` |  | ✓ | `schema:orderedItem` | the variant sold (var_ id; API variant_id). |
| `product_group_id` |  | ✓ | `schema:isVariantOf` | the product the variant belongs to (prd_ id; API product_id). |
| `category_id` |  | ✓ | `schema:category` | the product's primary standard category (cat_ id); NULL when it has none. |
| `campaign_id` |  | ✓ | `acom:campaignId` | campaign the order came from (cmp_ id); NULL when it came from anywhere else. |
| `order_quantity` |  | ✓ | `schema:orderQuantity` | units sold on the line. |
| `revenue_value` |  | ✓ | `acom:revenue` | price × quantity − line discount, in major units; shipping is not on lines. |
| `revenue_currency` |  | ✓ | `acom:revenue` | ISO 4217 code of revenue_value. |

### `analytics.order_summary` (view)

`schema:Order/acom:OrderSummary` — one row per paid order with only what aggregates need (day, state, tier, campaign, amount, return request), never who placed it; orders still awaiting payment are left out.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` |  | ✓ | `-` | store (tenant) the order belongs to. |
| `order_id` |  | ✓ | `acom:orderId` | the order (ord_ id). |
| `observation_date` |  | ✓ | `schema:observationDate` | calendar day in the store's time zone on which the order was placed. |
| `order_status` |  | ✓ | `schema:orderStatus` | where the order is now; cancelled orders count for the cancellation rate only. values: order_status |
| `member_tier` |  | ✓ | `acom:tierId` | the tier the shopper is in now; NULL for guest orders. |
| `campaign_id` |  | ✓ | `acom:campaignId` | campaign the order came from (cmp_ id); NULL when it came from anywhere else. |
| `revenue_value` |  | ✓ | `acom:revenue` | total_payment_due of the order (after discounts, with shipping and gift wrap), in major units. |
| `revenue_currency` |  | ✓ | `acom:revenue` | ISO 4217 code of revenue_value. |
| `has_return_request` |  | ✓ | `acom:hasReturnRequest` | true when a return was asked for on the order and not declined or withdrawn. |

### `analytics.variant_stock_out` (view)

`schema:Offer/acom:StockOut` — one row per variant that is out of stock now, with when it ran out and when it is expected back.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` |  | ✓ | `-` | store (tenant) the variant belongs to. |
| `product_id` |  | ✓ | `schema:itemOffered` | the variant that ran out (var_ id; API variant_id). |
| `product_group_id` |  | ✓ | `schema:isVariantOf` | the product the variant belongs to (prd_ id; API product_id). |
| `product_name` |  | ✓ | `schema:name` | name of the product, for example Cotton chore coat. |
| `size` |  | ✓ | `schema:size` | size word of the variant; NULL for one-size products. |
| `color` |  | ✓ | `schema:color` | color name of the variant; NULL when the product has one color. |
| `is_bestseller` |  | ✓ | `acom:isBestseller` | true when the store tags the product bestseller (catalog keywords). |
| `sold_out_at` |  | ✓ | `acom:soldOutAt` | when the variant ran out. |
| `availability_starts` |  | ✓ | `schema:availabilityStarts` | when stock is expected back; NULL when unknown. |

## `catalog`

### `catalog.answer`

`schema:Answer` — the accepted answer to a product question; one per question. id prefix ans_. · id `ans_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix ans_; the same value the API, logs and events use (API answer_id). |
| `question_id` | text |  | `schema:parentItem` | the question answered (qst_ id); it shows the answer as acceptedAnswer. → `catalog.question` |
| `author_id` | text |  | `schema:author` | who answered: an operator (adm_), the system (sys_), or a shopper (cus_) when catalog.yml question.answered_by allows it. |
| `author_name` | text | ✓ | `schema:author` | the masked name shown for a shopper's answer; NULL when the store answered, shown as the store. |
| `text` | text |  | `schema:text` | the answer shoppers read, word for word as previewed. |
| `date_created` | timestamptz |  | `schema:dateCreated` | when the answer was posted (store clock). |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

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

### `catalog.copy_draft`

`schema:CreativeWork/acom:CopyDraft` — generated text for a product (generate_copy), kept as a draft until update_product_group uses it. id prefix cpd_. · id `cpd_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix cpd_; the same value the API, logs and events use (API copy_draft_id). |
| `product_group_id` | text |  | `schema:about` | the product the copy is for (prd_ id, API product_id). → `catalog.product_group` |
| `copy_type` | text |  | `acom:copyType` | which text of the product this drafts. values: copy_type Values: `title` · `summary` · `description` · `seo`. |
| `text` | text |  | `schema:text` | the drafted text. |
| `keywords` | text[] |  | `schema:keywords` | words the draft was asked to use; empty when none. |
| `tone` | text |  | `acom:tone` | the tone the draft was written in: the request's, or catalog.yml content.copy.tone. |
| `is_ai_generated` | boolean |  | `acom:aiGenerated` | true for generated content (catalog.yml content.ai_content_disclosure). |
| `date_created` | timestamptz |  | `schema:dateCreated` | when the draft was made (store clock). |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `catalog.data_feed`

`schema:DataFeed` — a supplier sheet as uploaded, one element per row, read by batch_import. id prefix upl_. API: upload_id. · id `upl_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix upl_; the same value the API, logs and events use. |
| `name` | text |  | `schema:name` | file name as uploaded, for example fw26_supplier.xlsx. |
| `encoding_format` | text |  | `schema:encodingFormat` | media type of the uploaded file, for example text/csv. |
| `provider_name` | text | ✓ | `schema:provider` | name of the supplier that sent the sheet; NULL when unknown. |
| `data_feed_element` | jsonb |  | `schema:dataFeedElement` | the rows in sheet order, each a map from column name to cell text: [{supplier_code, style_name, color, category, brand, sizes, cost, units_per_size, material, image, supplier_text}]. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `catalog.data_feed_item`

`schema:DataFeedItem` — one supplier sheet row and the draft product it became, with what is still open before the draft can be published.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `product_import_id` | text |  | `acom:productImportId` | the import run that read the row (imp_ id). → `catalog.product_import` |
| `position` | integer |  | `schema:position` | row number in the sheet, from 1. |
| `product_group_id` | text |  | `schema:item` | the draft product made from the row (prd_ id). → `catalog.product_group` |
| `supplier_code` | text |  | `acom:supplierCode` | the supplier's own code for the row, for example FW26-OT-014. |
| `supplier_text` | text | ✓ | `acom:supplierText` | the supplier's description, kept so the store can write its own copy in its tone; NULL when the sheet had none. |
| `draft_status` | text |  | `acom:draftStatus` | whether the draft is complete or waits for a person. values: draft_status Values: `ready` · `needs_you` · `working`. |
| `open_fields` | text[] |  | `acom:openFields` | required fields that were missing or unclear in the sheet, from catalog.yml import.required_fields, for example {material}; empty when none. |
| `draft_note` | text | ✓ | `acom:draftNote` | one sentence for the person about what is open, for example Fabric mix is missing in the sheet; NULL when nothing is. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `catalog.image_object`

`schema:ImageObject` — a generated product image (generate_image), kept as a draft off the product page until it is used. id prefix img_. · id `img_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix img_; the same value the API, logs and events use (API image_id). |
| `product_group_id` | text |  | `schema:about` | the product pictured (prd_ id, API product_id). → `catalog.product_group` |
| `content_url` | text |  | `schema:contentUrl` | where the image is; while no image generator is connected (catalog.yml content.generator: placeholder), a placeholder picture that says so. |
| `encoding_format` | text |  | `schema:encodingFormat` | media type of the image, for example image/svg+xml. |
| `caption` | text |  | `schema:caption` | the alternative text shown with the image. |
| `prompt` | text |  | `acom:prompt` | what the operator asked the image to show. |
| `image_style` | text |  | `acom:imageStyle` | how the image is staged. values: image_style Values: `studio` · `lifestyle` · `flat_lay`. |
| `is_ai_generated` | boolean |  | `acom:aiGenerated` | true for generated content (catalog.yml content.ai_content_disclosure). |
| `date_created` | timestamptz |  | `schema:dateCreated` | when the image was made (store clock). |
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

### `catalog.product_import`

`schema:CreateAction/acom:ProductImport` — one batch_import run that turned a supplier sheet into product drafts. id prefix imp_. API: import_id. · id `imp_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix imp_; the same value the API, logs and events use. |
| `data_feed_id` | text |  | `schema:object` | the supplier sheet imported (upl_ id). → `catalog.data_feed` |
| `import_mode` | text |  | `acom:importMode` | create makes new drafts only; upsert also refreshes drafts an earlier import made. values: import_mode Values: `create` · `upsert`. |
| `action_status` | text |  | `schema:actionStatus` | whether the run finished. values: action_status Values: `potential` · `active` · `completed` · `failed`. |
| `agent_id` | text |  | `schema:agent` | who ran the import: the change-set runner (sys_) or an operator (adm_). |
| `change_set_id` | text | ✓ | `acom:changeSetId` | the approved change set the import ran in (cs_ id); undoing the set removes its drafts. → `platform.change_set` |
| `start_time` | timestamptz |  | `schema:startTime` | when the run started. |
| `end_time` | timestamptz | ✓ | `schema:endTime` | when the run finished; NULL while it runs. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `catalog.question`

`schema:Question` — a shopper's question about a product; a hidden question hides its answer too. id prefix qst_. · id `qst_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix qst_; the same value the API, logs and events use (API question_id). |
| `product_group_id` | text |  | `schema:about` | the product asked about (prd_ id, API product_id). → `catalog.product_group` |
| `author_id` | text |  | `schema:author` | the shopper who asked (cus_ id); only they may edit or delete it. → `customer.person` |
| `author_name` | text |  | `schema:author` | the name shown with the question, masked as catalog.yml author_masking says when it was asked, for example Ja***. |
| `text` | text |  | `schema:text` | the question as the shopper wrote it. |
| `is_private` | boolean |  | `acom:isPrivate` | true when only the asker and the store may read it (catalog.yml question.private_allowed). |
| `creative_work_status` | text |  | `schema:creativeWorkStatus` | visible, or hidden by moderate_question or held by question.profanity_filter. values: question_status Values: `visible` · `hidden`. |
| `date_created` | timestamptz |  | `schema:dateCreated` | when the shopper asked (store clock). |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `catalog.reorder`

`schema:Order/acom:Reorder` — stock the store orders from its supplier for one product; id prefix ro_. API: reorder. · id `ro_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | reorder id (ro_ prefix, API reorder_id). |
| `product_group_id` | text |  | `schema:orderedItem` | the product the reorder is for (prd_ id, API product_id); its items name the variants. → `catalog.product_group` |
| `order_status` | text |  | `schema:orderStatus` | processing once ordered, in_transit when the supplier sends it, delivered when received, cancelled when called off before that. values: order_status Values: `payment_due` · `processing` · `in_transit` · `delivered` · `returned` · `cancelled` · `problem`. |
| `order_date` | timestamptz |  | `schema:orderDate` | when the store placed the reorder (store clock). |
| `expected_arrival_until` | timestamptz |  | `schema:expectedArrivalUntil` | the end of the day the stock is expected (order date + reorder.lead_time_days). |
| `total_payment_due_value` | numeric(12,2) |  | `schema:totalPaymentDue` | what the store owes the supplier: the sum of each item's quantity times its unit cost. |
| `total_payment_due_currency` | char(3) |  | `schema:priceCurrency` | currency of total_payment_due_value (ISO 4217). |
| `description` | text | ✓ | `schema:description` | a note for the supplier or why it was ordered; NULL when none. |
| `created_at` | timestamptz |  | `-` | when the row was written. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `catalog.reorder_item`

`schema:OrderItem` — one variant of a reorder and how many of it. API: reorder_item.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `reorder_id` | text |  | `-` | the reorder this item belongs to (ro_ id). → `catalog.reorder` |
| `position` | integer |  | `-` | the item's place in the reorder, from 1. |
| `product_id` | text |  | `schema:orderedItem` | the variant ordered (var_ id, API variant_id). → `catalog.product` |
| `order_quantity` | integer |  | `schema:orderQuantity` | how many units of the variant are ordered. |
| `price` | numeric(12,2) |  | `schema:price` | what the store pays the supplier for one unit: the variant's cost when it was ordered. |
| `price_currency` | char(3) |  | `schema:priceCurrency` | currency of price (ISO 4217). |
| `created_at` | timestamptz |  | `-` | when the row was written. |

### `catalog.restock_alert`

`schema:SubscribeAction/acom:RestockAlert` — a shopper asks to be told when a sold-out variant is back; a transactional alert, not marketing. id prefix rsa_. · id `rsa_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix rsa_; the same value the API, logs and events use (API restock_alert_id). |
| `customer_id` | text |  | `schema:agent` | the shopper who asked (cus_ id); only they see it. → `customer.person` |
| `product_id` | text |  | `schema:object` | the variant waited for (var_ id, API variant_id). → `catalog.product` |
| `message_channel` | text |  | `acom:messageChannel` | how the shopper wants to hear (catalog.yml restock_alert.channels). values: message_channel Values: `email` · `sms` · `push` · `in_app`. |
| `start_time` | timestamptz |  | `schema:startTime` | when the shopper asked (store clock). |
| `expires` | timestamptz |  | `schema:expires` | when the request lapses if the variant is not back by then (start_time + restock_alert.expires_after_days). |
| `notified_at` | timestamptz | ✓ | `acom:notifiedAt` | when the variant came back and the alert was handed to messaging (platform.outbox topic restock_alert.notify); NULL while waiting. |
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
| `image` | text[] |  | `schema:image` | URLs of photos the reviewer added; empty when none. |
| `creative_work_status` | text |  | `schema:creativeWorkStatus` | moderation state. values: review_status Values: `visible` · `hidden` · `reported`. |
| `date_published` | date |  | `schema:datePublished` | date the review was posted. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `catalog.review_report`

`schema:Action/acom:ContentReport` — a shopper's report that a review is inappropriate; it puts the review in the moderation queue. id prefix rpt_. · id `rpt_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix rpt_; the same value the API, logs and events use. |
| `review_id` | text |  | `schema:object` | the review reported (rev_ id). → `catalog.review` |
| `customer_id` | text |  | `schema:agent` | the shopper who reported it (cus_ id); one report per shopper and review. → `customer.person` |
| `report_reason` | text |  | `acom:reportReason` | why it was reported (catalog.yml review.report_reasons). values: report_reason Values: `spam` · `abuse` · `obscene` · `advertisement` · `fake` · `irrelevant` · `other`. |
| `description` | text | ✓ | `schema:description` | the reporter's own words; NULL when they gave none. |
| `start_time` | timestamptz |  | `schema:startTime` | when the report was made (store clock). |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `catalog.review_vote`

`schema:VoteAction/acom:ReviewVote` — one shopper's helpful vote on a review; a review's helpful count is its rows with is_helpful true.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `review_id` | text |  | `schema:object` | review voted on (rev_ id). → `catalog.review` |
| `customer_id` | text |  | `schema:agent` | shopper who voted (cus_ id). → `customer.person` |
| `is_helpful` | boolean |  | `acom:isHelpful` | true while the shopper marks the review helpful; false after they take the vote back. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `catalog.type_and_quantity_node`

`schema:TypeAndQuantityNode` — one component of a bundle (create_bundle): a variant and how many of it; the bundle's price and stock are derived from its components.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `product_group_id` | text |  | `acom:productGroupId` | the bundle this is a component of (prd_ id); schema:includesObject seen from the bundle. → `catalog.product_group` |
| `product_id` | text |  | `schema:typeOfGood` | the component variant (var_ id, API variant_id). → `catalog.product` |
| `amount_of_this_good` | integer |  | `schema:amountOfThisGood` | units of the variant in one bundle (API order_quantity). |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `catalog.unit_price_specification`

`schema:UnitPriceSpecification` — a list price scheduled for a period (schedule_price_change); it applies from valid_from, and the offer price applies again after valid_through. id prefix ups_. · id `ups_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix ups_; the same value the API, logs and events use. |
| `offer_id` | text |  | `acom:offerId` | the offer whose list price this schedules (off_ id). → `catalog.offer` |
| `price` | numeric(12,2) |  | `schema:price` | the list price per unit during the period. |
| `price_currency` | char(3) |  | `schema:priceCurrency` | ISO 4217 code of price. |
| `price_type` | text |  | `schema:priceType` | always list_price here; member and sale prices live in promotion.unit_price_specification. values: price_type Values: `list_price` · `sale_price`. |
| `valid_from` | timestamptz |  | `schema:validFrom` | first moment the scheduled price applies. |
| `valid_through` | timestamptz | ✓ | `schema:validThrough` | last moment it applies (inclusive); NULL means until changed. |
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

### `customer.wishlist_item`

`schema:ListItem/acom:WishlistItem` — one product a shopper keeps for later; the product id names the item, there is no id of its own.

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `customer_id` | text |  | `schema:customer` | shopper whose wishlist this is (cus_ id). → `customer.person` |
| `product_group_id` | text |  | `schema:item` | the product kept (prd_ id; API product_id). → `catalog.product_group` |
| `created_at` | timestamptz |  | `-` | when the shopper added the product (API added_at). |
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

### `merchandising.category_mapping`

`schema:Intangible/acom:CategoryMapping` — links a standard category to a display category, so the display tree shows the standard category's products. id prefix map_. · id `map_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix map_; the same value the API, logs and events use. |
| `standard_category_id` | text |  | `acom:standardCategory` | the internal category whose products are shown (cat_ id, term set standard). → `merchandising.defined_term` |
| `display_category_id` | text |  | `acom:displayCategory` | the shopper menu category that shows them (cat_ id, term set display). → `merchandising.defined_term` |
| `is_primary` | boolean |  | `acom:isPrimary` | true for the one display category a standard category's product pages show as their path. |
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

### `messaging.campaign`

`schema:CreativeWork/acom:Campaign` — a marketing message sent to many shoppers at a scheduled time from one template; sending it reaches customers. id prefix cmp_. · id `cmp_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix cmp_; the same value the API, logs and events use. |
| `name` | text |  | `schema:name` | name operators know the campaign by, for example Weekly email. |
| `message_channel` | text |  | `acom:messageChannel` | channel it is sent on; the template must be written for the same channel. values: message_channel Values: `email` · `sms` · `push` · `in_app`. |
| `template_id` | text | ✓ | `acom:templateId` | template the message is made from (tpl_ id); campaigns of one template are the same recurring send. → `messaging.message_template` |
| `valid_for_member_tier` | text | ✓ | `schema:validForMemberTier` | tier the campaign goes to; NULL means every member who agreed to marketing messages. → `customer.member_program_tier` |
| `campaign_status` | text |  | `acom:campaignStatus` | where the campaign is; failed means the send stopped before any message went out. values: campaign_status Values: `draft` · `scheduled` · `sending` · `sent` · `failed` · `cancelled`. |
| `scheduled_time` | timestamptz | ✓ | `acom:scheduledTime` | when the send is due, on the store clock; NULL while a draft has no time. |
| `date_sent` | timestamptz | ✓ | `schema:dateSent` | when the messages went out; NULL while not sent, and for failed and cancelled campaigns. |
| `recipient_count` | integer | ✓ | `acom:recipientCount` | shoppers the campaign was addressed to after consent checks; NULL until the list is built. |
| `error` | jsonb | ✓ | `schema:error` | why the send failed, as {code, message}; NULL unless campaign_status is failed. |
| `change_set_id` | text | ✓ | `acom:changeSetId` | change set that approved the campaign; NULL for sends set up before change sets. → `platform.change_set` |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

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

### `messaging.message_template`

`schema:CreativeWork/acom:MessageTemplate` — a reusable message for one channel, with named variables; creating one sends nothing. id prefix tpl_. · id `tpl_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix tpl_; the same value the API, logs and events use. |
| `name` | text |  | `schema:name` | name operators pick the template by, for example Back in stock. |
| `message_channel` | text |  | `acom:messageChannel` | channel the template is written for; a campaign must use the same channel. values: message_channel Values: `email` · `sms` · `push` · `in_app`. |
| `message_purpose` | text |  | `acom:messagePurpose` | marketing (needs opt-in consent, held in quiet hours) or transactional (about an order or claim). |
| `headline` | text | ✓ | `schema:headline` | subject line for email and push; NULL for sms and in-app. |
| `text` | text |  | `schema:text` | body with variables written as {{name}}, up to 2,000 characters. |
| `variables` | text[] |  | `acom:variables` | variable names the headline and text may use, for example {customer_name,product_name}. |
| `in_language` | text |  | `schema:inLanguage` | BCP 47 language tag of the text, for example en-US. |
| `template_status` | text |  | `acom:templateStatus` | draft until a campaign uses the template. |
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
| `organization_id` | text | ✓ | `schema:worksFor` | the organization (org_ id) the operator belongs to; NULL when none. API: org_id. → `platform.organization` |

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

### `platform.api_key`

`schema:Intangible/acom:ApiKey` — a credential an outside agent or integration client signs in with (login); the secret is shown once when issued and kept only as a hash. id prefix key_. · id `key_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix key_; the same value the API, logs and events use; the client_id at login. |
| `name` | text |  | `schema:name` | what the key is for, for example Outside shopping agent; unique among active keys. |
| `key_prefix` | text |  | `acom:keyPrefix` | the first characters of the secret, shown in lists so people can recognize a key. |
| `secret_hash` | text |  | `-` | SHA-256 (hex) of the secret; the secret itself is never stored. |
| `scopes` | text[] |  | `acom:scopes` | the scopes a session of this key may get, for example {external:read,external:cart}. |
| `api_key_status` | text |  | `acom:apiKeyStatus` | whether the key can still sign in. values: api_key_status Values: `active` · `revoked`. |
| `expires` | timestamptz | ✓ | `schema:expires` | when the key stops working; NULL means it does not expire. |
| `creator_id` | text | ✓ | `schema:creator` | the operator (adm_ id) who asked for the key. → `platform.admin_user` |
| `last_used_at` | timestamptz | ✓ | `acom:lastUsedAt` | the last successful sign-in with the key; NULL if never. |
| `revoked_at` | timestamptz | ✓ | `acom:revokedAt` | when the key was revoked; NULL while active. |
| `revocation_reason` | text | ✓ | `acom:revocationReason` | why the key was revoked, in a sentence; NULL while active. |
| `failed_attempt_count` | integer |  | `-` | wrong secrets since first_failed_at; customer.yml password.lockout locks the key when it reaches failed_attempts. |
| `first_failed_at` | timestamptz | ✓ | `-` | the first wrong secret of the current lockout window; NULL when there is none. |
| `locked_until` | timestamptz | ✓ | `acom:lockedUntil` | sign-in is refused until then after too many wrong secrets; NULL when not locked. |
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

### `platform.menu_favorite`

`-` — whether an operator keeps a back-office menu entry among their favorites (set_favorite).

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `admin_user_id` | text |  | `-` | the operator account (adm_ id; API admin_id). → `platform.admin_user` |
| `site_navigation_element_id` | text |  | `-` | the menu entry (mnu_ id; API menu_id). → `platform.site_navigation_element` |
| `is_favorite` | boolean |  | `-` | true while the entry is a favorite; turning it off keeps the row. |
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
| `seed_fingerprint` | text | ✓ | `-` | fingerprint (SHA-256 of the rows) of the demo seed this store was loaded from; NULL for a store not loaded from a seed. |
| `store_status` | text |  | `acom:storeStatus` | whether the store is open for business; a suspended store keeps its data but serves no one. values: store_status Values: `active` · `suspended`. |
| `feature_flags` | jsonb |  | `acom:featureFlags` | features switched on for this store, as {flag: true\|false}; {} when none. |

### `platform.organization`

`schema:Organization` — a department, vendor or branch that operator accounts belong to; organizations nest. id prefix org_. API: org. · id `org_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix org_; the same value the API, logs and events use (API: org_id). |
| `name` | text |  | `schema:name` | organization name shown to administrators, unique in the store. |
| `organization_type` | text |  | `acom:organizationType` | what kind of organization this is (API: org_type). values: organization_type Values: `department` · `vendor` · `branch`. |
| `parent_organization_id` | text | ✓ | `schema:parentOrganization` | the organization this one belongs to; NULL at the top. Never itself or one of its own sub-organizations (orgs.no_cycle). → `platform.organization` |
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
| `subject_id` | text | ✓ | `-` | who holds the session: a person (adm_ or cus_ id), an API client (key_ id), or NULL for an anonymous guest. |
| `token_hash` | text |  | `-` | hash of the session cookie value; the cookie itself is never stored. |
| `csrf_token_hash` | text |  | `-` | hash of the CSRF token paired with the session. |
| `expires_at` | timestamptz |  | `-` | when the session stops being accepted. |
| `last_seen_at` | timestamptz | ✓ | `-` | last request made with the session. |
| `revoked_at` | timestamptz | ✓ | `-` | when the session was signed out or revoked; NULL while active. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |
| `scopes` | text[] | ✓ | `-` | scopes of an API client session, fixed at login; NULL for a browser session, whose scopes come from roles. |
| `refresh_token_hash` | text | ✓ | `-` | SHA-256 (hex) of an API client session's refresh token; NULL for a browser session. |
| `refresh_expires_at` | timestamptz | ✓ | `-` | when the refresh token stops being accepted; NULL for a browser session. |
| `replaced_by_id` | text | ✓ | `-` | the session its refresh token was exchanged for; a refresh token that comes back after that is reuse and ends every session of the client. → `platform.session` |

### `platform.site_navigation_element`

`schema:SiteNavigationElement` — one entry of the back-office menu; entries nest one level under a parent. id prefix mnu_. API: menu. · id `mnu_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix mnu_; the same value the API, logs and events use (API: menu_id). |
| `name` | text |  | `schema:name` | the label operators see, for example Promotions. |
| `url` | text |  | `schema:url` | the back-office path the entry opens, for example /bo/promotions (API: path). |
| `required_scope` | text |  | `acom:requiredScope` | the scope an operator needs to see the entry, for example tools:read:promotion. |
| `position` | integer |  | `schema:position` | order among the entries with the same parent, smallest first. |
| `is_visible` | boolean |  | `acom:isVisible` | false hides the entry from every operator without removing it. |
| `is_part_of_id` | text | ✓ | `schema:isPartOf` | the parent entry (mnu_ id); NULL for a top-level entry. → `platform.site_navigation_element` |
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

### `promotion.conditional_gift`

`schema:Offer/acom:ConditionalGift` — a gift added to an order whose subtotal before member benefits reaches a threshold, with one gift per threshold. id prefix cgf_. · id `cgf_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix cgf_; the same value the API, logs and events use. |
| `name` | text |  | `schema:name` | name people read, for example Holiday gift over $120. |
| `gift_tiers` | jsonb |  | `acom:giftTiers` | thresholds in ascending order, each with its gift: [{trigger_threshold: {value, currency}, gift_product_id}]. |
| `valid_for_member_tier` | text | ✓ | `schema:validForMemberTier` | tier the gift is for; NULL means everyone (the reserved word everyone is never stored). → `customer.member_program_tier` |
| `sales_channels` | text[] | ✓ | `acom:salesChannel` | channels where the gift applies; NULL means every channel. values: sales_channel |
| `valid_from` | timestamptz |  | `schema:validFrom` | first moment the gift applies. |
| `valid_through` | timestamptz |  | `schema:validThrough` | last moment it applies (inclusive, minute precision). |
| `change_set_id` | text | ✓ | `acom:changeSetId` | change set that created it; NULL for seeded ones. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

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

### `promotion.draw_record`

`schema:ChooseAction/acom:DrawRecord` — the audit record of one draw run: method, seed, entrant snapshot and result, so anyone can repeat it. Rows are only added. id prefix drw_. · id `drw_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix drw_; the same value the API, logs and events use. |
| `event_id` | text |  | `schema:object` | event drawn (evt_ id). → `promotion.event` |
| `draw_position` | integer |  | `acom:drawPosition` | which run of the event's draws this is, from 1; part of the shuffle key. |
| `draw_mode` | text |  | `acom:drawMode` | what the run did, one of promotion.yml event.draw.modes: draw, redraw, cancel_winner, promote_waitlist, add_waitlist or manual_select. |
| `rank` | integer | ✓ | `acom:rank` | the reward rank drawn; NULL for every rank. |
| `method` | text |  | `acom:method` | how winners were chosen, from promotion.yml event.draw.fairness.method, for example seeded_shuffle. |
| `seed_commitment` | text |  | `acom:seedCommitment` | sha256 of the seed, as published before entries closed. |
| `seed` | text |  | `acom:seed` | the seed the shuffle used, revealed by the draw. |
| `entrant_snapshot_digest` | text |  | `acom:entrantSnapshotDigest` | sha256 of the entry ids the draw chose from, in order. |
| `entrant_count` | integer |  | `acom:entrantCount` | how many entries the draw chose from. |
| `excluded` | jsonb |  | `acom:excluded` | entries left out and why: {previous_winners, invalid_entries}. |
| `result` | jsonb |  | `schema:result` | [{rank, winners[], waitlist[], cancelled[]}] with entry ids. |
| `reason` | text | ✓ | `acom:reason` | why a person redrew, cancelled a winner or chose by hand. |
| `agent_id` | text |  | `schema:agent` | who asked for the draw (adm_ or sys_ id). |
| `change_set_id` | text | ✓ | `acom:changeSetId` | change set whose approval ran the draw; its approver is the person who approved it. |
| `start_time` | timestamptz |  | `schema:startTime` | when the draw ran, on the store clock. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version; draw records are never changed. |

### `promotion.event`

`schema:Event` — a promotion event: shoppers enter by an entry mechanic and winners are chosen by a selection method, with rewards by rank. id prefix evt_. · id `evt_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix evt_; the same value the API, logs and events use. |
| `name` | text |  | `schema:name` | name people read, for example Fall check-in week. |
| `event_status` | text |  | `acom:eventStatus` | stored status; scheduled and running follow the entry period and the store clock, closed after it ends. values: event_status Values: `draft` · `scheduled` · `running` · `paused` · `closed` · `drawing` · `announced` · `fulfilling` · `completed` · `cancelled`. |
| `start_date` | timestamptz |  | `schema:startDate` | first moment entries are taken. |
| `end_date` | timestamptz |  | `schema:endDate` | last moment entries are taken (inclusive, minute precision). |
| `display_valid_from` | timestamptz | ✓ | `acom:displayValidFrom` | when shoppers start seeing the event; NULL means from start_date. |
| `display_valid_through` | timestamptz | ✓ | `acom:displayValidThrough` | when shoppers stop seeing it; NULL means until end_date. |
| `entry_mechanic` | text |  | `acom:entryMechanic` | how shoppers enter, an id of the store registry promotion.yml event.entry_mechanics, for example check_in. |
| `selection_method` | text |  | `acom:selectionMethod` | how winners are chosen, an id of the store registry promotion.yml event.selection_methods, for example random_draw. |
| `entry_payload_schema` | jsonb | ✓ | `acom:entryPayloadSchema` | fields an entry must send, {field: type}; NULL when an entry needs no data. |
| `reward_tiers` | jsonb |  | `acom:rewardTiers` | rewards by rank: [{rank, reward_type, reward_ref, reward_value: {value, currency}, winner_count, waitlist_count, odds, round}]. |
| `eligibility` | jsonb |  | `acom:eligibility` | who may enter: {valid_for_member_tier, segment_id, member_types[], purchase_condition: {min_subtotal, window, scope}, channels[]}. |
| `valid_for_member_tier` | text | ✓ | `schema:validForMemberTier` | tier allowed to enter, the same as eligibility.valid_for_member_tier; NULL means everyone. → `customer.member_program_tier` |
| `entry_limit` | jsonb |  | `acom:entryLimit` | entries one member may make: {count, period: day, week, event or unlimited}. |
| `draw_at` | timestamptz | ✓ | `acom:drawAt` | when the draw is planned; NULL when not planned. |
| `announce_at` | timestamptz | ✓ | `acom:announceAt` | when winners are announced; NULL when not planned. |
| `dispatch_at` | timestamptz | ✓ | `acom:dispatchAt` | when rewards are sent; NULL when not planned. |
| `compliance_jurisdictions` | text[] |  | `acom:complianceJurisdictions` | store-defined jurisdiction ids whose rules apply; empty means chosen by promotion.yml event.compliance.jurisdiction_from. |
| `seed_commitment` | text |  | `acom:seedCommitment` | sha256 of the draw seed, published when the event is created so the draw can be checked afterwards. |
| `draw_seed` | text |  | `acom:seed` | the secret draw seed; shown only in draw records once a draw ran. |
| `change_set_id` | text | ✓ | `acom:changeSetId` | change set that created the event; NULL for seeded ones. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `promotion.event_entry`

`schema:JoinAction/acom:EventEntry` — one entry of a member in an event, and whether it won. id prefix ent_. · id `ent_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix ent_; the same value the API, logs and events use. |
| `event_id` | text |  | `schema:object` | event entered (evt_ id). → `promotion.event` |
| `customer_id` | text |  | `schema:agent` | member who entered (cus_ id). → `customer.person` |
| `entry_status` | text |  | `acom:entryStatus` | entered until a draw or an instant result decides it. values: entry_status Values: `entered` · `invalid` · `won` · `lost` · `waitlist` · `cancelled`. |
| `entry_payload` | jsonb |  | `acom:entryPayload` | what the member sent, following the event's entry_payload_schema (answers, a caption, a code). |
| `entry_consent` | jsonb |  | `acom:entryConsent` | consents given when entering, {consent type: true}. |
| `order_id` | text | ✓ | `acom:orderId` | order the entry is based on for purchase_triggered events (ord_ id); NULL otherwise. |
| `period_bucket` | text |  | `acom:periodBucket` | the entry-limit period the entry counts in: a store date for day, an ISO week for week, event or unlimited. |
| `entry_number` | integer |  | `acom:entryNumber` | which entry of the member in that period it is, from 1. |
| `rank` | integer | ✓ | `acom:rank` | reward rank won or waited for; NULL until decided. |
| `waitlist_position` | integer | ✓ | `acom:waitlistPosition` | place on the waitlist of its rank, from 1; NULL when not waiting. |
| `draw_record_id` | text | ✓ | `acom:drawRecordId` | draw that last decided the entry (drw_ id); NULL for instant results. |
| `start_time` | timestamptz |  | `schema:startTime` | when the member entered, on the store clock. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `promotion.event_reward`

`schema:Offer/acom:EventReward` — the reward of one winning entry and how it is sent. id prefix rwd_. · id `rwd_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix rwd_; the same value the API, logs and events use. |
| `event_id` | text |  | `acom:eventId` | event the reward belongs to (evt_ id). → `promotion.event` |
| `entry_id` | text |  | `acom:entryId` | winning entry (ent_ id). → `promotion.event_entry` |
| `customer_id` | text |  | `schema:customer` | member who gets the reward (cus_ id). → `customer.person` |
| `rank` | integer |  | `acom:rank` | reward rank won. |
| `reward_type` | text |  | `acom:rewardType` | kind of reward, an id of the store registry promotion.yml event.reward_types, for example product. |
| `reward_ref` | text | ✓ | `acom:rewardRef` | what is given: a product (prd_), a coupon (cpn_) or free text; NULL when the type needs none. |
| `reward_value` | numeric(12,2) | ✓ | `acom:rewardValue` | value of the reward, in major units of the currency; NULL when the type has no value. |
| `reward_currency` | char(3) | ✓ | `acom:rewardValue` | ISO 4217 code of reward_value. |
| `fulfillment` | text |  | `acom:fulfillment` | how it is sent: ship, issue, grant, email or manual (promotion.yml event.reward_types.fulfillment_methods). |
| `reward_status` | text |  | `acom:rewardStatus` | compliance_pending while a store rule waits for the winner. values: reward_status Values: `pending` · `compliance_pending` · `ready` · `dispatched` · `delivered` · `failed` · `forfeited`. |
| `compliance_requirements` | text[] |  | `acom:complianceRequirements` | what the winner still has to provide (legal_name, address, tax_id); empty when nothing. |
| `fulfillment_ref` | text | ✓ | `acom:fulfillmentRef` | what sending made: a member coupon (mcp_), a ledger entry (lyt_), or the reference of a stubbed shipment or email. |
| `reward_batch_id` | text |  | `acom:rewardBatchId` | the dispatch run that made it (rwb_ id). |
| `dispatched_at` | timestamptz | ✓ | `acom:dispatchedAt` | when it was sent; NULL while held or pending. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version, +1 on every change; previews record it in basis and execution compares it. |

### `promotion.loyalty_transaction`

`schema:MoneyTransfer/acom:LoyaltyTransaction` — one entry of a member's store-point ledger; entries are only added and the balance is their sum. id prefix lyt_. · id `lyt_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix lyt_; the same value the API, logs and events use. |
| `customer_id` | text |  | `schema:customer` | member whose points these are (cus_ id). → `customer.person` |
| `transaction_type` | text |  | `acom:loyaltyTransactionType` | accrual, grant and restore add points; spend, expire and reversal take them away. values: loyalty_transaction_type Values: `accrual` · `spend` · `expire` · `grant` · `restore` · `reversal`. |
| `amount_value` | numeric(12,2) |  | `schema:amount` | points as money, always positive (the type says which way), in major units of the currency. |
| `amount_currency` | char(3) |  | `schema:amount` | ISO 4217 code of amount_value. |
| `order_id` | text | ✓ | `acom:orderId` | order that earned, spent or gave back the points (ord_ id); NULL for grants and expiries. |
| `source_transaction_id` | text | ✓ | `acom:sourceTransaction` | earlier entry this one expires or reverses (lyt_ id); NULL otherwise. → `promotion.loyalty_transaction` |
| `expires` | date | ✓ | `schema:expires` | last day (store time) the points of an accrual, grant or restore can be spent; NULL for entries that take points away. |
| `description` | text | ✓ | `schema:description` | why, in a sentence, for example the reason a person gave for a grant. |
| `agent_id` | text | ✓ | `schema:agent` | who recorded it: an operator (adm_), a shopper (cus_) or the system (sys_). |
| `start_time` | timestamptz |  | `schema:startTime` | when the entry happened on the store clock. |
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
| `promotion_type` | text | ✓ | `acom:promotionType` | kind of benefit a promotion made by create_promotion gives; NULL for member and sale prices (set_sale_prices). values: promotion_type Values: `bogo` · `bundle` · `threshold` · `amount_off`. |
| `promotion_terms` | jsonb |  | `acom:promotionTerms` | parameters of the type: {threshold_benefit: amount_off or free_shipping, buy_quantity, get_quantity, bundle_price: {value, currency}, cost_share: {store, seller} rates 0-1}. |
| `sales_channels` | text[] | ✓ | `acom:salesChannel` | channels where the promotion applies; NULL means every channel. values: sales_channel |

### `promotion.segment`

`schema:Audience/acom:Segment` — a group of customers defined by conditions, used to issue coupons, send messages and limit events; the row version is the segment version. id prefix seg_. · id `seg_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | id with prefix seg_; the same value the API, logs and events use. |
| `name` | text |  | `schema:name` | name people read, for example Lapsed knit buyers. |
| `condition_rules` | jsonb |  | `acom:conditions` | who is in it: {member_tiers[], purchased_category_ids[], last_order_within_days, min_spend: {value, currency}, exclude_segment_ids[]}; every given condition must hold. |
| `estimated_size` | integer |  | `acom:estimatedSize` | how many active members matched when the segment was last defined. |
| `estimated_at` | timestamptz |  | `acom:estimatedAt` | store-clock time estimated_size was counted. |
| `created_at` | timestamptz |  | `-` | when the row was created. |
| `updated_at` | timestamptz |  | `-` | when the row was last changed. |
| `version` | integer |  | `-` | row version and segment version, +1 on every change; previews record it in basis and execution compares it. |

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

### `sales.gift_order`

`schema:AcceptAction/acom:GiftOrder` — a gift waiting for its recipient to accept it and give an address; id prefix gift_. API: gift_order. · id `gift_…`

| column | type | null | term | description |
|---|---|:--:|---|---|
| `tenant_id` | text |  | `-` | store (tenant) that owns this row; row-level security key. → `platform.online_store` |
| `id` | text |  | `-` | gift id (gift_ prefix), the part of the accept link that names the gift (API gift_order_id). |
| `order_id` | text |  | `schema:object` | the gift order the recipient accepts (ord_ id). → `sales.sales_order` |
| `recipient_name` | text |  | `schema:recipient` | the name the sender gave for the recipient, printed on the card and the parcel; used for this gift only. |
| `recipient_contact` | text |  | `acom:recipientContact` | the email or phone the accept link goes to; used for this gift only and never saved to anyone's profile. |
| `action_status` | text |  | `schema:actionStatus` | the recipient's acceptance: potential until they accept, completed once they did, failed when it lapsed. values: action_status Values: `potential` · `active` · `completed` · `failed`. |
| `expires` | timestamptz |  | `schema:expires` | the last moment the recipient can accept (order time + gift.auto_cancel_if_no_address_days); payment waits until then. |
| `accepted_at` | timestamptz | ✓ | `acom:acceptedAt` | when the recipient accepted and gave the address; NULL until then. |
| `created_at` | timestamptz |  | `-` | when the gift was ordered. |
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
| `campaign_id` | text | ✓ | `acom:campaignId` | campaign whose message the shopper followed to place the order (cmp_ id); NULL when the order came from anywhere else. → `messaging.campaign` |

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
