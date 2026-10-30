# A-Commerce

**AI-native commerce, built on Meta models.**

> Built during the [Meta Global AI Developer Hackathon](https://cerebralvalley.ai/e/meta-ai-developer-hackathon) (Oct 27 – Nov 4, 2026).
> Every line of code in this repository was written during the hackathon. See [What we built during the hackathon](#what-we-built-during-the-hackathon).

- **Track:** Indie Developer
- **Problem statement:** Build for Your World

---

## Why

Commerce back-ends are big, and most of them grew over years. Products, options, prices, stock, display categories, promotions, coupons, orders, claims, members: each is its own service with its own screens. A single business task such as *"run a 20% weekend sale on summer outerwear for VIP members"* means going through many back-office screens and APIs in the right order.

Handing those APIs to an LLM as they are does not work well. There are hundreds of endpoints, legacy names, and ordering rules that only people know. Large models struggle; small models fail. So most "AI commerce" today is a chatbot that answers questions next to screens that people still operate.

Shoppers get the same treatment. The storefront is the same set of pages for everyone, and personalization means a recommendation carousel.

## What A-Commerce does

**A-Commerce turns a commerce back-end into tools that an AI agent can find, chain, and run, so that operators and shoppers just talk, and the right UI shows up when it is needed.**

1. **Say it.** An operator (back office) or a shopper (storefront) asks in natural language.
2. **The harness does the work.** It finds the right commerce tools, plans how they connect (find products → create a promotion → issue a coupon → schedule the display), runs them, and stops for a person only at checkpoints such as price changes, publishing, or refunds.
3. **The answer is a UI, not a paragraph.** Results come back as components picked for the task and the person: a product table to review, a promotion preview, a comparison view, a checkout card.

## AI-native, not AI-added

Most commerce platforms bolt a chatbot onto screens built for people. A-Commerce starts from the other side: AI agents do the work, and people step in only where judgment is needed.

The principles we hold to:

- **Tools, not screens.** Every capability is a tool an agent can call, with a clear name, typed inputs, and declared connections (which tool's output feeds which tool).
- **Small model first.** The tool layer and the harness are designed so that a small open-weight model can do routine work. A larger model is used only for steps that need it.
- **Do the work, not just answer.** A request ends when the task is done, not when a reply is written.
- **People approve; they don't operate.** Actions that move money or reach customers need a person's confirmation. Every action is logged.
- **UI on demand.** The agent picks components from a fixed library and fills them with data. It does not write free-form HTML.
- **The conversation is the personalization.** On the storefront, what the shopper says shapes both what is shown and how it is shown.

## Back office and storefront

| Area | Today | With A-Commerce |
|---|---|---|
| Product registration | Fill long forms per product, upload and crop images | Describe or upload; the agent drafts the listing and images, a person reviews |
| Promotions and coupons | Configure target, discount, period, and display across several screens | One request; the agent builds the promotion and shows a preview to approve |
| Display | Arrange categories and banners by hand | The agent proposes and schedules display changes |
| Orders and claims | Search, check status, process step by step | Ask about an order; the agent checks and processes it within approval rules |
| Storefront | Same pages for everyone | A conversation that recommends, compares, and checks out, with the UI fitted to the shopper |

## How we use Meta models

> Proposed split. To be confirmed and measured during the hackathon.

| Model | Where it runs | What it does in A-Commerce |
|---|---|---|
| Muse Glimmer 30B | Open weights (self-hosted) | Main worker: tool search, tool calls, routine tasks |
| Muse Spark | Meta Model API | Planning multi-step tasks and steps the small model fails |
| Muse Image | Meta Model API | Product and banner images for listings and promotions |
| SAM 3.1 | Meta Model API | Product image segmentation (cut-outs, thumbnails) |
| Muse Voice Transcribe | Meta Model API | Voice requests (optional) |

**Why Meta models matter here:** open weights let a merchant run the agent on its own infrastructure, keeping catalog and customer data in-house, and a small model keeps the cost per request low enough to put an agent behind every request.

## How we will know it works

We measure on a fixed set of back-office and storefront tasks, not on hand-picked demos:

- Task completion rate
- Tool selection accuracy (right tool, right order)
- Steps and model calls per task, and cost per task
- Consistency across repeated runs
- The same harness with Muse Glimmer vs. Muse Spark

Numbers will be added here once measured.

## Architecture

```
   Shopper (storefront chat)        Operator (back-office chat)
                \                          /
                 v                        v
        +------------------------------------------+
        |              Commerce harness            |
        |  tool search · planner · executor        |
        |  approval checkpoints · action log       |
        |  UI component picker                     |
        +----------------+-------------------------+
     Meta models         |  tool calls
  (Glimmer / Spark / ...)|
                         v
        +------------------------------------------+
        |            Commerce tool layer           |
        |  product · price · stock · display ·     |
        |  promotion · coupon · order · member     |
        +----------------+-------------------------+
                         v
              Commerce back-end services
```

## How to read the docs

Start with the row that matches what you are about to do. Each document links to the next one it depends on.

| If you want to… | Read, in this order |
|---|---|
| Understand the product | This README → [docs/design.md](docs/design.md) (rules for AI-friendly tools, harness, UI protocol) → [docs/research.md](docs/research.md) → mockups in [design/](design) |
| See the whole system | [docs/architecture.md](docs/architecture.md) (services and trust boundaries, API hierarchy, schema.org data model, AI-native features) |
| Know what we ship and when | [docs/plan.md](docs/plan.md) (scope, milestones M0–M6, definition of done) |
| Know the demo store and scenes | [docs/scenarios.md](docs/scenarios.md) (people, products, policies, each scene with expected results) |
| Build back-end code | [AGENTS.md](AGENTS.md) → [docs/backend/README.md](docs/backend/README.md) (workspace, crates, rules) → [docs/backend/database.md](docs/backend/database.md) (schema.org tables and columns) → [docs/backend/commerce.md](docs/backend/commerce.md) → [docs/backend/agent.md](docs/backend/agent.md) → [docs/backend/gateway.md](docs/backend/gateway.md) → [docs/backend/testing.md](docs/backend/testing.md) |
| Add or change an operation (tool) | [docs/api/README.md](docs/api/README.md) (hierarchy, kinds, index) → [docs/api/conventions.md](docs/api/conventions.md) → [docs/api/vocabulary.md](docs/api/vocabulary.md) (schema.org types and our extensions) → the domain file (for example [docs/api/promotion.md](docs/api/promotion.md)) → [docs/api/enums.md](docs/api/enums.md) and [docs/api/errors.md](docs/api/errors.md) → [policies/](policies/README.md) |
| Run, package, or deploy | [infra/README.md](infra/README.md) (serving architecture, images, Compose, Kubernetes) → [infra/k8s/README.md](infra/k8s/README.md) |
| Build front-end code | [docs/frontend-ui.md](docs/frontend-ui.md) (Meta Astryx, setup, look rules) → [docs/frontend/README.md](docs/frontend/README.md) (AI layer, elements, person buttons) → [docs/api/chat.md](docs/api/chat.md) (stream and blocks) → [design/](design) |
| Look up a table, column, or term | [docs/backend/schema.generated.md](docs/backend/schema.generated.md) (generated from the migrations), [vocab/acom.jsonld](vocab/acom.jsonld) (every `acom:` term), or the `describe_data` tool |
| Load or change demo data | [docs/scenarios.md](docs/scenarios.md) → [seed/demo/README.md](seed/demo/README.md) |

Specifications that implementers read daily (`docs/api/`, `docs/backend/`, `docs/frontend/`, `docs/scenarios.md`) are written in Korean; the overview documents are in English.

## Getting started

Everything runs in containers with the same layout as production; only internal settings differ (where the database lives, how many replicas, where secrets come from). The serving architecture, Compose, and Kubernetes setups are described in [infra/README.md](infra/README.md).

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

Sign in to the back office as **`admin` / `admin123`** (a demo account with every role). In demo mode the
sign-in pages also offer the scenario's people: operator Son SeongJun (`adm_sj`), shoppers Jamie (VIP) and
Alex (member), or continuing as a guest. These accounts and the local keys are for a local demo only;
don't expose this setup publicly.

Add model access in `.env` (`META_API_KEY`, `GLIMMER_BASE_URL`, …) to use real models. `make up-models` serves the worker model on a local GPU, and `make up-observability` adds Prometheus and Grafana. `make help` lists every command.

### Develop one service outside containers

```bash
make up-debug                      # the stack, plus Postgres on :5432 and each service on its own port
cargo run -p acom-commerce -- serve   # replace the container you are working on (stop it first: docker compose ... stop commerce)
cd web && npm install && npm run dev  # web on :3000 (Meta Astryx; docs/frontend-ui.md)
```

### Local Kubernetes (optional)

```bash
make kind-up     # kind cluster, images loaded, the local overlay applied (infra/k8s)
make kind-down
```

### Checks

```bash
make check       # Rust (fmt, clippy, tests, docs/schema/seed checks), web (types, build), infra (Compose, Kubernetes manifests)
```

### Environment variables

`.env.example` lists every variable with a comment. The ones you set yourself:

| Name | Description |
|---|---|
| `META_API_KEY`, `META_API_BASE_URL`, `SPARK_MODEL` | Meta Model API for the planner model (never commit the key) |
| `GLIMMER_BASE_URL`, `GLIMMER_MODEL` | OpenAI-compatible endpoint serving the worker model |
| `ACOM_HTTP_PORT` | Host port of the gateway (default 8080) |

## Demo

- Demo video (3 min): TODO
- Live demo: TODO

## What we built during the hackathon

Everything in this repository was created between Oct 27 and Nov 4, 2026. The commit history is the record.

| Date | What was built |
|---|---|
| Oct 27 | Repository created, project scope defined |
| Oct 28 | Build plan: MVP scope, tool signatures, seed data, UI block schema, roles and schedule |
| Oct 30 | Architecture document: back-end structure, API hierarchy, schema.org data model, AI-native features |
| Oct 30 | API specification: 13 domains, every operation, data model, policies |
| Oct 30 | Infrastructure base: every component in a container (gateway, web, commerce, worker, agent, migrations, Postgres), Docker Compose with the production topology, Kubernetes manifests (Kustomize: local and production overlays), a shared service runtime (health, readiness, metrics, graceful shutdown), and the Next.js web on Meta Astryx with the AI layer |
| Oct 30 | Data foundation (M1): 12 migrations with schema.org tables and columns, a comment with its term on every table and column, a generated data dictionary and vocabulary, a rule checker, and the demo store seed (812 rows) |
| Oct 30 | Direction set: the whole server side in Rust (gateway, commerce, agent). Rust workspace with the contract, money, time, policy loader, and a docs checker; implementation design; demo scenarios from the mockups; conventions, enumerations, errors, and the chat stream |

## References

The commerce domain model (products, orders, members, display, promotions) is informed by our team's experience building enterprise commerce platforms. No code, schemas, or data from any existing product are included; all of it was written from scratch for this project.

## Team

| Name | GitHub | Role |
|---|---|---|
| Son SeongJun | [@SonAIengine](https://github.com/SonAIengine) | TODO |
| Ha-Ryeom Jang | [@CocoRoF](https://github.com/CocoRoF) | TODO |
| Haesoo Kim | [@haesookimDev](https://github.com/haesookimDev) | TODO |
| DongUk Kim | [@Createyouracccount](https://github.com/Createyouracccount) | TODO |

## License

[Apache License 2.0](LICENSE)
