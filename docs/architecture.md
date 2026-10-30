# A-Commerce architecture

This document is the reference for how A-Commerce is built: the back-end structure, how the API is organized, what makes it friendly to AI agents, how every interface follows [schema.org](https://schema.org), and the AI-native features on top.

- [1. System at a glance](#1-system-at-a-glance)
- [2. Back-end structure](#2-back-end-structure)
- [3. API design: one hierarchy](#3-api-design-one-hierarchy)
- [4. An interface built for agents](#4-an-interface-built-for-agents)
- [5. Data model: schema.org first](#5-data-model-schemaorg-first)
- [6. AI-native features](#6-ai-native-features)
- [7. From the hackathon MVP to the target](#7-from-the-hackathon-mvp-to-the-target)

The full API specification, every operation in this hierarchy with inputs, outputs, checks, and errors, is in [api/](api/README.md). Related documents: why and design rules in [docs/design.md](design.md), the build contract (MVP scope, tool signatures, seed data, UI block schema) in [docs/plan.md](plan.md), market research in [docs/research.md](research.md), UI mockups in [design/](../design).

## 1. System at a glance

```
  Storefront chat (/fo)        Back-office chat (/bo)          Outside agents
     shoppers                     operators                 (ChatGPT, Gemini, ...)
         │  SSE                      │  SSE                        │  MCP
         └──────────────┬────────────┘                             │
                        ▼                                          │
     ┌──────────────────────────────────────────┐                  │
     │ Harness                                  │                  │
     │  router → specialist agent               │                  │
     │  tool search · plan · execute · check    │   models         │
     │  change sets · read back · undo          │◀─ Glimmer        │
     │  action log · UI block picker            │◀─ Spark Open     │
     └────────────────────┬─────────────────────┘                  │
                          │ tool calls                             │
                          ▼                                        ▼
     ┌──────────────────────────────────────────────────────────────────┐
     │ Tool layer: 4 groups · 13 domains · about 200 operations          │
     │ one definition per operation → MCP tool + REST endpoint           │
     ├──────────────────────────────────────────────────────────────────┤
     │ Policy engine: policy.yaml (limits, floors, stacking, approvals)  │
     ├──────────────────────────────────────────────────────────────────┤
     │ Store: schema.org-shaped tables · event log · search index        │
     └──────────────────────────────────────────────────────────────────┘
          ▲
          │ person-only endpoints: approve / undo a change set,
          └ place an order, send a return request, undo a memory (UI buttons only)
```

Every caller, whether our operator agent, our shopper agent, an outside agent, or a button in the UI, goes through the same tool layer and the same policies. The price an operator sets in a change set is the price the shopper agent quotes and the price an outside agent sees.

## 2. Back-end structure

### 2.1 Layers

| Layer | What it does | Rules it enforces |
|---|---|---|
| **Interfaces** | SSE chat for `/bo` and `/fo`, MCP for agents, REST for the web app and tests, person-only endpoints | Authentication, store (tenant) context, rate limits, request ids |
| **Harness** | Turns a request into tool calls: picks the agent, searches tools, plans, executes previews, groups them into a change set, runs after approval, reads back, logs | Models only see read and write tools; never person-only actions |
| **Tool layer** | The commerce operations, organized as group → domain → resource → operation (§3) | One business verb per tool, typed inputs, preview before write (§4) |
| **Policy engine** | Loads `policy.yaml` and evaluates checks during previews and runs | Business rules live in data, not in prompts or code constants |
| **Store** | Tables named after schema.org types, an append-only event log for state changes, a search index | Money paths are transactional; state history is never overwritten |

### 2.2 Code layout

```
server/
  contract/    operation definitions: inputs, outputs, consumes/produces, errors
               → generates MCP tool specs and the REST/OpenAPI view from one source
  schema/      schema.org types and our acom: extensions (§5)
  policy/      policy.yaml loader, rule checks, cart benefit calculation
  tools/       one package per domain: catalog/ merchandising/ search/ sales/
               fulfillment/ promotion/ customer/ messaging/ platform/ ...
  harness/     router, tool search, planner, executor, change sets, read back, action log
  store/       tables, event log, seed data, demo clock
  api/         chat (SSE), person-only endpoints, REST view of the tools
web/           back office (/bo) and storefront (/fo), one component per UI block
eval/          task set, screen-shaped facade, runner, results
```

The `server/` and `web/` split follows [docs/plan.md §6.1](plan.md).

### 2.3 Two stages, one contract

| | Stage 1: reference implementation | Stage 2: optimized implementation |
|---|---|---|
| Language | Python, FastAPI | Rust, Axum on Tokio |
| Storage | SQLite (hackathon) | PostgreSQL |
| Purpose | Build every operation quickly and prove the design with real models | Lower latency and stronger guarantees on the paths agents call most |
| When | The hackathon MVP | Module by module after the MVP |

The contract (operation names, inputs, outputs, errors, preview behavior) is the same in both stages. A module moves to Rust only when it passes the same contract tests and replays the same scenarios (the VIP weekend change set, Jamie's checkout, Alex's return) with identical results. The planned order is search, catalog, sales and fulfillment, promotion, then the rest.

Why Rust for the second stage: an agent makes many tool calls for one request, so per-call latency adds up; money paths (checkout, refunds) gain from typed state machines and checked money arithmetic; and one small binary per service is easy to run on a merchant's own servers next to open-weight models.

### 2.4 Data and state

- **Tables named after schema.org types**, one schema per domain in PostgreSQL (`catalog`, `sales`, `fulfillment`, ...). Naming rules in §5.6.
- **Store context on every row** (`tenant_id`) with row-level security, so one deployment can serve several stores. The MVP runs one store.
- **State changes are events.** Order status, return status, refunds, loyalty points, and change-set runs are appended to event tables; the current state is derived. This is what makes read-back, undo, and the action log reliable.
- **Search index** (keyword plus vector) behind `search_products`; filters for audience and stock are applied inside the query, not after it.
- **Idempotency keys and locks** in Redis for writes that may be retried; an object store for product and banner images.

## 3. API design: one hierarchy

A flat list of 200 tools is as hard for a model as 2,000 endpoints. A-Commerce organizes every operation in a four-level hierarchy, and the harness uses that hierarchy to narrow down what a model sees.

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

| Group | Domain | Resources | Representative operations | MVP |
|---|---|---|---|---|
| **Offer**: what is for sale | `catalog` | ProductGroup, Product (variant), Offer, inventory, Brand, Review, Question | `search_products`, `get_product`, `create_product`, `set_variant_stock`, `get_review_summary`, `get_product_measurements` | reads |
| | `merchandising` | category (DefinedTerm), display slot, banner, CollectionPage | `list_display_slots`, `schedule_banner`, `create_curation` | yes |
| | `search` | search, lexicon, index | `search_products`, `compare_products`, `autocomplete`, `reindex` | yes |
| **Transact**: from cart to delivery | `sales` | cart, checkout, Order, OrderItem, payment, Invoice, subscription, gift order | `add_to_cart`, `get_cart`, `prepare_order`, `get_order`, `get_my_orders` | yes |
| | `fulfillment` | ReturnAction (return request), refund, inspection, ParcelDelivery, shipping rate | `search_return_requests`, `evaluate_return_request`, `get_return_options`, `refund_return`, `get_delivery_dates` | yes |
| **Engage**: customers and benefits | `promotion` | promotion, member price (UnitPriceSpecification), coupon, cart benefit, loyalty points, event, segment | `list_active_promotions`, `set_sale_prices`, `create_coupon`, `calculate_benefits`, `grant_loyalty` | yes |
| | `customer` | Person, ProgramMembership, MemberProgramTier, PostalAddress, consent, preference (memory), inquiry, remedy | `list_tiers`, `get_my_profile`, `update_my_preferences`, `get_return_history`, `set_consent` | yes |
| | `messaging` | Message, template, campaign | `send_customer_message`, `create_campaign` | one tool |
| | `community` | post, reply, board | `list_posts`, `create_reply`, `report_post` | later |
| **Operate**: running the store | `platform` | policy, change set, action log, enumerations, roles, store (tenant) | `get_policy`, `verify_change_set`, `search_action_log`, `list_enums` | yes |
| | `analytics` | sales, customer, and service metrics | `explain_sales_change`, `get_sales_overview` | later |
| | `partner` | Organization (seller), contract | `onboard_seller`, `get_seller` | later |
| | `integration` | external channels, bulk import, carrier and payment events | `import_products`, `receive_carrier_event` | later |

"MVP" marks domains with tools in the hackathon build ([docs/plan.md §3](plan.md)). The others are designed and kept in the hierarchy so that the MVP tools land in the place they will stay.

### 3.3 Three kinds of operations

| Kind | Called by | Behavior | schema.org action status |
|---|---|---|---|
| **read** | agents | No side effects. Lists return items and the total together. `fields` keeps responses small. | n/a |
| **write** | agents | Runs with `preview=true` first. The preview joins a change set; the real run uses the `preview_id` after approval, or directly when policy needs no approval (for example remembering a shopper preference). | `PotentialActionStatus` → `ActiveActionStatus` → `CompletedActionStatus` / `FailedActionStatus` |
| **person-only** | UI buttons | Approving or undoing a change set, placing an order, sending a return request, undoing a memory. These are HTTP endpoints, not tools, so no prompt can make an agent do them. | the person's `AuthorizeAction` or `OrderAction` |

### 3.4 The MVP tools in the hierarchy

| Tool | Address | Kind | Side |
|---|---|---|---|
| `get_policy` | `platform.policy.get` | read | operator |
| `search_products` | `search.product.search` | read | both |
| `get_product` | `catalog.product.get` | read | operator |
| `list_tiers` | `customer.membership.list_tiers` | read | operator |
| `list_active_promotions` | `promotion.promotion.list_active` | read | operator |
| `list_display_slots` | `merchandising.display_slot.list` | read | operator |
| `set_sale_prices` | `promotion.offer.set_sale_prices` | write | operator |
| `create_coupon` | `promotion.coupon.create` | write | operator |
| `schedule_banner` | `merchandising.banner.schedule` | write | operator |
| `verify_change_set` | `platform.change_set.verify` | read | operator |
| `search_return_requests` | `fulfillment.claim.search_return_requests` | read | operator |
| `get_order` | `sales.order.get` | read | operator |
| `evaluate_return_request` | `fulfillment.claim.evaluate_return_request` | read | operator |
| `get_return_history` | `customer.person.get_return_history` | read | operator |
| `refund_return` | `fulfillment.refund.create` | write | operator |
| `send_customer_message` | `messaging.message.send` | write | operator |
| `compare_products` | `search.product.compare` | read | shopper |
| `get_my_profile` | `customer.person.get_my_profile` | read | shopper |
| `update_my_preferences` | `customer.preference.update_my_preferences` | write (no approval) | shopper |
| `add_to_cart` | `sales.cart.add` | write (no approval) | shopper |
| `get_cart` | `sales.cart.get` | read | shopper |
| `prepare_order` | `sales.order.prepare` | write (preview only) | shopper |
| `get_my_orders` | `sales.order.list_mine` | read | shopper |
| `get_return_options` | `fulfillment.claim.get_return_options` | read | shopper |
| `get_product_measurements` | `catalog.product.get_measurements` | read | shopper |
| `get_review_summary` | `catalog.review.summarize` | read | shopper |
| `get_delivery_dates` | `fulfillment.parcel_delivery.get_delivery_dates` | read | shopper |

Signatures, inputs, and return shapes for these tools are fixed in [docs/plan.md §3](plan.md).

## 4. An interface built for agents

### 4.1 Rules for every operation

Code review checks these ([docs/design.md §3](design.md)):

1. **One business verb, plain name.** `verb_noun` in full English words. No screen, popup, or grid names. The name and a one-line description are enough to pick the tool.
2. **Typed inputs with meaning.** Enumerations are words (`"vip"`, `"in_stock"`), never codes. Every field has a description and an example.
3. **Safe reads.** A read never changes state. A list returns items and the total in one call.
4. **Writes go through preview.** The preview returns what will change, the policy checks, and whether a person must approve.
5. **Declared links.** Each operation declares what it consumes and produces, so the harness can chain tools without guessing.
6. **Rules are data.** Limits, floors, stacking, and approval thresholds come from `policy.yaml`. A violation comes back as a reason and the tool that can fix it.
7. **Actionable errors.** A code, a sentence a person can read, and the next tool to try.
8. **Retries are safe.** Writes take an idempotency key; repeating a call returns the first result.

### 4.2 Response shapes

The shapes below are the contract from [docs/plan.md §3.1](plan.md):

```jsonc
// read
{"items": [...], "total": 42}

// write, preview=true
{
  "preview_id": "pv_8f2c",
  "changes": [{"op": "update", "entity": "price", "id": "prd_windbreaker", "from": 89.00, "to": 71.20}],
  "checks": [{"rule": "price_floor", "passed": true, "detail": "Overshirt moved to 15% to stay at cost + 15%", "fix_tool": null}],
  "needs_approval": true,
  "approval_reason": "Changes 40 prices"
}

// write, with preview_id (only the executor sends this, after approval)
{"ok": true, "id": "promo_1042", "undo": {"tool": "undo_change_set", "until": "2026-11-09T23:59:00"}}

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

**Tool search** uses both the hierarchy and the graph. For a request, the harness picks the likely domains, matches tool descriptions inside them, and adds graph neighbors (what usually comes before or after). The model sees a handful of candidates, not 200. This is what lets a 30B model do routine work.

### 4.4 One definition, several surfaces

Each operation is defined once in `server/contract/`. From that definition we generate:

- **MCP tools** for our own agents and for outside agents. Outside agents get a narrower set: reads, cart, and `prepare_order`; placing the order stays with the shopper.
- **REST endpoints and an OpenAPI document** for the web app, tests, and the evaluation runner.
- **A schema.org action description** of each operation (`potentialAction` with an `EntryPoint` and input specifications), so the store's capabilities can be discovered the same way schema.org describes a search box or an order button today.

### 4.5 Small model first

Glimmer handles tool search, tool calls, and routine plans. Spark Open plans multi-step requests and takes over when a read-back check fails. Each agent step records the model, the number of tool calls, and the time taken ([docs/plan.md §5.1](plan.md)), which is what the evaluation measures.

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
5. **Identifiers.** Readable prefixed ids (`prd_windbreaker`, `promo_1042`) stored in `identifier`; the database key is separate.
6. **Money.** A number with two decimals plus `price_currency` (USD in the demo store).

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

| Term | Subtype of | Why schema.org is not enough |
|---|---|---|
| `acom:Promotion` | `Offer` | schema.org has offers and prices but no promotion that groups member prices for a period |
| `acom:Coupon` | `Offer` | schema.org has only `discountCode` on an order; we need stacking and issue rules |
| `acom:Cart` | `Order` | no cart type; a cart is an order before `OrderPaymentDue` |
| `acom:Refund` | `MoneyTransfer` | no refund type with method and status |
| `acom:ChangeSet` | `ItemList` | a versioned group of actions with checks and an undo deadline |
| `acom:Preference` | `PropertyValue` | a remembered shopper preference with its source and an undo |
| `acom:CompareAction`, `acom:RefundAction` | `AssessAction`, `PayAction` | the verbs behind `compare_products` and `refund_return` |
| `acom:percentOff` | property of `UnitPriceSpecification` | the requested discount behind a computed sale price |

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
    "valid_for_member_tier": "tier_vip",
    "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00"
  }
}
```

### 5.6 Database naming

- Tables are schema.org types in snake_case, singular: `product_group`, `product`, `offer`, `unit_price_specification`, `order_item`, `merchant_return_policy`, `parcel_delivery`.
- SQL reserved words get a domain prefix: `Order` → `sales_order`.
- Columns are schema.org properties in snake_case: `price_currency`, `valid_from`, `order_status`. Foreign keys are `<type>_id`.
- Status columns store the enumeration word (`in_stock`, `delivered`), never a code.
- Money is `numeric` with a currency column; timestamps are stored in UTC.
- Every store-scoped table has `tenant_id`; every row has `created_at` and `updated_at`.
- State history lives in event tables (`order_status_event`, `return_action_event`, `action_log`), appended and never updated.

### 5.7 Designing one interface, step by step

The VIP weekend sale shows how one interface is designed.

**A screen-shaped back-end** splits this task across a promotion target grid read with `POST` plus a separate count call, abbreviated names, a `promoType: "T03"` code, and a bulk `save…List(rows[])` with a row-status flag ([docs/design.md §2](design.md)).

**A-Commerce** designs it as one operation:

1. **Name the business verb and its action type.** Putting products on sale for a tier for a period is an `UpdateAction` on offer prices: `set_sale_prices`.
2. **Describe the inputs in schema.org terms.** `name`, `valid_for_member_tier` (`"vip"`), `valid_from`, `valid_through`, and `items[]` of `{product_id, percent_off}`, plus `preview`.
3. **Return schema.org data.** Each new price is a `UnitPriceSpecification` with `price_type: "sale_price"` and the member tier, as in §5.5.
4. **Declare the links.** Consumes `product_id[]` and a tier; produces `promotion_id`.
5. **Attach the policy checks.** `max_discount`, `price_floor`, `min_stock`, `promotion_overlap`, all read from `policy.yaml`. A failed check names the products and the fix.
6. **Write actionable errors.** `below_price_floor` with the product and `next_tool: "set_sale_prices"`.
7. **Make it verifiable.** After the run, `verify_change_set` reads every price back and marks each change as matched or not.

The same seven steps apply to every operation in the hierarchy.

### 5.8 Alignment with the MVP contract

[docs/plan.md §3](plan.md) fixed the MVP tool signatures before these naming rules. The rules above rename a few inputs to their schema.org properties; plan.md will be updated by pull request:

| docs/plan.md | schema.org-aligned name |
|---|---|
| `starts_at`, `ends_at` | `valid_from`, `valid_through` |
| `audience` on sale prices and coupons | `valid_for_member_tier` (and `"everyone"` stays as no tier) |
| `ship_to` | `delivery_address` |
| `qty` | `order_quantity` |

## 6. AI-native features

### 6.1 Change sets

Every write an agent wants to make is collected into a change set: the request as understood, the checks, the grouped changes, a preview, and what happens after approval. People review a change set the way they review a pull request.

```
drafting → waiting_for_approval → running → verifying → live
                    ↑  │                         └→ failed (shows which change did not match)
     edit by talking ┘  └→ rejected                live → undone (until the undo deadline)
```

Editing by talking re-runs the previews and bumps the version (v1 → v2). Approval starts the run; the harness reads every change back and compares it with the request; all changes can be undone together until the deadline. Refund batches use the same mechanism ("Approve 9 refunds and send 3 messages").

### 6.2 The harness loop

```
request ─► route ─► tool search ─► plan ─► preview ─► change set ─► approve ─► run ─► read back ─► UI blocks
            │           │            │                                  (person)          │
       specialist   hierarchy +   Glimmer, or Spark Open                          Spark Open takes over
         agent      tool graph    for multi-step plans                            when a check fails
```

### 6.3 Specialist agents on one tool layer

Each area of the business has its own agent that people can talk to. A specialist is the same harness with a scoped part of the hierarchy, its own instructions, and the same policies. Agents hand work to each other by passing ids (what one tool produces, the next consumes), not by summarizing in text.

| Agent | Side | Domains it can use | Example request | Writes |
|---|---|---|---|---|
| Router | both | picks the specialist | any | no |
| Catalog agent | back office | catalog, search, merchandising (read) | "Which jackets are missing size charts?" | through change sets |
| Promotion agent | back office | promotion, merchandising, customer tiers (read) | "Run a 20% VIP weekend sale and put it on the main page" | through change sets |
| Orders and returns agent | back office | sales, fulfillment, messaging | "Go through the returns over $200" | through change sets |
| Customer care agent | back office | customer, messaging, community | "Answer the open order-status questions" | through change sets |
| Insights agent | back office | analytics | "Why were Thursday's sales down?" | no |
| Shopper agent | storefront | search, catalog (read), sales (cart, `prepare_order`), fulfillment (return options), customer (own profile) | "A light jacket that packs small for a trip" | cart and preferences only |
| Outside agents | MCP | reads, cart, `prepare_order` | via ChatGPT, Gemini | cart only |

In the MVP, the operator agent and the shopper agent run with domain-scoped tool search; the specialist split is the same mechanism with narrower scopes.

### 6.4 Policy as data

Business rules are values in `policy.yaml`, read by the back end, cited in checks, and shown to people. A selection from the demo store ([docs/plan.md §4.1](plan.md)):

| Key | Value |
|---|---|
| `max_discount.vip` | 30% |
| `price_floor` | cost + 15% |
| `promo_min_stock` | 10 units |
| `member_benefits_per_order` | 1 (coupons don't stack with tier coupons) |
| `refund_without_person_max` | $200.00 |
| `return_window_days` | 30 |
| `customer_messages` | always ask |

### 6.5 Agents that start work

Agents do not only wait for requests. They watch the store and propose work: twelve returns over $200 waiting, linen shirts selling out in about four days, products listed below cost. Each item lands in the back-office queue with a short reason and one action, and becomes a change set when the operator opens it.

### 6.6 UI on demand

The agent never writes HTML. It returns UI blocks that name a component from a fixed library and fill it with data ([docs/plan.md §5](plan.md)):

```jsonc
{
  "ui": "compare_table", "schema": 1, "block_id": "blk_31",
  "title": "Side by side",
  "why": "Shown as a table because you compare specs. Say \"show photos\" to change it.",
  "data": { },
  "actions": [{"intent": "add_to_cart", "label": "Add M", "args": {"product_id": "prd_field_jacket", "size": "M"}, "by": "shopper"}]
}
```

Buttons send intents back; they never change state by themselves. Back office: `change_set`, `task_list`, `refund_review`. Storefront: `understood_chips`, `compare_table`, `product_list`, `look_gallery`, `cart`, `memory_note`, `return_options`. The layout follows the person: a comparison table for a shopper who compares specs, a gallery for one who browses by look.

### 6.7 Memory people can see, actions only people take

- What the store remembers about a shopper appears as one line with Undo, and the full profile can be read and edited.
- Requests about someone else (a gift) are not saved to the shopper's profile.
- Placing an order, sending a return request, approving a change set, and undoing are person-only endpoints. The agent prepares; the person commits.

### 6.8 Evaluation

We build a screen-shaped facade over the same database, with the patterns from [docs/design.md §2](design.md), and run the same tasks through both tool layers with Glimmer (and Spark Open if credits allow): 12 design tasks and 8 held-out tasks, three runs each, each checked by reading the database ([docs/plan.md §3.7](plan.md)).

## 7. From the hackathon MVP to the target

| | Hackathon MVP | Target |
|---|---|---|
| Back end | Python, FastAPI, SQLite | Rust (Axum, Tokio), PostgreSQL with row-level security |
| Operations | The 27 tools in §3.4 | 13 domains, about 200 operations |
| Agents | Operator agent and shopper agent | Router and specialist agents per area |
| Stores | One demo store with a fixed clock | Many stores on one deployment |
| Search | Filters over the seed catalog | Hybrid keyword and vector search |
| Out of scope | Real payments and shipping, settlement, sellers, auth beyond one demo user per persona | Payment, carrier, and channel integrations |
