# A-Commerce build plan

Status: rewritten Oct 30, 2026. This file says **what we ship and by when**. Why we build it this way is in [design.md](design.md) and [architecture.md](architecture.md); how each part is built is in [backend/](backend/README.md); the operations are in [api/](api/README.md); the demo store, people, and expected results are in [scenarios.md](scenarios.md). Change this file by pull request.

Contents: 1 direction · 2 MVP scope · 3 tools · 4 milestones · 5 work tracks · 6 definition of done · 7 risks · 8 changes

## 1. Direction

| Decision | Choice |
|---|---|
| Server side | **Rust, all of it**: gateway, commerce back end, agent runtime, evaluation runner. No Python stage. |
| Front end | Next.js (App Router, latest stable), `/bo` and `/fo`, types generated from the Rust contract, components from Meta Astryx ([frontend-ui.md](frontend-ui.md)) |
| Storage | PostgreSQL 17 + pgvector, one database for the MVP |
| Contract | Operations defined once in Rust; REST, MCP, the agent's tool manifest, OpenAPI, and TS types are generated from that definition; `cargo xtask check-docs` keeps it equal to [api/](api/README.md) |
| Trust boundary | The agent plans and previews; only `commerce` runs approved change sets; person-only actions need scopes a chat turn never has |
| Models | Muse Glimmer 30B (self-hosted, OpenAI-compatible vLLM) as the worker; Muse Spark (Meta Model API) as the planner and fallback |
| Scenario data | From the mockups in [design/](../design), with disagreements settled in [scenarios.md](scenarios.md) §8 |

## 2. MVP scope

The demo video is the product. Scenes are ranked by whether they carry the story: *one request becomes a reviewed change set, and what the operator approved is exactly what the shopper gets*. Scene ids are from [scenarios.md](scenarios.md) §6.

| Priority | Scene | Shows | Done when |
|---|---|---|---|
| **P0** | B1 VIP weekend sale | Request → change set v1 with 2 failed checks → edit by talking → v2, 6 of 6 checks → person approves → `commerce` runs 3 writes and verifies → live → undo works | Replay test passes; the same request from Glimmer produces v1 and v2 with the expected changes |
| **P0** | F2 Jamie finds and buys | Compare by packed size → remembered preference → add to cart → $71.20 with the VIP price from B1 → `prepare_order` → Jamie places the order | Replay test passes; the conversation from Glimmer reaches the checkout card |
| **P0** | F1 two shoppers | Same question, `compare_table` for Jamie, `look_gallery` for Mina, same prices and stock | Both layouts from one runtime |
| **P0** | Evaluation, small | Same tasks through A-Commerce tools and the screen-shaped facade, Glimmer | Numbers for the design set; held-out numbers on Nov 4 |
| P1 | B2 returns over $200 | 12 requests sorted by policy: 9 refunds, 2 photo requests, 1 decline; one change set with 9 refunds and 3 messages | Replay test; Glimmer produces the same split |
| P1 | F5 Alex's defective boots | `return_options`, refund over $200 goes to a person, request sent by Alex, appears in B2 | Replay test |
| P1 | F6 Mina changes the layout | "Less text, bigger photos" → `look_gallery` + one undoable memory line | From Glimmer |
| P1 | F3 fit check | Coat M vs L against the windbreaker Jamie owns | From Glimmer |
| P2 | F4 gift for Dad | `gift_picker`, nothing saved to the profile, $76.20 | |
| P2 | B3 sales question | `sales_explainer`, read only | |
| P2 | B4 supplier sheet, B5 home tasks | `product_drafts`, `task_list` | |
| P2 | Outside agents | MCP with reads, cart, `prepare_order` | One outside client finds a product and prepares an order |

Out of scope: real payments and carriers (stubs that succeed), settlement, sellers, taxes, multiple stores in the demo (row-level security is still on).

## 3. Tools

Tools are the operations in [api/](api/README.md); bold there marks the MVP set. By priority:

| Priority | Tools |
|---|---|
| P0 operator | `get_policy`, `search_products`, `get_product`, `list_tiers`, `list_active_promotions`, `list_display_slots`, `set_sale_prices`, `create_coupon`, `schedule_banner`, `get_change_set`, `verify_change_set` |
| P0 shopper | `search_products`, `compare_products`, `get_my_profile`, `update_my_preferences`, `add_to_cart`, `get_cart`, `prepare_order` |
| P0 runtime (not model tools) | change sets: create, add items, submit, get; person-only approve, reject, undo; `POST /v1/sales/orders`; memory undo |
| P1 | `search_return_requests`, `get_order`, `evaluate_return_request`, `get_return_history`, `refund_return`, `send_customer_message`, `get_return_options`, `get_my_orders`, `get_product_measurements`, `get_review_summary`; person-only `POST /v1/fulfillment/returns` |
| P2 | `get_delivery_dates`, `prepare_gift_order`, `explain_sales_change`, `list_tasks`, product draft operations |

