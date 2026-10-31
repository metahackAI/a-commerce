# A-Commerce design

Status: written Oct 27; principles set Oct 31; §4–§9 checked against the code on Oct 31. This document is the brief for everyone building A-Commerce, people and coding agents alike.

## Principles

**People state the goal and approve the result; the system does the rest. Intelligence comes from the model; trust comes from the structure.**

This is the one list of principles. The README repeats it in short and links here. Each principle names the sign that it is broken, so a code review or a demo can check it.

1. **Tools work in the units people use.** A tool finds things by what people say (an order number, an email, a product name) and returns what the decision needs, not raw rows.
   *Broken when:* the agent asks the person for an internal id.
2. **Every change is a proposal first.** The agent never changes the store directly. It drafts a change set with a preview and checks, and a person reviews it like a pull request where the rules require a person. A change that can be undone says how; one that cannot (a sent message, a refund) says so before approval.
   *Broken when:* a call changes state without a preview.
3. **Rules are data, and the system checks them.** Business rules such as discount limits and approval thresholds live in policy files, and the back-end enforces them, so a wrong model answer cannot break them.
   *Broken when:* a business number (a discount rate, an amount) appears in a prompt or in harness code as a rule.
4. **The system brings the work.** Events such as low stock or a spike in returns create tasks, and the agent prepares a draft before anyone asks.
   *Broken when:* nothing happens until a person asks.
5. **The screen is the result of the work.** Instead of menus to navigate, the UI for the task at hand appears, fitted to the person.
   *Broken when:* the answer is text only and the person has to finish the job on another screen.

**Evidence rule.** A claim that a principle holds is made only with numbers from the shared task set ([../eval](../eval), design and held-out tasks). One scene working is not evidence.

The tool rules in §3 are how principles 1–3 are built (rule 9 is principle 1 for references). [research.md](research.md) §3 records the research that led to this list.

## 1. Goal

Build a commerce back-end that is **AI-friendly by design**, plus a **commerce harness** that uses it to do real work, and show that:

1. A small open-weight model (Muse Glimmer) can find the right tools, chain them, and finish back-office and storefront tasks.
2. The same tasks fail or cost much more when the same model faces a typical screen-shaped commerce API.
3. Operators and shoppers only talk; the UI they need appears as components.

## 2. What makes a commerce API hard for AI

From building enterprise commerce platforms, we see the same patterns again and again. Each is fine for a team clicking through screens, and each confuses a model:

| Pattern | Example shape | Why it hurts a model |
|---|---|---|
| Screen-shaped endpoints | `getRefundPopupInitData`, `getOrderGridList` | One business action is split across many screen calls; the model must know the screen flow |
| Abbreviated names | `getPrmtTgtMbrMgmtList` | The model can't tell what a tool does from its name |
| Codes instead of meaning | `promoType: "T03"` | Valid values live in a code table the model never sees |
| Reads as POST, separate count calls | `POST /getXList` + `GET /getXListCount` | Unclear which calls are safe; two calls for one answer |
| Bulk saves | `saveXList(rows[])` with a row-status flag | Create, update, and delete hidden behind one call |
| Version duplicates | `XController` and `XControllerV2` | Two tools look like the same thing |
| Rules only in people's heads | "coupons don't stack with tier discounts" | The model breaks rules it was never told |

## 3. Tool design rules

Every capability in A-Commerce is exposed as a tool that follows these rules. Code review checks them.

1. **One business verb per tool.** `create_promotion`, `issue_coupon`, `cancel_order_lines`. No screens, popups, or grids.
2. **Plain names.** `verb_noun`, full English words. The name and a one-line description must be enough to pick the tool.
3. **Typed inputs with meaning, not codes.** Enums are words (`"percent_off"`, `"vip"`). Every field has a description and an example.
4. **Safe reads.** Reads never change state. A list returns items and the total together. A `fields` argument keeps responses small.
5. **Writes go through preview.** Every write supports `preview=true`, which returns what will change, the rule checks, and whether a person must approve. The real write takes the `preview_id`.
6. **Declared links.** Each tool declares what it consumes and produces (for example, `search_products` produces `product_id[]`; `create_promotion` consumes `product_id[]` and produces `promotion_id`). The harness builds a tool graph from these declarations.
7. **Rules are data.** Business rules (maximum discount, stacking, approval thresholds) live in a policy file that the back-end enforces. A violation comes back as a readable reason plus the tool that can fix it. Rules are never hidden in a prompt.
8. **Actionable errors.** Each error has a code, a human sentence, and a suggested next tool.
9. **References take what people say.** A field that takes an order, change set, or customer id also takes the order number, the change set number, or the email; the server finds the record before the tool runs. Values that can match several records, such as product names, go through a search tool. The table is in [api/conventions.md](api/conventions.md) §2.

