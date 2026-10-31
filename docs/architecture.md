# A-Commerce architecture

This document is the reference for how A-Commerce is built: the services and their trust boundaries, how the API is organized, what makes it friendly to AI agents, how every interface follows [schema.org](https://schema.org), and the AI-native features on top. **The whole server side is written in Rust**: the gateway, the commerce back end, and the agent runtime.

- [1. System at a glance](#1-system-at-a-glance)
- [2. Back-end structure](#2-back-end-structure)
- [3. API design: one hierarchy](#3-api-design-one-hierarchy)
- [4. An interface built for agents](#4-an-interface-built-for-agents)
- [5. Data model: schema.org first](#5-data-model-schemaorg-first)
- [6. AI-native features](#6-ai-native-features)
- [7. From the hackathon build to the target](#7-from-the-hackathon-build-to-the-target)

Related documents: why and the tool design rules in [design.md](design.md); every operation with inputs, outputs, checks, and errors in [api/](api/README.md); the Rust implementation design in [backend/](backend/README.md); the demo store and its scenes in [scenarios.md](scenarios.md); the build plan in [plan.md](plan.md); market research in [research.md](research.md); UI mockups in [design/](../design).

## 1. System at a glance

```
  Storefront chat (/fo)        Back-office chat (/bo)          Outside agents
     shoppers                     operators                 (ChatGPT, Gemini, ...)
         │                           │                             │ MCP (planned)
         └───────── Next.js web ─────┘                             │
                        │ HTTPS (session)                          │
                        ▼                                          ▼
     ┌────────────────────────────── gateway (Rust) ──────────────────────────────┐
     │ who is calling · which store · scopes · signed internal token · CSRF        │
     └──────────────┬─────────────────────────────┬─────────────────────┬──────────┘
                    │ chat turns (SSE)            │ REST, person-only   │ MCP (planned)
                    ▼                             ▼                     ▼
     ┌──────────── agent (Rust) ─────────┐   ┌──────────── commerce (Rust) ───────────────┐
     │ router → specialist agent         │   │ tool layer: 4 groups · 13 domains           │
     │ tool search · plan · execute      │──▶│ one definition per operation → REST + tools │
     │ intent check · UI blocks · memory │   │ policy engine (policies/*.yml)              │
     │ Glimmer worker, Spark planner     │   │ previews · change sets: approve, run,       │
     └─────────────────┬─────────────────┘   │ verify, undo · events · scheduled jobs      │
                       │                     └──────────────────────┬──────────────────────┘
                       └──────────── PostgreSQL + pgvector ─────────┘
```

The MCP path for outside agents is designed but not served yet: `acom-mcp` is an empty crate and commerce has no `/mcp` route.

Every caller, whether our operator agent, our shopper agent, an outside agent, or a button in the UI, goes through the same tool layer and the same policies. The price an operator sets in a change set is the price the shopper agent quotes and the price an outside agent sees.

The split between `agent` and `commerce` is a trust boundary. The agent turns words into tool calls; it cannot approve anything, run an approved change set, or call a person-only endpoint, because the gateway never gives a chat turn the scopes for those actions and `commerce` checks them again. Whatever a prompt says, `commerce` only runs what its rules and a person allow.

## 2. Back-end structure

### 2.1 Services

| Service | Owns | Never does |
|---|---|---|
| **gateway** | Sign-in and sessions, CSRF, store resolution, scopes, the signed internal token, routing, SSE passthrough, handing tasks a watcher opens to the agent | Business rules |
| **commerce** | Every operation and its rules: catalog to platform, the policy engine, the preview store, the change-set engine (approve, run, verify, undo), events, scheduled jobs and watchers, the REST surface and the tool manifest | Calling models |
| **agent** | Conversations, routing to specialist agents, tool search, planning, running reads and previews, intent checks, UI blocks, visible memory, model calls | Touching commerce tables, approving, running change sets, person-only actions |

Design detail: [backend/README.md](backend/README.md) (services and crates), [backend/commerce.md](backend/commerce.md), [backend/agent.md](backend/agent.md), [backend/gateway.md](backend/gateway.md).

### 2.2 Layers inside commerce

| Layer | What it does | Rules it enforces |
|---|---|---|
| **Surfaces** | REST `/v1` for the web and tests, the tool manifest for our agent (`GET /v1/tools`), person-only endpoints, the event stream; MCP for outside agents is planned | Internal token, scopes, store context |
| **Tool layer** | The commerce operations, organized as group → domain → resource → operation (§3) | One business verb per tool, closed typed inputs, preview before write (§4) |
| **Change sets** | Stores previews, groups them, waits for a person, runs them in order, reads them back, undoes them together | Nothing that needs approval runs without it; a person approves exactly what will run |
| **Policy engine** | Loads `policies/*.yml` plus the store's overrides and evaluates checks during previews and runs | Business rules live in data, not in prompts or code constants |
| **Store** | Tables named after schema.org types, append-only event tables, an outbox, a job queue, full-text search | Money paths are transactional; state history is never overwritten |

### 2.3 Code layout

One Cargo workspace ([backend/README.md](backend/README.md) §2):

```
crates/   core · contract · policy · store · auth · changeset
          domains/{platform, catalog, customer, promotion, merchandising, search, sales,
                   fulfillment, messaging, analytics, community, partner, integration}
          registry · http · mcp (empty) · llm · agent · facade · server · testkit
services/ gateway · commerce · agent
policies/ agents/ seed/ eval/ migrations/ infra/ xtask/
web/      Next.js: /bo and /fo, one component per UI block
```

An operation `domain.resource.operation` lives in `crates/domains/{domain}/src/{resource}/{tool}.rs`, one file per operation (`cargo xtask check-layout`). `core`, `contract`, and `policy` do no I/O. Domain crates depend on each other only as a DAG, and a domain reads another only through that domain's `service/for_<caller>.rs`. Where the dependency would be a cycle, the domain above hands a reader over at start-up (for example `acom-registry::build()` installs promotion's order history, messaging's segment members, and integration's seller status). The agent crate does not depend on any domain crate: it knows tools only through the manifest that `commerce` serves.

### 2.4 Data and state

- **Tables named after schema.org types**, one PostgreSQL schema per domain (`catalog`, `sales`, `fulfillment`, ...), plus `platform` and `agent`. Naming rules in §5.6.
- **Store context on every row** (`tenant_id`) with row-level security, so one deployment can serve several stores. The demo runs one store.
- **State changes are events.** Order status, claim status, refunds, loyalty points, and change-set runs are appended to event tables in the same transaction as the state change. This is what makes read-back, undo, and the action log reliable.
- **One database.** Idempotency, locks, the job queue, and search all run on PostgreSQL. Search ranks by full text; the pgvector column is in place but stays empty, because no embedding model is connected. A cache, a search cluster, or a message bus is added when load justifies it.
- **One clock.** All code reads time from a clock; the demo store runs on a fixed demo clock (Fri Nov 6 and Sat Nov 7, 2026), so scheduled jobs and undo deadlines behave as in the scenes.

## 3. API design: one hierarchy

A flat list of 290 tools is as hard for a model as 2,000 endpoints. A-Commerce organizes every operation in a four-level hierarchy, and the harness uses that hierarchy to narrow down what a model sees.

### 3.1 Addressing

| Level | Meaning | Example |
|---|---|---|
| Group | A part of the business | Engage |
| Domain | One area with its own rules and policies | `promotion` |
| Resource | A schema.org type, or our extension of one | `Offer` (member price) |
| Operation | One business verb, the tool | `set_sale_prices` |

Each operation has three names generated from one definition:

| Surface | Form | Example |
|---|---|---|
| Logical address | `domain.resource.operation` | `promotion.offer.set_sale_prices` |
| Tool name (what the model sees) | `verb_noun`, plain English | `set_sale_prices` |
| REST endpoint | `METHOD /v1/{domain}/{resources}/...` | `POST /v1/promotion/sale-prices` |

### 3.2 Four groups, thirteen domains

| Group | Domain | Resources | Representative operations |
|---|---|---|---|
| **Offer**: what is for sale | `catalog` | ProductGroup, Product (variant), Offer, inventory, Brand, Review, Question | `search_product_groups`, `get_product`, `create_product`, `set_variant_stock`, `get_review_summary`, `get_product_measurements` |
| | `merchandising` | category (DefinedTerm), display slot, banner, CollectionPage | `list_display_slots`, `schedule_banner`, `create_curation` |
| | `search` | search, lexicon, index | `search_products`, `compare_products`, `autocomplete`, `reindex` |
| **Transact**: from cart to delivery | `sales` | cart, checkout, Order, OrderItem, payment, Invoice, subscription, gift order | `add_to_cart`, `get_cart`, `prepare_order`, `get_order`, `get_my_orders` |
| | `fulfillment` | ReturnAction (return request), refund, inspection, ParcelDelivery, shipping rate | `search_return_requests`, `evaluate_return_request`, `get_return_options`, `refund_return`, `get_delivery_dates` |
| **Engage**: customers and benefits | `promotion` | promotion, member price (UnitPriceSpecification), coupon, cart benefit, loyalty points, event, segment | `list_active_promotions`, `set_sale_prices`, `create_coupon`, `calculate_benefits`, `grant_loyalty` |
| | `customer` | Person, ProgramMembership, MemberProgramTier, PostalAddress, consent, preference (memory), inquiry, remedy | `list_tiers`, `get_my_profile`, `update_my_preferences`, `get_return_history`, `set_consent` |
| | `messaging` | Message, template, campaign | `send_customer_message`, `create_campaign` |
| | `community` | post, reply, board | `list_posts`, `create_reply`, `report_post` |
| **Operate**: running the store | `platform` | policy, change set, action log, enumerations, roles, store (tenant), task | `get_policy`, `verify_change_set`, `search_action_log`, `list_enums` |
| | `analytics` | sales, customer, and service metrics | `explain_sales_change`, `get_sales_trend`, `get_sales_overview` |
| | `partner` | Organization (seller), contract, grade | `onboard_seller`, `get_seller` |
| | `integration` | external channels, bulk import, carrier and payment events | `import_products`, `receive_carrier_event` |

Every domain is built: 290 tools (131 reads, 119 writes, 40 writes that move money or reach customers) plus 18 person-only endpoints. `cargo xtask check-coverage` holds the registry to the whole specification, and the index is [api/README.md](api/README.md) §4. Payment providers, bank deposits, tax receipts, carriers, outside sales channels, email/SMS/push delivery, business-registry checks, payouts, and image and copy generation sit outside the store; their adapters are deterministic stubs that say so in their output.

### 3.3 Three kinds of operations

| Kind | Called by | Behavior | schema.org action status |
|---|---|---|---|
| **read** | agents | No side effects. Lists return items and the total together. `fields` keeps responses small. | n/a |
| **write** | agents | Runs with `preview=true` first. The preview joins a change set; the real run uses the `preview_id` after approval, or directly when policy needs no approval (for example remembering a shopper preference, or a refund at or under the store's threshold). Kind **W$** marks writes that always need a person (prices shown to customers, coupons, banners, messages). | `PotentialActionStatus` → `ActiveActionStatus` → `CompletedActionStatus` / `FailedActionStatus` |
| **person-only** | UI buttons | Approving, rejecting, or undoing a change set, placing an order, sending a cancel, return or exchange request, deleting an account, resetting a password, undoing a memory. These are HTTP endpoints, not tools, and they need `human:*` scopes that a chat turn never has, so no prompt can make an agent do them. | the person's `AuthorizeAction` or `OrderAction` |

### 3.4 The tools of the demo scenes in the hierarchy

| Tool | Address | Kind | For |
|---|---|---|---|
| `get_policy` | `platform.policy.get` | read | operator |
| `search_products` | `search.product.search` | read | both |
| `get_product` | `catalog.product.get` | read | both |
| `list_tiers` | `customer.membership.list_tiers` | read | operator |
| `list_active_promotions` | `promotion.promotion.list_active` | read | both |
| `list_display_slots` | `merchandising.display_slot.list` | read | both |
| `set_sale_prices` | `promotion.offer.set_sale_prices` | write | operator |
| `create_coupon` | `promotion.coupon.create` | write | operator |
| `schedule_banner` | `merchandising.banner.schedule` | write | operator |
| `get_change_set` | `platform.change_set.get` | read | operator |
| `verify_change_set` | `platform.change_set.verify` | read | operator |
| `search_return_requests` | `fulfillment.claim.search_return_requests` | read | operator |
| `get_order` | `sales.order.get` | read | both |
| `evaluate_return_request` | `fulfillment.claim.evaluate_return_request` | read | operator |
| `get_return_history` | `customer.person.get_return_history` | read | operator |
| `refund_return` | `fulfillment.refund.create` | write (a person approves over $200) | operator |
| `send_customer_message` | `messaging.message.send` | write | operator |
| `compare_products` | `search.product.compare` | read | both |
| `get_my_profile` | `customer.person.get_my_profile` | read | shopper |
| `update_my_preferences` | `customer.preference.update_my_preferences` | write (no approval) | shopper |
| `add_to_cart` | `sales.cart.add` | write (no approval) | shopper |
| `get_cart` | `sales.cart.get` | read | shopper |
| `prepare_order` | `sales.order.prepare` | write (preview only) | shopper |
| `get_my_orders` | `sales.order.list_mine` | read | shopper |
| `get_return_options` | `fulfillment.claim.get_return_options` | read | shopper |
| `get_product_measurements` | `catalog.product.get_measurements` | read | both |
| `get_review_summary` | `catalog.review.summarize` | read | both |
| `get_delivery_dates` | `fulfillment.parcel_delivery.get_delivery_dates` | read | both |
| `explain_sales_change` | `analytics.sales.explain_sales_change` | read | operator |
| `get_sales_trend` | `analytics.sales.get_sales_trend` | read | operator |

Inputs, outputs, checks, and errors for each tool are in the domain files of [api/](api/README.md). The scenes that use them, with expected results, are in [scenarios.md](scenarios.md) §6.

## 4. An interface built for agents

### 4.1 Rules for every operation

Code review checks these ([design.md §3](design.md)), and the registry refuses an operation whose input schema is not closed:

1. **One business verb, plain name.** `verb_noun` in full English words. No screen, popup, or grid names. The name and a one-line description are enough to pick the tool.
2. **Typed inputs with meaning.** Enumerations are words (`"vip"`, `"in_stock"`), never codes. Every field has a description and an example.
3. **Safe reads.** A read never changes state. A list returns items and the total in one call.
4. **Writes go through preview.** The preview returns what will change, the policy checks, and whether a person must approve.
5. **Declared links.** Each operation declares what it consumes and produces, so the harness can chain tools without guessing.
6. **Rules are data.** Limits, floors, stacking, and approval thresholds come from `policies/*.yml` and the store's overrides. A violation comes back as a reason and the tool that can fix it.
7. **Actionable errors.** A code, a sentence a person can read, and the next tool to try.
8. **Retries are safe.** Running the same `preview_id` twice returns the first result; person-only endpoints take an idempotency key.

### 4.2 Response shapes

Full rules (money, time, paging, ids, tool schemas) are in [api/conventions.md](api/conventions.md).

```jsonc
// read
{"items": [...], "total": 42}

// write, preview=true
{
  "preview_id": "pv_01jc8x4m2q",
  "expires_at": "2026-11-06T11:32:00-05:00",
  "changes": [{"op": "update", "entity": "unit_price_specification", "id": "prd_windbreaker",
               "from": {"value": 89.00, "currency": "USD"}, "to": {"value": 71.20, "currency": "USD"}}],
  "checks": [{"rule": "price_floor", "passed": true, "detail": "Overshirt moved to 15% to stay at cost + 15%", "fix_tool": null}],
  "needs_approval": true,
  "approval_reason": "Changes 40 prices",
  "change_set_id": "cs_1042"
}

// write, run with the preview_id (inside an approved change set, or directly when no approval is needed)
{"ok": true, "id": "promo_1042", "undo": {"by": "person", "target": "cs_1042", "until": "2026-11-09T23:59:00-05:00"}}

// error
{"error": {"code": "below_price_floor", "message": "At 20% the Seersucker overshirt would sell below cost + 15%.", "next_tool": "set_sale_prices"}}
```

### 4.3 The tool graph

`consumes` and `produces` turn the hierarchy into a graph the planner can walk:

```
search_products ──product_id[]──▶ set_sale_prices ──promotion_id──▶ create_coupon
list_tiers ─────────tier────────▶ set_sale_prices                   │
list_display_slots ──slot_id──▶ schedule_banner ◀──promotion_id─────┘
                                       │
          set_sale_prices, create_coupon, schedule_banner ──▶ verify_change_set

search_products ──▶ compare_products ──▶ add_to_cart ──cart_id──▶ get_cart ──▶ prepare_order
search_return_requests ──▶ evaluate_return_request ──▶ refund_return ──▶ send_customer_message
```

**Tool search** uses both the hierarchy and the graph. For a request, the harness keeps to the agent's domains, scores tool names and descriptions against the words (BM25, with a boost for the agent's home domains), and adds graph neighbors (what usually comes before or after). The model sees five to eight candidates, not 290. This is what lets a 30B model do routine work.

### 4.4 One definition, several surfaces

Each operation is defined once in Rust: static metadata (address, tool name, kind, REST path, consumes and produces, checks, errors) plus typed input and output. The input type generates a closed JSON Schema. From that one definition the registry produces:

- **The tool manifest** our agent searches (`GET /v1/tools`), already filtered by the caller's scopes and the tool's audience.
- **REST endpoints** for the web app, tests, and the evaluation runner (`/v1/{domain}/...`, and `POST /v1/tools/{tool}` by tool name).

Designed on the same definition but not built yet: MCP tools for outside agents (reads, the cart, and `prepare_order`), an OpenAPI document with TypeScript types for the web, and a schema.org action description of each operation (`potentialAction` with an `EntryPoint`).

The specification and the code are held together by tests and checks: a test in `acom-registry` compares every registered operation's tool name, address, kind, and REST path with its section in [api/](api/README.md); `cargo xtask check-docs` keeps the operation index in [api/README.md](api/README.md) and the domain files equal; `cargo xtask check-coverage` finds every specified tool and person-only endpoint built; `cargo xtask check-layout` finds each operation in its file.

### 4.5 Small model first

The model our agent sees is smaller than the wire. It sees a handful of tools picked by tool search, each with a closed schema, and it only says *what* to change: the runtime runs previews, attaches them to a change set, and binds ids between steps by slot name (`product_id[]`, `promotion_id`), so a model never copies 40 ids by hand. Guided decoding fixes the shape of plans and arguments, and a deterministic repair step maps "20%" to `20` and "VIP members" to `vip` before validation.

Muse Glimmer 30B does routing, tool selection, arguments, and short plans. Muse Spark plans again when a plan has more than three steps or spans domains, and takes over when a check fails. A model that is not configured is skipped; on the live demo Spark is not configured, so every model call runs on Glimmer. Each step records the model, tool calls, tokens, and time, which is what the evaluation measures ([backend/agent.md](backend/agent.md)).

## 5. Data model: schema.org first

### 5.1 Why schema.org

- **Models already know it.** schema.org is the vocabulary behind structured data on billions of web pages, so `Product`, `Offer`, `Order`, and `availability` mean the same thing to a model that they mean to us. Fewer invented names means fewer wrong guesses.
- **It describes verbs as well as nouns.** schema.org has a full hierarchy of `Action` types with a status, an agent, an object, and a result. That fits tool calls, previews, approvals, and the action log directly (§5.5).
- **It is how outside agents already read stores.** Product feeds and agent commerce protocols build on the same vocabulary, so our tool layer and outside agents speak one language.
- **It is precise about commerce.** Member prices (`validForMemberTier`), return policies (`MerchantReturnPolicy`, `merchantReturnDays`, `returnFees`), availability (`ItemAvailability`), and order states (`OrderStatus`) are already defined, with values that are words, not codes.

### 5.2 Naming rules

1. **Types.** Use the schema.org type when one exists (`Product`, `Offer`, `Order`, `OrderItem`, `Person`, `PostalAddress`, `ParcelDelivery`, `MerchantReturnPolicy`).
2. **Properties.** Use the schema.org property name. On the wire and in the database it is written in snake_case (`validFrom` → `valid_from`, `priceCurrency` → `price_currency`); a published JSON-LD context maps each name back to its schema.org term, so every payload is valid JSON-LD.
3. **Values.** Use schema.org enumeration members, written as snake_case words: `InStock` → `in_stock`, `OrderDelivered` → `delivered`, `FreeReturn` → `free_return`.
4. **Extensions only when nothing fits.** Our own terms live in the `acom:` namespace, each declared as a subtype of the closest schema.org type and reusing its properties (§5.4).
5. **Identifiers.** Readable prefixed ids (`prd_windbreaker`, `promo_1042`); the prefix table is in [api/conventions.md](api/conventions.md) §9.
6. **Money.** schema.org `MonetaryAmount`, `{"value": 71.20, "currency": "USD"}`, read and written as an exact decimal ([api/conventions.md](api/conventions.md) §3).

### 5.3 Entities by domain

| Domain | Entity | schema.org type | Key properties |
|---|---|---|---|
| catalog | product with variants | `ProductGroup` | `name`, `brand`, `category`, `has_variant`, `varies_by` |
| catalog | variant (what is bought) | `Product` | `sku`, `color`, `size`, `offers`, `additional_property` |
| catalog | price and availability | `Offer` | `price`, `price_currency`, `availability`, `inventory_level` |
| catalog | rating, reviews | `AggregateRating`, `Review` | `rating_value`, `review_count`, `review_body` |
| merchandising | category | `DefinedTerm` in a `DefinedTermSet` | `name`, `term_code`, `in_defined_term_set` |
| merchandising | curated page | `CollectionPage`, `ItemList` | `item_list_element` |
| promotion | member sale price | `UnitPriceSpecification` | `price`, `price_type` (`sale_price`), `valid_for_member_tier`, `valid_from`, `valid_through` |
| promotion | tier program | `MemberProgram`, `MemberProgramTier` | `has_tiers`, `has_tier_benefit`, `membership_points_earned` |
| sales | order and lines | `Order`, `OrderItem` | `order_number`, `order_status`, `customer`, `ordered_item`, `order_quantity`, `discount_code` |
| sales | invoice, payment | `Invoice`, `PaymentMethod` | `total_payment_due`, `payment_status` |
| fulfillment | delivery | `ParcelDelivery` | `tracking_number`, `expected_arrival_until`, `delivery_address`, `part_of_order` |
| fulfillment | return policy | `MerchantReturnPolicy` | `merchant_return_days`, `return_fees`, `refund_type`, `return_method` |
| fulfillment | return request | `ReturnAction` | `object` (order line), `agent` (customer), `action_status`, `result` |
| customer | member | `Person`, `ProgramMembership` | `name`, `email`, `address`, `member_of`, `membership_number` |
| messaging | message to a customer | `Message` | `recipient`, `text`, `about` |
| platform | change set | `ItemList` of actions | `item_list_element`, `version` |

### 5.4 Our extensions

We extend schema.org the least we can ([api/vocabulary.md](api/vocabulary.md)):

1. **Use the schema.org type when one exists.** A return is a `ReturnAction`, a cancellation a `CancelAction`, an inspection a `CheckAction`, a display slot a `WebPageElement`, a promotion an `OfferCatalog`, a payment a `PayAction`.
2. **Specialize with `additionalType`, keep the schema.org `@type`.** A coupon is `{"@type": "Offer", "additionalType": "acom:Coupon", "discountCode": "VIPWEEKEND20"}`. An agent or a search engine that knows only schema.org still reads it as an offer with a price, a period, and a member tier.
3. **New properties and types are a last resort**, each declared under the closest schema.org term (`rdfs:subClassOf`, `domainIncludes`, `rangeIncludes`) in a machine-readable vocabulary, [`vocab/acom.jsonld`](../vocab/acom.jsonld).

| Concept | `@type` | `additionalType` |
|---|---|---|
| Promotion (sale prices for a tier and a period) | `OfferCatalog` | `acom:Promotion` |
| Coupon | `Offer` | `acom:Coupon` |
| Cart | `Order` | `acom:Cart` |
| Return or exchange request / cancellation | `ReturnAction` / `CancelAction` | `acom:Claim` |
| Refund | `MoneyTransfer` | `acom:Refund` |
| Banner / display booking | `CreativeWork` / `Schedule` | `acom:Banner` / `acom:DisplayBooking` |
| Remembered shopper preference | `PropertyValue` | `acom:Preference` |
| Change set | `ItemList` | `acom:ChangeSet` |
| Operator task | `Action` | `acom:Task` |
| Requested discount behind a sale price | property of `UnitPriceSpecification` | `acom:percentOff` |

### 5.5 Operations are schema.org actions

This is the core of the design. Nouns are schema.org things; **verbs are schema.org actions**. Each operation is classified by the action type it performs, and every call, preview, approval, and run is recorded as an action instance.

| Operation | schema.org action |
|---|---|
| `search_products`, `list_*` | `SearchAction` |
| `evaluate_return_request`, `verify_change_set` | `AssessAction` |
| `set_sale_prices` | `UpdateAction` on `Offer` prices |
| `create_coupon` | `CreateAction` |
| `schedule_banner` | `ScheduleAction` |
| `add_to_cart` | `AddAction` |
| `send_customer_message` | `SendAction` with a `Message` |
| `prepare_order` | `OrderAction` in `PotentialActionStatus`; the shopper's tap completes it |
| approving a change set (person-only) | `AuthorizeAction` by a `Person` |

The action status gives one vocabulary to the whole lifecycle: a preview is an action in `PotentialActionStatus`, a run in progress is `ActiveActionStatus`, a verified change is `CompletedActionStatus` with its `result`, and a failed read-back is `FailedActionStatus` with its `error`. The action log is a list of these records with `agent` (which model or person), `object`, `start_time`, and `end_time`.

```jsonc
// one change inside change set #1042, as stored in the action log
{
  "@type": "UpdateAction",
  "identifier": "act_1042_03",
  "action_status": "potential",            // PotentialActionStatus: previewed, waiting for approval
  "agent": {"@type": "SoftwareApplication", "name": "muse-glimmer-30b"},
  "instrument": "set_sale_prices",
  "object": {"@type": "Offer", "identifier": "off_windbreaker_m_slate"},
  "result": {
    "@type": "UnitPriceSpecification",
    "price": 71.20, "price_currency": "USD", "price_type": "sale_price",
    "valid_for_member_tier": "vip",
    "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00"
  }
}
```

### 5.6 A database an AI can read

The database is designed to be read by agents as well as people ([backend/database.md](backend/database.md)):

- **schema.org names.** Tables are schema.org types in snake_case, singular (`product_group`, `product`, `offer`, `unit_price_specification`, `order_item`, `parcel_delivery`); columns are schema.org properties (`price_currency`, `valid_from`, `order_status`, `inventory_level`). SQL reserved words get the schema name: `Order` → `sales.sales_order`.
- **Self-describing.** Every table and column carries a comment that starts with its term (`schema:inventoryLevel — units that can be sold now`). A generated data dictionary is served to agents by the `describe_data` tool.
- **One id.** The primary key is the same prefixed id the API and logs use (`prd_windbreaker`, `ord_20402`). There is no hidden numeric key.
- **Words and units in plain sight.** Status columns store enumeration words under a `CHECK` list; money is `price` + `price_currency` or `*_value` + `*_currency`; quantities are `*_value` + `*_unit_text`. No abbreviations.
- **Store isolation.** Every store-scoped table has `tenant_id` and a row-level security policy.
- **History as actions.** Event tables use the properties of schema.org `Action` (`agent`, `object`, `instrument`, `action_status`, `result`, `start_time`, `end_time`) and are appended, never updated.

`cargo xtask check-schema` enforces these rules on every migration.

### 5.7 Designing one interface, step by step

The VIP weekend sale shows how one interface is designed.

**A screen-shaped back-end** splits this task across a promotion target grid read with `POST` plus a separate count call, abbreviated names, a `promoType: "T03"` code, and a bulk `save…List(rows[])` with a row-status flag ([docs/design.md §2](design.md)).

**A-Commerce** designs it as one operation:

1. **Name the business verb and its action type.** Putting products on sale for a tier for a period is an `UpdateAction` on offer prices: `set_sale_prices`.
2. **Describe the inputs in schema.org terms.** `name`, `valid_for_member_tier` (`"vip"`), `valid_from`, `valid_through`, and `items[]` of `{product_id, percent_off}`, plus `preview`.
3. **Return schema.org data.** Each new price is a `UnitPriceSpecification` with `price_type: "sale_price"` and the member tier, as in §5.5.
4. **Declare the links.** Consumes `product_id[]` and a tier; produces `promotion_id`.
5. **Attach the policy checks.** `max_discount`, `price_floor`, `promo_min_stock`, `promotion_overlap`, all read from `policies/*.yml`. A failed check names the products and the fix.
6. **Write actionable errors.** `below_price_floor` with the product and `next_tool: "set_sale_prices"`.
7. **Make it verifiable.** After the run, `verify_change_set` reads every price back and marks each change as matched or not.

The same seven steps apply to every operation in the hierarchy.

### 5.8 One list of values

Every state and type enumeration (order status, claim status, change-set state, liability, reasons, channels) has one canonical list in [api/enums.md](api/enums.md), with its schema.org member where one exists. Domain files, policies, the database, and the Rust enums all use that list.

## 6. AI-native features

### 6.1 Change sets

Every write an agent wants to make is collected into a change set: the request as understood, the checks, the grouped changes, a preview, and what happens after approval. People review a change set the way they review a pull request.

```
drafting → waiting_for_approval → running → verifying → live
                    ↑  │                         └→ failed (shows which change did not match)
     edit by talking ┘  └→ rejected                live → undone (until the undo deadline)
```

Editing by talking re-runs the previews and bumps the version (v1 → v2). Approval re-checks every preview and starts the run inside `commerce`; every change is read back and compared with the plan; all changes can be undone together until the deadline. Refund batches use the same mechanism ("Approve 9 refunds and send 3 messages"). The change-set API is in [api/platform.md](api/platform.md#change_set).

### 6.2 The harness loop

```
request ─► route ─► tool search ─► plan ─► preview ─► change set ─► approve ─► run ─► read back ─► UI blocks
            │           │            │                                  (person)  (commerce)    │
       specialist   hierarchy +   Glimmer, or Spark                                          Spark takes over
         agent      tool graph    for multi-step plans                                       when a check fails
```

### 6.3 Specialist agents on one tool layer

Areas of the business have their own agents that people can talk to. A specialist is the same harness with a scoped part of the hierarchy, its own instructions, and the same policies. Within a turn, steps pass ids (what one tool produces, the next consumes); work for another agent goes to it in the next turn with the ids and the blocks already shown, not as a text summary.

| Agent | Side | Domains it can use | Example request | Writes |
|---|---|---|---|---|
| Router | both | picks the specialist (a runtime step, not an agent file) | any | no |
| Promotion agent | back office | promotion, merchandising, catalog, search, customer, platform | "Run a 20% VIP weekend sale and put it on the main page" | through change sets |
| Orders agent | back office | fulfillment, sales, messaging, customer, catalog, platform | "Go through the returns over $200" | through change sets |
| Catalog agent | back office | catalog, merchandising, search, platform | "Which jackets are missing size charts?" | through change sets |
| Insights agent | back office | analytics | "Why were Thursday's sales down?" | no |
| Shopper agent | storefront | search, catalog, merchandising, promotion, sales, fulfillment, customer, messaging | "A light jacket that packs small for a trip" | only writes that need no approval (cart, preferences); placing the order is the shopper's button |

Specialists are data (`agents/*.yml`: tool scope, home domains, instructions, allowed blocks, models), not code. The five agents after the router are the ones built ([backend/agent.md](backend/agent.md) §2). No agent's scope includes `community`, `partner`, or `integration` yet, so those tools are reached over REST, not in a conversation.

### 6.4 Policy as data

Business rules are values in `policies/*.yml`, overridden per store, read by the back end, cited in checks, and shown to people. A selection from the demo store ([scenarios.md](scenarios.md) §5):

| Key | Value |
|---|---|
| `max_discount.everyone` / `max_discount.by_tier.vip` | 20% / 30% |
| `price_floor` | cost + 15% |
| `promo_min_stock` | 10 units |
| `member_benefits_per_order` | 1 (coupons don't stack with tier coupons) |
| refunds a person must approve | over $200.00 |
| `return_window_days` | 30 |
| `customer_messages` | always ask |

### 6.5 Agents that start work

Agents do not only wait for requests. Watchers in `commerce` check the store against policy thresholds and open tasks: `watcher.returns_over_refund_threshold` (twelve returns over $200 waiting), `watcher.stock_out_forecast` (linen shirts selling out in about four days), `watcher.price_below_cost` (products listed below cost; this one drafts its own fix, a change set that raises them to the price floor). When the store turns on `task_pickup` (the demo store does), the gateway hands each new returns or stock-out task to the agent, as the operator the policy names, so a draft change set is waiting before anyone asks. Nothing runs until a person approves, unless the store gave a watcher's fix a standing approval (the demo store does for the below-cost fix).

### 6.6 UI on demand

The agent never writes HTML. It returns UI blocks that name a component from a fixed library and fill it with data from tool outputs ([api/chat.md](api/chat.md)):

```jsonc
{
  "ui": "compare_table", "schema": 1, "block_id": "blk_31",
  "title": "Side by side",
  "why": "Shown as a table because you compare specs. Say \"show photos\" to change it.",
  "data": { },
  "actions": [{"kind": "say", "action_id": "add_m", "label": "Add M", "args": {"product_id": "prd_field_jacket", "size": "M"}}]
}
```

`say` buttons send an intent back to the agent; `person` buttons call a person-only endpoint directly and never pass through the agent. Back office: `change_set`, `refund_review`, `product_drafts`, `sales_explainer`, `task_list`, `product_table`, `action_log`. Storefront: `looking_for`, `compare_table`, `look_gallery`, `product_list`, `fit_check`, `gift_picker`, `cart`, `checkout_card`, `return_options`, `order_detail`, `memory_note`, `profile_card`. The layout follows the person: a comparison table for a shopper who compares specs, a gallery for one who browses by look.

Most of the more than 160 tools added on Oct 31 have REST endpoints and manifest entries but no dedicated UI block or back-office page yet; when an agent reads them, a list shows in the generic `product_table`.

### 6.7 Memory people can see, actions only people take

- What the store remembers about a shopper appears as one line with Undo, and the full profile can be read and edited.
- Requests about someone else (a gift) are not saved to the shopper's profile.
- Placing an order, sending a return request, approving a change set, and undoing are person-only endpoints. The agent prepares; the person commits.

### 6.8 Evaluation

We build a screen-shaped facade over the same domain services, with the patterns from [design.md §2](design.md) written as our own generic examples, and run the same tasks through both tool sets with the same agent runtime. Tasks are split into a design set and a held-out set; each task runs several times and is scored on the tools it called, the change set it drafted, and the facts in its answer ([backend/testing.md](backend/testing.md) §3). On the design set (12 tasks, 3 runs each, every call on Glimmer, Oct 31) A-Commerce's tools completed 58% (21/36, pass^3 50%) and the screen-shaped facade 0% (0/36). The held-out set runs once at the end and is not in yet.

## 7. From the hackathon build to the target

The stack is the same from the first commit; what grows is the scope.

| | Hackathon build | Target |
|---|---|---|
| Server side | Rust: gateway, commerce, agent | Same services, split further only where load says so |
| Storage | PostgreSQL + pgvector (search ranks by full text) | Same, plus embeddings, read replicas, a cache, or a search cluster |
| Operations | 290 tools in 13 domains and 18 person-only endpoints, all built | Dedicated UI blocks and back-office pages for the tools added on Oct 31; MCP for outside agents |
| Agents | Promotion, orders, catalog, insights, shopper | A specialist per area, including community, partners, and integrations (§6.3) |
| Stores | One demo store on a demo clock | Many stores on one deployment (row-level security is on from the start) |
| Outside services | Deterministic stubs that say so: payments, deposits, tax receipts, carriers, outside channels, email/SMS/push, registry checks, payouts, image and copy generation | Real providers behind the same adapters |
