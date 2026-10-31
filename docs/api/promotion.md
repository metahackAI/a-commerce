# API 명세 — `promotion` (프로모션·회원 판매가·쿠폰·혜택 계산·사은품·적립금·이벤트·세그먼트)

> 계층: `promotion.{resource}.{operation}`. 공통 규약은 [README.md](README.md) §3과 [conventions.md](conventions.md), 값 목록은 [enums.md](enums.md), 에러는 [errors.md](errors.md), 데이터 타입은 [data-model.md](data-model.md). 정책은 [promotion.yml](../../policies/promotion.yml) · [catalog.yml](../../policies/catalog.yml) · [approval.yml](../../policies/approval.yml). 예시의 등급·상품·금액·시각은 데모 스토어([../scenarios.md](../scenarios.md))의 값이고, 변경셋 화면 블록은 [chat.md](chat.md)의 `change_set`입니다.
> **핵심**: 할인 로직을 주문에 심지 않는다. `calculate_benefits`(혜택 계산기)가 장바구니·주문 준비·검증의 **단일 기준**이다. 모든 쓰기는 먼저 프리뷰를 반환한다. 회원 등급·기간 한정 퍼센트 할인은 `set_sale_prices`, 그 혜택을 주문에 기록하는 것은 `create_coupon`이다.
> 비율: 툴 입력의 `percent_off`는 0–100, 정책의 상한은 0–1([conventions.md](conventions.md) §4). 할인 금액은 양수(빼는 금액)로 씁니다.

