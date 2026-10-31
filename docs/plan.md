# A-Commerce build plan

Status: rewritten Oct 30, 2026; milestones added Oct 30; status updated Oct 31 (M5 done, every specified operation built). This file says **what we ship and by when**. Why we build it this way is in [design.md](design.md) and [architecture.md](architecture.md); how each part is built is in [backend/](backend/README.md), [backend/database.md](backend/database.md), and [frontend/](frontend/README.md); the operations are in [api/](api/README.md); the demo store, people, and expected results are in [scenarios.md](scenarios.md). Change this file by pull request.

Contents: 1 direction · 2 MVP scope · 3 tools · 4 milestones · 5 work tracks · 6 definition of done · 7 risks · 8 changes

## 1. Direction

| Decision | Choice |
|---|---|
| Server side | **Rust, all of it**: gateway, commerce back end, agent runtime, evaluation runner. No Python stage. |
| Front end | Next.js (App Router, latest stable), `/bo` and `/fo`, types generated from the Rust contract, components from Meta Astryx ([frontend-ui.md](frontend-ui.md)). Every element is an AI target: people can ask about it or ask to change it, and the agent can point at it ([frontend/](frontend/README.md)) |
| Data | PostgreSQL with schema.org table and column names, a comment with the schema.org term on every table and column, one prefixed id, words for states ([backend/database.md](backend/database.md)) |
| Storage | PostgreSQL 17 + pgvector, one database for the MVP |
| Contract | Operations defined once in Rust; the REST routes and the agent's tool manifest are generated from that definition (MCP, OpenAPI and TS types were planned and are not built); the registry test and `cargo xtask check-docs` keep it equal to [api/](api/README.md) |
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
| P2 | Outside agents | MCP with reads, cart, `prepare_order` | Not built: the `mcp` crate is a placeholder (out of scope for the showcase) |

Out of scope: everything outside the store. Payment providers, bank deposits, tax receipts, carriers, outside channels, message delivery, business-registry checks, payouts and image or copy generation are deterministic stand-ins that say so in their output and still go through preview → change set → run. Also out: settlement, taxes, multiple stores in the demo (row-level security is still on). The product is a showcase of AI-native commerce inside the store.

## 3. Tools

Tools are the operations in [api/](api/README.md); bold there marks the MVP set. By priority:

| Priority | Tools |
|---|---|
| P0 operator | `get_policy`, `search_products`, `get_product`, `list_tiers`, `list_active_promotions`, `list_display_slots`, `set_sale_prices`, `create_coupon`, `schedule_banner`, `get_change_set`, `verify_change_set` |
| P0 shopper | `search_products`, `compare_products`, `get_my_profile`, `update_my_preferences`, `add_to_cart`, `get_cart`, `prepare_order` |
| P0 runtime (not model tools) | change sets: create, add items, submit, get; person-only approve, reject, undo; `POST /v1/sales/orders`; memory undo |
| P1 | `search_return_requests`, `get_order`, `evaluate_return_request`, `get_return_history`, `refund_return`, `send_customer_message`, `get_return_options`, `get_my_orders`, `get_product_measurements`, `get_review_summary`; person-only `POST /v1/fulfillment/returns` |
| P2 | `get_delivery_dates`, `prepare_gift_order`, `explain_sales_change`, `list_tasks`, product draft operations |

## 4. Milestones

Each milestone ends with something that can be checked without a person explaining it. Unit and contract tests run in every milestone; **tests that need a running database or several services together (integration tests) run only when asked**. Since Oct 30 every merge to main also deploys to the demo server (`https://a-commerce.ailism.team`, [infra/deploy/](../infra/deploy/README.md)).

Every milestone is held to the same AI-native bar:
- **Back end:** each new operation has a closed schema with a description and example per field, a preview for writes, checks that name the policy key and the fix tool, errors with a class and a next tool, and a schema.org action type.
- **Data:** each new table and column has a schema.org comment.
- **Front end:** each new element registers itself as an AI target (ask about it; suggested intents when it can change).

