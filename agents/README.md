# Agents

[README](../README.md)

Each agent is a definition (`{name}.yml`: surface, model, tools, blocks, layout rules) and its instructions (`{name}.md`). How a turn uses them: [docs/backend/agent.md](../docs/backend/agent.md).

**Every other `.md` file in this folder is a model prompt, read into the prompt as written.** Do not add links or headers to them: the words reach the model, and a change is a change to the agent's behavior.

## Agents

| Agent | Surface | What it does | Files |
|---|---|---|---|
| promotion | back office | Sales, member and period prices, coupons, and the banners and slots that announce them | [promotion.yml](promotion.yml), [promotion.md](promotion.md) |
| orders | back office | Orders, returns and refunds, deliveries, customers and their history, messages to customers | [orders.yml](orders.yml), [orders.md](orders.md) |
| catalog | back office | Products and drafts, supplier sheets, variants and list prices, categories, brands, stock and reorders, moderation | [catalog.yml](catalog.yml), [catalog.md](catalog.md) |
| insights | back office | Questions about how the store is doing, answered from the numbers | [insights.yml](insights.yml), [insights.md](insights.md) |
| marketplace | back office | Sellers on the marketplace and the sales channels the store is connected to | [marketplace.yml](marketplace.yml), [marketplace.md](marketplace.md) |
| shopper | storefront | Finds and compares products, checks fit, picks gifts, keeps the cart, prepares the order, lays out return options | [shopper.yml](shopper.yml), [shopper.md](shopper.md) |

## Step prompts

Named in [runtime.yml](runtime.yml) under `prompts`.

| Prompt | What it does |
|---|---|
| [prompts/router.md](prompts/router.md) | Chooses which agent handles a request |
| [prompts/planner.md](prompts/planner.md) | Turns one request into a short list of tool calls |
| [prompts/arguments.md](prompts/arguments.md) | Fills in the arguments a plan left incomplete |
| [prompts/reply.md](prompts/reply.md) | Writes the short text that goes with the blocks on screen |
| [prompts/bot.md](prompts/bot.md) | Ailism Bot's analysis of the store on each cycle |

## Settings

| File | What it holds |
|---|---|
| [models.yml](models.yml) | The models the runtime can use, by alias |
| [runtime.yml](runtime.yml) | How every turn runs: limits and step prompts. Harness settings, not store rules; those are in [policies/](../policies/README.md) |
