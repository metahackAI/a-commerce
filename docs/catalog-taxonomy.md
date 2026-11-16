# Product taxonomy and attributes (draft)

[README](../README.md) › [docs](README.md)

How the platform describes products from any vertical (apparel, electronics, food) so that the shopping agent can turn a shopper's words into exact filters, merchants enter products in the same shape in the back office, and the same facts go out as schema.org and feed data. The draft policy files are in [policies/taxonomy/](../policies/taxonomy/README.md). Nothing reads them yet; this document is for review before any code.

Status: draft of 2026-11-12. Items marked *to verify* need a primary source checked before they become rules.

## 1. Why

- The store sells more than clothes. Electronics and food are large categories, and each asks different questions: "256 GB or more", "fits my phone", "no peanuts", "cheapest per 100 g".
- Today the catalog has closed values only for `fit` and `sleeve_length`. Everything else is free text (`closure` has 43 forms, `product_type` 177, `fabric` 101) or tags that a model picked. In 609 products with fact cards, 225 (37%) are missing at least one tag that their facts support, because the model chose 4–7 tags from the allowed list. Filters built on that data miss products.
- Search filters are fixed to category, tags, brand, color, size, price and availability. A shopper's "light", "rainy commute" or "gray" has no field to land in. "gray" does not find Slate or Charcoal, and Korean words are dropped by the tokenizer.
- Evidence that structured attributes help: turning queries into attribute filters raised exact match by about 9.6% in an Amazon study (CIKM 2024); web shopping agents fail most often on filter constraints (DeepShop, 2025, best agent 32% task success); Etsy raised attribute coverage from 31% to 91% with LLM extraction and saw more filter use and conversion. No study measures end-to-end gains for LLM shopping agents, so we measure our own (§9).

## 2. Principles

1. **Vertical knowledge lives in data only.** Search, the agent, back-office forms and JSON-LD know value kinds (§4), not verticals. Adding a vertical means adding policy files, words and test queries, not code.
2. **One definition per attribute.** An attribute and its value list are defined once; taxonomy nodes pick the attributes they use, and children inherit from parents.
3. **Short value lists.** About 20 values per list. Long-tail words ("cinnamon", "melange") go into `words`, which feed the synonym dictionary and the agent's mapping. Big lists are useless as filters: one public taxonomy has 22,366 color values.
4. **One language in the data.** Definitions and values are English; other languages are handled on the query side and by translation files when a store needs them (§4.3).
5. **Facts come from rules, merchants or people.** The demo generator fills values by rule. In merchant intake the model only suggests values with a confidence and a source sentence, and a person confirms. Every value records where it came from.
6. **Unknown is not none.** An empty value never matches a "without" filter.
7. **Claims with legal or safety weight need evidence.** Allergens, KC certification, energy grade, IP rating, organic and similar values open only with a certification record.
8. **Use the outside world's lists where they exist.** Gender, age group and size system use the lists that Google Merchant Center, OpenAI's product feed and schema.org share, so export is a rename.
9. **Measure.** A set of shopper questions with expected results is run before and after every taxonomy change.

## 3. Three layers

| Layer | Who decides | Where |
|---|---|---|
| Platform taxonomy and attribute dictionary | The platform, versioned | [policies/taxonomy/](../policies/taxonomy/README.md) |
| Store categories | Each store | `merchandising.defined_term`: the `standard` set links to one taxonomy node; the `display` set stays for merchandising |
| Product values | Merchants, rules, people | `additionalProperty` on `catalog.product_group` and `catalog.product` |

The database already fits: categories have a standard and a display set with a parent link (migrations 0006, 0017), `varies_by` accepts any axis (storage, flavor, pack size), and `additional_property` stores `[{name, value, unit_code}]`. The only new database pieces are a link from a standard category to a node, and later stock lots for food (§8).

Each node in [nodes.yml](../policies/taxonomy/nodes.yml) has an English label, a parent, required and optional attributes, variant axes, product types, whether KC certification applies, its Korean product-notice group, and its Google product category.

We write our own nodes and map them to outside taxonomies instead of importing one. The largest public taxonomy has 6,212 electronics nodes and 4,311 food nodes, and still has no numeric attribute for storage, RAM, screen size, battery capacity or wattage.

