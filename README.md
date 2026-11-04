# A-Commerce

**An online store you run by talking.** For owners of small stores: the agent does the routine work, you approve it. AI-native commerce, built on Meta models, with Muse Glimmer 30B, open weights, on one GPU. The live showcase is **Ailism Commerce**: <https://a-commerce.ailism.team>.

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
| Store clock | Stands at **Fri Nov 6 2026, 11:00 New York**, so every scene below gives the same numbers. An admin moves it with *Move time* at the foot of the back office's menu (jobs that come due run on the way), and *Start the demo over* loads the store again. The live demo is shared, so someone else's approval, *Move time* or *Start the demo over* can change the numbers. |

What to ask (the scenes of [docs/scenarios.md](docs/scenarios.md) §6, all on the demo store). The agent runs on a 30B model, so wording and sometimes the result differ from run to run; [Results](#results) says how often a task gets done.

| Where | Ask | What you should see |
|---|---|---|
| Back office | *Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page.* | A change set with the VIP sale prices and a main-page banner for the sale, checked against the store's rules: the overshirt would go under its price floor ($63.20 against $66.70), and two products are too low on stock for a sale. |
| Back office | *Drop the low-stock ones. Give the overshirt 15% instead.* | A new version of the same change set, revised by talking, with the checks run again: the overshirt at 15% ($67.15), the two low-stock products left out. Nothing changes until a person approves it; after that it can be undone until Mon Nov 9, 23:59. |
| Back office | *Make a collection page called Cozy knits with our knitwear, and generate a picture for the top of the page.* | A draft page with a section of knitwear (two products in our runs) and a banner Muse Image drew at preview, so you see it before approving. After approval, *Publish col_cozy_knits from now until Nov 30.* puts it on the storefront at `/collections/col_cozy_knits`. |
| Back office | *what needs my attention today?* | The task list: three decisions (the Linen shirt reorder a watcher drafted before anyone asked, and Ailism Bot's reorder of the Light rain parka and its answer to a question about the windbreaker's hood), the returns review and the size-chart check at work, and what finished this morning. |
| Back office | *go through the return requests over $200 and tell me which ones to refund* | 12 requests sorted by the return policy: 9 refunds ($2,463.00), 2 photo requests, 1 decline. Press *Draft 9 refunds and 3 messages* and they go into a change set that waits for a person. |
| Back office | *why did sales drop yesterday?* | A sales explainer: the gap against a normal Thursday, the sizes that sold out, the weekly email that did not go out. |
| Storefront | *I need a light jacket for spring trips.* | Jamie, who compares specs, usually gets a comparison table; the video's run drew a list (6 of 20). Ask *Less text. Just show me bigger photos.* and the results come back as a gallery, remembered for next time; *Compare them* brings the table back. |
| Storefront | *The windbreaker. Same size as last time.* → *Add the black one in M to my cart.* → *Check out with my usual card.* | The product with Jamie's last purchase (M, Black, order #20268), then the cart, then a checkout card only Jamie can press. With the sale live (approve it, then move the clock to Saturday) the back end prices the windbreaker at $71.20 instead of $89.00. |
| Storefront | *Will M fit in the wool overcoat? I want to wear it over a hoodie.* | A fit check from the measurements and reviews: 9 of the 41 reviews say to size up for a hoodie. |
| Back office | *What did Jamie just buy?* | The order Jamie just placed, from the owner's side. |
| Back office | *Which promotion brought the most sales this week?* | Sales by promotion in a table, with the leading promotion named in the reply. |

The demo video runs its scenes on Friday 11:00 and Saturday 8:00. Between them a simulator (`cargo xtask sim`) plays the night through the same REST API a browser uses: 199 orders and 45 return requests, after which the Inbox holds six tasks nobody asked for, among them Ailism Bot's reorder of the Light rain parka. The simulator runs from the command line, so moving the clock on the live store alone gives a quieter Saturday.

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

Most AI commerce puts a chatbot on screens built for people (AI-added) or opens the old APIs to outside agents (agent-ready). A-Commerce rebuilds what is underneath, so the agent works the store through tools made for it ([docs/research.md](docs/research.md) §2). The picture is rendered from frame 13 of the demo storyboard ([design/demo](design/demo/README.md)).

| Principle | How the code holds it |
|---|---|
| **1. Tools work in the units people use.** Find by an order number, an email, a product name; get back what the decision needs. | Id fields also take the order number, change set number or email. Reads return the fields a decision needs and refuse a field name that does not exist instead of returning an empty object. |
| **2. Every change is a proposal first.** The agent drafts a change set; a person approves it where the rules require, like a pull request. | Every write has a preview. Change sets are approved, re-previewed, run, verified and undone by the commerce service (`crates/changeset`). The agent can draft but never approve or run; person-only actions (placing an order, approving) need scopes a chat turn never has. |
| **3. Rules are data, and the system checks them.** | Discount limits, price floors, approval thresholds and return windows live in [`policies/*.yml`](policies/README.md). The back end checks them and names the rule and the tool that fixes it, so a wrong model still cannot break a rule. |
| **4. The system brings the work.** | Watchers (price under cost, stock-out forecast, returns waiting for a refund decision) open tasks with a draft change set, and the agent picks the task up before anyone asks. **Ailism Bot**, the back office's manager AI, reads the whole store on its schedule (or on *Run now*), proposes what to fix across eight areas, and opens each proposal as a task the operations agent drafts — with its reason, evidence and expected effect, a history of every cycle, *ask again* with a correction, and feedback it reads in every analysis. It never approves. |
| **5. The screen is the result of the work.** | 21 UI blocks (7 back office, 14 storefront) chosen per task and per person; every element on the page is an AI target people can ask about. |

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
- **Measured on the Nov 3 code and the 400-product store: 390 runs, not run again since.** The catalog grew to 1,009 products on Nov 4 and the agent kept changing. The new products stay out of summer outerwear, knitwear and every order, so the sale and the sales history do not move, but searches see them, and one held-out fact now counts 97 outerwear products instead of 62.

## How we use Meta models

| Model | Where it runs | What it does |
|---|---|---|
| **Muse Glimmer 30B** | Open weights, self-hosted: NVIDIA's 4-bit build on vLLM, one RTX 5090. 72 output tokens/s for one request, 0.1 s to the first token when the prompt's start is cached. A crash on this GPU traced to cuDNN and fixed ([docs/serving.md](docs/serving.md)) | Every agent's worker: routing, tool search, planning, arguments, replies. On the live demo it handles every call. Offline it also wrote the descriptions, summaries and tags of the 991 generated products and the text of 4,140 reviews, from their facts ([seed/demo/catalog_content.yml](seed/demo/catalog_content.yml)). |
| **Muse Spark** | Meta Model API | Wired as every agent's planner and fallback ([agents/models.yml](agents/models.yml)); used when `META_API_KEY` and `SPARK_MODEL` are set. Off on the live demo, to show what the open model does alone. |
| **Muse Voice Transcribe** | Meta Model API | The composer's microphone: speech becomes the draft message, with the names on the screen (products, sizes) as spelling hints ([docs/api/chat.md](docs/api/chat.md) §4); used when `META_API_KEY` and `VOICE_MODEL` are set. |
| **Muse Image** | Meta Model API | The photos of all 1,009 demo products, one for each color (2,051 files), made offline with each response id kept beside its photo ([seed/demo/catalog_content.yml](seed/demo/catalog_content.yml), [seed/demo/03_catalog.yml](seed/demo/03_catalog.yml)). In the product: `generate_page_image` draws the picture across the top of a curated page at preview, so the person approving sees it; shoppers see it only after approval ([docs/api/catalog.md](docs/api/catalog.md) page_image). Used when `META_API_KEY` and `IMAGE_MODEL` are set; otherwise a placeholder that says so. |

**The proof is in [docs/meta-models.md](docs/meta-models.md).** Each of the 2,051 product photos carries the id of the Muse Image response that made it, and those ids match the seed and a list of all 2,128 calls ($20.94). The 6,390 Glimmer calls that wrote the store's text are listed too; Glimmer runs on our own GPU, so that list is our own record.

![The Cozy knits page in the demo video, with the banner Muse Image drew.](docs/img/evidence-page-image-live.jpg)

**Why Meta models matter here:** open weights let a merchant run the agent on its own infrastructure, keeping catalog and customer data in-house, and a small model keeps the cost per request low enough to put an agent behind every request. The structure (closed tools, previews, policy checks, a reply that may only use numbers found in the tool results) is what makes a 30B model dependable.

## What is in the box

| | |
|---|---|
| Commerce operations | **298 tools in 13 domains** (138 reads, 119 writes, 41 writes that move money or reach customers) and **23 person-only endpoints**. Every one is specified in [docs/api/](docs/api/README.md) and built; CI fails if one is missing or outside its folder. |
| Domains | platform · catalog · customer · promotion · merchandising · search · sales · fulfillment · messaging · analytics · community · partner · integration |
| Agents | `promotion`, `orders`, `catalog`, `insights`, `marketplace` (back office), `shopper` (storefront), defined in [agents/README.md](agents/README.md) |
| Background work | 3 watchers, Ailism Bot's cycle and 8 scheduled jobs (promotion start and end, points expiry, tier evaluation, subscription renewal, campaign sending, seller status changes, held events) |
| Data | PostgreSQL, 139 tables and views named after schema.org, a comment with its term on every table and column ([data dictionary](docs/backend/schema.generated.md)); a demo store of about 158,000 rows (Northwind Apparel: 14,000 members, 1,009 products with 9,239 variants, five weeks of daily orders, about 8,300 in all) |

### A showcase, on purpose

This is a showcase of AI-native commerce, not a store that ships parcels. What runs for real is everything inside the store: the tools, change sets, approvals, policy checks, watchers, agents and UI. Everything **outside** the store is a deterministic stand-in that says so in its output: payment providers and bank deposits, tax receipts, carriers, outside sales channels, email/SMS/push delivery, business-registry checks, payouts, and the product tools `generate_image` and `generate_copy`. Those stand-ins still go through preview → change set → run, so the agent's side of the work is the same as it would be against real providers. The pictures on curated pages are real Muse Image calls. Orders placed after the store clock's start come from the simulator or from you. The reviews on the demo products are generated too: Muse Glimmer writes the text from each product's facts, and a review tied to an order comes from the buyer of a delivered line.

## Architecture

![Figure 1. Context construction (agent profile, step prompts, conversation memory, tool search) feeds Muse Glimmer 30B. Harness guards (plan check and binder, reply check, route hints, limits) sit between the model and commerce. Commerce has a tool interface (298 tools, human handles, previews) and trust (change sets, policies, person-only actions). The store brings work: watchers, store events, task pickup, which starts the next turn.](docs/img/fig-harness-components.png)

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
        |  Meta Astryx)    |      |  router · tool search|--->|  298 operations     |
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

