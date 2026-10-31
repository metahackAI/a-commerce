# A-Commerce design

Status: draft (Oct 27; principles set Oct 31). This document is the brief for everyone building A-Commerce, people and coding agents alike.

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

1. **One business verb per tool.** `create_promotion`, `issue_coupon`, `cancel_order`. No screens, popups, or grids.
2. **Plain names.** `verb_noun`, full English words. The name and a one-line description must be enough to pick the tool.
3. **Typed inputs with meaning, not codes.** Enums are words (`"percent_off"`, `"vip"`). Every field has a description and an example.
4. **Safe reads.** Reads never change state. A list returns items and the total together. A `fields` argument keeps responses small.
5. **Writes go through preview.** Every write supports `preview=true`, which returns what will change, the rule checks, and whether a person must approve. The real write takes the `preview_id`.
6. **Declared links.** Each tool declares what it consumes and produces (for example, `search_products` produces `product_id[]`; `create_promotion` consumes `product_id[]` and produces `promotion_id`). The harness builds a tool graph from these declarations.
7. **Rules are data.** Business rules (maximum discount, stacking, approval thresholds) live in a policy file that the back-end enforces. A violation comes back as a readable reason plus the tool that can fix it. Rules are never hidden in a prompt.
8. **Actionable errors.** Each error has a code, a human sentence, and a suggested next tool.
9. **References take what people say.** A field that takes an order, change set, or customer id also takes the order number, the change set number, or the email; the server finds the record before the tool runs. Values that can match several records, such as product names, go through a search tool. The table is in [api/conventions.md](api/conventions.md) §2.

## 4. Domain scope

Written from scratch; the names and shapes are ours. The tools below are first examples; the full hierarchy with the final names (for example `set_sale_prices`, `create_coupon`, and `schedule_banner` from the mockups) is in [api/](api/README.md).

| Domain | Entities | Example tools |
|---|---|---|
| Catalog | product, variant, brand, category | `search_products`, `get_product`, `create_product`, `update_product`, `set_variant_stock` |
| Pricing | price, price schedule | `set_price`, `schedule_price_change` |
| Display | category page, slot, banner | `list_display_slots`, `schedule_display` |
| Promotion | promotion, coupon, benefit rule | `create_promotion`, `issue_coupon`, `list_active_promotions` |
| Member | member, tier, preference | `search_members`, `get_member`, `list_tiers` |
| Order | order, order line, payment | `search_orders`, `get_order`, `place_order` |
| Claim | cancel, return, exchange, refund | `request_cancel`, `approve_return`, `refund_order` |
| Shopper | cart, wishlist, profile | `add_to_cart`, `get_cart`, `checkout`, `remember_preference` |

Out of scope for the hackathon: settlement, partner management, real payment gateways, real shipping carriers. They are stubbed.

## 5. Harness

```
request ─► tool search ─► plan ─► execute ─► check ─► answer as UI
              │             │        │          │
          tool graph     Spark    Glimmer    read back
                       (if needed)  + preview/approve
```

- **Tool search**: the model never sees all tools. The harness retrieves a few candidates by description match plus graph neighbors (what usually comes before or after).
- **Plan**: a short ordered list of tool calls. Glimmer plans simple tasks; Muse Spark plans multi-step ones or takes over after a failed check.
- **Execute**: tool calls with preview first; writes that need approval pause and show an approval card.
- **Check**: after a write, the harness reads the state back and compares it with the request (for example, the promotion exists, covers the right products, and has the right discount and period).
- **Action log**: every call, preview, approval, and check is recorded and visible.

## 6. UI protocol

The agent never writes HTML. It returns UI blocks that name a component from a fixed library and fill it with data:

```json
{"ui": "promotion_preview", "data": {"promotion_id": "...", "products": 42, "discount": "20%", "period": "..."}}
```

Initial components: `product_table`, `product_gallery`, `product_compare`, `promotion_preview`, `approval_card`, `order_detail`, `cart`, `checkout_card`, `action_log`. The mockups refined these into the library in [api/chat.md](api/chat.md) §3 (for example `compare_table`, `look_gallery`, and `change_set`, which joins the promotion preview and the approval card).

Storefront personalization: what the shopper says is stored as preferences (`remember_preference`). Preferences change both the content (ranking, filters) and the form (for example, a comparison table for a shopper who compares specs, a gallery for one who browses by look).

## 7. Proof: AI-friendly vs. screen-shaped

To show that the tool design matters, we build a **screen-shaped facade** over the same back-end that reproduces the patterns in section 2 (generic examples, written by us). Then we run the same task set through both:

| | Screen-shaped facade | A-Commerce tools |
|---|---|---|
| Muse Glimmer | ? | ? |
| Muse Spark | ? | ? |

Metrics: task completion, tool selection accuracy, calls per task, cost per task, consistency over repeated runs. Tasks are split into a design set (used while building) and a held-out set (scored only at the end). Only held-out numbers go into the README.

## 8. Stack

Decided Oct 30 (details in [architecture.md](architecture.md) and [backend/](backend/README.md)):

- Back-end: Rust (Axum on Tokio), PostgreSQL with pgvector, seed data generated by us from the scenes in [scenarios.md](scenarios.md)
- Tools: defined once in Rust; MCP, REST/OpenAPI, and the agent's tool manifest are generated from the same definitions
- Harness: the Rust agent service; Glimmer through a self-hosted vLLM endpoint, Spark and the other models through the Meta Model API
- Gateway: Rust; the only entry point for browsers and outside agents
- Front-end: Next.js (TypeScript); one chat shell for the back office and one for the storefront, both rendering the UI blocks ([api/chat.md](api/chat.md)); components from Meta Astryx ([frontend-ui.md](frontend-ui.md))

## 9. Demo scenarios

- **Back office**: "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page." → search products → preview promotion (rule check: maximum discount) → approval card → create → issue coupon → schedule display → check → promotion preview and action log.
- **Storefront**: a shopper looks for a light jacket in conversation → gallery → shopper asks about material and weight → the view switches to a comparison table → add to cart → checkout card, with the VIP coupon from the back-office scenario applied.

The two scenarios connect: what the operator sets up is what the shopper sees.
