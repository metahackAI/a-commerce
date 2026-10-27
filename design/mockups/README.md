# UI mockups v2 (Oct 27)

Static HTML (open in a browser) with 1440×900 captures, and 390×844 for mobile. Background: [docs/research-ai-native-commerce.md](../../docs/research-ai-native-commerce.md).

The idea behind the screens: **the conversation is where you ask; the result is a piece of work you can review.** Every screen keeps the conversation on the left and the work on the right, in the back office and on the storefront.

| Screen | File | What it shows |
|---|---|---|
| Inbox | `inbox.html` | The operator's day: what needs a decision, what agents are doing, what's done and verified. Work comes from the operator, from automations, and from things the agent noticed. The rail shows the three kinds of agents on this store (operator, shopper, outside agents through MCP) using the same tools and policies. |
| Change set | `changeset.html` (`-2` scrolled) | One request becomes a change set: the request as understood, 6 checks, grouped changes (prices, coupon, main page, excluded items), a storefront preview, and what happens after approval (run, read back, compare, undo together). The operator edits it by talking; it moves to version 2. |
| Storefront | `store.html` | A shopper's conversation on the left; on the right, the page assembled for this request: what the store understood (removable chips, some from the profile), a comparison because this shopper compares specs, the cart with the coupon from the change set, and the shopper's editable profile. |
| Mobile | `mobile.html`, `mobile.html#confirm` | The same flow on a phone, ending in a confirm sheet. The agent prepares the order; only the shopper places it. |

The two sides tell one story: the sale approved in the change set (40 products, the overshirt at 15%, coupon VIPWEEKEND20) is what the shopper sees on Nov 7.

## Where each element comes from

| Element | Reference |
|---|---|
| Sidebar with task list, task on the left and result on the right | Codex app (task list, chat beside the diff), Claude (chat beside an artifact) |
| Status circles (needs decision, running, done) and flat list rows | Linear issue lists |
| Agent-noticed items with a short reason and one action | Shopify Sidekick Pulse cards |
| Change set: grouped changes with `create` / `update` labels, checks list, version 2 | GitHub pull request (files changed, checks), Codex diff review |
| Approve before applying, preview of what changes | Shopify Sidekick (shows changes for approval before applying) |
| "Understood as" chips, removable, some from the profile | Google AI Mode filter chips, Daydream Style Passport |
| Comparison table chosen for a shopper who compares specs | Generative UI (Google Research), A2UI / MCP UI: the agent picks components the store owns |
| Visible, editable memory ("Remembered… Undo", "What Northwind knows about you") | ChatGPT memory controls, Daydream Style Passport |
| Cart with coupon applied, "only you can place it" | Trust research (shoppers reject agent purchases without permission), OpenAI's move back to merchant checkout |
| Mobile confirm sheet | iOS bottom sheets, Stripe/Shop Pay confirm step |

Not used on purpose: colored side bars, gradients, glows, drop-shadow cards, dot backgrounds.

All store, product, and person names are made up. Product images are placeholders until we generate real ones (Muse Image).
