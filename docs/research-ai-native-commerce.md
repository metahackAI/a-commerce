# What is AI-native commerce? (research notes, Oct 27, 2026)

Most sources below are press releases and secondary coverage; vendor numbers are their own claims, not independent measurements.

## 1. Where the industry is in late 2026

### Shoppers: agents moved outside the store

- **Protocols.** OpenAI and Stripe's Agentic Commerce Protocol (ACP, Sep 2025) and Google's Universal Commerce Protocol (UCP, co-developed with Shopify, Etsy, Wayfair, Target, Walmart) standardize how an agent discovers products, builds a cart, and hands off payment. Google's Agent Payments Protocol (AP2) covers payment mandates within spending limits.
- **Surfaces.** Google announced Universal Cart at I/O 2026: one cart across Search, Gemini, YouTube, and Gmail, on UCP. Amazon renamed Rufus to Alexa for Shopping (May 13, 2026), with scheduled actions ("buy only if the price drops below X") and "Buy for Me" on other retailers' sites. Naver has been running an AI shopping agent beta in the Naver Plus Store since Feb 2026, and from May 2026 combines it with membership and delivery so that a purchase can be finished inside the agent.
- **A retreat worth noting.** OpenAI moved away from Instant Checkout in March 2026: ChatGPT focuses on discovery and sends shoppers to the merchant's checkout. Reports say the first version "did not offer the flexibility it wanted". Merchants want to keep checkout, payment, and the customer relationship.

### Trust is the bottleneck, not model capability

- Riskified (Q1 2026, US/UK): about 55% of consumers are not comfortable with AI agents buying for them; 73.9% expect strong safeguards.
- ACI Worldwide/YouGov: 70% say an agent transaction made without their permission would make them stop using it.
- Visa's CPO: "it's not a question of AI model capabilities"; the problem is making it secure, permissioned, and reliable.

### Merchants: assistants on top of the admin

- **Shopify Sidekick** (Winter '26 / Spring '26): Sidekick Pulse watches store data and shows a few recommended actions as cards on the home screen; Sidekick plans multi-step work (for example, building a promotion), shows changes for approval before applying them, generates admin apps from a description, and lets teams save prompts as reusable skills. It now keeps working in the background.
- **Salesforce Agentforce Commerce** (Jul 2026): Merchant Agent (organize catalogs from plain language), Shopper Agent on the brand's own storefront, Buyer Agent for B2B on WhatsApp/SMS.
- **VTEX** (Apr 2026) calls itself an "AI-native commerce operating system": an AI Workspace with agents for catalog, promotions, and search, plus autonomous post-sales (claims "over 91% AI-assisted automation" for order status, exchanges, returns).
- In all three, the existing admin screens stay the system of record and the agent operates on top of them.

### Interfaces: from text to components the client owns

- **Shopify MCP UI** (Aug 2025): agents return interactive commerce components (variant pickers, bundles, cart). Key design choice: *components do not change state directly; they send intents back to the agent.* MCP Apps became the standard for this in early 2026; OpenAI's Apps SDK builds on it.
- **Google A2UI** (v0.9): the agent describes the UI as data and the client renders it with its own pre-built components, instead of running agent-written code.
- **Google Research, "Generative UI"**: raters preferred generated interfaces over markdown 82.8% of the time (90.5% for information-seeking prompts); human-expert sites still ranked first. Caveat: speed was not part of the evaluation, and generation often took 1–2 minutes.
- **Daydream** (fashion): each conversation refines a "Style Passport" of remembered preferences (color, fit, brands, budget, occasion). Its founder argues that keyword search tied to a site's taxonomy doesn't match how customers actually talk.

## 2. Three levels

| Level | What it looks like | Examples |
|---|---|---|
| **AI-added** | AI buttons and a chatbot on top of screens built for people. Screens are still the system. | "AI copywriter" button in the product editor, recommendation carousel, FAQ chatbot |
| **Agent-ready** | The catalog and checkout are exposed to outside agents through protocols. The back office is unchanged. | UCP/ACP catalog syndication, agentic storefronts |
| **AI-native** | The business itself can be run by agents. Tools are the primary interface, for our own agents and for outside ones. People decide; agents do the work. | What A-Commerce aims for |