Every markdown file can be reached from here by following links. Each folder's README lists what is under it, and each file names its way back up ([the rules](docs/README.md#how-these-files-link)).

- [AGENTS.md](AGENTS.md): the rules for coding agents, which [CLAUDE.md](CLAUDE.md) imports
- [docs/README.md](docs/README.md): the map of the documents
  - [design.md](docs/design.md), [research.md](docs/research.md), [architecture.md](docs/architecture.md), [scenarios.md](docs/scenarios.md), [plan.md](docs/plan.md), [serving.md](docs/serving.md), [meta-models.md](docs/meta-models.md)
  - [api/README.md](docs/api/README.md): every operation, its conventions, and one file per domain
  - [backend/README.md](docs/backend/README.md): database, commerce, agent, gateway, testing
  - [frontend/README.md](docs/frontend/README.md): the AI layer, the UI library and look, the theme
- [agents/README.md](agents/README.md): agent definitions and prompts
- [policies/README.md](policies/README.md): the store's rules as data
- [seed/demo/README.md](seed/demo/README.md): the demo store's data
- [eval/README.md](eval/README.md): task sets and every result
- [infra/README.md](infra/README.md): containers, Kubernetes, the demo server, local secrets
- [design/README.md](design/README.md): mockups, layout options, the demo storyboard

