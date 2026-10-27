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

| Model | Where it runs | Role in A-Commerce | Status (Oct 28) |
|---|---|---|---|
| Muse Glimmer 30B | Open weights on a partner inference server | Worker: tool search, tool calls, routine requests | Wired (OpenAI-compatible client); waiting for keys to measure |
| Muse Spark Open | Open weights on Baseten (`meta-models/muse-spark-open-1026`) | Planner: multi-step requests, revisions, takeover when the worker fails | Wired as the fallback; waiting for keys to measure |
| Llama 3.2 3B | Open weights, on a laptop (Ollama) | Stand-in used to measure the tool layer before Muse keys arrived | Measured, see below |
| Muse Image, SAM 3.1, Voice | Meta Model API | Product images, cut-outs, voice requests | Not started (mockups only) |

**Why open weights matter here:** the whole agent (worker and planner) can run on infrastructure the merchant controls, so catalog and customer data stay in-house, and a small worker keeps the cost per request low enough to put an agent behind every request.

## How we know it works

Every claim below is a test or a command you can run. `make check` runs all of them except the ones that need a model.

| What | How it is checked |
|---|---|
| The VIP weekend story, end to end: request → change set v1 (4 of 6 checks) → "Drop the low-stock ones…" → v2 (6 of 6, 40 products, overshirt $67.15) → approve → every change read back → Jamie pays $71.20 on Saturday → undo together | `server/tests/test_scenario_vip_weekend.py`, `test_harness.py`, and in a real browser `web/e2e/back-office.spec.ts`, `storefront.spec.ts` |
| Alex's defect return → the operator's review of 12 returns over $200 → one approval: 9 refunds ($2,463.00) and 3 messages, read back; refunds cannot be undone | `test_returns.py`, `web/e2e/returns.spec.ts` |
| Agents never approve, apply, order, or send refunds; shoppers never see cost or operator tools | `test_scenario_vip_weekend.py::test_agent_cannot_apply_its_own_preview`, `test_storefront.py`, `test_returns.py` |
| A reply that contradicts the change set ("the sale is now live") is replaced by facts | `test_grounding.py` (with replies a real 3B model gave) |
| A real model does the demo request | `make try-model RUNS=3` (model in `.env`) |
| Our tools vs a screen-shaped API, same model and tasks | `make compare` |

First measurements, with a stand-in model (details and limits in [eval/README.md](eval/README.md)):

| Model | Demo request, first message | Our tools: tasks passed | Screen-shaped API: tasks passed |
|---|---|---|---|
| Llama 3.2 3B (local) | 3 of 3 runs right | 2 of 3 (20 of 21 criteria, 0 errors) | 0 of 3 (3 errors per task) |
| Muse Glimmer 30B | pending keys | pending | pending |
| Muse Spark Open | pending keys | pending | pending |

These are early numbers: one run per task at temperature 0, tasks written by us, and a 3B stand-in. The 3B model still fails the revision step ("Drop the low-stock ones…"), which is the step designed for the planner.

## Architecture

```
  web/  React                         Back office (layout A)        Storefront (/fo)
        one component per UI block    change_set · refund_review    compare_table · look_gallery · cart · return_options
                    │  person-only buttons: approve, undo, place order, send return
                    ▼
  server/  FastAPI  ───────────────────────────────────────────────────────────────────
        harness/   tool search (words + consumes/produces graph) · operator and shopper agents
                   · nudges and retries · replies checked against the change set · worker → planner handoff
        changesets  previews grouped for review · approve · run · read back · undo together
        tools/     plan_sale · set_sale_prices · create_coupon · schedule_banner · plan_return_review
                   · refund_return · compare_products · add_to_cart · get_return_options · …   (A-Commerce tools)
        facade.py  the same store behind a screen-shaped API, for the comparison
        policy.yaml  business rules as data          SQLite + seed (docs/plan.md §4)
                    ▲
        models  OpenAI-compatible: Muse Glimmer (worker), Muse Spark Open (planner); scripted stand-in without keys
```