## 3. Our definition

**AI-native commerce is a store whose work can be done by agents, where people decide and agents carry it out, and where every screen is assembled for the task and the person in front of it.**

Five properties, each with what we build:

1. **Tools are the primary interface.** Every business action is a well-described tool. The same tool layer serves our operator agent, our shopper agent, and outside agents (MCP / UCP-style). → *AI-friendly tool layer* (design.md §3)
2. **Agents start work, not only answer.** Agents notice things (stock running out, a promotion underperforming) and propose work, the way Pulse does, then carry it through. → *Inbox with agent-proposed work*
3. **People review change sets, not screens.** Every write is grouped into a change set with a plain interpretation of the request, checks, previews, approval, and undo, similar to reviewing a pull request. This answers the trust problem: nothing reaches customers or money without a person's approval, and everything can be undone together. → *Change set review*
4. **UI is assembled per intent from components the store owns.** The agent chooses components and fills them with data (A2UI / MCP UI pattern); components send intents back. The layout changes with the person: a comparison table for someone who compares specs, a gallery for someone who browses. → *UI protocol* (design.md §6)
5. **Memory is visible and editable.** What the agent remembers about a shopper or an operator is shown, and it can be corrected or deleted. Purchases always need the shopper's own confirmation. → *"What I know about you", confirm-to-buy*

## 4. What this means for the demo

- Show a **change set**, not just a chat answer. That is the moment a judge sees the difference between "AI-added" and "AI-native".
- Show **one tool layer, several agents**: the operator agent creates the sale; the shopper agent uses the same tools and the coupon applies at checkout.
- Show **trust built into the flow**: interpretation shown before acting, checks, approval, read-back verification, undo.
- Avoid claiming speed or accuracy we have not measured.

## 5. Similar products and how we differ

| Product | What it does | Difference from A-Commerce |
|---|---|---|
| OpenAI ACP / ChatGPT shopping | Open protocol (Apache-2.0) for agents to buy from merchants; product feeds into ChatGPT. Since Mar 2026 ChatGPT focuses on discovery and hands checkout to merchants | Shopper side, outside the store. Nothing about running the store |
| Google UCP / Universal Cart | Shared cart and checkout across Search, Gemini, YouTube | Same: a channel for outside agents |
| Shopify Sidekick | Assistant in the Shopify admin: proposals (Pulse), multi-step tasks with approval, skills, background work | Closest on the operator side, but sits on top of Shopify's admin screens and works only with Shopify |
| Genstore (seed $10M, Sep 2025) | Merchants run a whole store by chat with a team of agents (design, product, launch, analytics); builds a store from a description | Closest pitch. Aimed at new small stores; nothing on large existing back-ends, change review, or measured tool use |
| Saleor (Feb 2026) | Calls itself "the first fully open, end-to-end AI-native commerce stack": ACP entry, conversational storefront (Ink AI), config as files, AI skills | Open-source and agent-ready, but agents still call the regular GraphQL API; no operator agent or change sets |
| Salesforce Agentforce Commerce, VTEX AI Workspace | Merchant, shopper, and buyer agents on enterprise platforms | Vendor-locked, closed models; agents layered on existing platforms |
| Thesys C1 and other GenUI toolkits | Generative UI components for shopping agents | UI layer only |

Where A-Commerce is different:

1. **It starts from the back-end.** We redesign a large commerce back-end into an AI-friendly tool layer (the "A2API" idea) and measure that a small open model can use it. Others either expose the existing API or build a new small-store product.
2. **Change sets.** Operator work is reviewed as a change set with interpretation, checks, preview, read-back, and undo. Sidekick has approval; nobody we found treats store changes like reviewable pull requests.
3. **One tool layer for every agent.** Our operator agent, our shopper agent, and outside agents (ChatGPT, Gemini through ACP/UCP-style endpoints) use the same tools and policies.
4. **Open weights, on our own servers.** Muse Glimmer runs inside the company, so catalog and customer data stay in-house.