## 4. Domain scope

Written from scratch; the names and shapes are ours. The store has 13 domains with 291 tools (132 reads, 119 writes, 40 writes that always need a person) and 18 person-only endpoints, all built. The full hierarchy is in [api/](api/README.md) §4, and the code follows it: `domain.resource.operation` is `crates/domains/{domain}/src/{resource}/{tool}.rs` (`cargo xtask check-layout`, `cargo xtask check-coverage`).

| Domain | Entities | Example tools |
|---|---|---|
| catalog | product, variant, offer, stock, reorder, brand, review, question | `get_product`, `set_price`, `set_variant_stock`, `create_reorder`, `batch_import` |
| merchandising | category, display slot, banner, popup, curated page | `list_display_slots`, `schedule_banner`, `publish_curation` |
| search | product search, lexicon, index | `search_products`, `compare_products`, `reindex` |
| sales | cart, order, payment, receipt, subscription, gift order | `add_to_cart`, `prepare_order`, `get_order`, `prepare_gift_order` |
| fulfillment | cancel, return, exchange, inspection, refund, delivery, shipping zone | `evaluate_return_request`, `refund_return`, `get_delivery_dates` |
| promotion | promotion, member price, coupon, benefit, store points, event, segment | `set_sale_prices`, `create_coupon`, `calculate_benefits` |
| customer | member, tier program, address, consent, preference, inquiry, remedy | `search_members`, `list_tiers`, `update_my_preferences` |
| messaging | customer message, template, campaign | `send_customer_message`, `create_campaign` |
| community | board, post, reply | `list_posts`, `report_post` |
| platform | policy, change set, task, action log, role, operator, API key | `get_policy`, `verify_change_set`, `list_tasks` |
| analytics | sales, customer, and service figures (read only) | `explain_sales_change`, `get_sales_trend` |
| partner | seller, contract, seller grade | `onboard_seller`, `set_contract` |
| integration | outside channel, product import, carrier and payment events | `import_products`, `sync_order_status` |

Everything outside the store is a deterministic stub that says so in its output: payment providers, bank deposits, tax receipts, carriers, outside sales channels, email/SMS/push delivery, business-registry checks, payouts, and image and copy generation. That is by design for a showcase. The store's own data, rules, change sets, and jobs are real.

## 5. Harness

```
request ─► route ─► tool search ─► plan ─► execute ─► check ─► answer as UI blocks
                         │            │         │          │
                  words + tool     Glimmer   preview →   preview vs request,
                      graph       (Spark if  change set  reply vs tool results
                                 configured)
```

- **Route**: five agents (`agents/*.yml`): `promotion`, `orders`, `catalog`, and `insights` in the back office, `shopper` on the storefront. A request that contains a phrase an agent names goes to it without a model call; otherwise the router model picks.
- **Tool search**: the model never sees all tools. The harness scores the agent's tools by word match (BM25 over name, description, and fields, plus a share for the agent's home domains) and adds graph neighbors (tools that produce what a hit consumes, or consume what it produces), 5 to 8 candidates. No model call. Embedding search is not built.
- **Plan**: a short ordered list of tool calls. Glimmer (the worker) plans first; a plan of more than three steps or across domains, or one that failed, goes to Muse Spark (the planner) when Spark is configured. Every agent names worker `glimmer` and planner `spark` (`agents/models.yml`). On the live demo Spark is not configured, so every model call runs on Glimmer.
- **Execute**: reads run directly. Writes are previewed first. A write that needs a person goes into a change set, which the agent submits and a person approves with a button; the agent can never approve or run it. Writes that need no approval (cart, a remembered preference) run once their preview passes.
- **Check**: rule checks are the back-end's (the preview's `checks`, and `verify_change_set`, which reads the store back after an approved change set runs). The agent compares, deterministically, what was asked with what the preview will do, and the numbers in its reply with the turn's tool results.
- **Limits**: each turn stops at 12 model calls, 20 tool calls, 2 plan revisions, or 60 seconds (`agents/runtime.yml`) and says why.
- **Action log**: every call, preview, approval, and check is recorded and visible (`action_log` block, `/bo/activity`).
- **Work the system brings**: watchers (`watcher.price_below_cost`, `watcher.stock_out_forecast`, `watcher.returns_over_refund_threshold`) open tasks from the policy's thresholds. When a store turns on `task_pickup` (the demo store does), the gateway starts an agent turn on a new task in the name of the policy's operator, so a draft is waiting before anyone asks.

## 6. UI protocol

The agent never writes HTML. It returns UI blocks that name a component from a fixed library and fill it with data from the tool results; the model chooses the block and writes only the short words (`title`, `why`):