## 4. Value kinds, search and languages

### 4.1 Value kinds

| Kind | Example | Stored as | Search | Back-office input |
|---|---|---|---|---|
| `one`, `many` | resolution, connectivity, diet | words | equals, contains | pick from list |
| `scale` | thickness, spiciness, warmth | word; order in policy | range (`at_least`, `at_most`) | ordered list |
| `number` | screen 6.7 in, storage 256 GB, 500 mL | one canonical unit; the original input is kept | range; facets use policy `bands` | number + unit |
| `identifier` | GTIN, model number | normalized text | exact, ignoring hyphens, spaces, case | format and check digit |
| `compatibility` | case fits device `nova_x5` | ids of products or device models | contains | search and pick |
| `evidence` | KC number, energy grade, organic | `{scheme, code, rating, issued_by, valid_until}` | opens other values; ratings can filter | number, issuer, expiry |
| `tri_state` | allergens | per item: contains, may contain, free from | "without X" matches only free from | one of three per item |
| `date`, `text`, `computed` | release month, ingredients, unit price | as named | text is not searched; computed is sortable | text is entered; computed is never entered |

Number rules: the policy names a canonical unit per attribute (GB, in, W, mAh, g, mL, cm) and stores UN/CEFACT unit codes, which schema.org `unitCode` uses. "256 GB or more" becomes `storage >= 256`; with no result, the search widens one band and says so.

### 4.2 Data that helps search

How search reads a product today (origin/main `af8fd95`): Postgres full text in English with weights name A; brand, category names and tags B; material C; description D. Filters cover category, tags, brand, color, size, price and availability. Query words are ASCII only, no embedding model is connected, and writes do not refresh the search document.

A product helps search when:

| # | Shape | Why it helps | How we check it |
|---|---|---|---|
| 1 | Every required attribute has a closed value | A missing value drops the product out of every filtered result | Search readiness: required coverage per node, 100% for demo data |
| 2 | Each value has one meaning, a `means` sentence and synonyms in `words`; lists are short | The agent maps a shopper's words to the right value; facets list only values that exist | No two values with the same meaning; every value has `means` |
| 3 | Specs are numbers in one canonical unit | Ranges, sorting and comparison work; "256 GB or more" is a query, not a text guess | Unit and range checks on write |
| 4 | The name contains what shoppers call the thing: key attribute, material, product type ("Packable nylon rain jacket") | The name has the highest weight; a name without the product-type noun cannot rank for it | The node's product-type word appears in the name |
| 5 | A search text is generated from the attribute values by a fixed template and indexed at weight B in place of model-picked tags ("rain jacket, water-resistant, warm, relaxed fit, navy, nylon, city commute, fall, winter") | Text search and filters read the same values, so they agree, and nothing is skipped the way tags were (37% of products missed a true tag). The same text feeds embeddings later | Built by code from values; never written by a model |
| 6 | The description is factual and consistent with the values; its first sentence names the product type and main use | It is weight D: it supports ranking and answers questions, but does not drive filters | Contradiction checks against the values |
| 7 | Variant facts sit on the variant (color family, storage, flavor) | "Navy" finds the navy variant, and the card shows it | Values at the level the policy names |
| 8 | Identifiers are exact (model number as one token, GTIN with check digit) | Shoppers who type a model number and miss conclude the store does not carry it | Format and check digit |
| 9 | Compatibility and accessory links are ids | "A case for the Nova X5" is a lookup, not a text guess | Links point to existing models |
| 10 | Ranking signals come from data: stock, ratings, sales, newness | They order results without stuffing words into text | Already in the search policy |

What hurts: free-text values in many spellings, marketing names without the product noun, keyword stuffing, mixed units, missing values, text that contradicts the values, and near-duplicate values ("relaxed" and "loose" as two values).

How we know it helped: the scene tests (§9) and a broader question set are measured before and after each change, with three numbers: the zero-result rate, the share of expected products in the top 10, and wrong products shown. The ranks the current Northwind scenes depend on must not move.

### 4.3 Languages