Sources: [OpenAI ACP docs](https://developers.openai.com/commerce), [OpenAI, Buy it in ChatGPT](https://openai.com/index/buy-it-in-chatgpt/), [Forbes on Genstore](https://www.forbes.com/sites/tanyaakim/2026/03/26/genstore---ai-agents-now-run-your-entire-e-commerce-store/), [Saleor](https://saleor.io/blog/end-to-end-agentic-commerce), [Thesys](https://www.thesys.dev/solutions/ecommerce).

## Sources

- OpenAI Instant Checkout and ACP: [TechCrunch, Sep 2025](https://techcrunch.com/2025/09/29/openai-takes-on-google-amazon-with-new-agentic-shopping-system/), shift in Mar 2026: [Digital Commerce 360](https://digitalcommerce360.com/2026/03/06/openai-shifts-checkout-plans-agentic-commerce-strategy/), [Checkout.com](https://cfworker-www.checkout.com/blog/openai-agentic-commerce-shift)
- Google UCP: [PhocusWire](https://www.phocuswire.com/google-universal-commerce-protocol-agentic-ai-mcp); Universal Cart: [Search Engine Journal](https://www.searchenginejournal.com/google-announces-new-universal-cart-at-i-o/575301/), [PPC Land](https://ppc.land/googles-universal-cart-wants-to-follow-you-everywhere-you-shop/)
- Amazon Alexa for Shopping: [Digital Commerce 360](https://www.digitalcommerce360.com/2026/05/13/amazon-launches-ai-agent-alexa-for-shopping/), [eMarketer](https://www.emarketer.com/content/amazon-alexa-for-shopping-rufus-integration)
- Naver AI shopping agent: [디지털데일리](https://www.ddaily.co.kr/page/view/2026042416082146478), [파이낸셜뉴스](https://www.fnnews.com/news/202604300951240870)
- Shopify Editions: [The Fintech Times](https://thefintechtimes.com/shopify-unveils-agentic-commerce-era-with-winter-26-edition/), [Coalition Technologies, Spring '26](https://coalitiontechnologies.com/blog/shopify-editions-spring-26)
- Salesforce Agentforce Commerce: [Salesforce, Jul 6 2026](https://www.salesforce.com/ap/news/press-releases/2026/07/06/as-ai-agents-transform-commerce-salesforce-unleashes-its-biggest-agentforce-commerce-release-yet/)
- VTEX: [VTEX Vision 2026](https://www.vtex.com/es-mx/press/vtex-vision-ai-native-commerce-suite-2026)
- Shopify MCP UI: [Shopify Engineering](https://shopify.engineering/mcp-ui-breaking-the-text-wall); MCP Apps vs Apps SDK: [mcp.directory](https://mcp.directory/blog/mcp-apps-standard-vs-openai-apps-sdk-2026); A2UI: [The Decoder](https://the-decoder.com/google-launches-generative-ui-standard-for-ai-agents/)
- Google Generative UI: [Google Research blog](https://research.google/blog/generative-ui-a-rich-custom-visual-interactive-user-experience-for-any-prompt/), [arXiv 2604.09577](https://arxiv.org/pdf/2604.09577), critique: [Search Engine Journal](https://www.searchenginejournal.com/googles-generated-interfaces-could-compete-con-tool-pages/586589)
- Daydream: [RetailBoss](https://retailboss.co/why-fashions-biggest-brands-are-betting-on-daydreams-shop-by-chat-ai-revolution)
- Trust: [Digital Transactions (Riskified)](https://www.digitaltransactions.net/a-survey-shows-a-need-to-bolster-trust-in-agentic-commerce/), [Visa](https://corporate.visa.com/en/sites/visa-perspectives/innovation/visa-trust-index-trust-powers-agentic-commerce.html)
