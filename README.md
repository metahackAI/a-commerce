# A-Commerce

**AI-native commerce, built on Meta models.** The live showcase is **Ailism Commerce**: <https://a-commerce.ailism.team>.

> Built during the [Meta Global AI Developer Hackathon](https://cerebralvalley.ai/e/meta-ai-developer-hackathon) (Oct 27 – Nov 4, 2026).
> Every line of code in this repository was written during the hackathon. See [What we built during the hackathon](#what-we-built-during-the-hackathon).

- **Track:** Indie Developer
- **Problem statement:** Build for Your World

---

## Try it

| | |
|---|---|
| Storefront | <https://a-commerce.ailism.team> (sign in as Jamie (VIP), Alex (member), or continue as a guest) |
| Back office | <https://a-commerce.ailism.team/bo> (`admin` / `admin123`, or the scenario's operator Son SeongJun) |
| Store clock | Fixed at **Fri Nov 6 2026, 11:00 New York**, so every scene below gives the same numbers. An admin can reset the store from the back office. |

What to ask (the scenes of [docs/scenarios.md](docs/scenarios.md) §6, all on the demo store). The agent runs on a 30B model, so wording and sometimes the result differ from run to run; [Results](#results) says how often a task gets done.

| Where | Ask | What you should see |
|---|---|---|
| Back office | *Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page.* | A change set with the VIP sale prices and a main-page banner for the sale, checked against the store's rules: the overshirt would go under its price floor, and two products are too low on stock for a sale. |
| Back office | *Drop the low-stock ones. Give the overshirt 15% instead.* | Version 2 of the same change set, revised by talking, with the checks run again. Nothing changes until a person approves it. |
| Back office | *Make a collection page called Cozy knits with our knitwear and scarves, and generate a picture for the top of the page.* | A draft page with a banner Muse Image drew at preview, so you see it before approving. After approval, *Publish col_cozy_knits from now until Nov 30.* puts it on the storefront at `/collections/col_cozy_knits`. |
| Back office | *what needs my attention today?* | The task list: the Linen shirt reorder a watcher drafted before anyone asked, the returns review, and what finished this morning. |
| Back office | *go through the return requests over $200 and tell me which ones to refund* | 12 requests sorted by the return policy: 9 refunds ($2,463.00), 2 photo requests, 1 decline. The refunds wait in a change set for a person. |
| Back office | *why did sales drop yesterday?* | A sales explainer: the gap against a normal Thursday, the sizes that sold out, the weekly email that did not go out. |
| Storefront | *I need a light jacket for spring trips.* | Jamie, who compares specs, gets a comparison table. Ask *Less text. Just show me bigger photos.* and the same results come back as a gallery, remembered for next time. |
| Storefront | *Which one packs smallest?* → *The windbreaker. Same size as last time.* | A comparison by packed size and a remembered preference she can undo, then her usual size in the cart and a checkout card only she can press. |
| Storefront | *Will M fit in the wool overcoat? I want to wear it over a hoodie.* | A fit check against the windbreaker Jamie owns: L over a hoodie, M over a sweater. |

## Why

Commerce back ends are big, and most of them grew over years. Products, options, prices, stock, display categories, promotions, coupons, orders, claims, members: each is its own service with its own screens. A single business task such as *"run a 20% weekend sale on summer outerwear for VIP members"* means going through many back-office screens and APIs in the right order.

Handing those APIs to an LLM as they are does not work well. There are hundreds of endpoints, legacy names, and ordering rules that only people know. Large models struggle; small models fail. So most "AI commerce" today is a chatbot that answers questions next to screens that people still operate. Shoppers get the same treatment: the storefront is the same set of pages for everyone, and personalization means a recommendation carousel.

## What A-Commerce does

**A-Commerce turns a commerce back end into tools that an AI agent can find, chain, and run, so that operators and shoppers just talk, and the right UI shows up when it is needed.**

1. **Say it.** An operator (back office) or a shopper (storefront) asks in natural language.
2. **The harness does the work.** It finds the right commerce tools, plans how they connect (find products → set sale prices → issue a coupon → schedule the banner), previews every write, and puts the changes in one change set. People step in only where the store's rules say so: prices under the floor, publishing, refunds over a limit.
3. **The answer is a UI, not a paragraph.** Results come back as blocks picked for the task and the person: a change set to review, a refund triage, a comparison, a checkout card.

## AI-native, not AI-added

**People state the goal and approve the result; the system does the rest. Intelligence comes from the model; trust comes from the structure.** That is why a small open-weight model can do routine work here. Five principles, each with the mechanism that holds it in the code:

![AI-added, agent-ready, AI-native: A-Commerce rebuilds the store under the agent. Web: the screen is the result of the work. Agent: find, plan, run, check, on Muse Glimmer 30B. Commerce: tools in the store's words, every change a proposal, rules as data, and the store brings the work.](docs/img/ai-native-architecture.png)

Most AI commerce puts a chatbot on screens built for people (AI-added) or opens the old APIs to outside agents (agent-ready). A-Commerce rebuilds what is underneath, so the agent works the store through tools made for it ([docs/research.md](docs/research.md) §2). The picture is rendered from the demo video's frame 13 ([design/demo](design/demo/README.md)).

| Principle | How the code holds it |
|---|---|
| **1. Tools work in the units people use.** Find by an order number, an email, a product name; get back what the decision needs. | Id fields also take the order number, change set number or email. Reads return the fields a decision needs and refuse a field name that does not exist instead of returning an empty object. |
| **2. Every change is a proposal first.** The agent drafts a change set; a person approves it where the rules require, like a pull request. | Every write has a preview. Change sets are approved, re-previewed, run, verified and undone by the commerce service (`crates/changeset`). The agent can draft but never approve or run; person-only actions (placing an order, approving) need scopes a chat turn never has. |
| **3. Rules are data, and the system checks them.** | Discount limits, price floors, approval thresholds and return windows live in [`policies/*.yml`](policies/README.md). The back end checks them and names the rule and the tool that fixes it, so a wrong model still cannot break a rule. |
| **4. The system brings the work.** | Watchers (price under cost, stock-out forecast, returns waiting for a refund decision) open tasks with a draft change set, and the agent picks the task up before anyone asks. **Ailism Bot**, the back office's manager AI, reads the whole store on its schedule (or on *Run now*), proposes what to fix across eight areas, and opens each proposal as a task the operations agent drafts — with its reason, evidence and expected effect, a history of every cycle, *ask again* with a correction, and feedback it reads in every analysis. It never approves. |
| **5. The screen is the result of the work.** | 19 UI blocks (7 back office, 12 storefront) chosen per task and per person; every element on the page is an AI target people can ask about. |

Each principle has a sign that it is broken; the full text is in [docs/design.md § Principles](docs/design.md#principles).

## Results

We measure on a fixed set of operator tasks collected from public material (what merchandisers, marketers, operators and owners actually ask), not on hand-picked demos. The same tasks run through three tool layers over the **same back end and the same model**:

- the A-Commerce tools;
- a *screen-shaped facade* that exposes the back end the way admin screens do: one endpoint per screen, codes instead of words, writes that run at once;
- the same screen API converted into plain-named tools by our [a2api](https://github.com/metahackAI/a2api).

Method: [eval/README.md](eval/README.md).

| Tasks done, Muse Glimmer 30B, Nov 3 | Screen-shaped facade | a2api over the facade | A-Commerce tools |
|---|---|---|---|
| Held-out set (9 tasks × 10 runs) | 0% (0 of 90) | 33% (30 of 90) | **44%** (40 of 90) |
| Fresh set (8 tasks × 5 runs) | 0% (0 of 40) | 12% (5 of 40) | **50%** (20 of 40) |
| Customer message sent without a person's approval (held-out) | 0 of 10 (never got that far) | 10 of 10 | **0 of 10** (drafted for approval) |

- **Counted by reading every reply.** Done means the answer is right and the reply commits to it. "I can't confirm from this page", or a "none" with no data read behind it, is not done. A write must wait for approval. The runner's automatic score misses some right answers and passes some hedged ones; it gives 0% / 11% / 22% on held-out and 12% / 25% / 75% on fresh. The rules, every held-out task and both scores: [eval/results/2026-11-03-glimmer-judged.md](eval/results/2026-11-03-glimmer-judged.md).
- **The fresh set is the cleaner number.** It was written on Nov 2 against the code we measured and shown to no one building the agent. The held-out tasks are in this repository and were run on Nov 1 to check whether design-set changes carry over.
- **On held-out, a2api and A-Commerce answered the same three read tasks; the writes differ.** The screen API runs writes at once, so through it the photo request to a customer went out, and the knit sale was paused, with no approval in all 10 runs each. A-Commerce drafted it as a change set waiting for a person, all 10 times (principle 2).
- **About half of the asks are still not done.** The agent answers from the first page of a long list, or describes a change wrongly ("paused until Monday", when a pause has no end).

## How we use Meta models

| Model | Where it runs | What it does |
|---|---|---|
| **Muse Glimmer 30B** | Open weights, self-hosted: NVIDIA's 4-bit build on vLLM, one RTX 5090. 72 output tokens/s for one request, 0.1 s to the first token when the prompt's start is cached. A crash on this GPU traced to cuDNN and fixed ([docs/serving.md](docs/serving.md)) | Every agent's worker: routing, tool search, planning, arguments, replies. On the live demo it handles every call. |
| **Muse Spark** | Meta Model API | Wired as every agent's planner and fallback ([agents/models.yml](agents/models.yml)); used when `META_API_KEY` and `SPARK_MODEL` are set. |
| **Muse Voice Transcribe** | Meta Model API | The composer's microphone: speech becomes the draft message, with the names on the screen (products, sizes) as spelling hints ([docs/api/chat.md](docs/api/chat.md) §4); used when `META_API_KEY` and `VOICE_MODEL` are set. |
| **Muse Image** | Meta Model API | The photos of all 400 demo products, made offline with each response kept as provenance ([seed/demo/catalog_content.yml](seed/demo/catalog_content.yml)). In the product: `generate_page_image` draws the picture across the top of a curated page at preview, so the person approving sees it; shoppers see it only after approval ([docs/api/catalog.md](docs/api/catalog.md) page_image). Used when `META_API_KEY` and `IMAGE_MODEL` are set; otherwise a placeholder that says so. |

**Why Meta models matter here:** open weights let a merchant run the agent on its own infrastructure, keeping catalog and customer data in-house, and a small model keeps the cost per request low enough to put an agent behind every request. The structure (closed tools, previews, policy checks, a reply that may only use numbers found in the tool results) is what makes a 30B model dependable.

## What is in the box

| | |
|---|---|
| Commerce operations | **291 tools in 13 domains** (132 reads, 119 writes, 40 writes that move money or reach customers) and **18 person-only endpoints**. Every one is specified in [docs/api/](docs/api/README.md) and built; CI fails if one is missing or outside its folder. |
| Domains | platform · catalog · customer · promotion · merchandising · search · sales · fulfillment · messaging · analytics · community · partner · integration |
| Agents | `promotion`, `orders`, `catalog`, `insights` (back office), `shopper` (storefront), defined in [agents/](agents) |
| Background work | 3 watchers and 8 scheduled jobs (promotion start and end, points expiry, tier evaluation, subscription renewal, campaign sending, seller status changes, held events) |
| Data | PostgreSQL, 135 tables and views named after schema.org, a comment with its term on every table and column ([data dictionary](docs/backend/schema.generated.md)); a demo store of about 128,000 rows (Northwind Apparel: 14,000 members, 400 products, five weeks of daily orders) |

### A showcase, on purpose

This is a showcase of AI-native commerce, not a store that ships parcels. What runs for real is everything inside the store: the tools, change sets, approvals, policy checks, watchers, agents and UI. Everything **outside** the store is a deterministic stand-in that says so in its output: payment providers and bank deposits, tax receipts, carriers, outside sales channels, email/SMS/push delivery, business-registry checks, payouts, and image or copy generation. Those stand-ins still go through preview → change set → run, so the agent's side of the work is the same as it would be against real providers.

## Architecture

![Figure 1. Context construction (agent profile, step prompts, conversation memory, tool search) feeds Muse Glimmer 30B. Harness guards (plan check and binder, reply check, route hints, limits) sit between the model and commerce. Commerce has a tool interface (291 tools, human handles, previews) and trust (change sets, policies, person-only actions). The store brings work: watchers, store events, task pickup, which starts the next turn.](docs/img/fig-harness-components.png)

**Figure 1. The harness around one open-weights model.** Context construction (C1–C4) decides what Muse Glimmer sees. Harness guards (H1–H4) are code the model cannot skip. Commerce is a separate service rebuilt for the agent: tools in the store's words, a preview before every write, and change sets only a person approves (T1–R3). The store also brings work (S1–S3): a watcher's task becomes the agent's next turn. Each card names the file it lives in.

![Figure 2. The turn loop: request, route, tool search, plan, execute, check, reply, with a revise arrow from check back to plan. Writes go to the store loop: change set, person approves, run and read back, watchers open a task, and task pickup brings the task back as the next request.](docs/img/fig-two-loops.png)

**Figure 2. One request, two loops.** The turn loop runs route → tool search → plan → execute → check → reply, and plans again up to twice with the error and the steps not run yet. Tool search and check make no model call; route and execute call it only sometimes. The store loop runs what a person approved and reads it back, and its watchers open the next task. Both figures are drawn from `docs/figures/` (`python docs/figures/render.py`).

The services:

```
        Shopper (storefront)                          Operator (back office)
                 \                                          /
                  v                                        v
        +------------------------------------------------------------------------+
        |  gateway   sign-in, sessions, CSRF, internal tokens, routing, SSE        |
        +---------+--------------------------+-------------------------+---------+
                  |                          |                         |
                  v                          v                         v
        +------------------+      +----------------------+    +--------------------+
        |  web (Next.js,   |      |  agent (Rust)        |    |  commerce (Rust)    |
        |  Meta Astryx)    |      |  router · tool search|--->|  291 operations     |
        |  blocks, AI      |      |  planner · executor  |    |  previews, checks   |
        |  targets, person |      |  reply checks · log  |    |  change sets        |
        |  buttons         |      +----------+-----------+    |  policies           |
        +------------------+                 |                +----------+---------+
                                             v                           |
                                   Muse Glimmer / Muse Spark   commerce-worker: watchers, jobs
                                                                         |
                                                                         v
                                                      PostgreSQL 17 + pgvector (row-level security)
```

The agent never touches the database: it reads the tool manifest and calls commerce over HTTP like any client, and commerce enforces scopes, policies and approvals. Services, trust boundaries and the API hierarchy: [docs/architecture.md](docs/architecture.md).

## Repository layout

```
crates/
  core, contract, policy      types, the operation contract, policy loading (no I/O)
  store, changeset            database access, previews, change sets, jobs
  domains/{domain}/src/{resource}/{tool}.rs   one file per operation, in the API hierarchy
  registry, http              every operation → REST routes and the tool manifest (mcp: a placeholder)
  auth, server, llm, agent    principals and scopes, service runtime, model client, agent runtime
  facade, testkit             the evaluation's screen-shaped tools, seed generator and loader
services/   gateway, commerce, agent
web/        Next.js storefront and back office on Meta Astryx
agents/     agent profiles, prompts, models          policies/   the store's rules as data
migrations/ schema.org tables                         seed/demo/  the demo store
eval/       task sets and results                     infra/      containers, Compose, Kubernetes, deploy
docs/       design, architecture, API, back end, front end, scenarios
xtask/      checks (docs, schema, seed, layout, coverage) and the evaluation runner
```

## How to read the docs

Start with the row that matches what you are about to do. Each document links to the next one it depends on.

| If you want to… | Read, in this order |
|---|---|
| Understand the product | This README → [docs/design.md](docs/design.md) (principles, tool rules, harness, UI protocol, results) → [docs/research.md](docs/research.md) → mockups in [design/](design) |
| See the whole system | [docs/architecture.md](docs/architecture.md) (services and trust boundaries, API hierarchy, schema.org data model, AI-native features) |
| Know what was planned and where it stands | [docs/plan.md](docs/plan.md) (scope, milestones, status, open work) |
| Know the demo store and scenes | [docs/scenarios.md](docs/scenarios.md) (people, products, policies, each scene with expected results) |
| Build back-end code | [AGENTS.md](AGENTS.md) → [docs/backend/README.md](docs/backend/README.md) (workspace, crates, dependency rules, folder layout) → [docs/backend/database.md](docs/backend/database.md) → [docs/backend/commerce.md](docs/backend/commerce.md) → [docs/backend/agent.md](docs/backend/agent.md) → [docs/backend/gateway.md](docs/backend/gateway.md) → [docs/backend/testing.md](docs/backend/testing.md) |
| Add or change an operation (tool) | [docs/api/README.md](docs/api/README.md) (hierarchy, kinds, index) → [docs/api/conventions.md](docs/api/conventions.md) → [docs/api/vocabulary.md](docs/api/vocabulary.md) → the domain file (for example [docs/api/promotion.md](docs/api/promotion.md)) → [docs/api/enums.md](docs/api/enums.md) and [docs/api/errors.md](docs/api/errors.md) → [policies/](policies/README.md) |
| Build front-end code | [docs/frontend-ui.md](docs/frontend-ui.md) (Meta Astryx, the Ailism theme, look rules) → [docs/frontend/README.md](docs/frontend/README.md) (AI layer, elements, person buttons) → [docs/api/chat.md](docs/api/chat.md) (stream and blocks) → [design/](design) |
| Measure the agent | [eval/README.md](eval/README.md) (task format, sources, how to run) → [docs/backend/testing.md](docs/backend/testing.md) |
| Run, package, or deploy | [infra/README.md](infra/README.md) (serving architecture, images, Compose, Kubernetes) → [infra/deploy/README.md](infra/deploy/README.md) (the demo server) |
| Serve the model | [docs/serving.md](docs/serving.md) (Glimmer's image and flags, speed, the RTX 5090 crash and its fix, scripts to measure again) |
| Look up a table, column, or term | [docs/backend/schema.generated.md](docs/backend/schema.generated.md) (generated from the migrations), [vocab/acom.jsonld](vocab/acom.jsonld) (every `acom:` term), or the `describe_data` tool |
| Load or change demo data | [docs/scenarios.md](docs/scenarios.md) → [seed/demo/README.md](seed/demo/README.md) |

Specifications that implementers read daily (`docs/api/`, `docs/backend/`, `docs/frontend/`, `docs/scenarios.md`, `eval/`) are written in Korean; the overview documents are in English.

## Getting started

Everything runs in containers with the same layout as production; only configuration differs (where the database lives, how many replicas, where secrets come from). Details: [infra/README.md](infra/README.md).

### Requirements

- Docker with Compose v2 (the whole stack)
- GNU Make
- For development outside containers: Rust (the version in `rust-toolchain.toml`; `rustup` installs it) and Node.js 24
- For local Kubernetes (optional): kind and kubectl

### Run the whole stack locally

```bash
make up          # creates .env and local keys if missing, builds the images, runs migrations,
                 # loads the demo store once, starts everything
                 # → http://localhost:8080   storefront /   back office /bo
make ps          # containers and health
make logs SERVICE=gateway
make reseed      # empty the store and load the demo store again
make down        # stop (make clean also deletes the database volume)
```

Sign in to the back office as **`admin` / `admin123`** (a demo account with every role). In demo mode the sign-in pages also offer the scenario's people: operator Son SeongJun (`adm_sj`), shoppers Jamie (VIP) and Alex (member), or a guest. These accounts and the local keys are for a demo only.

Add model access in `.env` (`GLIMMER_BASE_URL`, `GLIMMER_MODEL`; optionally `META_API_KEY`, `META_API_BASE_URL`, `SPARK_MODEL`) to use real models. Without a model the pages still work and the assistant says plainly that its model is not answering. `make up-models` serves the worker model on a local GPU, and `make up-observability` adds Prometheus and Grafana. `make help` lists every command.

### Develop one service outside containers

```bash
make up-debug                         # the stack, plus Postgres on :5432 and each service on its own port
cargo run -p acom-commerce -- serve   # replace the container you are working on (stop that container first)
cd web && npm install && npm run dev  # web on :3000
```

### Checks

```bash
make check       # Rust (fmt, clippy, tests, docs/schema/seed/layout/coverage checks), web (types, build), infra (Compose, Kubernetes)
cargo xtask check-coverage            # every specified operation is built
cargo xtask check-layout              # every operation lives in {domain}/src/{resource}/{tool}.rs
cargo xtask eval --base http://localhost:8080 --set design --tool-set both   # the agent task set against a running stack
```

### Environment variables

`.env.example` lists every variable with a comment. The ones you set yourself:

| Name | Description |
|---|---|
| `GLIMMER_BASE_URL`, `GLIMMER_MODEL` | OpenAI-compatible endpoint serving the worker model |
| `META_API_KEY`, `META_API_BASE_URL`, `SPARK_MODEL` | Meta Model API for the planner model (optional; never commit the key) |
| `VOICE_MODEL`, `VOICE_LANGUAGES` | Speech to text for the composer's microphone through the same API (optional) |
| `ACOM_HTTP_PORT` | Host port of the gateway (default 8080) |

## TODO

The back end is complete for the showcase: every specified operation is built. What remains is about the AI-native experience inside the store, not about connecting outside services.

**Agent quality** (measured by the design set; each item names the task that shows it)
- [x] Bind later items of a change set to what earlier items produce (`$item[n]`), instead of an invented id: the B1 banner points at `$item[1].promotion_id`, a curated page at the picture drafted before it. Still open: publishing a page in the same change set that creates it (`publish_curation` checks the id before the page exists).
- [ ] Look up what the person named instead of asking for its id: "markdown on tees" should find the tees category, not ask for `category_id` (`md-end-of-season-markdown`).
- [ ] Re-run `b2-returns-over-200`: the agent guessed a customer id to message about an order; `send_customer_message` now takes the order alone (Oct 31), which should remove the guess.
- [ ] Say the exact times and amounts the results hold (an undo deadline of Nov 7 23:59, a banner verified at 09:36) (`mgr-what-needs-me-today`, `mgr-what-changed-this-morning`).
- [ ] Read a long list to its end, or filter it, before saying "none" or "these ones". Seen on held-out and fresh (answers from the first 100 of 400 products or 412 orders), so a fix has to be checked on the design set and a new held-out set.
- [x] Run the held-out set and put its numbers in [Results](#results) (Nov 3). Still to do: compare Glimmer with Spark on the same tasks, and fix the runner problems listed in [the Nov 3 judging](eval/results/2026-11-03-glimmer-judged.md#runner-problems-found-here).

**The new domains in the agents and on screen**
- [x] Give the agents the domains added on Oct 31: `community` is in the catalog agent, `partner` and `integration` in the new marketplace agent.
- [ ] The checkout card built from a `prepare_subscription` preview posts to the order endpoint; it should start the subscription (`POST /v1/sales/subscriptions`).
- [ ] UI blocks and back-office pages for the tools added on Oct 31 (campaigns, community moderation, sellers, channels and imports, subscriptions, receipts, inquiries, remedies). Today they answer in text and tables only (principle 5).

**The system brings more of the work**
- [ ] Watchers for the new domains, each opening a task with a draft: reported community posts over the threshold, sellers whose grade drops, a scheduled campaign that failed.
- [ ] Expire gifts nobody accepted and orders never paid. The policies exist; no job applies them yet. It needs a standing approval for a system cancel.

**Spec gaps found while building** (internal; small)
- [ ] An operation that publishes a community post or reply held for review (today the store can only hide or delete it).
- [ ] A tool that links a product to a seller (today only the seed does).
- [ ] Endpoints that issue the code for re-authenticating a withdrawal and for sign-in id recovery (the flows accept such codes; nothing issues them yet).
- [ ] Tier evaluation reads each member's orders one by one; a bulk reader from sales would make an all-member run fast.

**Story**
- [ ] Demo video (3 min).

Out of scope for the showcase: real payment, carrier, channel and messaging providers; a secret store for channel signing secrets; outside agents (the MCP endpoint is a placeholder crate, and the gateway does not yet sign outside clients in); settlement and taxes.

## What we built during the hackathon

Everything in this repository was created between Oct 27 and Nov 4, 2026 (dates in KST). The commit history is the record.

| Date | What was built |
|---|---|
| Oct 27 | Repository, problem and principles, the design document and coding-agent rules, research on AI-native commerce, UI mockups for the back office and the storefront |
| Oct 29 | Build plan: MVP scope, tool signatures, seed data, UI block schema, roles and schedule |
| Oct 30 | Architecture (services, trust boundaries, API hierarchy, schema.org data model) and the API specification for 13 domains. Direction: the whole server side in Rust. Data foundation: schema.org migrations with a commented term on every column, the data dictionary, the demo seed. Infrastructure: every component in a container, Compose with the production topology, Kubernetes manifests, the demo server deploying every merge |
| Oct 30 | M2–M4: 100 commerce tools in 9 domains, the change-set engine (preview, approve, run, verify, undo), sign-in with demo personas, the agent runtime (model client, tool search, planner, executor, blocks, SSE), and the back office and storefront on Meta Astryx |
| Oct 31 | M5: the remaining scenes (returns triage, sales explainer, supplier sheet, fit check, gifts, layout by profile, the task list), scheduled jobs and the demo clock; the evaluation task set, the screen-shaped facade, `cargo xtask eval`, and the first numbers; ids that take the order number, change set number or email |
| Oct 31 | The rest of the back end: every specified operation (291 tools) and 18 person-only endpoints in 13 domains, each in `{domain}/src/{resource}/{tool}.rs`, held by CI. Watchers' tasks picked up by the agent. Reply checks (times, weekdays, copied instructions). The **Ailism Commerce** look, name and voice |

## References

The commerce domain model (products, orders, members, display, promotions) is informed by our team's experience building enterprise commerce platforms. No code, schemas, or data from any existing product are included; all of it was written from scratch for this project. The evaluation tasks cite the public sources they come from ([eval/README.md](eval/README.md) §2).

## Team

| Name | GitHub |
|---|---|
| Son SeongJun | [@SonAIengine](https://github.com/SonAIengine) |
| Ha-Ryeom Jang | [@CocoRoF](https://github.com/CocoRoF) |
| Haesoo Kim | [@haesookimDev](https://github.com/haesookimDev) |
| DongUk Kim | [@Createyouracccount](https://github.com/Createyouracccount) |

## License

[Apache License 2.0](LICENSE)
