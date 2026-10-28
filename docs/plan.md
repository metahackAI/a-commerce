# A-Commerce build plan

Status: draft (Oct 28). This is the contract the four of us build against: what we ship, the tool signatures, the seed data, the UI block schema, and who owns what. [design.md](design.md) stays the source for *why*; this file is *what, by when*. Change it by pull request so everyone sees the change.

All numbers below match the mockups in [design/](../design). Where the mockups disagree with each other, §7 lists the disagreement and the value this plan uses.

Contents: 1 scope · 2 models · 3 tools · 4 seed data · 5 UI blocks · 6 roles and schedule · 7 mockup fixes

## 1. MVP scope

The demo video is the product. Everything is ranked by whether it shows up in the video.

| Priority | What | Done when |
|---|---|---|
| **P0** | Back end: FastAPI + SQLite, `policy.yaml`, seed, the P0 tools in §3 | Every P0 tool runs against the seed with `preview=true` and with a `preview_id` |
| **P0** | Harness: tool search, plan, execute, change set, read back, undo, action log | The VIP weekend request ends as change set #1042 v2 with 6 of 6 checks, runs 4 tools after approval, reads everything back, and undoes together |
| **P0** | Back office: layout A (conversation + work panel) with `change_set` | The flow in [changeset.png](../design/mockups/changeset.png) works with live data, including the v1 → v2 edit |
| **P0** | Storefront: Jamie's flow with `compare_table`, `cart`, memory line | Jamie sees $71.20 for the windbreaker on Sat Nov 7 and only the shopper's own tap places the order |
| **P1** | Alex's defect return (`return_options`) feeding the back office `refund_review` | Alex's request #20402 shows up as one of the 12 returns over $200 |
| **P1** | Mina's layout change (`product_list` → `look_gallery`) | "Less text, bigger photos" swaps the layout and leaves one undoable memory line |
| **P1** | Thin task list (layout E) as back office home | Lists the tasks in §4.8 and opens each in layout A |
| **P1** | Small evaluation: A-Commerce tools vs a screen-shaped facade | Numbers in the README from the held-out set (§3.7) |
| **P1** | Live demo deployed | A public URL in the submission |
| **P2** | `product_drafts`, `sales_explainer`, `fit_check`, `gift_picker`, layouts B, C, D, desktop app, Muse Image, SAM | Mockups only; the README says so |

Out of scope: real payments and shipping, settlement, partners, auth beyond a fixed demo user per persona.

**Demo clock.** The seed is a fixed moment, not today's date. `POST /demo/clock` sets it, `POST /demo/reset` reloads the seed. Story order equals clock order:

| Scene | Clock | Who |
|---|---|---|
| 1. Alex reports the boots | Fri Nov 6, 09:30 | Storefront (P1) |
| 2. Operator's morning: returns review, VIP weekend change set | Fri Nov 6, 10:40 → 11:05 | Back office |
| 3. Jamie shops the VIP weekend | Sat Nov 7, 10:00 | Storefront |
| 4. Mina's first visit on a phone | Sat Nov 7 | Storefront (P1) |

## 2. Models

| Role | Model | Where | Status |
|---|---|---|---|
| Worker: tool search, tool calls, routine steps | Muse Glimmer 30B | Partner inference server (Baseten, Fireworks or Modal) | **Check today** which partner serves it |
| Planner: multi-step plans, takeover after a failed check | Muse Spark Open | Baseten, model name `meta-models/muse-spark-open-1026` | Available |

Both are open weights on a partner server, so the whole agent runs without a closed model API. That is the "in-house open weights" story and the condition for the Best Build on Meta Open Weight Models prize. The harness talks to both through one OpenAI-compatible client; endpoints and keys come from `.env`. The README model table and design.md §8 (Spark via Meta Model API, OpenRouter during development) need updating to match.

## 3. Tools

### 3.1 Conventions (all tools)

From [design.md §3](design.md). The shapes are fixed here so the back end, harness and UI agree.

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

- Money is a number in dollars with two decimals. Dates are ISO 8601 in store time.
- Every tool definition carries `consumes` and `produces` (entity ids). The harness builds the tool graph from them.
- **Person-only actions are not tools.** Approving or undoing a change set, placing an order, and sending a return request are HTTP endpoints that only UI buttons call (§3.6). The models never see them, so no prompt can make an agent do them.

### 3.2 Operator tools: change the store (P0)