| # | Days (KST) | Milestone | Deliverables | Done when |
|---|---|---|---|---|
| M0 | Oct 30 | Direction | Docs, contract, scenarios, Rust workspace, CI | Merged (#26) |
| **M1** ✓ | Oct 30 | **Data foundation** | `migrations/` for the P0 and P1 schemas following [backend/database.md](backend/database.md); `cargo xtask check-schema` (rules, generated data dictionary `schema.generated.md` + `schema-map.json`); `acom-core` enums and ids for those tables; `acom-store` (pool, store-scoped transaction with RLS, migrator, `Cx`/`TxCx`); demo seed files `seed/demo/*.yml` from [scenarios.md](scenarios.md) | `check-schema` passes and the dictionary is generated; Rust enums equal the migrations' `CHECK` lists (unit test); seed files cover every number the scenes assert; migrations parse with the PostgreSQL parser |
| **M1b** ✓ | Oct 30 | **Infrastructure base** | Containers for every component (gateway, web, commerce, commerce-worker, agent, migrate, Postgres) from `infra/docker/`; Docker Compose with the production topology (`infra/compose/`); Kubernetes manifests (`infra/k8s/`: base, components, local and production overlays, kind); `acom-server` (health, readiness, metrics, request ids, graceful shutdown); the gateway's streaming proxy; Next.js web on Meta Astryx with the AI layer (`AiProvider`, `useAiTarget`, `PersonButton`, Workspace from the `ai-chat` template); `Makefile` | `make check-infra` renders and validates Compose and both overlays; images build; web type-checks and builds; `astryx doctor` has no failures. Bringing the stack up is an integration check, run on request |
| **M2** ✓ | Oct 30 | **Commerce P0** | Seed generator and loader (`xtask db reset`, from `seed/demo/generate.yml`); API JSON-LD context from the contract types; `acom-http` (registry → routes, manifest, errors); typed policy views and P0 checks; `calculate_benefits`; P0 reads and writes; preview store; change-set engine; person-only order placement and memory undo; `describe_data` | Contract tests per operation; B1 and F2 replay tests written (run on request) |
| **M3** ✓ | Oct 30 | **Gateway and agent** | Demo login, internal tokens, routing, SSE passthrough; model client with guided JSON and argument repair; tool search; planner with slot binding; executor; intent check; blocks with `suggested_intents`; `ui.command`; `agents/*.yml` | Unit tests for parsing, repair, tool search, binding, and blocks against recorded model outputs; the B1 request produces v1 → v2 against a model (run on request) |
| **M4** ✓ | Oct 30 | **Web** | Next.js app with Meta Astryx (pinned versions, our theme, [frontend-ui.md](frontend-ui.md)); AI layer (`AiProvider`, `useAiTarget`, `AskAbout`, `NaturalInput`, `commands`); blocks for P0 (`change_set`, `task_list`, `looking_for`, `compare_table`, `look_gallery`, `product_list`, `cart`, `checkout_card`, `memory_note`); `PersonButton`; JSON-LD on product pages | Component tests on fixtures; every element registers `data-acom-*`; `person` buttons ignore `ui.command` |
| **M5** ✓ | Oct 31 | **P1 and evaluation** | Returns (B2, F5), fit check (F3), layout change (F6), gifts (F4), sales explainer (B3), supplier sheet (B4), task list (B5); facade and task set; then every remaining operation of [api/](api/README.md) | Replay tests for P1; design-set numbers ([design.md](design.md) §7); `check-coverage` and `check-layout` pass in CI |
| M6 | Nov 4–5 | Story | Held-out evaluation, README numbers, video recorded from the live demo or a local run | Submitted by Nov 5, 10:00 KST |

**Where M2–M4 stand (Oct 30).** Built and merged:
- 100 commerce tools (72 reads, 28 writes) and 6 person-only endpoints in 9 domains (platform, catalog, search, promotion, merchandising, sales, customer, fulfillment, messaging), each checked against its `docs/api` section;
- the change-set engine (preview, approve with re-preview and approver roles, run with `$item[n]` binding, verify, undo);
- gateway sign-in (`admin` / `admin123`, demo personas, guests, CSRF, signed internal tokens);
- the agent runtime (model client, tool search, planner, executor, change-set drafting, blocks, SSE);
- the back office (`/bo`) and storefront (`/`) on Astryx.

Unit tests reproduce the B1, B2, F1, F2 and F5 numbers. The SQL paths have database tests that are written but not yet run (`#[ignore]`, integration); they run on request together with bringing the stack up, which is the next check.

**Where it stands (Oct 31, evening).** M5 is done, and the back end is complete for the showcase.
- **Operations:** all 290 tools of [api/](api/README.md) in 13 domains (131 reads, 119 writes, 40 writes that move money or reach customers) and all 18 person-only endpoints are built. Each lives in `crates/domains/{domain}/src/{resource}/{tool}.rs`; CI runs `check-layout` and `check-coverage`, so an operation that is missing or out of place fails the build.
- **Background work:** 3 watchers (price under cost, stock-out forecast, returns over the refund limit) and 8 scheduled jobs (promotion start and end, points expiry, tier evaluation, subscription renewal, campaign sending, seller status changes, held outside events). The gateway hands a watcher's new task to the operations agent in the name of the store's operator (`task_pickup` in `platform.yml`), so a draft waits before anyone opens the task.
- **Scenes:** B1–B5 and F1–F6 run on the demo server (`https://a-commerce.ailism.team`, back office at `/bo`), which deploys every merge to main and reloads the demo store when the seed changes.
- **Evaluation:** design set, 12 tasks × 3 runs on Glimmer: A-Commerce tools 58% (21 of 36), the screen-shaped facade 0% (0 of 36); pass^3 50% against 0% ([design.md](design.md) §7, [eval/results](../eval/results)).
- **Look:** the web app is **Ailism Commerce**, with the Ailism theme, logo and voice ([frontend-ui.md](frontend-ui.md)).

**Open work.** Everything left is inside the store; the list with the eval task behind each item is the TODO section of the [README](../README.md#todo).
- Agent quality on the design set: bind later change-set items to earlier ones' results, look up what a person names instead of asking for its id, say the exact times the results hold.
- The domains added on Oct 31 in the agents (no profile lists `community`, `partner` or `integration` yet) and on screen (UI blocks and back-office pages).
- More watchers (reported posts, seller grades, failed campaigns) and the expiry of unaccepted gifts and unpaid orders (the policies exist; no job applies them).
- Small spec gaps: publishing a post held for review, linking a product to a seller, issuing the codes for withdrawal and sign-in id recovery.
- M6: the held-out run, its numbers in the README, the video.

## 5. Work tracks

Each track has a first deliverable that unblocks others. Owners are set at kickoff; each owner may hand pieces to coding agents following [AGENTS.md](../AGENTS.md).

| Track | Crates and folders | First deliverable | Depends on |
|---|---|---|---|
| **A. Contract and core** | `core`, `contract`, `policy`, `xtask` | Typed policy views for P0 checks; manifest and codegen (`tools.json`, `ui-blocks.schema.json`, TS types) | — |
| **B. Data and seed** | `migrations/`, `store`, `seed/demo/`, `testkit`, `xtask check-schema` | schema.org-named, commented P0/P1 tables + data dictionary + demo seed matching [scenarios.md](scenarios.md) | A (types) |
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
- new tables and columns follow [backend/database.md](backend/database.md) and `cargo xtask check-schema` passes;
- new web elements register as AI targets and use `PersonButton` for any person-only action ([frontend/](frontend/README.md));
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
| Oct 31 | Status: M5 done; every specified operation built in the API hierarchy and held by CI; design-set numbers in; open work moved to the README's TODO. Out of scope restated: everything outside the store is a stand-in (a showcase). |
| Oct 30 | M1b done: infrastructure base ([infra/README.md](../infra/README.md)). |
| Oct 30 | M1 done: 12 migrations (57 tables and one view, every table and column commented with its schema.org or `acom:` term), `check-schema` (rules, data dictionary, generated vocabulary `vocab/acom.jsonld`), `check-seed`, demo seed (812 rows), enums and ids in `acom-core`, principals in `acom-auth`, `acom-store`. Extension terms are schema.org `@type` + `additionalType` ([api/vocabulary.md](api/vocabulary.md)). |
| Oct 30 | Milestones M0–M6 with the AI-native bar for back end, data, and front end; database design ([backend/database.md](backend/database.md)); front-end design ([frontend/](frontend/README.md)). Integration tests run only on request. |
| Oct 30 | Rewritten. The server side is Rust from the start (gateway, commerce, agent), with PostgreSQL and a Next.js front end. Scenario data now comes from the mockups through [scenarios.md](scenarios.md); tool contracts from [api/](api/README.md); UI blocks and the chat stream from [api/chat.md](api/chat.md). The earlier plan (Python back end, flat `policy.yaml`, signatures and seed data in this file) is replaced. |