리소스: [promotion](#promotion) · [offer](#offer) · [coupon](#coupon) · [benefit](#benefit) · [conditional_gift](#conditional_gift) · [loyalty](#loyalty) · [event](#event) · [segment](#segment)

---

## promotion

프로모션 상태는 `draft` · `scheduled` · `running` · `paused` · `ended`([enums.md](enums.md) §1.11)이고, 기간(`valid_from` ~ `valid_through`, 둘 다 포함)과 데모 시계로 `scheduled → running → ended`가 계산됩니다. 상시 프로모션은 `valid_through: null`입니다. 할인 층 `discount_layer`는 `item_price`(회원가·기간가) · `order_discount`(주문 할인) · `add_on`(적립금·배송비)입니다.

### `list_active_promotions` — Kind: R  (데모 장면)
- 논리주소/REST: `promotion.promotion.list_active` · `GET /v1/promotion/promotions/active`
- 요약: 주어진 기간과 **겹치는** 진행 중·예정 프로모션과 각 프로모션의 적용 범위. `set_sale_prices`·`create_promotion`의 `promotion_overlap` 검사가 읽는 것과 같은 데이터라, 에이전트가 겹침을 미리 설명할 수 있습니다.
- 쇼퍼(비회원 포함)가 부르면 기간과 상관없이 **지금 진행 중인** 프로모션 중 모두 대상이거나 자기 등급 대상인 것만 돌려줍니다. 세일 페이지(`/sale/{slug}`)가 이것으로 세일을 찾습니다. `valid_for_member_tier`·`status`는 운영자만 씁니다(쇼퍼가 보내면 `forbidden`).
- consumes → produces: `tier_id` → `promotion_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | valid_from | DateTime | ✓ | 확인할 기간의 시작(포함). 오프셋이 없으면 스토어 시간대 | `"2026-11-07T00:00"` |
  | valid_through | DateTime | ✓ | 확인할 기간의 끝(포함, 분 단위). `valid_from` 이후 | `"2026-11-08T23:59"` |
  | status | enum[] | | `draft` \| `scheduled` \| `running` \| `paused` \| `ended`. 기본 `["running", "scheduled"]` | `["running", "scheduled"]` |
  | valid_for_member_tier | Text, `^[a-z][a-z0-9_]*$` | | 등급 id(`list_tiers`) 또는 `everyone`. 주면 그 등급에 닿는 프로모션만(그 등급 대상 + `everyone` 대상). 생략하면 대상 전부 | `"vip"` |
  | limit | Integer | | 1–100, 기본 20 | `20` |
  | cursor | Text | | 이전 응답의 `next_cursor` | — |
- 출력(Fri Nov 6, VIP 주말 세일 기간을 확인, 대상 필터 없음):
  ```jsonc
  {"items": [
     {"id": "promo_autumn_knit", "name": "Autumn knitwear 15%", "status": "running", "discount_layer": "item_price",
      "valid_for_member_tier": "everyone",
      "valid_from": "2026-11-01T00:00:00-04:00", "valid_through": "2026-11-15T23:59:00-05:00",
      "applies_to": {"scope": "categories", "category_ids": ["cat_knitwear"], "product_count": 26}},
     {"id": "promo_free_ship_80", "name": "Free shipping over $80", "status": "running", "discount_layer": "add_on",
      "valid_for_member_tier": "everyone",
      "valid_from": "2026-01-01T00:00:00-05:00", "valid_through": null,
      "applies_to": {"scope": "order", "category_ids": [], "product_count": null}},
     {"id": "promo_birthday", "name": "Birthday coupon", "status": "running", "discount_layer": "order_discount",
      "valid_for_member_tier": "member",
      "valid_from": "2026-01-01T00:00:00-05:00", "valid_through": null,
      "applies_to": {"scope": "order", "category_ids": [], "product_count": null}},
     {"id": "promo_gold_10", "name": "Gold tier 10%", "status": "running", "discount_layer": "order_discount",
      "valid_for_member_tier": "gold",
      "valid_from": "2026-01-01T00:00:00-05:00", "valid_through": null,
      "applies_to": {"scope": "order", "category_ids": [], "product_count": null}}],
   "total": 4}
  ```
  - `promo_denim_bundle`(scheduled, Nov 13–20)은 기간이 겹치지 않아 빠집니다. `applies_to.scope`: `products` \| `categories` \| `order`(주문 전체). `product_count`는 상품 범위일 때만.
- 프리뷰: 해당 없음(읽기)
- 정책: `promotion_overlap.compare_statuses`
- 에러: `invalid_period`(`valid_from`이 `valid_through`보다 늦음) → `list_active_promotions` · `tier_not_found` → `list_tiers`
- 멱등성: 읽기.

### `search_promotions` — Kind: R
- 논리주소/REST: `promotion.promotion.search` · `GET /v1/promotion/promotions`
- 요약: 상태·기간·대상 등급·층으로 프로모션을 찾습니다(겹침과 상관없이).
- consumes → produces: → `promotion_id[]`
- 입력: `status`(enum[] `draft` \| `scheduled` \| `running` \| `paused` \| `ended`), `valid_for_member_tier`(Text), `discount_layer`(enum `item_price` \| `order_discount` \| `add_on`), `valid_from` / `valid_through`(DateTime, 이 기간과 겹치는 것), `query`(이름), `limit`(기본 20), `cursor`
- 출력: `{"items": [{"id": "promo_linen_clearance", "name": "Linen clearance", "status": "ended", "discount_layer": "item_price", "valid_for_member_tier": "everyone", "valid_from": "2026-10-01T00:00:00-04:00", "valid_through": "2026-10-20T23:59:00-04:00"}], "total": 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_filter` → `list_enums`
- 멱등성: 읽기.

### `get_promotion` — Kind: R
- 논리주소/REST: `promotion.promotion.get` · `GET /v1/promotion/promotions/{promotion_id}`
- 요약: 프로모션 한 건과 적용 범위, 연결된 회원 판매가 수·쿠폰, 만든 변경셋.
- consumes → produces: `promotion_id` → `coupon_id[]`
- 입력: `promotion_id`(`^promo_`, ✓, 예 `"promo_1042"`), `fields`(Text[])
- 출력: `{"id": "promo_1042", "name": "VIP weekend", "status": "scheduled", "discount_layer": "item_price", "valid_for_member_tier": "vip", "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00", "applies_to": {"scope": "products", "category_ids": ["cat_summer_outerwear"], "product_count": 40}, "price_specification_count": 40, "coupon_ids": ["cpn_vipweekend20"], "change_set_id": "cs_1042"}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `not_found`(`detail.entity: promotion`) → `search_promotions`
- 멱등성: 읽기.

### `create_promotion` — Kind: W$
- 논리주소/REST: `promotion.promotion.create` · `POST /v1/promotion/promotions`
- 요약: 회원 등급 퍼센트 판매가 **이외의** 프로모션을 만듭니다(1+1, 묶음, 금액 조건, 정액 할인). **회원 등급·기간 한정 퍼센트 할인은 `set_sale_prices`를 쓴다.** 돈이 움직이므로 항상 사람 승인.
- consumes → produces: `product_id[]`, `category_id`, `tier_id` → `promotion_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | name | Text | ✓ | 장바구니 할인 줄 라벨 | `"$10 off $120"` |
  | promotion_type | enum | ✓ | `bogo`(n개 사면 m개 무료) \| `bundle`(묶음 가격) \| `threshold`(금액 조건 혜택) \| `amount_off`(정액 할인) | `"threshold"` |
  | discount_layer | enum | ✓ | `item_price` \| `order_discount` \| `add_on` | `"order_discount"` |
  | amount_off | MonetaryAmount | `amount_off`·`threshold`일 때 ✓ | 깎는 금액 | `{"value": 10.00, "currency": "USD"}` |
  | threshold | MonetaryAmount | `threshold`일 때 ✓ | 회원 혜택 전 소계 기준 | `{"value": 120.00, "currency": "USD"}` |
  | threshold_benefit | enum | `threshold`일 때 ✓ | `amount_off` \| `free_shipping` | `"amount_off"` |
  | buy_quantity / get_quantity | Integer | `bogo`일 때 ✓ | n개 사면 m개 무료 | `2` / `1` |
  | bundle_price | MonetaryAmount | `bundle`일 때 ✓ | 묶음 가격 | `{"value": 150.00, "currency": "USD"}` |
  | valid_from / valid_through | DateTime | ✓ | 기간(둘 다 포함) | `"2026-11-20T00:00"` / `"2026-11-30T23:59"` |
  | valid_for_member_tier | Text | | 등급 id 또는 `everyone`(기본) | `"everyone"` |
  | scope_mode | enum | ✓ | `products` \| `categories` \| `order` | `"order"` |
  | product_ids / category_ids | Text[] | scope_mode에 맞게 ✓ | 대상 | `["cat_denim"]` |
  | exclude_product_ids | Text[] | | 빼는 상품 | `[]` |
  | cost_share | object | | `{store, seller}` 비율(0–1), 합 = 1 | `{"store": 1.0, "seller": 0.0}` |
  | channels | enum[] | | `web` \| `mobile_web` \| `app` \| `agent`. 기본 전부 | `["web", "app"]` |
  | percent_off | Number | | 받지 않는 필드. 등급·기간 퍼센트 할인을 여기로 요청하면 `percent_off_for_tier`로 `set_sale_prices`를 안내하기 위해서만 있음 | — |
- 저장: 기준 금액은 `eligible_transaction_volume`, 정액은 `amount_off`로 저장해 `calculate_benefits`가 `threshold`·`amount_off`의 주문 할인과 무료 배송(`add_on`)을 계산합니다. `bogo`·`bundle`은 조건(`promotion_terms`)을 기록하며 혜택 계산기는 아직 이를 가격에 반영하지 않습니다. `bundle`은 `scope_mode: products`의 상품 둘 이상(각 1개)입니다.
- `max_discount`가 실패하면 `detail`에 그 할인율을 허용하는 등급을 `max_discount.by_tier`에서 읽어 적습니다(예: "25% is over the 20% limit for everyone; vip allows up to 30%"). 허용하는 등급이 없으면 앞 문장만 씁니다. `set_sale_prices`도 같습니다.
- 출력(프리뷰): `{"preview_id": "pv_p51a", "expires_at": "2026-11-06T12:10:00-05:00", "change_set_id": "cs_1051", "changes": [{"op": "create", "entity": "promotion", "id": null, "from": null, "to": {"name": "$10 off $120", "promotion_type": "threshold", "valid_from": "2026-11-20T00:00:00-05:00", "valid_through": "2026-11-30T23:59:00-05:00"}}], "checks": [{"rule": "max_discount", "passed": true, "detail": "At most 8.3% on a $120.00 order; limit 20%", "fix_tool": null}, {"rule": "price_floor", "passed": true, "detail": "No line goes under cost + 15%", "fix_tool": null}, {"rule": "promotion_overlap", "passed": true, "detail": "No order discount runs Nov 20–30", "fix_tool": null}, {"rule": "discount_layers.max_total_discount_rate", "passed": true, "detail": "Combined discount stays under 50%", "fix_tool": null}, {"rule": "cost_share.sum_must_equal", "passed": true, "detail": "Store 100%", "fix_tool": null}], "needs_approval": true, "approval_reason": "New order discount for everyone, Nov 20–30"}`
- 출력(실행): `{"ok": true, "id": "promo_1051", "undo": {"by": "person", "target": "cs_1051", "until": "2026-12-01T23:59:00-05:00"}}`
- 프리뷰: checks `max_discount`, `price_floor`, `promotion_overlap`, `discount_layers.max_total_discount_rate`, `cost_share.sum_must_equal`. 항상 `needs_approval: true`.
- 정책: `max_discount`, `promotion_overlap`, `discount_layers`, `cost_share.sum_must_equal`, `price_floor`([catalog.yml](../../policies/catalog.yml)), `thresholds.promotion_publish`([approval.yml](../../policies/approval.yml))
- 에러: `below_price_floor` → `create_promotion` · `stacking_conflict`(같은 층에 이미 같은 대상 할인) → `list_active_promotions` · `cost_share_sum`(합이 1이 아님) → `create_promotion` · `percent_off_for_tier`(등급 퍼센트 할인을 여기로 요청) → `set_sale_prices` · `tier_not_found` → `list_tiers` · `invalid_period` → `create_promotion`
- 멱등성: `Idempotency-Key`. 같은 `preview_id` 재실행은 첫 결과.

### `update_promotion` — Kind: W
- 논리주소/REST: `promotion.promotion.update` · `PATCH /v1/promotion/promotions/{promotion_id}`
- 요약: 프로모션의 이름·기간·범위를 고칩니다. `running`인 프로모션은 대상을 좁히거나 기간을 앞당기는 변경만 막지 않고, 혜택을 줄이는 변경은 `pause_promotion` 후에 합니다.
- consumes → produces: `promotion_id` → `promotion_id`
- 입력: `promotion_id`(`^promo_`, ✓), `name`, `valid_from`, `valid_through`, `product_ids`, `category_ids`, `exclude_product_ids` — 바꿀 것 하나 이상
- 해석: 입력에 할인액이 없으므로 진행 중인 프로모션도 이름·끝·범위를 고칠 수 있습니다. 다만 시작은 이미 지났으므로 옮기지 않습니다(혜택을 멈추려면 `pause_promotion`·`end_promotion`, `invalid_state`). 회원 판매가(`set_sale_prices`)를 가진 프로모션은 기간만 바뀌고(판매가 기간이 따라감) 상품 범위는 바꾸지 않습니다(`invalid_state` → `set_sale_prices`).
- 출력(프리뷰): `changes: [{"op": "update", "entity": "promotion", "id": "promo_1051", "from": {"valid_through": "2026-11-30T23:59:00-05:00"}, "to": {"valid_through": "2026-12-02T23:59:00-05:00"}}]`, checks `promotion_overlap`, `needs_approval: false` · 출력(실행): `{"ok": true, "id": "promo_1051", "undo": {"by": "person", "target": "cs_1052", "until": "2026-12-03T23:59:00-05:00"}}`
- 프리뷰: `update promotion`.
- 정책: `promotion_overlap`, `max_discount`
- 에러: `invalid_state`(`ended`) → `get_promotion` · `not_found`(`detail.entity: promotion`) → `search_promotions`
- 멱등성: `Idempotency-Key`.

### `pause_promotion` — Kind: W
- 논리주소/REST: `promotion.promotion.pause` · `POST /v1/promotion/promotions/{promotion_id}/pause`
- 요약: 진행 중인 프로모션을 멈춥니다(`running → paused`). 멈춘 동안 판매가·쿠폰이 적용되지 않습니다.
- consumes → produces: `promotion_id` → `promotion_id`
- 입력: `promotion_id`(`^promo_`, ✓), `reason`(Text)
- 출력(프리뷰): `changes: [{"op": "update", "entity": "promotion", "id": "promo_1042", "from": "running", "to": "paused"}]`, `needs_approval: false` · 출력(실행): `{"ok": true, "id": "promo_1042", "undo": {"by": "person", "target": "cs_1053", "until": "2026-11-08T23:59:00-05:00"}}`
- 프리뷰: `update promotion`.
- 정책: 없음(상태 전이는 [enums.md](enums.md) §1.11)
- 에러: `invalid_state`(`running`이 아님) → `get_promotion`
- 멱등성: 이미 `paused`면 같은 결과.

### `end_promotion` — Kind: W
- 논리주소/REST: `promotion.promotion.end` · `POST /v1/promotion/promotions/{promotion_id}/end`
- 요약: 프로모션을 기간 전에 끝냅니다(`scheduled | running | paused → ended`, 끝 상태). 연결된 회원 판매가는 그 시각부터 적용되지 않습니다.
- consumes → produces: `promotion_id` → `promotion_id`
- 입력: `promotion_id`(`^promo_`, ✓), `reason`(Text)
- 출력(프리뷰): `changes: [{"op": "update", "entity": "promotion", "id": "promo_1042", "from": "running", "to": "ended"}]`, `needs_approval: false` · 출력(실행): `{"ok": true, "id": "promo_1042", "undo": {"by": "person", "target": "cs_1053", "until": "2026-11-08T23:59:00-05:00"}}`
- 프리뷰: `update promotion`.
- 정책: 없음
- 에러: `invalid_state`(이미 `ended`) → `get_promotion`
- 멱등성: 이미 `ended`면 같은 결과.

---

## offer

### `set_sale_prices` — Kind: W$  (데모 장면)
- 논리주소/REST: `promotion.offer.set_sale_prices` · `POST /v1/promotion/sale-prices`
- 요약: 회원 등급·기간 한정 판매가를 상품별로 정하고 하나의 프로모션으로 묶습니다. **정가(list price)는 바꾸지 않습니다.** 상품의 변형 `Offer`마다 schema.org `UnitPriceSpecification`(`price_type: "sale_price"`, `valid_for_member_tier`, `valid_from`, `valid_through`, `acom:percent_off`)을 붙입니다. 판매가 = 정가 × (1 − percent_off / 100), 센트 단위 반올림(0.5 올림). 재고가 `promo_min_stock`보다 적은 상품은 기본으로 빼고 그 사실을 `exclude` 줄로 남깁니다. schema.org로는 `Offer` 가격에 대한 `UpdateAction`.
- consumes → produces: `category_id` 또는 `product_id[]`, `tier_id` → `promotion_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | name | Text | ✓ | 프로모션 이름. 장바구니 할인 줄 라벨이 됨 | `"VIP weekend"` |
  | valid_for_member_tier | Text, `^[a-z][a-z0-9_]*$` | ✓ | 등급 id(`list_tiers`) 또는 `everyone`. `x-aliases`: "VIP members" → `vip` | `"vip"` |
  | valid_from | DateTime | ✓ | 시작(포함). 오프셋이 없으면 스토어 시간대 | `"2026-11-07T00:00"` |
  | valid_through | DateTime | ✓ | 끝(포함, 분 단위) | `"2026-11-08T23:59"` |
  | scope_mode | enum | ✓ | `category`: `category_id`의 상품 전부(하위 포함); `products`: `product_ids`만 | `"category"` |
  | category_id | Text, `^cat_[a-z0-9_]+$` | scope_mode=`category`일 때 ✓ | 대상 카테고리(`list_categories`) | `"cat_summer_outerwear"` |
  | product_ids | Text[], `^prd_[a-z0-9_]+$` | scope_mode=`products`일 때 ✓ | 대상 상품 1–500개(`search_products`) | `["prd_windbreaker", "prd_field_jacket"]` |
  | percent_off | Number | ✓ | 기본 할인율, 0 초과 100 미만, 소수 둘째 자리까지 | `20` |
  | overrides | object[] | | 범위 안 상품별로 다른 할인율 `{product_id, percent_off}` | `[{"product_id": "prd_seersucker_overshirt", "percent_off": 15}]` |
  | exclude_low_stock | Boolean | | 기본 `false`. `false`: 재고가 `promo_min_stock` 미만인 상품도 넣고 `promo_min_stock` 체크를 실패로 돌려 사람이 판단하게 함; `true`: 그 상품을 빼고 `exclude` 줄로 기록(체크 통과, `detail`에 뺀 상품). 요청 범위를 몰래 줄이지 않기 위해 기본은 `false` | `true` |
- 출력(프리뷰, 변경셋 #1042 v2 — Fri Nov 6 11:05):
  ```jsonc
  {"preview_id": "pv_8f2c", "expires_at": "2026-11-06T11:35:00-05:00", "change_set_id": "cs_1042",
   "changes": [
     {"op": "create", "entity": "promotion", "id": null, "from": null,
      "to": {"name": "VIP weekend", "valid_for_member_tier": "vip", "discount_layer": "item_price",
             "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00", "products": 40}},
     {"op": "update", "entity": "unit_price_specification", "id": null,
      "from": {"product_id": "prd_field_jacket", "price": 128.00, "price_currency": "USD", "price_type": "list_price"},
      "to": {"product_id": "prd_field_jacket", "price": 102.40, "price_currency": "USD", "price_type": "sale_price", "percent_off": 20}},
     {"op": "update", "entity": "unit_price_specification", "id": null,
      "from": {"product_id": "prd_windbreaker", "price": 89.00, "price_currency": "USD", "price_type": "list_price"},
      "to": {"product_id": "prd_windbreaker", "price": 71.20, "price_currency": "USD", "price_type": "sale_price", "percent_off": 20}},
     {"op": "update", "entity": "unit_price_specification", "id": null,
      "from": {"product_id": "prd_seersucker_overshirt", "price": 79.00, "price_currency": "USD", "price_type": "list_price"},
      "to": {"product_id": "prd_seersucker_overshirt", "price": 67.15, "price_currency": "USD", "price_type": "sale_price", "percent_off": 15}},
     {"op": "update", "entity": "unit_price_specification", "id": null,
      "from": {"product_id": "prd_utility_jacket", "price": 118.00, "price_currency": "USD", "price_type": "list_price"},
      "to": {"product_id": "prd_utility_jacket", "price": 94.40, "price_currency": "USD", "price_type": "sale_price", "percent_off": 20}},
     // … 36개 상품의 update 줄이 더 있음 (모두 20%)
     {"op": "exclude", "entity": "product", "id": "prd_chore_coat", "from": null,
      "to": {"why": "promo_min_stock", "inventory_level": 7}},
     {"op": "exclude", "entity": "product", "id": "prd_rain_parka", "from": null,
      "to": {"why": "promo_min_stock", "inventory_level": 4}}],
   "checks": [{"rule": "max_discount", "passed": true, "detail": "Highest is 20%; policy allows 30% for vip", "fix_tool": null},
              {"rule": "price_floor", "passed": true, "detail": "Overshirt moved to 15% ($67.15) to stay at cost + 15% ($66.70)", "fix_tool": null},
              {"rule": "promo_min_stock", "passed": true, "detail": "2 products under 10 units excluded: chore coat (7), rain parka (4)", "fix_tool": null},
              {"rule": "promotion_overlap", "passed": true, "detail": "Checked 4 running promotions; none gives an item price on summer outerwear", "fix_tool": null}],
   "needs_approval": true,
   "approval_reason": "Changes 40 prices"}
  ```
  - 상품마다 VIP가 내는 가격을 바꾸는 `update unit_price_specification` 한 줄. 새 `UnitPriceSpecification`은 실행 때 상품의 모든 변형 `Offer`에 만들어지므로 `id`는 `null`입니다. 빠진 상품은 `op: "exclude"`(실행하지 않고 기록만, [enums.md](enums.md) §4.1).
  - **v1(11:02)**: `percent_off: 20`, `overrides` 없음, `exclude_low_stock` 기본값(`false`)으로 42개 전부를 미리 본 결과는 체크 2개가 실패합니다 — `{"rule": "price_floor", "passed": false, "detail": "Seersucker overshirt at 20% would be $63.20, under the floor $66.70 (cost $58.00 + 15%)", "fix_tool": "set_sale_prices"}`, `{"rule": "promo_min_stock", "passed": false, "detail": "Cotton chore coat (7) and Light rain parka (4) are under 10 units", "fix_tool": "set_sale_prices"}`, `approval_reason: "Changes 42 prices"`. 대화 수정("Drop the low-stock ones. Give the overshirt 15% instead.") 뒤 런타임이 `overrides`와 `exclude_low_stock: true`로 다시 프리뷰해 변경셋 항목을 바꾸고 버전이 오릅니다(v1 → v2).
  - `max_discount`의 30%는 데모 스토어의 덮어쓰기 `max_discount.by_tier.vip`입니다([../scenarios.md](../scenarios.md) §5). 기본값만 있으면 `max_discount.everyone`(20%)을 씁니다.
- 출력(실행): `{"ok": true, "id": "promo_1042", "undo": {"by": "person", "target": "cs_1042", "until": "2026-11-09T23:59:00-05:00"}}` — 되돌리기 기한은 기간 끝 + 1일(`change_set.undo_window_after_end_days`). 실행 후 변형 `Offer`마다 저장되는 값:
  ```jsonc
  {"@type": "UnitPriceSpecification", "id": "ups_windbreaker_m_slate_1042", "offer_id": "off_windbreaker_m_slate", "promotion_id": "promo_1042",
   "price": 71.20, "price_currency": "USD", "price_type": "sale_price",
   "valid_for_member_tier": "vip", "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00",
   "acom:percent_off": 20}
  ```
- 프리뷰: checks 4가지 — `max_discount`(할인율 상한), `price_floor`(판매가 ≥ 원가 + 15%), `promo_min_stock`(재고 부족 상품), `promotion_overlap`(같은 기간·같은 대상의 같은 층 할인, `list_active_promotions`와 같은 데이터). 실패한 체크는 `passed: false`와 `fix_tool`을 담고, 프리뷰 자체는 성공합니다. 항상 `needs_approval: true`([approval.yml](../../policies/approval.yml) `thresholds.promotion_publish`).
- 정책: `max_discount`, `promo_min_stock`, `promotion_overlap`, `sale_price`, `price_floor`([catalog.yml](../../policies/catalog.yml)), `thresholds.promotion_publish`, `change_set.undo_window_after_end_days`
- 에러(실행·승인 때 다시 계산해도 체크가 실패하면 `policy_violation`, `detail.rule`에 체크 키): `below_price_floor` → `set_sale_prices` · `over_max_discount` → `get_policy` · `low_stock` → `set_sale_prices` · `promotion_overlap` → `list_active_promotions` · `tier_not_found` → `list_tiers` · `not_found`(`detail.entity`: `product` \| `category`) → `search_products` · `invalid_period`(`valid_from`이 `valid_through`보다 늦음) → `set_sale_prices`
- 멱등성: `Idempotency-Key`. 승인 뒤 실행기가 `preview_id`로 한 번 실행하며, 같은 `preview_id`로 다시 오면 첫 결과를 돌려줍니다.

---

## coupon

쿠폰 적용 방식 `application`은 `automatic_at_checkout`(대상 회원의 결제에 자동, 발급 단계 없음) · `code_entry`(코드 입력) · `download`(쇼퍼가 받아 둠)입니다([enums.md](enums.md) §1.11, `coupon.applications`). 회원에게 발급된 쿠폰은 `mcp_…`(member coupon)입니다.

### `create_coupon` — Kind: W$  (데모 장면)
- 논리주소/REST: `promotion.coupon.create` · `POST /v1/promotion/coupons`
- 요약: 프로모션에 연결된 쿠폰을 만듭니다. 쿠폰은 체크아웃에서 **그 프로모션 혜택을 주문에 기록하는 수단이며, 두 번째 할인을 더하지 않습니다**(`coupon.records_benefit_only`). 장바구니에는 할인 한 줄("VIP weekend −$17.80")만 보입니다. 기간은 연결된 프로모션을 따르고(`coupon.validity`), `automatic_at_checkout` 쿠폰은 따로 발급하지 않아도 대상 등급 회원 모두의 결제에 적용됩니다. schema.org `CreateAction`.
- consumes → produces: `promotion_id`, `tier_id` → `coupon_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | code | Text, `^[A-Z0-9]{4,20}$` | ✓ | 스토어 안에서 하나뿐인 코드 | `"VIPWEEKEND20"` |
  | promotion_id | Text, `^promo_[a-z0-9_]+$` | ✓ | 연결할 프로모션(`set_sale_prices`, `list_active_promotions`). 같은 변경셋에서 아직 실행 전인 `set_sale_prices`를 가리키면 런타임이 항목 플레이스홀더(`$item[1].promotion_id`, 항목 `position`은 1부터)로 저장하고 실행 때 바인딩 | `"promo_1042"` |
  | valid_for_member_tier | Text | ✓ | 프로모션의 대상 등급과 같아야 함(`coupon.tier_match`) | `"vip"` |
  | application | enum | ✓ | `automatic_at_checkout` \| `code_entry` \| `download` | `"automatic_at_checkout"` |
  | stacking | enum | | `no_tier_coupons`(등급 쿠폰과 함께 안 씀) \| `with_tier_coupons` \| `no_other_coupons`. 기본 `coupon.stacking.default` | `"no_tier_coupons"` |
- 출력(프리뷰, 변경셋 #1042):
  ```jsonc
  {"preview_id": "pv_8f31", "expires_at": "2026-11-06T11:33:00-05:00", "change_set_id": "cs_1042",
   "changes": [{"op": "create", "entity": "coupon", "id": null, "from": null,
                "to": {"code": "VIPWEEKEND20", "promotion_id": "$item[1].promotion_id", "valid_for_member_tier": "vip",
                       "application": "automatic_at_checkout", "stacking": "no_tier_coupons",
                       "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00",
                       "eligible_members": 1284}}],
   "checks": [{"rule": "member_benefits_per_order", "passed": true, "detail": "Doesn't stack with tier coupons; one member benefit per order", "fix_tool": null}],
   "needs_approval": true,
   "approval_reason": "Applies automatically at checkout for 1,284 VIP members"}
  ```
- 출력(실행): `{"ok": true, "id": "cpn_vipweekend20", "undo": {"by": "person", "target": "cs_1042", "until": "2026-11-09T23:59:00-05:00"}}`
- 프리뷰: `create coupon`(`id: null`). check `member_benefits_per_order`. 항상 `needs_approval: true`. 대상 회원 수는 `list_tiers`의 `member_count`.
- 정책: `member_benefits_per_order`, `coupon.stacking`, `coupon.records_benefit_only`, `coupon.validity`, `coupon.tier_match`, `thresholds.promotion_publish`([approval.yml](../../policies/approval.yml))
- 에러: `not_found`(`detail.entity: promotion`) → `list_active_promotions` · `code_taken` → `search_coupons` · `coupon_tier_mismatch`(쿠폰 등급 ≠ 프로모션 등급) → `get_promotion` · `stacking_conflict`(같은 대상에 함께 쓸 수 없는 쿠폰이 이미 있음) → `search_coupons`
- 멱등성: `Idempotency-Key`. `code`는 스토어 안에서 하나.

### `issue_coupon` — Kind: W$
- 논리주소/REST: `promotion.coupon.issue` · `POST /v1/promotion/coupons/{coupon_id}/issue`
- 요약: `code_entry`·`download` 쿠폰을 회원에게 발급합니다(고객에게 닿으므로 항상 승인). `automatic_at_checkout` 쿠폰은 발급하지 않습니다. 방식: 회원 목록 / 세그먼트 / 조건 충족 시(생일 등).
- consumes → produces: `coupon_id`, `customer_id[]` 또는 `segment_id` → `member_coupon_id[]`
- 입력: `coupon_id`(`^cpn_`, ✓), `issue_mode`(✓, `customers` \| `segment` \| `on_condition`), `customer_ids`(Text[], customers일 때 ✓), `segment_id`(`^seg_`, segment일 때 ✓), `condition`(enum `birthday` \| `first_order` \| `tier_entry`, on_condition일 때 ✓)
- 출력(프리뷰): `{"preview_id": "pv_k20b", "expires_at": "2026-11-06T12:00:00-05:00", "change_set_id": "cs_1048", "changes": [{"op": "create", "entity": "member_coupon", "id": null, "from": null, "to": {"coupon_id": "cpn_birthday", "customers": 320}}], "checks": [{"rule": "coupon.per_member_limit", "passed": true, "detail": "Nobody gets a second one", "fix_tool": null}, {"rule": "coupon.total_pool_limit", "passed": true, "detail": "No pool limit", "fix_tool": null}, {"rule": "coupon.tier_match", "passed": true, "detail": "All 320 are in the member tier", "fix_tool": null}], "needs_approval": true, "approval_reason": "Reaches 320 customers"}`
- 출력(실행): `{"ok": true, "id": "cpn_birthday", "undo": {"by": "person", "target": "cs_1048", "until": "2026-11-07T23:59:00-05:00"}}` — 되돌리면 쓰지 않은 발급분을 `revoked`로.
- 프리뷰: `create member_coupon` 묶음 한 줄. 항상 `needs_approval: true`.
- 정책: `coupon.issue_modes`, `coupon.per_member_limit`, `coupon.total_pool_limit`, `coupon.tier_match`, `thresholds.promotion_publish`
- 에러: `pool_exhausted`(발급 한도 소진, 한도 상향은 정책 덮어쓰기로) → null · `recipient_tier_mismatch`(받는 회원의 등급 ≠ 쿠폰 등급) → `preview_segment` · `invalid_state`(`automatic_at_checkout` 쿠폰) → `get_coupon` · `not_found`(`detail.entity: coupon`) → `search_coupons`
- 멱등성: `(coupon_id, customer_id)` 한 번 발급.

### `redeem_coupon` — Kind: W
- 논리주소/REST: `promotion.coupon.redeem` · `POST /v1/promotion/coupons/{coupon_id}/redeem`
- 요약: 주문하기(`POST /v1/sales/orders`) 처리 안에서 commerce가 부르는 **내부 쓰기**입니다. 회원 쿠폰 상태 `available → used`, 혜택 금액을 주문에 기록합니다. 등급 일치·최소 주문·한도를 다시 검증합니다. 에이전트 매니페스트에는 나오지 않습니다.
- consumes → produces: `coupon_id`, `order_id` → `member_coupon_id`
- 입력: `coupon_id`(`^cpn_`, ✓), `order_id`(`^ord_`, ✓) — 주문한 회원, 소계, 주문에 기록된 할인은 sales가 넘겨 준 주문 읽기(`service::for_sales::provide_order_history`)로 읽습니다.
- 출력: `{"ok": true, "id": "mcp_jamie_vipweekend20", "benefit_recorded": {"value": 17.80, "currency": "USD"}, "undo": null}` — 주문 취소·반품 때는 `restore_coupon`.
- 프리뷰: 주문하기의 재검증 안에서 계산(따로 프리뷰하지 않음).
- 정책: `coupon.tier_match`, `member_benefits_per_order`
- 에러: `not_eligible`(등급·한도) → null · `coupon_expired` → null · `min_order_not_met` → null
- 멱등성: `(coupon_id, order_id)` 한 번.

### `restore_coupon` — Kind: W
- 논리주소/REST: `promotion.coupon.restore` · `POST /v1/promotion/coupons/{coupon_id}/restore`
- 요약: 취소·반품 때 회원 쿠폰을 복원합니다(`used → restored`). 이미 만료된 쿠폰에는 유예일을 줍니다. 전체 취소는 전부, 부분 취소는 상품 쿠폰만(`coupon.restore_on_cancel`).
- consumes → produces: `coupon_id`, `order_id` → `member_coupon_id`
- 입력: `coupon_id`(`^cpn_`, ✓), `order_id`(`^ord_`, ✓), `scope`(✓, `full` \| `partial`)
- 출력(프리뷰): `changes: [{"op": "update", "entity": "member_coupon", "id": "mcp_alex_socks4", "from": "used", "to": "restored"}]`, `needs_approval: false` · 출력(실행): `{"ok": true, "id": "mcp_alex_socks4", "undo": {"by": "person", "target": "cs_1043", "until": "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: `update member_coupon`.
- 정책: `coupon.restore_on_cancel`, `coupon.expiry_grace_days`
- 에러: `invalid_state`(`used`가 아님) → `get_coupon`
- 멱등성: 이미 `restored`면 같은 결과.

### `revoke_coupon` — Kind: W
- 논리주소/REST: `promotion.coupon.revoke` · `POST /v1/promotion/coupons/{coupon_id}/revoke`
- 요약: 쓰지 않은 회원 쿠폰을 회수합니다(`issued | available → revoked`, 실적 제외).
- consumes → produces: `coupon_id` → `member_coupon_id[]`
- 입력: `coupon_id`(`^cpn_`, ✓), `revoke_mode`(✓, `member_coupon`: `member_coupon_id` 하나; `all_unused`: 그 쿠폰의 쓰지 않은 발급분 전부), `member_coupon_id`(`^mcp_`, member_coupon일 때 ✓), `reason`(Text, ✓)
- 출력(프리뷰): `changes: [{"op": "revoke", "entity": "member_coupon", "id": null, "from": "available", "to": {"revoked": 41}}]`, `needs_approval: false` · 출력(실행): `{"ok": true, "id": "cpn_birthday", "undo": {"by": "person", "target": "cs_1049", "until": "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: `revoke member_coupon`.
- 정책: 없음
- 에러: `already_used` → `get_coupon` · `not_found`(`detail.entity: member_coupon`) → `list_member_coupons`
- 멱등성: 이미 `revoked`면 같은 결과.

### `search_coupons` — Kind: R
- 논리주소/REST: `promotion.coupon.search` · `GET /v1/promotion/coupons`
- 요약: 쿠폰 정의 목록.
- consumes → produces: → `coupon_id[]`
- 입력: `code`(Text), `promotion_id`(`^promo_`), `application`(enum), `valid_for_member_tier`(Text), `limit`(기본 20), `cursor`
- 출력: `{"items": [{"id": "cpn_vipweekend20", "code": "VIPWEEKEND20", "promotion_id": "promo_1042", "valid_for_member_tier": "vip", "application": "automatic_at_checkout", "stacking": "no_tier_coupons", "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00"}], "total": 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_filter` → `list_enums`
- 멱등성: 읽기.

### `get_coupon` — Kind: R
- 논리주소/REST: `promotion.coupon.get` · `GET /v1/promotion/coupons/{coupon_id}`
- 요약: 쿠폰 정의 한 건과 상태별 발급·사용 수.
- consumes → produces: `coupon_id` → `promotion_id`
- 입력: `coupon_id`(`^cpn_`, ✓)
- 출력: `{"id": "cpn_vipweekend20", "code": "VIPWEEKEND20", "promotion_id": "promo_1042", "valid_for_member_tier": "vip", "application": "automatic_at_checkout", "stacking": "no_tier_coupons", "records_benefit_only": true, "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00", "counts": {"issued": 0, "available": 0, "used": 0, "restored": 0, "expired": 0, "revoked": 0}}` — `counts`는 `coupon_status`별 회원 쿠폰 수(자동 적용 쿠폰은 쓰일 때 `used`로 생김)
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `not_found`(`detail.entity: coupon`) → `search_coupons`
- 멱등성: 읽기.

### `list_member_coupons` — Kind: R
- 논리주소/REST: `promotion.coupon.list_member_coupons` · `GET /v1/promotion/customers/{customer_id}/coupons`
- 요약: 회원 한 명이 가진 쿠폰(회원 쿠폰 `mcp_…`)과 상태. 쇼퍼는 `customer_id: "me"`로 자기 것을 봅니다.
- consumes → produces: `customer_id` → `member_coupon_id[]`
- 입력: `customer_id`(`^cus_` 또는 `me`, ✓), `status`(enum[] `issued` \| `available` \| `used` \| `restored` \| `expired` \| `revoked`), `limit`(기본 20), `cursor`
- 출력: `{"items": [{"id": "mcp_jamie_vipweekend20", "coupon_id": "cpn_vipweekend20", "code": "VIPWEEKEND20", "status": "used", "order_id": "ord_20518", "valid_through": "2026-11-08T23:59:00-05:00"}], "total": 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `not_found`(`detail.entity: customer`) → `search_members`
- 멱등성: 읽기.

---

## benefit

### `calculate_benefits` — Kind: R (단일 기준)
- 논리주소/REST: `promotion.benefit.calculate` · `POST /v1/promotion/benefits/calculate`
- 요약: 장바구니 + 고객 + 쿠폰 + 적립금으로 **할인 층 전부를 근거와 함께 계산**합니다. `get_cart`, `prepare_order`(`checkout_validation.server_total_matches_client`), `create_promotion`이 공통으로 부릅니다. 부작용이 없어 POST여도 읽기입니다. 층은 `discount_layers.order` 순서(`item_price` → `order_discount` → `add_on`)로 적용합니다.
- consumes → produces: `cart_id` → benefit breakdown
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | cart_id | Text, `^cart_` | ✓ | 줄·단가 | `"cart_jamie"` |
  | customer_id | Text, `^cus_` | ✓ | 등급·세그먼트를 서버가 조회 | `"cus_jamie"` |
  | coupon_ids | Text[] | | 적용할 쿠폰(코드 입력·다운로드 쿠폰). 자동 적용 쿠폰은 서버가 찾음 | `[]` |
  | spend_loyalty | MonetaryAmount | | 사용할 적립금 | `{"value": 0.00, "currency": "USD"}` |
- 출력(한 건의 계산 결과):
  ```jsonc
  {"cart_id": "cart_jamie",
   "layers": {
     "item_price": [{"cart_line_id": "cln_1", "promotion_id": "promo_1042", "label": "VIP weekend",
                     "list_price": {"value": 89.00, "currency": "USD"}, "item_price": {"value": 71.20, "currency": "USD"},
                     "amount": {"value": 17.80, "currency": "USD"}, "recorded_by": "cpn_vipweekend20"}],
     "order_discount": [],
     "add_on": [{"kind": "free_shipping", "promotion_id": "promo_free_ship_80", "amount": {"value": 6.00, "currency": "USD"}}]},
   "subtotal": {"value": 89.00, "currency": "USD"},
   "discount_total": {"value": 17.80, "currency": "USD"},
   "loyalty_applied": {"value": 0.00, "currency": "USD"},
   "shipping": {"value": 0.00, "currency": "USD"},
   "total": {"value": 71.20, "currency": "USD"},
   "rule_violations": [],
   "below_price_floor_items": []}
  ```
  - `add_on`의 무료 배송은 기본 배송비 $6.00을 면제한 것이라 `discount_total`에 넣지 않고 `shipping`이 0이 됩니다.
- 프리뷰: 해당 없음(읽기)
- 정책: `discount_layers`, `member_benefits_per_order`, `max_discount`, `loyalty.spend`, `price_floor`([catalog.yml](../../policies/catalog.yml)), `free_shipping_min_subtotal`([order.yml](../../policies/order.yml))
- 에러: `not_found`(`detail.entity: cart`) → `get_cart`. 정책 위반은 에러가 아니라 `rule_violations[]`로 돌려줍니다.
- 멱등성: 읽기.

---

## conditional_gift

### `create_conditional_gift` — Kind: W
- 논리주소/REST: `promotion.conditional_gift.create` · `POST /v1/promotion/conditional-gifts`
- 요약: 구매 금액 조건 사은품(구간별)을 만듭니다. 회원 혜택 적용 전 소계가 기준 이상이면 사은품을 주문에 붙입니다(`conditional_gift.trigger`).
- consumes → produces: `product_id` → `conditional_gift_id`
- 입력: `name`(✓), `tiers`(✓, `[{trigger_threshold: MonetaryAmount, gift_product_id: "prd_…"}]`, 기준 오름차순), `valid_for_member_tier`(기본 `everyone`), `channels`(enum[] `web` \| `mobile_web` \| `app` \| `agent`), `valid_from` / `valid_through`(✓)
- 출력(프리뷰): `changes: [{"op": "create", "entity": "conditional_gift", "id": null, "from": null, "to": {"tiers": [{"trigger_threshold": {"value": 120.00, "currency": "USD"}, "gift_product_id": "prd_merino_tee"}]}}]`, `needs_approval: false` · 출력(실행): `{"ok": true, "id": "cgf_holiday_120", "undo": {"by": "person", "target": "cs_1054", "until": "2026-12-01T23:59:00-05:00"}}`
- 프리뷰: `create conditional_gift`.
- 정책: `conditional_gift.trigger`, `conditional_gift.tiers_by_amount`
- 에러: `not_found`(`detail.entity: product`) → `search_products` · `invalid_period` → `create_conditional_gift`
- 멱등성: `Idempotency-Key`.

### `list_conditional_gifts` — Kind: R
- 논리주소/REST: `promotion.conditional_gift.list` · `GET /v1/promotion/conditional-gifts`
- 요약: 사은품 조건 목록.
- consumes → produces: → `conditional_gift_id[]`
- 입력: `status`(enum[] `scheduled` \| `running` \| `ended`), `limit`(기본 20), `cursor`
- 출력: `{"items": [{"id": "cgf_holiday_120", "name": "Holiday gift over $120", "tiers": [{"trigger_threshold": {"value": 120.00, "currency": "USD"}, "gift_product_id": "prd_merino_tee"}], "valid_from": "2026-11-20T00:00:00-05:00", "valid_through": "2026-11-30T23:59:00-05:00"}], "total": 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_filter` → `list_enums`
- 멱등성: 읽기.

---

## loyalty

적립금은 원장(`loyalty_ledger_entry`, 거래 id `lyt_…`)에 이벤트로 쌓고, 잔액은 합계입니다.

### `get_loyalty_balance` — Kind: R
- 논리주소/REST: `promotion.loyalty.get_balance` · `GET /v1/promotion/customers/{customer_id}/loyalty`
- 요약: 회원의 적립금 잔액과 곧 만료될 금액. 쇼퍼는 `me`.
- consumes → produces: `customer_id` → balance
- 입력: `customer_id`(`^cus_` 또는 `me`, ✓)
- 출력: `{"customer_id": "cus_jamie", "balance": {"value": 12.40, "currency": "USD"}, "expiring_soon": [{"amount": {"value": 2.00, "currency": "USD"}, "expires": "2026-11-30"}]}`
- 프리뷰: 해당 없음(읽기)
- 정책: `loyalty.expiry`
- 에러: `not_found`(`detail.entity: customer`) → `search_members`
- 멱등성: 읽기.

### `list_loyalty_transactions` — Kind: R
- 논리주소/REST: `promotion.loyalty.list_transactions` · `GET /v1/promotion/customers/{customer_id}/loyalty/transactions`
- 요약: 적립금 원장 거래 목록(적립·사용·만료·지급·복원).
- consumes → produces: `customer_id` → `loyalty_transaction_id[]`
- 입력: `customer_id`(`^cus_` 또는 `me`, ✓), `type`(enum[] `accrual` \| `spend` \| `expire` \| `grant` \| `restore` \| `reversal`), `limit`(기본 20), `cursor` — `reversal`은 `grant_loyalty`를 되돌린 차감 거래
- 출력: `{"items": [{"id": "lyt_88", "type": "accrual", "amount": {"value": 0.89, "currency": "USD"}, "order_id": "ord_20311", "at": "2026-09-18T12:00:00-04:00", "expires": "2027-09-18"}], "total": 14}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `not_found`(`detail.entity: customer`) → `search_members`
- 멱등성: 읽기.

### `grant_loyalty` — Kind: W$
- 논리주소/REST: `promotion.loyalty.grant` · `POST /v1/promotion/customers/{customer_id}/loyalty/grant`
- 요약: 적립금을 수동으로 지급합니다(원장에 추가). 돈과 같은 가치라 항상 사람 승인이고, `thresholds.loyalty_grant`를 넘으면 사유에 적습니다.
- consumes → produces: `customer_id` → `loyalty_transaction_id`
- 입력: `customer_id`(`^cus_`, ✓), `amount`(MonetaryAmount, ✓, 예 `{"value": 5.00, "currency": "USD"}`), `reason`(Text, ✓), `expires`(Date, 생략 시 `loyalty.expiry.window_days`)
- 출력(프리뷰): `changes: [{"op": "grant", "entity": "loyalty_transaction", "id": null, "from": null, "to": {"customer_id": "cus_alex", "amount": {"value": 5.00, "currency": "USD"}}}]`, `needs_approval: true`, `approval_reason: "Grants $5.00 store points"` · 출력(실행): `{"ok": true, "id": "lyt_91", "undo": {"by": "person", "target": "cs_1055", "until": "2026-11-07T23:59:00-05:00"}}` — 되돌리면 같은 금액의 차감 거래를 원장에 추가.
- 프리뷰: `grant loyalty_transaction`.
- 정책: `loyalty.expiry`, `loyalty.grant.max_amount`(한 번에 지급할 수 있는 상한, 넘으면 `over_limit`), `thresholds.loyalty_grant`([approval.yml](../../policies/approval.yml))
- 에러: `over_limit`(정책 상한 초과, 상한 변경은 정책 덮어쓰기) → null · `not_found`(`detail.entity: customer`) → `search_members`
- 멱등성: `Idempotency-Key`.

### `spend_loyalty` — Kind: W
- 논리주소/REST: `promotion.loyalty.spend` · `POST /v1/promotion/customers/{customer_id}/loyalty/spend`
- 요약: 주문하기 처리 안에서 commerce가 부르는 **내부 쓰기**. 적립금을 차감합니다(원장에 추가). 부분 사용 허용. 에이전트 매니페스트에는 나오지 않습니다.
- consumes → produces: `customer_id`, `order_id` → `loyalty_transaction_id`
- 입력: `customer_id`(✓), `order_id`(✓), `amount`(MonetaryAmount, ✓)
- 출력: `{"ok": true, "id": "lyt_92", "undo": null}` — 취소·반품 때는 `loyalty.refund_restore`로 복원 거래가 생김
- 프리뷰: 주문하기의 재검증 안에서 계산.
- 정책: `loyalty.spend`
- 에러: `insufficient_balance` → null
- 멱등성: `(customer_id, order_id)` 한 번.

### `expire_loyalty` — Kind: W
- 논리주소/REST: `promotion.loyalty.expire` · `POST /v1/promotion/loyalty/expirations`(시스템 주체의 예약 작업 전용)
- 요약: 기한이 지난 적립금을 만료시키는 **배치 쓰기**입니다(원장에 `expire` 거래 추가). 작업 큐가 매일 부르며 에이전트 매니페스트에는 나오지 않습니다.
- consumes → produces: → `loyalty_transaction_id[]`
- 입력: `as_of`(Date, 생략 시 오늘)
- 예약 작업: `loyalty.expire`가 매일 스토어 자정에 같은 만료를 실행하고 다음 날 실행을 예약합니다. 기한(`expires`)이 지난 날짜의 적립금은 작업이 기록하기 전에도 잔액에 들어가지 않습니다.
- 출력: `{"ok": true, "id": "job_expire_20261107", "expired": 213, "undo": null}`
- 프리뷰: 없음(시스템 작업)
- 정책: `loyalty.expiry`
- 에러: 없음(공통 에러만)
- 멱등성: 같은 `as_of`로 다시 돌려도 이미 만료된 거래는 건너뜀.

---

## event

> 이벤트는 **응모 방식**(entry mechanic)과 **당첨 방식**(selection method)의 조합이고, 보상은 **보상 종류**(reward type)와 **지급 방식**(fulfillment)의 조합이다. 셋 다 고정 목록이 아니라 스토어가 늘리고 끌 수 있는 레지스트리다([promotion.yml](../../policies/promotion.yml) `event.entry_mechanics` · `event.selection_methods` · `event.reward_types`). 세금·신고·법적 요구는 금액을 박아 두지 않고 스토어가 관할별로 정의하는 `event.compliance` 규칙으로 건다. 상태 값은 [enums.md](enums.md) §1.11. 아래 값은 모두 **예시**다.

### `list_event_types` — Kind: R
- 논리주소/REST: `promotion.event.list_types` · `GET /v1/promotion/event-types`
- 요약: 이 스토어에서 켜진 응모 방식과 당첨 방식, 허용 조합. `create_event` 전에 읽는다. 스토어 정의(`custom`) 항목은 응모 데이터 스키마(`entry_payload_schema`)를 함께 준다.
- consumes → produces: → `entry_mechanic`, `selection_method`
- 입력: `kind`(enum `entry_mechanic` \| `selection_method`, 생략 시 둘 다), `enabled_only`(Boolean, 기본 true)
- 출력(예시):
  ```jsonc
  {"items": [
    {"kind": "entry_mechanic", "id": "purchase_triggered", "name": "Buy to enter", "enabled": true, "builtin": true},
    {"kind": "entry_mechanic", "id": "check_in", "name": "Daily check-in", "enabled": true, "builtin": true},
    {"kind": "entry_mechanic", "id": "photo_styling_contest", "name": "Styling photo", "enabled": true, "builtin": false,
     "entry_payload_schema": {"image": "ImageObject", "caption": "Text"}},
    {"kind": "selection_method", "id": "random_draw", "name": "Random draw", "enabled": true, "builtin": true},
    {"kind": "selection_method", "id": "instant_win", "name": "Instant win", "enabled": true, "builtin": true}],
   "allowed_combinations": "any",
   "total": 12}
  ```
- 프리뷰: 해당 없음(읽기)
- 정책: `event.entry_mechanics`, `event.selection_methods`
- 에러: 없음(공통 에러만)
- 멱등성: 읽기.

### `list_reward_types` — Kind: R
- 논리주소/REST: `promotion.event.list_reward_types` · `GET /v1/promotion/reward-types`
- 요약: 켜진 보상 종류와 각 종류의 기본 지급 방식(`ship` \| `issue` \| `grant` \| `email` \| `manual`), 가치 입력 여부.
- consumes → produces: → `reward_type`
- 입력: `enabled_only`(Boolean, 기본 true)
- 출력(예시): `{"items": [{"id": "product", "fulfillment": "ship", "has_value": true}, {"id": "coupon", "fulfillment": "issue", "has_value": true}, {"id": "loyalty_points", "fulfillment": "grant", "has_value": true}, {"id": "digital_code", "fulfillment": "email", "has_value": true}], "total": 7}`
- 프리뷰: 해당 없음(읽기)
- 정책: `event.reward_types`
- 에러: 없음(공통 에러만)
- 멱등성: 읽기.

### `search_events` — Kind: R
- 논리주소/REST: `promotion.event.search` · `GET /v1/promotion/events`
- 요약: 이벤트 목록.
- consumes → produces: → `event_id[]`
- 입력: `status`(enum[] `draft` \| `scheduled` \| `running` \| `paused` \| `closed` \| `drawing` \| `announced` \| `fulfilling` \| `completed` \| `cancelled`), `entry_mechanic`(Text), `selection_method`(Text), `valid_for_member_tier`(Text), `limit`(기본 20), `cursor`
- 출력: `{"items": [{"id": "evt_0311", "name": "Fall check-in week", "status": "running", "entry_mechanic": "check_in", "selection_method": "random_draw", "start_date": "2026-11-09T00:00:00-05:00", "end_date": "2026-11-15T23:59:00-05:00"}], "total": 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_filter` → `list_enums`
- 멱등성: 읽기.

### `get_event` — Kind: R
- 논리주소/REST: `promotion.event.get` · `GET /v1/promotion/events/{event_id}`
- 요약: 이벤트 상세 — 응모 방식·당첨 방식·보상 등수·자격·한도·추첨 기록 요약·보상 지급 현황.
- consumes → produces: `event_id` → `draw_record_id[]`
- 입력: `event_id`(`^evt_`, ✓), `fields`(Text[])
- 출력: `{"id": "evt_0311", "name": "Fall check-in week", "status": "drawing", "entry_mechanic": "check_in", "selection_method": "random_draw", "start_date": "2026-11-09T00:00:00-05:00", "end_date": "2026-11-15T23:59:00-05:00", "reward_tiers": [{"rank": 1, "reward_type": "product", "reward_ref": "prd_wool_overcoat", "reward_value": {"value": 289.00, "currency": "USD"}, "winner_count": 1, "waitlist_count": 2}], "eligibility": {"valid_for_member_tier": "everyone"}, "entry_limit": {"count": 1, "period": "day"}, "entrant_count": 3812, "draw_record_ids": ["drw_0311_01"], "rewards": {"pending": 0, "compliance_pending": 1, "dispatched": 100}}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `not_found`(`detail.entity: event`) → `search_events`
- 멱등성: 읽기.

### `create_event` — Kind: W
- 논리주소/REST: `promotion.event.create` · `POST /v1/promotion/events`
- 요약: 이벤트를 `draft`로 만든다. 응모 방식·당첨 방식은 `list_event_types`, 보상 종류는 `list_reward_types`에서 고른다.
- consumes → produces: `entry_mechanic`, `selection_method`, `reward_type`, `tier_id`?, `segment_id`? → `event_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | name | Text | ✓ | 이벤트 이름 | `"Fall check-in week"` |
  | start_date / end_date | DateTime | ✓ | 응모 가능 기간(둘 다 포함) | `"2026-11-09T00:00"` / `"2026-11-15T23:59"` |
  | display_period | object | | `{valid_from, valid_through}` 노출 기간(기본 = 응모 기간) | — |
  | entry_mechanic | Text | ✓ | 레지스트리 id | `"check_in"` |
  | selection_method | Text | ✓ | 레지스트리 id | `"random_draw"` |
  | entry_payload_schema | object | | 설문 문항·UGC 형식. `custom`이면 ✓ | — |
  | reward_tiers | object[] | ✓ | 등수별 `{rank, reward_type, reward_ref?, reward_value?: MonetaryAmount, winner_count, waitlist_count?, odds?}`. `instant_win`은 `odds`, `check_in`은 회차별 `round` 허용 | `[{"rank": 1, "reward_type": "product", "reward_ref": "prd_wool_overcoat", "reward_value": {"value": 289.00, "currency": "USD"}, "winner_count": 1, "waitlist_count": 2}, {"rank": 2, "reward_type": "loyalty_points", "reward_value": {"value": 5.00, "currency": "USD"}, "winner_count": 100}]` |
  | eligibility | object | | `{valid_for_member_tier, segment_id, member_types, purchase_condition: {min_subtotal: MonetaryAmount, window, scope: {product_ids[], category_ids[], brand_ids[], exclude_product_ids[]}}, channels[]}` | `{"valid_for_member_tier": "everyone"}` |
  | entry_limit | object | | `{count, period: day \| week \| event \| unlimited}` | `{"count": 1, "period": "day"}` |
  | schedule | object | | `{draw_at?, announce_at?, dispatch_at?}` — `event.schedule.order`를 지킴 | `{"draw_at": "2026-11-16T10:00"}` |
  | compliance_jurisdictions | Text[] | | 적용할 스토어 정의 관할 id. 없으면 `event.compliance.jurisdiction_from`으로 결정 | `[]` |
- 출력(프리뷰): `{"preview_id": "pv_e311", "expires_at": "2026-11-06T12:30:00-05:00", "changes": [{"op": "create", "entity": "event", "id": null, "from": null, "to": {"name": "Fall check-in week", "status": "draft"}}], "checks": [{"rule": "event.entry_mechanics", "passed": true, "detail": "check_in is enabled", "fix_tool": null}, {"rule": "event.selection_methods.allowed_combinations", "passed": true, "detail": "check_in + random_draw allowed", "fix_tool": null}, {"rule": "event.reward_types", "passed": true, "detail": "product and loyalty_points are enabled", "fix_tool": null}, {"rule": "event.schedule", "passed": true, "detail": "Draw is after the end date", "fix_tool": null}, {"rule": "event.compliance.rules", "passed": true, "detail": "Rank 1 reward triggers 1 store rule: collect winner info", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "evt_0311", "undo": {"by": "person", "target": "cs_1056", "until": "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: `create event`. `instant_win`이면 check `event.selection_methods.instant_win.odds_must_sum_at_most`도.
- 시드: 만들 때 추첨 시드를 만들고 그 해시(`seed_commitment`)만 공개합니다. 시드는 추첨 기록에서 공개됩니다. `eligibility.purchase_condition.scope.brand_ids`는 아직 받지 않습니다(`invalid_input`).
- 정책: `event.entry_mechanics`, `event.selection_methods`, `event.reward_types`, `event.schedule`, `event.compliance`. 보상이 쿠폰·적립금이면 `coupon`, `loyalty`도 확인
- 에러: `type_disabled` → `list_event_types` · `combination_not_allowed` → `list_event_types` · `reward_type_disabled` → `list_reward_types` · `odds_exceed_one` → `create_event` · `tier_not_found` → `list_tiers` · `invalid_schedule` → `create_event`
- 멱등성: `Idempotency-Key`.

### `update_event` — Kind: W
- 논리주소/REST: `promotion.event.update` · `PATCH /v1/promotion/events/{event_id}`
- 요약: 이벤트 수정·상태 전이(`scheduled`, `paused`, `cancelled`). 응모가 시작된 뒤에는 자격을 좁히거나 보상을 줄이는 변경을 막는다(`changes_after_entry`).
- consumes → produces: `event_id` → `event_id`
- 입력: `event_id`(`^evt_`, ✓), `status`(enum `scheduled` \| `paused` \| `cancelled`), `create_event`의 필드 중 바꿀 것
- 출력(프리뷰): `changes: [{"op": "update", "entity": "event", "id": "evt_0311", "from": "draft", "to": "scheduled"}]`, checks `event.schedule`, `needs_approval: false` · 출력(실행): `{"ok": true, "id": "evt_0311", "undo": {"by": "person", "target": "cs_1056", "until": "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: `update event`.
- 정책: `event.schedule`, `event.eligibility`
- 에러: `invalid_state` → `get_event` · `changes_after_entry` → `get_event`
- 멱등성: `Idempotency-Key`.

### `enter_event` — Kind: W
- 논리주소/REST: `promotion.event.enter` · `POST /v1/promotion/events/{event_id}/entries`
- 요약: 쇼퍼가 이벤트에 응모. 응모 방식에 맞는 데이터·자격·참여 한도·기간을 검증한다. `instant_win` · `first_come` · `everyone_qualifies`는 응답에 판정 결과가 바로 오고, `random_draw` · `ranked`는 `run_draw`로 분리된다. `purchase_triggered`는 주문 완료 시 내부 호출도 가능. 쇼퍼 본인의 쓰기라 승인 없이 실행된다.
- consumes → produces: `event_id` → `entry_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | event_id | Text, `^evt_` | ✓ | 대상 이벤트 | `"evt_0311"` |
  | entry_payload | object | | 응모 데이터(설문 응답, 댓글·이미지, 코드 등). 이벤트의 `entry_payload_schema`를 따름 | `{}` |
  | order_id | Text, `^ord_` | | `purchase_triggered`일 때 근거 주문 | `"ord_20518"` |
  | entry_consent | object | ✓ | `event.eligibility.consent_required` 항목별 동의 | `{"entry_terms": true, "winner_contact": true}` |
- 출력(프리뷰): `{"preview_id": "pv_n552", "expires_at": "2026-11-09T09:30:00-05:00", "changes": [{"op": "create", "entity": "event_entry", "id": null, "from": null, "to": {"event_id": "evt_0311"}}], "checks": [{"rule": "event.eligibility", "passed": true, "detail": "Open to everyone", "fix_tool": null}, {"rule": "event.entry_limit", "passed": true, "detail": "1 of 1 entries today", "fix_tool": null}, {"rule": "event.entry_period", "passed": true, "detail": "Entries open until Nov 15 23:59", "fix_tool": null}, {"rule": "event.payload_validation", "passed": true, "detail": "No payload needed for check-in", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "ent_5521", "entry_status": "entered", "reward": null, "undo": null}` — `entry_status`는 `entered` \| `won` \| `lost`(즉시형), `won`이면 `reward: {rank, reward_type}`
- 프리뷰: `create event_entry`. `needs_approval: false`.
- 정책: `event.eligibility`, `event.entry_limit`, `event.entry_period`, `event.payload_validation`
- 에러: `not_eligible` → `get_event` · `entry_limit_exceeded` → `list_my_entries` · `event_not_active` → `get_event` · `invalid_payload` → `get_event` · `mandatory_consent_missing`(응모 동의 없음, 화면에서 동의 받기) → null · `sold_out` → `get_event`
- 멱등성: `(event_id, customer_id, period_bucket)` — 기간당 한도 1이면 같은 기간 재전송은 첫 결과.

### `list_my_entries` — Kind: R
- 논리주소/REST: `promotion.event.list_my_entries` · `GET /v1/promotion/events/entries/mine`
- 요약: 쇼퍼 본인의 응모·당첨 내역. 컴플라이언스 요구가 남았으면 무엇을 내야 하는지 함께 보인다.
- consumes → produces: → `entry_id[]`
- 입력: `event_id`(`^evt_`), `status`(enum[] `entered` \| `invalid` \| `won` \| `lost` \| `waitlist` \| `cancelled`), `limit`(기본 20), `cursor`
- 출력: `{"items": [{"id": "ent_5521", "event_id": "evt_0311", "entry_status": "won", "reward": {"rank": 1, "reward_type": "product", "reward_status": "compliance_pending"}, "compliance_pending": ["legal_name", "address"], "entered_at": "2026-11-09T09:02:00-05:00"}], "total": 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: 없음(응모가 없으면 빈 목록)
- 멱등성: 읽기.

### `run_draw` — Kind: W$
- 논리주소/REST: `promotion.event.run_draw` · `POST /v1/promotion/events/{event_id}/draw`
- 요약: 추첨·재추첨·당첨 취소·예비 승격. `random_draw` · `ranked` 이벤트에 쓴다. 실행마다 추첨 기록(`get_draw_record`)이 남는다. 고객에게 닿으므로 항상 승인.
- consumes → produces: `event_id` → `draw_record_id`, `entry_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | event_id | Text, `^evt_` | ✓ | | `"evt_0311"` |
  | mode | enum | ✓ | `draw` \| `redraw` \| `cancel_winner` \| `promote_waitlist` \| `add_waitlist` \| `manual_select` | `"draw"` |
  | rank | Integer | | 대상 등수(없으면 전 등수) | `1` |
  | winner_count / waitlist_count | Integer | | 없으면 이벤트의 등수별 값 | — |
  | entry_ids | Text[] | | `cancel_winner` · `manual_select` 대상 | — |
  | reason | Text | | `cancel_winner` · `manual_select` · `redraw`에 ✓ | — |
- 출력(프리뷰): `{"preview_id": "pv_d311", "expires_at": "2026-11-16T10:30:00-05:00", "changes": [{"op": "update", "entity": "event_entry", "id": "ent_5521", "from": "entered", "to": "won"}], "checks": [{"rule": "event.draw.requires_entries_closed", "passed": true, "detail": "Entries closed Nov 15 23:59", "fix_tool": null}, {"rule": "event.draw.exclude_previous_winners", "passed": true, "detail": "No previous winners", "fix_tool": null}, {"rule": "event.draw.fairness.seed_commit_before_close", "passed": true, "detail": "Seed hash published before close", "fix_tool": null}, {"rule": "event.compliance.rules", "passed": true, "detail": "2 winners need winner info", "fix_tool": null}], "needs_approval": true, "approval_reason": "Picks 101 winners"}`
- 출력(실행): `{"ok": true, "id": "drw_0311_01", "undo": null}` — 발표 후에는 되돌리지 않고 `cancel_winner` · `promote_waitlist`로 고친다.
- 프리뷰: 당첨자마다 `update event_entry`. 항상 `needs_approval: true`.
- 추첨(`seeded_shuffle`): 응모를 `sha256(seed:추첨 순번:entry_id)` 순으로 늘어놓고 등수 순서대로 당첨자, 예비를 채웁니다. `draw`는 남은 응모를 `lost`로, `redraw`는 그 등수의 당첨·예비를 `lost`로 돌리고 새로 뽑습니다. `ranked` 이벤트는 심사이므로 `manual_select`·`cancel_winner`·`promote_waitlist`만 받습니다. 추첨·재추첨·직접 선정 뒤 이벤트는 `announced`(당첨 안내는 스텁).
- 정책: `event.draw`, `event.draw.fairness`, `event.draw.announcement`, `event.compliance`
- 에러: `entries_open` → `get_event` · `not_draw_event` → `get_event` · `reason_required` → `run_draw` · `waitlist_empty` → `run_draw`
- 멱등성: `Idempotency-Key`. 같은 시드·같은 응모자 스냅샷이면 같은 결과(재현 가능).

### `get_draw_record` — Kind: R
- 논리주소/REST: `promotion.event.get_draw_record` · `GET /v1/promotion/events/{event_id}/draws/{draw_record_id}`
- 요약: 추첨 감사 기록. 고치지 않고 추가만 한다. 운영자·감사자용이며 쇼퍼에게는 공개 항목(방식, 시드 해시, 발표 시각)만.
- consumes → produces: `draw_record_id` → `entry_id[]`
- 입력: `event_id`(`^evt_`, ✓), `draw_record_id`(`^drw_`, ✓)
- 출력(예시):
  ```jsonc
  {"id": "drw_0311_01", "event_id": "evt_0311", "mode": "draw",
   "method": "seeded_shuffle", "seed_commitment": "sha256:9c1e…", "seed": "…",
   "entrant_snapshot_digest": "sha256:41ab…", "entrant_count": 3812,
   "excluded": {"previous_winners": 0, "invalid_entries": 14},
   "results": [{"rank": 1, "winners": ["ent_5521"], "waitlist": ["ent_6002", "ent_0919"]}],
   "agent": "adm_mina", "approved_by": "adm_lee", "start_time": "2026-11-16T10:00:00-05:00"}
  ```
- 프리뷰: 해당 없음(읽기)
- 정책: `event.draw.fairness.record`, `event.draw.fairness.retention_days`
- 에러: `not_found`(`detail.entity: draw_record`) → `get_event`
- 멱등성: 읽기.

### `dispatch_reward` — Kind: W$
- 논리주소/REST: `promotion.event.dispatch_reward` · `POST /v1/promotion/events/{event_id}/rewards`
- 요약: 당첨 보상을 보상 종류의 지급 방식대로 처리한다 — `ship`(배송 요청, 기본 배송지), `issue`(쿠폰 발급, `issue_coupon`과 같은 경로), `grant`(적립금·스토어 크레딧, 원장에 추가), `email`(디지털 코드·안내 발송), `manual`(운영자 처리 기록). 컴플라이언스 규칙이 걸린 당첨자는 요구(정보 제출, 원천징수, 당첨자 납부 등)가 채워질 때까지 `compliance_pending`으로 보류한다. 즉시형 이벤트의 `issue` · `grant` 보상은 정책(`event.reward_types.auto_dispatch`)상 자동 지급된다.
- consumes → produces: `event_id`, `entry_id[]`? → `reward_id[]`
- 입력: `event_id`(`^evt_`, ✓), `entry_ids`(Text[], 없으면 지급 대기 당첨자 전체), `rank`(Integer), `notify`(Boolean, 기본 true)
- 출력(프리뷰): `{"preview_id": "pv_w311", "expires_at": "2026-11-17T10:30:00-05:00", "changes": [{"op": "create", "entity": "event_reward", "id": null, "from": null, "to": {"entry_id": "ent_5521", "reward_type": "product", "fulfillment": "ship"}}], "checks": [{"rule": "event.compliance.on_unsatisfied", "passed": true, "detail": "1 winner has not sent the required info (legal_name, address); held", "fix_tool": null}, {"rule": "event.reward_types.ship_to", "passed": true, "detail": "Shipping address on file for 1 product winner", "fix_tool": null}], "needs_approval": true, "approval_reason": "Sends rewards to 100 winners"}`
- 출력(실행): `{"ok": true, "id": "rwb_0311", "undo": null}` — 보낸 보상은 되돌리지 않음.
- 프리뷰: 당첨자마다 `create event_reward`. 항상 `needs_approval: true`.
- 정책: `event.reward_types`, `event.compliance`, `thresholds.promotion_publish`([approval.yml](../../policies/approval.yml))
- 에러: `compliance_pending` → `list_my_entries` · `missing_address`(당첨자에게 주소 요청) → null · `pool_exhausted`(쿠폰 보상 한도 소진) → null
- 멱등성: `(entry_id, rank)` 한 번 지급.

---

## segment

### `define_segment` — Kind: W
- 논리주소/REST: `promotion.segment.define` · `POST /v1/promotion/segments`
- 요약: 조건으로 고객 묶음을 정의합니다(버전 관리, `acom:Segment`). `issue_coupon`, `send_bulk`, 이벤트 자격에서 씁니다.
- consumes → produces: → `segment_id`
- 입력: `name`(✓), `condition_rules`(✓, `{member_tiers?: Text[], purchased_category_ids?: Text[], last_order_within_days?: Integer, min_spend?: MonetaryAmount, exclude_segment_ids?: Text[]}`, 하나 이상)
- 출력(프리뷰): `changes: [{"op": "create", "entity": "segment", "id": null, "from": null, "to": {"name": "Lapsed knit buyers", "estimated_size": 412}}]`, `needs_approval: false` · 출력(실행): `{"ok": true, "id": "seg_lapsed_knit", "undo": {"by": "person", "target": "cs_1057", "until": "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: `create segment`.
- 정책: 없음
- 에러: `tier_not_found` → `list_tiers` · `invalid_input`(조건 없음) → `define_segment`
- 멱등성: `Idempotency-Key`.

### `preview_segment` — Kind: R
- 논리주소/REST: `promotion.segment.preview` · `POST /v1/promotion/segments/preview`
- 요약: 조건(또는 저장된 세그먼트)에 지금 몇 명이 드는지와 표본을 계산합니다. 부작용이 없어 POST여도 읽기.
- consumes → produces: `segment_id`? → `customer_id[]`
- 입력: `segment_id`(`^seg_`) 또는 `condition_rules`(`define_segment`와 같은 모양) 중 하나 ✓, `sample_size`(Integer, 기본 5, 최대 20)
- 출력: `{"estimated_size": 412, "sample_customer_ids": ["cus_4431", "cus_8812"], "by_tier": {"member": 300, "gold": 90, "vip": 22}}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `not_found`(`detail.entity: segment`) → `list_segments`
- 멱등성: 읽기.

### `list_segments` — Kind: R
- 논리주소/REST: `promotion.segment.list` · `GET /v1/promotion/segments`
- 요약: 저장된 세그먼트 목록.
- consumes → produces: → `segment_id[]`
- 입력: `limit`(기본 20), `cursor`
- 출력: `{"items": [{"id": "seg_lapsed_knit", "name": "Lapsed knit buyers", "version": 2, "estimated_size": 412}], "total": 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: 없음(공통 에러만)
- 멱등성: 읽기.