| Tool | Kind | Inputs | Returns | Consumes → produces |
|---|---|---|---|---|
| `get_policy` | read | `name?` (e.g. `"max_discount"`) | policy values with a one-line meaning each | → policy |
| `search_products` | read | `query?`, `category?`, `tags?[]`, `min_stock?`, `max_stock?`, `audience?` (`"everyone"`, `"vip"`), `fields?[]`, `limit=20`, `offset=0` | products with price, cost, stock, and the price for `audience` | → `product_id[]` |
| `get_product` | read | `product_id`, `fields?[]` | one product with variants and stock per size | `product_id` → product |
| `list_tiers` | read | — | tiers with member counts (`vip`: 1,284) | → `tier` |
| `list_active_promotions` | read | `starts_at`, `ends_at`, `audience?` | promotions overlapping the period | → `promotion_id[]` |
| `list_display_slots` | read | `page` (`"main"`), `starts_at`, `ends_at` | slots with what is booked and for whom | → `slot_id` |
| `set_sale_prices` | write | `name`, `audience` (`"vip"`), `starts_at`, `ends_at`, `items[]: {product_id, percent_off}`, `preview` | preview with checks `max_discount`, `price_floor`, `min_stock`, `promotion_overlap` | `product_id[]`, `tier` → `promotion_id` |
| `create_coupon` | write | `code`, `promotion_id`, `audience`, `auto_apply`, `stacking` (`"no_tier_coupons"`), `preview` | preview with check `coupon_stacking` | `promotion_id`, `tier` → `coupon_id` |
| `schedule_banner` | write | `slot_id`, `audience`, `starts_at`, `ends_at`, `eyebrow`, `headline`, `cta`, `link`, `preview` | preview with check `slot_free` | `slot_id`, `promotion_id` → `display_id` |
| `verify_change_set` | read | `change_set_id` | each change read back, `matches: true/false` per change | `change_set_id` → verification |

`set_sale_prices` does not change list prices. It writes a promotion: per-product sale prices for an audience and a period. Sale price = list price × (1 − percent_off / 100), rounded to the cent. The coupon is how that same benefit is recorded on the order at checkout; **it never adds a second discount.** The cart shows one line, "VIP weekend −$17.80".

After approval the executor runs exactly four tools: `set_sale_prices`, `create_coupon`, `schedule_banner`, `verify_change_set` ("runs 4 tools after approval").

### 3.3 Operator tools: returns (P1)

| Tool | Kind | Inputs | Returns | Consumes → produces |
|---|---|---|---|---|
| `search_return_requests` | read | `status` (`"waiting"`), `min_amount?`, `limit`, `offset` | requests with order, item, amount, reason, photos, days since delivery | → `return_request_id[]` |
| `get_order` | read | `order_id` | order with lines, payment, delivery date, coupon used | `order_id` → order |
| `evaluate_return_request` | read | `return_request_id` | `suggested` (`"refund"`, `"ask_for_photos"`, `"decline"`), the policy rule cited, refund amount | `return_request_id` → suggestion |
| `get_return_history` | read | `customer_id`, `days=90` | returns in the window | `customer_id` → history |
| `refund_return` | write | `return_request_id`, `amount`, `preview` | preview with checks `refund_to_original_payment`, `coupon_deducted` | `return_request_id` → `refund_id` |
| `send_customer_message` | write | `customer_id`, `order_id`, `kind` (`"ask_for_photos"`, `"decline"`, `"refund_done"`), `text`, `preview` | preview with the message as the customer will read it | `customer_id` → `message_id` |

Policy `customer_messages: always_ask` makes every `send_customer_message` need approval.

### 3.4 Shopper tools (P0 unless marked)

The shopper agent acts as the signed-in shopper. `my_*` tools are scoped to that person.

| Tool | Kind | Inputs | Returns | Consumes → produces |
|---|---|---|---|---|
| `search_products` | read | same as §3.2; the price returned is the shopper's own price | | → `product_id[]` |
| `compare_products` | read | `product_ids[]`, `attributes?[]` (`"packed"`, `"weight"`, `"warmth"`, `"light_rain"`, `"price"`) | one row per attribute, one column per product | `product_id[]` → comparison |
| `get_my_profile` | read | — | preferences, tier, sizes, saved address and card (last 4 only) | → profile |
| `update_my_preferences` | write, no approval | `add?[]`, `remove?[]` (`{key, value, source: "conversation"}`) | the memory line shown to the shopper and an undo id | → `memory_id` |
| `add_to_cart` | write, no approval | `product_id`, `size`, `color`, `qty=1` | cart | `product_id` → `cart_id` |
| `get_cart` | read | — | lines, subtotal, member benefit, shipping, total | `cart_id` → cart |
| `prepare_order` | write, preview only | `cart_id`, `ship_to` (`"home"` or an address used once), `gift?` | order ready for the shopper to place | `cart_id` → `order_preview_id` |
| `get_my_orders` (P1) | read | `limit` | orders with lines and delivery dates | → `order_id[]` |
| `get_return_options` (P1) | read | `order_id`, `line_id`, `reason` | options (`"replacement"`, `"refund"`) with timing, cost to the shopper, and who confirms | `order_id` → options |
| `get_product_measurements` (P2) | read | `product_id`, `sizes[]` | chest, shoulder, sleeve, length | |
| `get_review_summary` (P2) | read | `product_id`, `height_cm_range?` | fit votes and repeated phrases | |
| `get_delivery_dates` (P2) | read | `product_id`, `address`, `by?` | earliest arrival | |

