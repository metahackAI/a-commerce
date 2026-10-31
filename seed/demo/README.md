# Demo seed: Northwind Apparel

Named, static data for the demo store in [docs/scenarios.md](../../docs/scenarios.md). The files hold
every row a scene names. The generator ([generate.yml](generate.yml)) adds the bulk rows (more products,
members and sales history) under fixed counts and a fixed random seed.

## Files and load order

| File | Holds |
|---|---|
| `01_platform.yml` | Store `store_northwind` (USD, America/New_York), operator `adm_sj` with roles `rol_merchandiser` and `rol_catalog_admin` (B4 saves and publishes drafts) and their scopes, the other system roles, store policy overrides, the demo clock (Fri Nov 6 2026 11:00), change sets #1039 (Fall layers banner, live), #1040 (below-cost fix, live, undo until Nov 7 23:59) and #1041 (linen reorder draft) with previews, items, events and action log rows, the five home tasks of B5, and scheduled jobs; organizations (Northwind Apparel, Merchandising, Store operations, vendor Fieldstone Boot Co.), operators `adm_mina` (operator) and `adm_jordan` (suspended), the back-office menu with SJ's favorites, API keys `key_31` (outside shopping agent, active) and `key_27` (revoked), stored only as hashes |
| `02_customer.yml` | 13 named shoppers (Jamie vip; Alex member; the customers behind the B2 returns), home addresses, program `prg_rewards` with tiers `member`, `gold`, `vip` and its saved version, memberships, Jamie's four remembered preferences, consent history, Jamie's and Alex's tier history with two past evaluations, employee `emp_041` (Owen), three inquiries with threads (`inq_771` Alex's boots, received), remedy `rmd_310` (requested), 8 FAQs and 3 notices (`ntc_holiday` is a draft) |
| `03_catalog.yml` | 3 brands, 2 return policies, the 18 named products with all 127 variants and offers (list price, cost, stock), measurements (windbreaker M; wool overcoat M and L), 41 wool overcoat reviews, 5 product questions (3 answered by the store; `qst_312` private, `qst_313` waiting for an answer) and 4 restock alerts on the sizes sold out Wed 15:00 (Jamie waits for the chore coat M by email, `rsa_204`) |
| `04_promotion.yml` | The six promotions of §4, autumn knitwear sale prices of the cashmere sweater, the birthday and Gold tier coupons, three used birthday coupons |
| `05_merchandising.yml` | 24 standard and 3 display categories, placement of the named products, the four main-page slots, hero banners and bookings (October edit ended 09:59; Fall layers for everyone from 10:00) |
| `06_sales.yml` | 21 named orders with items, payments and events |
| `07_fulfillment.yml` | Deliveries of those orders, the 12 B2 return requests (`requested`), cus_4431's three completed returns with inspections and refunds, and the three delivery zones of `list_shipping_zones` (contiguous US with the policy's default values; Alaska and Hawaii, +$12.00; Puerto Rico and US territories, +$15.00) |
| `08_messaging.yml` | Refund notices for cus_4431's completed returns; the weekly email template `tpl_weekly_email` and its sends: four sent Thursdays (Oct 8–29) and `cmp_weekly_1105`, failed Thu Nov 5 08:00 (list import failed) |
| `09_supplier_sheet.yml` | Scene B4's supplier sheet `upl_fw26_supplier` (`fw26_supplier.xlsx`, 30 rows); the scene imports it |
| `generate.yml` | Spec for the generator (not data) |

Load the files in name order, and the tables inside each file in the order written; foreign keys are
satisfied in that order. Run everything in one transaction with
`SELECT set_config('app.tenant_id', 'store_northwind', true)` first, since every table forces row-level
security. Then run the generator.

Row format: keys are exactly the column names in `migrations/*.sql`. A missing key takes the column
default (usually NULL), so `created_at`, `updated_at` and `version` are never written. Every row has
`tenant_id: store_northwind`, except `platform.online_store`, whose `id` is the tenant. Money is a
plain number (`89.00`). Times are quoted RFC 3339 strings with the store offset (`-04:00` until
Nov 1 02:00, then `-05:00`). `jsonb` columns are YAML maps and lists.

## Seed state and scene replays

The seed is the store **after Fri Nov 6, 10:40**. Alex's request from scene F5 (sent at 09:50) is already
`clm_20402`, so scene B2 sees all 12 requests. A replay of F5 starts by removing that claim and its
event rows (the scene setup in `acom-testkit`).

## Scenario numbers → rows