Design: [docs/design.md](docs/design.md) · Build plan and seed numbers: [docs/plan.md](docs/plan.md) · What works today: [docs/progress.md](docs/progress.md) · Research: [docs/research.md](docs/research.md) · Mockups: [design/](design)

## Getting started

### Requirements

- Python 3.12+ and [uv](https://docs.astral.sh/uv/)
- Node 22+

### Run it

```bash
make setup   # server and web dependencies, and Chromium for the browser tests
make check   # server tests, web build, browser tests (same as CI)
make demo    # API on :8000 and the app on http://localhost:5173
```

Without model keys the agents run a scripted stand-in that is labelled `scripted` on every step. To use real models, copy `.env.example` to `.env` and fill `WORKER_*` (Glimmer) and `PLANNER_*` (Spark Open), or point `WORKER_*` at any OpenAI-compatible server (for example Ollama).

```bash
make try-model RUNS=3   # the demo request through the model in .env, judged on 7 criteria
make compare            # our tools vs the screen-shaped API, same model and tasks
```

### Walk through the demo

The data sits at a fixed moment (Fri Nov 6, 11:00). The demo clock buttons in the back office move it.

1. `/fo?shopper=alex` — "The sole on my chelsea boots is coming apart after a week." → "Refund, please." → Send refund request.
2. `/` (back office) — "Check returns waiting over $200" → 12 requests, 9 refunds $2,463.00 → Approve 9 refunds and send 3 messages.
3. Back office — click **Fri Nov 6, 11:02**, then the VIP weekend suggestion → v1 with 2 problems → "Drop the low-stock ones. Give the overshirt 15% instead." → v2, 6 of 6 → Approve and run → read back.
4. Set the clock to Sat Nov 7 → `/fo?shopper=jamie` — ask for a light jacket → comparison table at the VIP price → "Which one packs smallest?" → "The windbreaker. Same size as last time." → Place order · $71.20.
5. `/fo?shopper=mina` — the same question gets a list; "Less text. Just show me bigger photos." switches the layout and leaves one memory you can undo.

Script for the 3-minute video: [docs/demo.md](docs/demo.md).

### Use the same tools from any MCP client

Outside agents get the same tools and policies over MCP. The MCP server is a thin proxy over the running API (`make demo`),
so what an outside agent proposes shows up in the back office task list for a person to approve. Approve, undo, place
order and send return are not tools, here or anywhere.

```json
{
  "mcpServers": {
    "a-commerce-operator": {
      "command": "uv",
      "args": ["run", "--project", "/path/to/a-commerce/server", "python", "-m", "a_commerce.mcp_server", "--role", "operator"]
    },
    "a-commerce-shopper": {
      "command": "uv",
      "args": ["run", "--project", "/path/to/a-commerce/server", "python", "-m", "a_commerce.mcp_server", "--role", "shopper"]
    }
  }
}
```

### Environment variables

| Name | Description |
|---|---|
| `WORKER_BASE_URL`, `WORKER_API_KEY`, `WORKER_MODEL` | Worker model (Muse Glimmer), any OpenAI-compatible endpoint |
| `PLANNER_BASE_URL`, `PLANNER_API_KEY`, `PLANNER_MODEL` | Planner model (Muse Spark Open on Baseten) |
| `META_API_KEY` | Meta Model API key (images, later) |

Never commit `.env`.

## Demo

- Demo video (3 min): TODO
- Live demo: TODO

## What we built during the hackathon

Everything in this repository was created between Oct 27 and Nov 4, 2026. The commit history is the record.

| Date | What was built |
|---|---|
| Oct 27 | Repository, research, design, UI mockups (layouts, intents, storefront) |
| Oct 28 | Build plan; server with policy, seed, tools and change sets; harness with tool search and model client; back office and storefront web apps; browser tests and CI; first real-model runs (Llama 3.2 3B) and the fixes they led to; replies checked against change sets; returns flow; comparison with a screen-shaped API |

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