```json
{"block_id": "blk_12", "ui": "change_set", "schema": 1, "title": "VIP weekend sale", "data": {"change_set_id": "cs_1042", "version": 2, "checks": []}, "actions": [{"kind": "person", "label": "Approve"}]}
```

The library ([api/chat.md](api/chat.md) §3) has one React component per block in `web/src/components/blocks/{bo,fo}`:

- back office: `change_set`, `refund_review`, `product_drafts`, `sales_explainer`, `task_list`, `product_table`, `action_log`;
- storefront: `looking_for`, `compare_table`, `look_gallery`, `product_list`, `fit_check`, `gift_picker`, `cart`, `checkout_card`, `return_options`, `order_detail`, `memory_note`, `profile_card`.

The first list in this document (`product_table`, `product_gallery`, `product_compare`, `promotion_preview`, `approval_card`, `order_detail`, `cart`, `checkout_card`, `action_log`) was refined by the mockups into this library (for example `compare_table`, `look_gallery`, and `change_set`, which joins the promotion preview and the approval card); api/chat.md §3.3 maps the old names to the new ones.

The blocks and back-office pages cover the scenes in [scenarios.md](scenarios.md). Most of the 160+ tools added on Oct 31 have REST endpoints and entries in the tool manifest but no dedicated block or back-office page yet: a back-office agent shows what such a tool reads in the generic `product_table` and drafts its writes that need a person into a `change_set`. The community, partner, and integration tools are in no agent's `tool_scope` yet, so only the REST API reaches them.

Storefront personalization: what the shopper says is stored as preferences (`update_my_preferences`). Preferences change both the content (ranking, filters) and the form (for example, a comparison table for a shopper who compares specs, a gallery for one who browses by look; the rules are data in `agents/shopper.yml`).

## 7. Proof: AI-friendly vs. screen-shaped

To show that the tool design matters, we build a **screen-shaped facade** over the same back-end that reproduces the patterns in section 2 (generic examples, written by us). Then we run the same task set through both:

| | Screen-shaped facade | A-Commerce tools |
|---|---|---|
| Muse Glimmer | 0% (0 of 36 runs) | 58% (21 of 36 runs) |
| Muse Spark | ? | ? |

The Glimmer row is the design set (12 tasks, 3 runs each) on Oct 31, commit 91970bc, every model call on Glimmer: pass^3 is 0% against 50%. Reports: [../eval/results](../eval/results) (`2026-10-31-glimmer-design-*`). Reruns of single tasks on the same commit moved by one or two runs out of three, so read the numbers as a gap, not a rate. Held-out numbers are not in yet.

Metrics: task completion, tool selection accuracy, calls per task, cost per task, consistency over repeated runs. Tasks are split into a design set (used while building) and a held-out set (scored only at the end). Only held-out numbers go into the README.

## 8. Stack

Decided Oct 30 (details in [architecture.md](architecture.md) and [backend/](backend/README.md)):

- Back-end: Rust (Axum on Tokio), PostgreSQL with pgvector, seed data generated by us from the scenes in [scenarios.md](scenarios.md). The search index has an embedding column, but no embedding model is connected, so search ranks by text.
- Tools: defined once in Rust (`acom-contract`); the REST routes and the agent's tool manifest come from the same definitions (`acom-registry`). The MCP endpoint for outside agents is not built yet (the gateway already routes `/mcp`), and no OpenAPI document is generated.
- Harness: the Rust agent service. Glimmer through a self-hosted, OpenAI-compatible (vLLM) endpoint; Spark through the Meta Model API. No other model is called: image and copy generation are placeholder stubs.
- Gateway: Rust; the only entry point for browsers.
- Front-end: Next.js (TypeScript), titled "Ailism Commerce" with the Ailism theme (`web/src/theme/ailism.ts`). The storefront (`/`) and the back office (`/bo`) are pages with the agent beside them (a full conversation and canvas at `/ask` and on the back-office home, a drawer or dock elsewhere), all rendering the UI blocks ([api/chat.md](api/chat.md)); components from Meta Astryx ([frontend-ui.md](frontend-ui.md)).

## 9. Demo scenarios

- **Back office**: "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page." → search products → preview the VIP sale prices (rule checks: maximum discount, price floor, minimum stock) → preview the coupon and the main-page banner → change set for approval → approve (a person) → run → `verify_change_set` reads it back → change set and action log.
- **Storefront**: a shopper looks for a light jacket in conversation → gallery → shopper asks about material and weight → the view switches to a comparison table → add to cart → checkout card, with the VIP price from the back-office scenario applied.

The two scenarios connect: what the operator sets up is what the shopper sees. Every number in them is in [scenarios.md](scenarios.md) §6 (scenes B1 and F2).
