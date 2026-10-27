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

Design details: [docs/design.md](docs/design.md). Research (what AI-native commerce means, who else is building it, with source captures): [docs/research.md](docs/research.md). UI mockups: [design/mockups](design/mockups).

## Getting started

### Requirements

- TODO

### Run locally

```bash
# TODO
```

### Environment variables

| Name | Description |
|---|---|
| `META_API_KEY` | Meta Model API key (never commit it) |

## Demo

- Demo video (3 min): TODO
- Live demo: TODO

## What we built during the hackathon

Everything in this repository was created between Oct 27 and Nov 4, 2026. The commit history is the record.

| Date | What was built |
|---|---|
| Oct 27 | Repository created, project scope defined |

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