For a task, start with the row that matches what you are about to do. Each document links to the next one it depends on.

| If you want to… | Read, in this order |
|---|---|
| Understand the product | This README → [docs/design.md](docs/design.md) (principles, tool rules, harness, UI protocol, results) → [docs/research.md](docs/research.md) → mockups in [design/README.md](design/README.md) |
| See the whole system | [docs/architecture.md](docs/architecture.md) (services and trust boundaries, API hierarchy, schema.org data model, AI-native features) |
| Know what was planned and where it stands | [docs/plan.md](docs/plan.md) (scope, milestones, status, open work) |
| Know the demo store and scenes | [docs/scenarios.md](docs/scenarios.md) (people, products, policies, each scene with expected results) |
| Build back-end code | [AGENTS.md](AGENTS.md) → [docs/backend/README.md](docs/backend/README.md) (workspace, crates, dependency rules, folder layout) → [docs/backend/database.md](docs/backend/database.md) → [docs/backend/commerce.md](docs/backend/commerce.md) → [docs/backend/agent.md](docs/backend/agent.md) → [docs/backend/gateway.md](docs/backend/gateway.md) → [docs/backend/testing.md](docs/backend/testing.md) |
| Add or change an operation (tool) | [docs/api/README.md](docs/api/README.md) (hierarchy, kinds, index) → [docs/api/conventions.md](docs/api/conventions.md) → [docs/api/vocabulary.md](docs/api/vocabulary.md) → the domain file (for example [docs/api/promotion.md](docs/api/promotion.md)) → [docs/api/enums.md](docs/api/enums.md) and [docs/api/errors.md](docs/api/errors.md) → [policies/](policies/README.md) |
| Build front-end code | [docs/frontend-ui.md](docs/frontend-ui.md) (Meta Astryx, the Ailism theme, look rules) → [docs/frontend/README.md](docs/frontend/README.md) (AI layer, elements, person buttons) → [docs/api/chat.md](docs/api/chat.md) (stream and blocks) → [design/README.md](design/README.md) |
| Change what an agent does or says | [agents/README.md](agents/README.md) (definitions and prompts) → [docs/backend/agent.md](docs/backend/agent.md) → [eval/README.md](eval/README.md) |
| Measure the agent | [eval/README.md](eval/README.md) (task format, sources, how to run) → [docs/backend/testing.md](docs/backend/testing.md) |
| Run, package, or deploy | [infra/README.md](infra/README.md) (serving architecture, images, Compose, Kubernetes) → [infra/deploy/README.md](infra/deploy/README.md) (the demo server) |
| See the proof that Meta models did the work | [docs/meta-models.md](docs/meta-models.md) (every run, the call lists, how to check them) |
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
| `IMAGE_MODEL` | Muse Image for the pictures on curated pages through the same API (optional) |
| `ACOM_HTTP_PORT` | Host port of the gateway (default 8080) |