### 3.5 Change set lifecycle

The harness groups the previews of one request into a change set.

```
drafting → waiting_for_approval → running → verifying → live
                    ↑  │                         └→ failed (shows which change did not match)
     edit by talking ┘  └→ rejected                live → undone (until the undo deadline)
```

Each edit by talking re-runs the previews and bumps the version (v1 → v2). The undo deadline is the promotion end plus one day (Nov 9, 23:59 for #1042).

### 3.6 Person-only endpoints (not tools)

| Endpoint | Who | What |
|---|---|---|
| `POST /change-sets/{id}/approve` `{version}` | operator | Starts the run; refuses if the version is not the latest |
| `POST /change-sets/{id}/undo` | operator | Undoes every change in the set together |
| `POST /orders` `{order_preview_id}` | shopper | Places the order |
| `POST /returns` `{order_id, line_id, option}` | shopper | Sends the return request; it becomes a waiting return request in the back office |
| `POST /memories/{id}/undo` | shopper | Removes a remembered preference |

Chat and blocks stream over SSE: `POST /bo/chat`, `POST /fo/chat`, both returning messages, agent steps and UI blocks (§5).

### 3.7 Evaluation (P1)

- **Facade.** A screen-shaped API over the same database with the patterns in design.md §2, written by us: a promotion target grid read as `POST` plus a separate count call, abbreviated names, code values (`promoType: "T03"`), a bulk `save…List(rows[])` with a row-status flag, and a V1/V2 duplicate.
- **Tasks.** 12 design tasks and 8 held-out tasks across sale, coupon, banner, return and shopper questions, each with a checker that reads the database.
- **Runs.** Glimmer on both tool layers, 3 runs per task (pass^3). Spark Open on both if credits allow.
- **Metrics.** Completion, right tools in the right order, calls per task, tokens per task. Only held-out numbers go in the README.

## 4. Seed data

Store **Northwind Apparel**, `northwind.example`, store time US Eastern. Operator **Son SeongJun**, Merchandiser. All names are made up.

### 4.1 Policies (`policy.yaml`)

| Key | Value | Seen in |
|---|---|---|
| `max_discount.vip` | 30% | inbox policies, change set check |
| `max_discount.everyone` | 20% | inbox policies |
| `price_floor` | cost + 15% | change set check, product drafts |
| `promo_min_stock` | 10 units (fewer are left out of sales) | change set check |
| `member_benefits_per_order` | 1 (coupons don't stack with tier coupons) | change set check |
| `refund_without_person_max` | $200.00 | inbox, returns, Alex |
| `return_window_days` | 30 | returns |
| `defect_refund` | full refund, free return shipping | returns, Alex |
| `changed_mind_return_shipping` | paid by the customer | returns |
| `damage_claim_needs_photo` | true | returns |
| `size_exchange_days` | 30, free | fit check |
| `gift_exchange_until` | Dec 31 | gift |
| `customer_messages` | always ask | inbox policies |
| `undo_window_after_end_days` | 1 | change set |
| `free_shipping_min_subtotal` | $80.00, measured before member benefits | Jamie's cart ($89.00 → free) |
| `pricing_rule` (P2) | cost × 2.5, rounded to the nearest price ending in 9 | product drafts |

### 4.2 Tiers and members

| Tier | Members | Notes |
|---|---|---|
| VIP | **1,284** | Jamie |
| Gold | generator | has "Gold tier 10%" coupon |
| Member | generator | Alex |
| Guest | — | Mina (no account) |

### 4.3 Summer outerwear (category of 42 products)

Named products. The other 35 are generated with cost ≈ 40% of price and stock 10–250, so they all pass the floor at 20%.

| Product | Color, fabric | List | Cost | Stock | VIP weekend | Notes |
|---|---|---|---|---|---|---|
| Linen blend field jacket | Sand, linen and cotton | $128.00 | $51.20 | 86 | 20% → **$102.40** | 340 g (M), folds flat, light, no rain; 3 colors |
| Packable windbreaker | Slate, recycled nylon | $89.00 | $35.60 | 214 | 20% → **$71.20** | 190 g, packs into its own pocket 14 × 18 cm, wind layer only, light rain; 4 colors; M: chest 57, shoulder 47, sleeve 64, length 71 cm |
| Seersucker overshirt | Clay, cotton | $79.00 | **$56.00** | 53 | 15% → **$67.15** | 360 g, folds flat, light, no rain; 2 colors. Floor $64.40, so 20% ($63.20) fails |
| Cotton utility jacket | — | $118.00 | $47.20 | 31 | 20% → **$94.40** | 480 g, folds flat |
| Linen camp overshirt | — | $88.00 | $35.20 | generator | 20% → **$70.40** | 300 g, folds flat |
| Cotton chore coat | — | **$142.00** | $56.80 | **7** | left out | 620 g; out of M and L since Wed Nov 4 15:00 |
| Light rain parka | — | $169.00 | $67.60 | **4** | left out | 420 g, rolls into its hood; out of M and L since Wed Nov 4 15:00 |

Result: 40 of 42 on sale, 2 left out, highest discount 20%.

### 4.4 Other products

| Product | Category | Price | Notes |
|---|---|---|---|
| Wool overcoat | Coats | $289.00 | Charcoal, 70% wool. M: chest 56, shoulder 46, sleeve 64, length 98 cm. L: 59, 48, 65.5, 100 cm. Reviews: 41 people 175–182 cm, 29 true to size, 12 a bit snug, "size up for a hoodie" 9 times (P2) |
| Leather chelsea boots | Footwear | $312.00 | Tan, size 9: 6 in stock |
| Cashmere sweater | Knitwear | $248.00 | |
| Denim jacket | Denim | generator | Bestseller, out of M and L since Wed Nov 4 15:00 |
| Linen shirt | Shirts | generator | 38 left, 61 sold in the last 7 days |
| Fleece half-zip | Active | $84.00 | Gift pick (P2) |
| Merino trail tee | Active | $58.00 | Gift pick (P2) |

24 categories in total (P2 drafts match supplier rows to them).

### 4.5 Promotions and display before the change set

| Promotion | Status | Who | When |
|---|---|---|---|
| Autumn knitwear 15% | running | everyone | Nov 1 – Nov 15 |
| Free shipping over $80 | running | everyone | always |
| Birthday coupon | running | members | always |
| Gold tier 10% | running | Gold tier | always |
| Denim bundle | scheduled | everyone | Nov 13 – Nov 20 |
| Linen clearance | ended | everyone | Oct 1 – Oct 20 |

4 running, so the overlap check reads "Checked 4 running promotions". Main page hero: "Fall layers" for everyone, live since Fri Nov 6 08:10. The VIP hero for Nov 7–8 is free.

### 4.6 Change set #1042 (expected result, not seeded)

| | v1 (11:02) | v2 (11:05) |
|---|---|---|
| Request | "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page." | + "Drop the low-stock ones. Give the overshirt 15% instead." |
| Products | 42, all at 20% | 40; overshirt 15%; chore coat and rain parka left out |
| Checks | 4 of 6: `min_stock` and `price_floor` fail | 6 of 6 |
| Who | VIP tier, 1,284 members | same |
| When | Sat Nov 7 00:00 – Sun Nov 8 23:59 | same |
| Coupon | `VIPWEEKEND20`, issued to 1,284 VIP members, auto-applies at checkout, not with tier coupons | same |
| Main page | Hero slot, VIP only: "VIP WEEKEND" / "20% off summer outerwear" / "Shop the sale"; everyone else keeps "Fall layers" | same |
| Undo | together until Nov 9, 23:59 | same |

### 4.7 Shoppers

| Shopper | Profile | Orders | Other |
|---|---|---|---|
| **Jamie** (VIP) | Size M, natural fabrics, compares specs | 6 orders, last 2 in size M | Home: 12 Harbor St, Apt 3; card ending 4821; standard delivery from Sat Nov 7 arrives Tue Nov 10 |
| **Alex** (Member) | — | **#20402**, leather chelsea boots, size 9, tan, $312.00, delivered Sat Oct 31, card ending 0937 | Sends 2 photos with the return; pickup Mon Nov 9, 9:00–12:00; refund lands 3–5 days after it is confirmed |
| **Mina** (guest) | none | none | On a phone |

Expected storefront results:

- Jamie, Sat Nov 7: compare windbreaker, field jacket, overshirt (sorted by packed size); adds windbreaker M slate. Cart: subtotal $89.00, VIP weekend −$17.80, shipping free, **total $71.20**. New memory: "Weight and packed size matter to you when traveling" (Undo). Profile then shows "Travels light".
- Mina: list of 3 at list prices ($89, $128, $79); after "Less text. Just show me bigger photos." the same results as a gallery. New memory: "Photos over details" (Undo).
- Alex: options "New pair, same size" (ships today, arrives Tue Nov 10, pays nothing) and "Full refund" ($312.00 to card ending 0937, confirmed by a person within 1 business day).

### 4.8 Back office queue on Fri Nov 6

| Item | Started | Status |
|---|---|---|
| VIP weekend sale: summer outerwear | You asked, 11:02 (v2 11:05) | Needs you: 6 of 6 checks |
| 12 return requests over $200 | Agent noticed, 10:40 | Needs you: 9 refunds, 2 photo requests, 1 decline |
| Linen shirts will sell out in about 4 days | Agent noticed, 09:12 | Needs you: reorder draft, 3 sizes, 240 units |
| List 30 products from the supplier sheet | You asked, 10:58 | Agent working: 18 of 30 drafted (P2) |
| Add missing size charts | Agent noticed, 11:00 | Agent working: 15 of 18 checked (P2) |
| Fix 4 products listed below cost | Automation, 08:31 | Done: 4 prices raised to the floor, read back, all match |
| Swap the main page banner to "Fall layers" | You asked, 08:10 | Done: live, undo until Nov 7 |
| Answer order-status questions | Shopper agent, 08:02 | Done: 2 handed to you, both closed |

Only the first two need to work live. The rest are seeded rows.

### 4.9 Return requests over $200 (P1)

12 waiting, all over $200. 11 are seeded; Alex's #20402 is the twelfth when scene 1 runs first (seeded as well if it does not).

| Order | Item | Amount | Reason | Evidence | Suggested |
|---|---|---|---|---|---|
| #20402 | Leather chelsea boots | $312.00 | Defect: sole coming apart | 2 photos, day 6 | Refund (defects in full, free return shipping) |
| #20431 | Wool overcoat | $289.00 | Color differs from photo | 1 photo, day 4 | Refund; customer has 4 returns in 90 days (the check to look at) |
| #20418 | Linen blend field jacket ×2 | $256.00 | Too small | day 7 | Refund (within 30 days, tags on) |
| #20377 | Field jacket + windbreaker | $217.00 | Changed mind | day 9 | Refund (customer pays return shipping) |
| #20366 | Rain parka ×2 | **$338.00** | Arrived damaged | no photo, day 2 | Ask for photos |
| #20290 | Cashmere sweater | $248.00 | Changed mind | delivered Sep 28, day 39 | Decline (outside 30 days) |
| 5 generated | — | $315.00, $302.00, $276.00, $268.00, $228.00 | within 30 days, refundable | — | Refund |
| 1 generated | — | over $200 | Arrived damaged | no photo | Ask for photos |

Totals: 9 refunds = **$2,463.00**; 2 photo requests; 1 decline. Checks: 12 of 12 payments can be refunded; 3 orders used a coupon; 11 of 12 inside 30 days; one customer with 4 returns in 90 days (#20431).

### 4.10 Sales (P2, `sales_explainer`)

Daily revenue Oct 23 – Nov 5. Thu Nov 5: $18,240, 201 orders. Last 4 Thursdays: $22,900 and 247 orders on average. Causes: 3 bestsellers out of M and L (−$2,700), weekly email not sent because the list import failed Thu 08:00 (−$1,300), not explained (−$660).

## 5. UI blocks

The agent never writes HTML. It returns blocks; the web app has one React component per `ui` name.

### 5.1 Envelope

```jsonc
{
  "ui": "compare_table",          // component name
  "schema": 1,
  "block_id": "blk_31",
  "title": "Side by side",
  "why": "Shown as a table because you compare specs. Say \"show photos\" to change it.",  // storefront only; shown under the block
  "data": { },                    // per component, below
  "actions": [                    // buttons; each sends an intent back, never changes state by itself
    {"intent": "add_to_cart", "label": "Add M", "args": {"product_id": "prd_field_jacket", "size": "M"}, "by": "shopper"}
  ]
}
```

`by` says who may trigger it: `agent`, `operator` or `shopper`. Intents marked for a person call the endpoints in §3.6. Agent steps in the conversation are their own message type:

```json
{"type": "agent_step", "model": "muse-glimmer-30b", "tool_calls": 9, "duration_ms": 3800, "read_only": false, "text": "I put this together as a change set. Nothing has changed in the store yet."}
```

### 5.2 Components

| `ui` | Side | Priority | Mockup |
|---|---|---|---|
| `change_set` | back office | P0 | [changeset](../design/mockups/changeset.png) |
| `task_list` | back office | P1 | [e-tasks](../design/options/e-tasks.png) |
| `refund_review` | back office | P1 | [intent-returns](../design/options/intent-returns.png) |
| `understood_chips` | storefront | P0 | [store](../design/mockups/store.png) |
| `compare_table` | storefront | P0 | [fo-two-shoppers](../design/storefront/fo-two-shoppers.png) |
| `cart` (with the place-order sheet) | storefront | P0 | [store](../design/mockups/store.png), [mobile confirm](../design/mockups/mobile-confirm.png) |
| `memory_note` | storefront | P0 | "Remembered: … Undo" |
| `product_list`, `look_gallery` | storefront | P1 | [fo-mobile-adapt](../design/storefront/fo-mobile-adapt.png) |
| `return_options` | storefront | P1 | [fo-return](../design/storefront/fo-return.png) |
| `product_drafts`, `sales_explainer`, `fit_check`, `gift_picker` | both | P2 | mockups only |

### 5.3 `change_set`

```jsonc
{
  "id": 1042, "version": 2, "status": "waiting_for_approval",
  "title": "VIP weekend sale: summer outerwear",
  "request": ["Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page.",
              "Drop the low-stock ones. Give the overshirt 15% instead."],
  "understood": [
    {"label": "Who", "value": "VIP tier", "detail": "1,284 members"},
    {"label": "Products", "value": "Summer outerwear", "detail": "40 of 42 · 2 excluded"},
    {"label": "Benefit", "value": "20% off", "detail": "Overshirt 15%"},
    {"label": "When", "value": "Nov 7–8", "detail": "Sat 00:00 – Sun 23:59"},
    {"label": "Where", "value": "Main page hero", "detail": "Shown to VIP only"}
  ],
  "checks": [
    {"rule": "max_discount", "passed": true, "title": "Discount within the VIP limit", "detail": "Highest is 20%; policy allows 30%"},
    {"rule": "price_floor", "passed": true, "title": "No price below the floor", "detail": "Overshirt moved to 15% to stay at cost + 15%"},
    {"rule": "min_stock", "passed": true, "title": "Enough stock", "detail": "2 products under 10 units excluded"},
    {"rule": "promotion_overlap", "passed": true, "title": "No overlap with running promotions", "detail": "Checked 4 running promotions"},
    {"rule": "coupon_stacking", "passed": true, "title": "Coupon doesn't stack with tier coupons", "detail": "Stacking policy: one member benefit per order"},
    {"rule": "slot_free", "passed": true, "title": "Main page slot is free", "detail": "Hero slot open Nov 7–8"}
  ],
  "groups": [
    {"op": "update", "title": "Sale prices", "summary": "40 products, Nov 7–8", "undo": "Undo restores current prices",
     "columns": ["Product", "Stock", "Discount", "VIP price"],
     "rows": [{"product_id": "prd_field_jacket", "name": "Linen blend field jacket", "stock": 86, "discount": "20%", "was": 128.00, "now": 102.40},
              {"product_id": "prd_overshirt", "name": "Seersucker overshirt", "stock": 53, "discount": "15%", "note": "floor", "was": 79.00, "now": 67.15}],
     "total_rows": 40},
    {"op": "create", "title": "Coupon", "summary": "1 new", "fields": {"Code": "VIPWEEKEND20", "Issued to": "1,284 VIP members", "Applies": "Automatically at checkout", "Stacking": "Not with tier coupons"}},
    {"op": "exclude", "title": "Left out", "summary": "2 products", "rows": [{"name": "Chore coat", "note": "7 left"}, {"name": "Rain parka", "note": "4 left"}]},
    {"op": "schedule", "title": "Main page", "summary": "Hero slot, Nov 7–8",
     "preview": {"audiences": ["VIP members", "Everyone else"], "date": "Sat Nov 7", "eyebrow": "VIP WEEKEND", "headline": "20% off summer outerwear", "cta": "Shop the sale"}}
  ],
  "after_approval": {"tools": ["set_sale_prices", "create_coupon", "schedule_banner", "verify_change_set"], "undo_until": "2026-11-09T23:59:00"},
  "verification": null   // filled after the run: [{"change": "...", "matches": true}]
}
```

Actions: `approve_change_set` (operator), `compare_versions` (operator), `undo_change_set` (operator, once live). Requests for changes go through the chat box.

### 5.4 `refund_review` (P1)

```jsonc
{
  "summary": {"waiting": 12, "refund": {"count": 9, "amount": 2463.00}, "ask_for_photos": 2, "decline": 1},
  "rows": [{"order_id": "#20402", "item": "Leather chelsea boots", "amount": 312.00, "reason": "Defect: sole coming apart",
            "evidence": "2 photos · Day 6", "suggested": "refund", "rule": "Defects are refunded in full, return shipping free"}],
  "checks": [{"title": "Return history", "passed": false, "detail": "One customer has 4 returns in 90 days (#20431)"}],
  "messages": [{"order_id": "#20290", "kind": "decline", "text": "…"}],
  "after_approval": "refund 9 · send 2 photo requests · send 1 decline note · read back each order"
}
```

Action: `approve_change_set` with label "Approve 9 refunds and send 3 messages". The batch is a change set under the hood.

### 5.5 Storefront blocks

```jsonc
// understood_chips
{"chips": [{"label": "Light jacket"}, {"label": "Spring travel"}, {"label": "Packs small"},
           {"label": "Size M", "source": "profile"}, {"label": "Natural fabrics", "source": "profile"}]}
// action per chip: remove_chip (shopper)

// compare_table
{"sort": "packed size", "products": [{"product_id": "prd_windbreaker", "name": "Packable windbreaker", "sub": "Slate · recycled nylon", "badge": "Best for carry-on"}],
 "rows": [{"label": "Packed", "values": ["Into its own pocket, 14 × 18 cm", "Folds flat", "Folds flat"]},
          {"label": "Weight, M", "values": ["190 g", "340 g", "360 g"]},
          {"label": "Your price", "values": [{"now": 71.20, "was": 89.00, "note": "VIP weekend, 20%"}, {"now": 102.40, "was": 128.00, "note": "VIP weekend, 20%"}, {"now": 67.15, "was": 79.00, "note": "VIP weekend, 15%"}]}]}

// product_list / look_gallery share one item shape; only the layout differs
{"items": [{"product_id": "prd_windbreaker", "name": "Packable windbreaker", "price": 89.00, "facts": ["190 g", "packs into own pocket", "light rain", "recycled nylon", "4 colors"], "look": "Over a tee for windy days"}]}

// cart
{"lines": [{"name": "Packable windbreaker", "variant": "Slate · M", "qty": 1, "note": "Same size as your last 2 orders", "price": 89.00}],
 "subtotal": 89.00, "benefit": {"label": "VIP weekend", "amount": -17.80}, "shipping": 0.00, "total": 71.20,
 "ship_to": "Home · 12 Harbor St, Apt 3", "arrives": "Tue, Nov 10", "pay_with": "Card ending 4821",
 "notice": "Northwind can prepare your order, but only you can place it."}
// action: place_order (shopper) → POST /orders

// memory_note
{"memory_id": "mem_12", "text": "Remembered: weight and packed size matter to you when traveling"}
// action: undo_memory (shopper)

// return_options (P1)
{"order": {"id": "#20402", "delivered": "Sat Oct 31", "item": "Leather chelsea boots", "variant": "Size 9 · Tan", "amount": 312.00, "photos": 2},
 "options": [{"kind": "replacement", "title": "New pair, same size", "ships": "Today, 6 in stock", "arrives": "Tue Nov 10", "you_pay": 0.00},
             {"kind": "refund", "title": "Full refund", "amount": 312.00, "to": "card ending 0937", "confirmed_by": "A person at Northwind, usually within 1 business day", "money_back": "3–5 days after it's confirmed"}],
 "send_back": {"cost": "free, defect", "pickup": "Mon Nov 9, 9:00–12:00"}}
// action: send_return_request (shopper) → POST /returns
```

## 6. Roles and schedule

### 6.1 Roles

Owners are a proposal; confirm at kickoff. Each owner can hand pieces to coding agents, following [AGENTS.md](../AGENTS.md).

| Role | Owns | First deliverable | Owner |
|---|---|---|---|
| **1. Tools and data** | `server/` app, `policy.yaml`, seed generator matching §4, all tools in §3.2–3.4, person-only endpoints, demo clock | P0 tools passing a script that replays §4.6 by hand | |
| **2. Harness and models** | Model endpoints (§2), tool search over `consumes`/`produces`, planner, executor, change set lifecycle, read back, action log, SSE, evaluation (§3.7) | VIP weekend request → change set v1 from a real model | |
| **3. Back office web** | `web/` shell, layout A, `change_set`, `task_list`, `refund_review` | `change_set` rendering the §5.3 JSON from a fixture | Son SeongJun (made the mockups) |
| **4. Storefront web and story** | Storefront blocks in §5.5, demo script and video, README, X post, submission | `compare_table` + `cart` from fixtures | |

The submitting member (team representative) also owns the submission form and the Discord `#office-hours` questions.

Proposed layout:

```
server/   FastAPI app, tools/ (one module per domain), policy.yaml, seed/, harness/
web/      React + Vite + TypeScript, routes /bo and /fo, blocks/ (one component per ui name)
eval/     tasks/, facade/, runner, results/
```

Roles 3 and 4 start from JSON fixtures copied from §5, so nobody waits for the harness.

### 6.2 Schedule (Korea time)

Submission closes **Wed Nov 4, 9:00 PM ET = Thu Nov 5, 11:00 KST** (US daylight saving ends Nov 1). Our internal deadline stays **Nov 5, 10:00 KST**.

| Day | Goal | Events |
|---|---|---|
| Wed Oct 28 | This plan merged. Model endpoints answer. Each member applies for credits (Meta $150 by Nov 4, Modal first come, Baseten reviewed, Fireworks per team) | |
| Thu Oct 29 | Server skeleton, policy, seed, P0 read tools. Fixtures render in both web routes | 01:00 Agents 102 "Best Practices for Verifiable Agents" |
| Fri Oct 30 | P0 write tools with preview. Change set runs end to end from a hard-coded plan: approve, run, read back, undo | |
| Sat Oct 31 | Real models plan and call tools. Back office shows live change set. Jamie's flow on live data | 01:00 Fireworks session (Muse inference to training) |
| Sun Nov 1 | P1: Alex → refund review, Mina's layout. Facade and task set | |
| Mon Nov 2 | Evaluation runs, numbers. Deploy live demo | `#office-hours` moves to 00:00 KST |
| Tue Nov 3 | Record the video. README (models, numbers, "what we built" by date). Post on X | |
| Wed Nov 4 | Buffer, final pass on rules (only hackathon work in the demo) | |
| Thu Nov 5 | Submit by 10:00 | Judging Nov 5–8, finals Nov 10 02:00 |

Posting the video on X before submitting gives the Most Viral entry the most time before its Nov 8 cutoff, and the link is part of the submission.

## 7. Mockup disagreements

The plan uses the value on the right. The mockups should be updated to match.

| # | Where | Mockups say | Plan uses |
|---|---|---|---|
| 1 | Chore coat price | $149.00 in `mockups/store.html`; $142.00 in `options/c-canvas.html` and `storefront/fo-two-shoppers.html` | $142.00 |
| 2 | Returns triage | "Approve 10, hold 2 for photos" in `mockups/inbox.html`; "12 refunds … 4/4 checks" in `options/e-tasks.html`; 9 refunds, 2 photos, 1 decline, 1 check to look at in `options/intent-returns.html` | 9 refunds ($2,463.00), 2 photo requests, 1 decline; 3 of 4 checks pass |
| 3 | Return #20366 | "Rain parka ×2, $236.00" ($118 each) but the rain parka is $169.00 | $338.00. It is a photo request, so the $2,463.00 refund total does not change |
| 4 | Inbox times | `mockups/inbox.html`: returns 09:00, supplier sheet 10:15, linen shirts 08:40, below cost 07:55, banner 09:30 | Times from options B and E (§4.8) |
| 5 | Supplier pricing | Quilted liner $52 → $139; the rule "cost × 2.5, ending in 9" gives $129 (the other rows match rounding to the nearest 9) | $129.00 (P2) |
| 6 | Fit check timing | `fo-fit` says Jamie already owns and kept the windbreaker; `store.html` has Jamie buying it on Nov 7 | Fit check is a later scene, after the Nov 10 delivery (P2) |
| 7 | Tool names | design.md §4: `set_price`, `schedule_price_change`, `create_promotion`, `issue_coupon`, `schedule_display`, `remember_preference`; option D: `set_sale_prices`, `create_coupon`, `schedule_banner`, `verify_change_set` | Option D's names plus §3. design.md §4 should be updated |
| 8 | VIP price and coupon | The change set has both "sale prices" and an auto-applied 20% coupon, which reads as two discounts | One discount. The coupon records the VIP weekend benefit on the order (§3.2) |
| 9 | Models | README and design.md: Spark through the Meta Model API, OpenRouter during development | Glimmer and Spark Open on partner inference servers (§2) |