- **Definitions and product data use one language.** Policy definitions are English only: codes, labels, `means` and synonyms. Values are language-free codes (`fit: relaxed`). A product's name and description are written in its store's language only.
- **The query side does the language work: translate the query, not the catalog.**
  - The agent reads the shopper in any language and calls the tools with English codes and terms; tool descriptions say so. It maps words to values through `means`, which is why every value needs one.
  - The storefront search box, which has no agent, gets a query-understanding step that rewrites the query into English terms and filters. The search policy already names `expansion: [llm, synonym_dictionary]`; only the dictionary is built. Rewrites are cached by query text.
  - Later, a multilingual embedding model can match meaning across languages without a rewrite. None is connected today.
- **Display in another language** comes when a store in that language exists: one translation file per language next to the definitions holds the labels of attributes and values. A check lists missing entries, and pages fall back to English. That store's product text is written in its own language.
- **The Korean notice is the exception.** Its item names are the legal Korean wording, so [notice.yml](../policies/taxonomy/notice.yml) stays Korean as a Korea-only module, together with the Korean names of food types.
- **Today's gap:** the tokenizer keeps only ASCII, so a Korean query sent straight to search finds nothing ("티셔츠" returns zero results). The agent rule and the query-understanding step close that gap without storing a second language.

## 5. Modules

### 5.1 Apparel

[apparel.yml](../policies/taxonomy/apparel.yml). Fit, warmth, thickness (five steps, as Korean fashion stores show), stretch, sheerness, hand feel, fabric, lining, layer, pattern, use, season, water resistance, performance, details, care, fiber content and foot length. Season is stored as the four seasons; "transitional" and "all year" are phrases that map to several values. `waterproof` is not a value until a test record can back it. Category-only attributes (sleeve, neckline, closure, rise, leg shape and length, skirt length, shoe height, bag carry) come in the next draft. Color families (16, including navy and khaki) are in [common.yml](../policies/taxonomy/common.yml), with a table from our 18 color names.

The demo values come from the generator's vocabulary rules, not from model-chosen tags. A trial over 991 generated products filled `closure_type` for all 991 (43 free-text forms became 14 words), `material_family` for 852, `stretch_level` for 793 and `thickness` for 683.

### 5.2 Electronics

[electronics.yml](../policies/taxonomy/electronics.yml). Release month, rated voltage, power, screen size, resolution, refresh rate, storage, memory, OS, battery capacity, output power, ports, cable length, connectivity, connector, charging standard, compatible devices, noise cancelling and form factor. Demo nodes: smartphones (unlocked only), laptops, headphones, chargers, power banks, cables and phone cases.

- Specs are numbers with units, because the Korean notice asks for screen specs, capacity and main specs, and shoppers ask in ranges.
- Compatibility has two levels: connectors and charging standards are closed lists; "fits Nova X5" links to a device-model list. Sites with compatibility filters had almost twice the task success on compatibility-dependent products in Baymard's testing (secondary summary).
- Model numbers are indexed as one token; shoppers who type a model number and miss conclude the store does not carry it.

### 5.3 Food

[food.yml](../policies/taxonomy/food.yml). Net quantity, unit price (computed), food type, ingredients, allergens, diet, free-from claims, nutrition, nutrition claims, storage temperature, shelf-life rule, spiciness, flavor and GMO labelling. Demo nodes: snacks, nuts, protein bars and instant noodles, all shelf-stable.

- **Allergens have three states.** "No peanuts" finds only products that declare peanut as free from; undeclared or "may contain" products are left out and the answer says how many. The list follows Korea's mandatory list and lives in the policy, because the regulator changes it: a draft of 2026-08-07 adds sesame, perilla seed, almond and cashew (*to verify*: the current list text and the date).
- Demo allergens come only from an ingredient-to-allergen table, never from a model. In the back office, a person picks each allergen state; model suggestions are not pre-filled.
- Diet, free-from and nutrition claims are three lists. Diet values like halal or organic need a certificate; nutrition claims need the labelling threshold (*to verify*).
- Nutrition follows Korea's nine mandatory nutrients with their basis (per 100 g or per serving). Generated data must pass an energy check: calories within 15% of 4 × carbohydrate + 4 × protein + 9 × fat.
- Unit price is computed by the platform from price and net quantity, never by the web.

## 6. Korean product notice