## What is not done yet

The back end is complete for the showcase: every specified operation is built. These are the gaps we know of, checked against the code on Nov 4. What is finished is in [What we built during the hackathon](#what-we-built-during-the-hackathon).

**Agent quality** (last measured Nov 3, in [the design set's report](eval/results/2026-11-03-glimmer-design-acom.md))
- [ ] Read a long list to its end, or filter it, before saying "none" or "these ones". Seen on held-out and fresh (answers from the first 100 of 400 products or 412 orders), so a fix has to be checked on the design set and a new held-out set.
- [ ] Say the exact times and amounts the results hold, such as an undo deadline of Nov 7 23:59 (`mgr-what-needs-me-today`, 0 of 3).
- [ ] Use the tool a sale needs: for a markdown on tees the agent now looks the category up, but plans `create_promotion` where the task expects `set_sale_prices` (`md-end-of-season-markdown`, 0 of 3).
- [ ] Draft the refunds when asked to go through the returns: in the runs the change set had no `refund_return` items (`b2-returns-over-200`, 0 of 3). On screen, the *Draft 9 refunds and 3 messages* button does it.
- [ ] Publish a page in the same change set that creates it (`publish_curation` checks the id before the page exists).
- [ ] Fill a curated page from all that the search found: the Cozy knits page gets 2 of the 26 knitwear products.
- [ ] Compare Glimmer with Spark on the same tasks, and fix the runner problems listed in [the Nov 3 judging](eval/results/2026-11-03-glimmer-judged.md#runner-problems-found-here).

**On screen**
- [ ] The checkout card built from a `prepare_subscription` preview posts to the order endpoint; it should start the subscription (`POST /v1/sales/subscriptions`).
- [ ] Back-office pages for the tools added on Oct 31 (campaigns, community moderation, sellers, channels and imports, subscriptions, receipts, inquiries, remedies). Today the agents answer with text and tables only (principle 5).

**The system brings more of the work**
- [ ] Watchers that open a task with a draft when a seller's grade drops or a scheduled campaign fails. Reported community posts are already covered: Ailism Bot proposes hiding them.
- [ ] Expire gifts nobody accepted and orders never paid. The policies exist; no job applies them yet. It needs a standing approval for a system cancel.

**Spec gaps found while building** (internal; small)
- [ ] An operation that publishes a community post or reply held for review (today the store can only hide or delete it).
- [ ] A tool that links a product to a seller (today only the seed does).
- [ ] Endpoints that issue the code for re-authenticating a withdrawal and for sign-in id recovery (the flows accept such codes; nothing issues them yet).
- [ ] Tier evaluation reads each member's orders one by one; a bulk reader from sales would make an all-member run fast.

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
| Nov 1 | Muse Image photos for every product and color, and the product page that follows the color you pick. Harness figures, the demo storyboard, the back office's menu. The facade's order screens for the evaluation |
| Nov 2 | The composer's microphone on Muse Voice Transcribe. Replies and blocks streaming over a chat WebSocket, the canvas, conversation history. Storefront browsing, collections, notices, FAQs and boards; orders shown as the products they were; phone layouts |
| Nov 3 | The final evaluation: 390 runs on Muse Glimmer 30B, every reply read. Glimmer's serving notes and the RTX 5090 crash fix. `generate_page_image` on Muse Image for curated pages. **Ailism Bot**. Promotion performance in sales answers |
| Nov 4 | The catalog grows from 400 to 1,009 products, with facts by rule, copy and reviews by Glimmer and photos by Muse Image, without moving a row. Ailism Bot's proposals become drafts, the Inbox as cards, planner and reply fixes. The 3-minute demo video |

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