## 4. Milestones (Korea time)

Submission closes at the end of Nov 4 (US Eastern). Our internal deadline is **Thu Nov 5, 10:00 KST**.

| Day | Goal | Demo-able at the end of the day |
|---|---|---|
| Fri Oct 30 | Direction, docs, and the Rust workspace merged (this plan) | `cargo test` green; `cargo xtask check-docs` runs |
| Sat Oct 31 | `core`/`contract`/`policy` complete for P0; `store` with migrations for P0 tables; demo seed loads; commerce serves the P0 reads; gateway demo login and internal tokens; agent calls Glimmer with a closed schema | `GET /api/v1/search/products?...` through the gateway returns the 42 summer outerwear products |
| Sun Nov 1 | P0 writes with preview; change-set engine (draft, submit, approve, run, verify, undo); cart, `prepare_order`, place order | B1 and F2 replay tests pass without a model |
| Mon Nov 2 | Agent runtime: router, tool search, planner, executor, intent check, UI blocks, SSE; web renders `change_set`, `compare_table`, `look_gallery`, `cart`, `checkout_card` | B1 and F2 from a real model in the browser |
| Tue Nov 3 | P1 scenes; facade and evaluation on the design set; deploy the demo | Numbers for the design set; demo URL |
| Wed Nov 4 | Held-out evaluation, README numbers, video, buffer | Video recorded |
| Thu Nov 5 | Submit by 10:00 | |

## 5. Work tracks

Each track has a first deliverable that unblocks others. Owners are set at kickoff; each owner may hand pieces to coding agents following [AGENTS.md](../AGENTS.md).

| Track | Crates and folders | First deliverable | Depends on |
|---|---|---|---|
| **A. Contract and core** | `core`, `contract`, `policy`, `xtask` | Typed policy views for P0 checks; manifest and codegen (`tools.json`, `ui-blocks.schema.json`, TS types) | — |
| **B. Store and seed** | `store`, `migrations/`, `seed/demo/`, `testkit` | P0 tables + demo seed matching [scenarios.md](scenarios.md) | A (types) |
| **C. Commerce domains** | `domains/*`, `registry`, `http`, `services/commerce` | P0 reads, then P0 writes with checks | A, B |
| **D. Change sets** | `changeset` | Preview store; approve → run → verify → undo for B1 | A, B |
| **E. Gateway and auth** | `auth`, `services/gateway` | Demo login, internal tokens, routing, SSE passthrough | — |
| **F. Agent runtime** | `llm`, `agent`, `agents/`, `services/agent` | Glimmer call with guided JSON; tool search over the manifest; B1 plan | A (manifest), C |
| **G. Web** | `web/` | Blocks from fixtures ([api/chat.md](api/chat.md) §3), then live SSE | A (TS types) |
| **H. Evaluation** | `facade`, `eval/` | Facade for the P0 tools; 10 design tasks | C, F |
| **I. Story** | README, video, post | Demo script from [scenarios.md](scenarios.md) | all |

Web and evaluation start from fixtures and the manifest, so nobody waits for the agent.

## 6. Definition of done

A piece of work is done when:

- its operations exist in [api/](api/README.md) with the same name, address, kind, and REST path, and `cargo xtask check-docs` passes;
- input schemas are closed, every field has a description and an example, and business numbers come from `policies/` or the store's overrides;
- contract tests cover the output shape and each listed error; writes prove that preview changes nothing and that running a `preview_id` twice gives one result;
- the scenes it touches pass their replay tests against the numbers in [scenarios.md](scenarios.md);
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` pass.

## 7. Risks

| Risk | Mitigation |
|---|---|
| Six days for an all-Rust stack | P0 is three scenes; replay tests before models; P1/P2 are cut first. Scaffolding, contract, and policy loader are already in place |
| Glimmer plans badly | Few closed tools per turn, ids bound by slot, guided decoding, argument repair; Spark takes over after two failed checks; every failure is in the turn log |
| Model endpoint or credits unavailable | Provider is configuration; any OpenAI-compatible endpoint works; replay tests and the web demo do not need a model |
| Spec and code drift | `check-docs` in CI; snapshot tests; spec changes in the same PR as code |
| Scenario numbers drift | One source ([scenarios.md](scenarios.md)); seed and tests read the same values |

## 8. Changes

| Date | Change |
|---|---|
| Oct 30 | Rewritten. The server side is Rust from the start (gateway, commerce, agent), with PostgreSQL and a Next.js front end. Scenario data now comes from the mockups through [scenarios.md](scenarios.md); tool contracts from [api/](api/README.md); UI blocks and the chat stream from [api/chat.md](api/chat.md). The earlier plan (Python back end, flat `policy.yaml`, signatures and seed data in this file) is replaced. |