The product page's "product information notice" table is built from a template per notice group in [notice.yml](../policies/taxonomy/notice.yml): clothing (1), shoes (2), bags (3), accessories (4), office machines and laptops (10), small electronics (12), mobile devices (13), processed food (21) and other goods (40, *to verify*). Each item names the attribute, product field or store setting it comes from. Warranty terms, after-sales contact and consumer phone come from store settings. When a store turns on the Korean notice, the template's items join its publish requirements.

KC: since 2025-10-01 the electrical and household products safety act requires online sellers and marketplaces to post certification information on the product page. A marketplace is exempt when it requires the information at listing, shows it to shoppers and removes unmarked listings at once. Our back-office flow (§7) is built to be exactly that. *To verify*: the fields the ministerial ordinance requires online.

## 7. Back-office entry

1. **Pick the node.** The model suggests three nodes from the name and photo; a person picks one.
2. **The form comes from the policy.** Required and optional attributes, list pickers with labels, number and unit fields, a device search for compatibility, three states per allergen, and certification records. Filter attributes have no free-text box; "not in the list" takes text into a review queue.
3. **Model-assisted fill.** From photos, spec sheets, food labels, supplier sheets and existing text, the model proposes values with a confidence and the sentence it read. Nothing reaches search filters until a person confirms. Allergens and certifications are never pre-filled.
4. **Checks** run in the write's `preview=true`: allowed values, unit ranges (screen 1–120 in), GTIN check digit, contradictions with the description, notice items, evidence.
5. **Search readiness.** Each product gets a score from required, recommended and evidence fields, shown in the back-office list so the lowest can be fixed first.
6. **Bulk import** maps sheet columns to attributes, values to words and units to canonical units, and keeps that mapping per supplier.
7. **Store-only attributes** may exist under a `store.` prefix for display. To become filters they map to a platform attribute or go to the platform as a proposal.
8. **The catalog agent** writes through the same checks and change sets. Its prompt carries no attribute lists; tool descriptions and `get_facets` are generated from the policy.

## 8. Needs outside attributes

| Vertical | Need | Today |
|---|---|---|
| Food | Use-by date per stock lot, first-expiry-first-out | Stock is per variant; no lots |
| Food | Room, chilled and frozen shipping | No distinction (the demo sells shelf-stable food only) |
| Electronics | Block publishing without KC | Not in `publish_requires` |
| Electronics | Installation fee and booking for large appliances | Out of the demo |
| Alcohol | Online sale | Out: Korean rules on online liquor sales are strict (*to verify*), and Google Shopping does not list South Korea for alcohol |

## 9. Showcase scenes

The demo shows that one platform handles several verticals. The scenes come first; the attributes in this draft are the ones they need, plus legal and safety items. Each scene becomes a test with products that must and must not appear.

| # | Vertical | What a shopper or operator says or does | What it proves |
|---|---|---|---|
| 1 | Apparel | "A warm jacket for a rainy commute, relaxed fit" | everyday words → several filters |
| 2 | Apparel | "A thin navy linen shirt" | scale range and color family |
| 3 | Apparel | "Pants for between seasons that aren't see-through" | a phrase → several values |
| 4 | Electronics | "A phone with 256 GB or more, under 6.5 inches, under $700" | several number ranges |
| 5 | Electronics | "A case for the Nova X5" | exact compatibility |
| 6 | Electronics | "A USB-C charger, 65 W or more" | number and connector |
| 7 | Electronics | Types the model number "NX5-256-BK" | exact identifier match |
| 8 | Electronics | "Compare these two laptops" | spec table from the policy's compare attributes |
| 9 | Food | "Snacks without peanuts" | three-state allergens; says what it left out |
| 10 | Food | "The cheapest nuts per 100 g" | computed unit price and sort |
| 11 | Food | "A vegan protein bar with more than 20 g protein" | nutrition range and an evidenced diet value |
| 12 | Food | "Instant noodles that aren't too spicy" | scale |
| 13 | Several | "Pack for a weekend camping trip" | one question across apparel, electronics and food |
| 14 | Product page | Open an electronics, a food and a clothing page | the notice table changes by group |
| 15 | Back office | A seller uploads a charger spec sheet | model fills values with sources; a person confirms |
| 16 | Back office | Publish without a KC number | blocked, with the reason and the next tool |
| 17 | Back office | Upload a snack label photo | allergens are left for a person to pick |
| 18 | Back office | "Show products with low search readiness" | data quality is something an operator manages |