| Scenario number | Rows |
|---|---|
| 42 summer outerwear products | 7 named in `03_catalog.yml` + 35 generated |
| 40 of 42 in the VIP sale | chore coat (7) and rain parka (4) are under `promo_min_stock` 10; all 35 generated have ≥ 12 units |
| About 400 products, 24 categories | 18 named + 382 generated; 24 standard `cat_` terms in `05_merchandising.yml` |
| VIP 1,284 / Gold 2,900 / Member 9,816 | named 1 / 3 / 9 in `02_customer.yml` + 1,283 / 2,897 / 9,807 generated |
| Stock: field jacket 86, windbreaker 214 (M Slate 18, L Slate 12, M Black 9), overshirt 53, utility 31, chore coat 7 (S 3, XL 4), rain parka 4 (S 2, XL 2), camp overshirt 40, overcoat 24, boots Tan 9: 6, cashmere 18, fleece 30, merino tee 45, linen shirt 38, denim jacket M·L 0 | `catalog.offer.inventory_level` per variant |
| Chore coat, rain parka, denim jacket M·L sold out Wed Nov 4 15:00; chore coat and denim back Nov 12 | `sold_out_at`, `availability_starts` on those offers |
| Overshirt floor $66.70 (20% would be $63.20) | `cost_value` 58.00 on the overshirt offers |
| Four below-cost products raised at 08:31, undo until Nov 7 23:59 | new list prices (cost × 1.15) on the offers; the old prices are the `from` of the nine `pv_floor_*` previews in change set `cs_1040`; `task_below_cost` |
| Fall layers banner for everyone since Fri 10:00; hero free for VIP on Nov 7–8 | `dsp_fall_layers` (`cs_1039`); no tier booking exists |
| 4 running promotions, denim bundle scheduled, linen clearance ended | `promotion.promotion` |
| Jamie: vip, Size M, Natural fabrics, Compares specs, Travels light, Home address, card 4821 | `cus_jamie`, `mem_03/07/09/11`, `adr_home` |
| Jamie's 6 past orders, the two latest in M, windbreaker M Black delivered in September and kept | #20105, #20151, #20188, #20236, #20268 (windbreaker M Black, delivered Sep 11), #20381 (merino tee M) |
| Alex: member, #20402 boots delivered Oct 31, card 0937 | `cus_alex`, `ord_20402`, `dlv_20402` |
| Fit check: windbreaker M 57/47/64/71; overcoat M 56/46/64/98, L 59/48/65.5/100 | `catalog.measurement` |
| 41 overcoat reviews (175–182 cm): 29 true to size, 12 a bit snug, 9 "size up for a hoodie", rating 4.6 | `rev_8772` … `rev_8812` (the sample is `rev_8812`) |
| B2: 12 requests over $200, oldest Nov 4 09:12 | 12 claims in `requested`; `clm_20333` is the oldest |
| 9 refunds $2,463.00 (hidden 5: $1,389.00), 2 photo requests, 1 decline | refunds: #20402 #20431 #20418 #20377 #20333 #20341 #20352 #20388 #20395; photos: #20366 #20409 (damage, no photo); decline: #20290 (39 days) |
| 3 coupon orders | #20352, #20388, #20395 used `BIRTHDAY10`; the $10.00 discount sits on a line that is not returned, so the refund equals the claim amount |
| #20431's customer returned 4 times in 90 days | `cus_4431`: `clm_20298`, `clm_20311`, `clm_20324` (completed) + `clm_20431` |
| Days since delivery 6 / 4 / 7 / 9 / 2 / 39 | `delivered_at` of the matching `parcel_delivery` |
| B5 tasks | `task_returns_1106`, `task_linen_reorder`, `task_size_charts`, `task_below_cost`, `task_fall_layers` |
| Demo clock Fri Nov 6 11:00 | `platform.demo_clock` |
| Discount caps, refund threshold, return window | defaults in `policies/*.yml`; only `max_discount.by_tier.vip: 0.30` and `watchers.price_below_cost.run_fix: standing_approval` differ, so only those are in `platform.policy_override` |
| B3: Nov 5 $18,240 / 201, Thursday average $22,900 / 247 | generated orders, `generate.yml` `orders.daily_sales` |
| B3: sold-out sizes −$2,700 (10,800.00 over the four compare Thursdays) | `sold_out_at` on the six M/L offers + `generate.yml` `b3_causes.out_of_stock` |
| B3: weekly email not sent −$1,300 | `cmp_weekly_1105` (failed) here; orders from the four sent emails carry `campaign_id` (1,240 + 1,410 + 1,180 + 1,370 = 5,200.00), `generate.yml` `orders.campaign_orders` |
| Linen shirt: 38 left, 61 sold in 7 days | offers here + `generate.yml` `orders.linen_shirt_velocity` |

## What the scenes create (not seeded)

- Change sets #1042 (B1 VIP sale) and #1043 (B2 returns), their previews, the VIP prices, `cpn_vipweekend20`, `dsp_1042`, and the three B2 messages.
- `clm_20402` is the result of F5 (Alex sends the request at 09:50; it is stored at 09:52). It is seeded because B2 at 10:40 needs it; a replay of F5 itself starts by removing `clm_20402` and its item and event.
- `task_returns_1106` is seeded as `working` with no change set; the B2 run attaches `cs_1043` and moves it to `needs_you`.

## Values chosen here (not in scenarios.md)

Family names, addresses and cards of the shoppers other than those in §2; colors and per-variant stock
splits; brand names Fieldstone Boot Co. and Ridgeline Outdoor; return policy `mrp_free_size_exchange`
for the overcoat; birthday coupon $10.00 off; denim bundle $20.00 off; linen clearance 20%; the earlier
hero banner "The October edit"; reorder split S 70 / M 90 / L 80; per-day revenue for Oct 23 – Nov 4
in `generate.yml`; the weekly email (template text, recipient counts 11,204–11,270, sends at 08:01–08:02) and the
split of its 5,200.00 over the four Thursdays (1,240 / 1,410 / 1,180 / 1,370).

## Open points

- Order numbers: §7 wants numbers to increase with order date across all orders, but B2's #20402 (Oct 27)
  and #20431 (Oct 29) leave no room for the several hundred daily orders B3 needs in between.
  `generate.yml` keeps named numbers and numbers only generated orders in date order.
- #1041 has no items: the docs define no reorder (purchase order) operation yet, so the draft holds the
  request and its interpretation only.