## 10. Data and models

- **Showcase store.** Northwind (apparel, English, USD) stays as it is; eval and the current scenes use its products. A second seeded store sells apparel (copied from Northwind), electronics and food, with its own `ACOM_STORE_ID` on the same images and database. Seed resets reload both, since both are seed data. Making it a multi-seller mall shows partner onboarding and the KC check (scene 16).
- **Facts by generator, text by model.** Each vertical module gets deterministic fact rules and consistency checks: energy against macros and allergens from ingredients for food; specs against each other and against price for electronics; the current trait checks for apparel. Brands and models are fictional and checked against a block list; photos carry no logos. Fake certificate numbers are marked as synthetic in provenance, and synthetic content stays out of search engines and feeds.
- **First about 1,000 products with the server model (Glimmer), then Haiku.**
  - Pilot: 30 per vertical, all read by a person. Glimmer has only written apparel so far.
  - Then about 500 electronics and 500 food products. The 2026-11-04 run wrote 628 products in 0.71 hours with 615 passing, so 1,000 take about 1–2 hours of model time, at low concurrency on the shared server.
  - Haiku cross-checks Glimmer's 1,000 against their fact cards (under $1), because a model judging its own text misses its own mistakes.
  - A blind comparison of 50 fact cards written by both models decides whether mixed styles show. Rewriting the 1,000 with Haiku costs about $1–3.
  - Haiku takes over for larger runs.
- **The writing tool is model-agnostic.** The model is one setting; prompts, fact cards, checks and output are the same. Results are stored by fact-card hash, because Haiku has no seed and fixes temperature. Length limits are checked in code, because structured output does not enforce them. Provenance records the model per product. The tool moves into this repository; today it is outside it.

## 11. Steps

1. Agree on the scenes (§9).
2. Agree on this draft and the policy files; verify the items marked *to verify*.
3. Build the engine, vertical-agnostic and spec first: policy loader and checks, node link, value checks on write and publish, search filters by value kind, facets and relaxation, unit price, JSON-LD export map, notice table, back-office form. Done when the 18 scenes pass on small test data.
4. Data, in parallel with step 3 once step 2 is done: generators and the pilot of 30 per vertical, then about 1,000 with Glimmer.
5. Load the showcase store and run the scene tests.
6. Larger runs with Haiku.

## 12. Open decisions

1. Policy shape: a `policies/taxonomy/` folder with a node tree and one file per module (this draft), or one file.
2. Showcase store language and currency. This draft assumes English and USD, like Northwind; Korean questions are handled on the query side (§4.3).
3. Device models for compatibility: fictional (this draft) or real device names.
4. Count per vertical: 300–500 (this draft) or more.
5. Gender defaults: unisex and adult for most nodes; dresses and skirts to decide.

## Sources

Read 2026-11-11.

- Shopify Standard Product Taxonomy (MIT): https://github.com/Shopify/product-taxonomy
- Google Merchant Center product data specification: https://support.google.com/merchants/answer/7052112
- schema.org Product, Certification, PropertyValue: https://schema.org/Product, https://schema.org/Certification, https://schema.org/PropertyValue
- Google merchant listing structured data: https://developers.google.com/search/docs/appearance/structured-data/merchant-listing
- GS1 Web Vocabulary: https://ref.gs1.org/voc
- 전자상거래 등에서의 상품 등의 정보제공에 관한 고시 (공정거래위원회고시 제2022-15호): https://www.law.go.kr/LSW/admRulInfoP.do?admRulSeq=2100000213455
- 전기용품 및 생활용품 안전관리법 (in force 2025-10-01): https://www.law.go.kr/LSW/lsInfoP.do?lsiSeq=276591
- 식품 등의 표시·광고에 관한 법률 시행규칙 개정안 입법예고 (2026-08-07): https://www.mfds.go.kr/brd/m_209/view.do?seq=44285
- Malik et al., "Building Natural Language Interface for Product Search", CIKM 2024: https://cdn.amazon.science/a1/97/116ffa7f44e28bcc4bcdfccaceee/building-natural-language-interface-for-product-search.pdf
- DeepShop, arXiv:2506.02839: https://arxiv.org/abs/2506.02839
