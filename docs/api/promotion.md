# API 명세 — `promotion` (프로모션·회원 판매가·쿠폰·혜택 계산·사은품·적립금·이벤트·세그먼트)

> 계층: `promotion.{resource}.{operation}`. 공통 규약은 [README.md](README.md). 데이터 타입은 [data-model.md](data-model.md). 정책은 [promotion.yml](policies/promotion.yml) · [catalog.yml](policies/catalog.yml) · [approval.yml](policies/approval.yml). MVP 툴 시그니처는 [plan.md §3.2](../plan.md), 위치는 [architecture.md §3.4](../architecture.md).
> **핵심**: 할인 로직을 주문에 심지 않는다. `calculate_benefits`(혜택 계산기)가 장바구니·주문 준비·검증의 **단일 기준**이다. 모든 쓰기는 먼저 프리뷰를 반환한다. 회원 등급·기간 한정 퍼센트 할인은 `set_sale_prices`, 그 혜택을 주문에 기록하는 것은 `create_coupon`이다.

리소스: [promotion](#promotion) · [offer](#offer) · [coupon](#coupon) · [benefit](#benefit) · [conditional_gift](#conditional_gift) · [loyalty](#loyalty) · [event](#event) · [segment](#segment)

---

## promotion

### `list_active_promotions` — Kind: R
- 논리주소/REST: `promotion.promotion.list_active` · `GET /v1/promotion/promotions/active?valid_from=&valid_through=&valid_for_member_tier=`
- 요약: 주어진 기간과 **겹치는** 진행 중·예정 프로모션 목록. `set_sale_prices`의 `promotion_overlap` 검사가 읽는 것과 같은 데이터.
- consumes → produces: → `promotion_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | valid_from | DateTime | ✓ | `2026-11-07T00:00:00` |
  | valid_through | DateTime | ✓ | `2026-11-08T23:59:00` |
  | valid_for_member_tier | Text | | 등급 id(`list_tiers`, 예 `"vip"`) 또는 `"everyone"`. 없으면 전체 |
  | limit / offset | Integer | | 기본 20 / 0 |
- 출력:
```jsonc
{"items": [{"promotion_id": "promo_0981", "name": "Autumn knitwear 15%", "status": "running", "valid_for_member_tier": "everyone",
            "valid_from": "2026-11-01T00:00:00", "valid_through": "2026-11-15T23:59:00"},
           {"promotion_id": "promo_0990", "name": "Gold tier 10%", "status": "running", "valid_for_member_tier": "gold",
            "valid_from": null, "valid_through": null}],
 "total": 4}
```
- 에러: `invalid_period` → (valid_from ≤ valid_through로 다시 지정)
- 멱등성: 읽기.

### `search_promotions` — Kind: R
- 논리주소/REST: `promotion.promotion.search` · `GET /v1/promotion/promotions?status=&valid_for_member_tier=&limit=&offset=`
- 요약: 상태·기간·대상 등급·범위로 프로모션 검색. 출력: `{items: acom:Promotion[], total}`. consumes → produces: → `promotion_id[]`.

### `get_promotion` — Kind: R
- 논리주소/REST: `promotion.promotion.get` · `GET /v1/promotion/promotions/{promotion_id}`
- 요약: 프로모션 한 건과 적용 범위. consumes → produces: `promotion_id` → promotion. 에러: `promotion_not_found` → search_promotions.

### `create_promotion` — Kind: W$
- 논리주소/REST: `promotion.promotion.create` · `POST /v1/promotion/promotions`
- 요약: 회원 등급 퍼센트 판매가 **이외의** 프로모션 생성(1+1, 묶음, 금액 조건, 정액 할인). **회원 등급·기간 한정 퍼센트 할인은 `set_sale_prices`를 쓴다.** 돈이 움직이므로 항상 사람 승인.
- consumes → produces: `product_id[]` → `promotion_id`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명·예 |
  |------|----------------------|:--:|--------|
  | name | Text | ✓ | `"Denim bundle"` |
  | acom:promotion_type | enum | ✓ | `bogo` \| `bundle` \| `threshold` \| `amount_off` |
  | acom:discount | object | ✓ | 예 `{amount_off: 10.00}`, `{threshold: 80.00, benefit: "free_shipping"}` |
  | valid_from / valid_through | DateTime | ✓ | 기간 |
  | valid_for_member_tier | Text | | 등급 id(`list_tiers`, 예 `"vip"`) 또는 `"everyone"`(기본) |
  | acom:scope | object | ✓ | `{product_ids[], categories[], exclude_product_ids[]}` |
  | acom:stacking_tier | Integer | ✓ | 1 \| 2 \| 3 ([promotion.yml](policies/promotion.yml) `stacking`) |
  | acom:cost_share | object | | `{platform, vendor}`, 합 = 1 |
  | acom:channel | Text[] | | `["web", "mobile"]` |
  | preview | Boolean | ✓ | true 먼저 |
- 출력(프리뷰): `{preview_id, changes[{op, entity, id, from, to}], checks[{rule, passed, detail, fix_tool}], needs_approval: true, approval_reason}`
- 출력(실행): `{ok: true, id: "promo_1051", undo: {tool: "undo_change_set", until: "..."}}`
- 프리뷰: 검사 `max_discount`, `price_floor`, `promotion_overlap`, `cost_share_sum`.
- 정책: [promotion.yml](policies/promotion.yml) `stacking.max_total_discount_rate` · `margin_protection` · `cost_share`, [catalog.yml](policies/catalog.yml) 가격 하한
- 에러: `below_price_floor` → set_price · `stacking_conflict` → update_promotion · `cost_share_sum` → update_promotion · `percent_off_for_tier` → set_sale_prices
- 멱등성: `Idempotency-Key`. `preview_id`는 한 번만 실행.

### `update_promotion` — Kind: W
- 논리주소/REST: `promotion.promotion.update` · `PATCH /v1/promotion/promotions/{promotion_id}`
- 요약: 프로모션 수정(프리뷰 먼저). 입력: promotion_id(✓), 바꿀 필드, preview(✓). 에러: `invalid_state` → get_promotion. 멱등성: `Idempotency-Key`.

### `pause_promotion` / `end_promotion` — Kind: W
- 논리주소/REST: `promotion.promotion.{pause|end}` · `POST /v1/promotion/promotions/{promotion_id}/{pause|end}`
- 요약: 상태 전이 `running → paused → ended`. 입력: promotion_id(✓), preview. 에러: `invalid_state` → get_promotion. 멱등성: 같은 상태로의 전이는 성공으로 처리.

---

## offer

### `set_sale_prices` — Kind: W$
- 논리주소/REST: `promotion.offer.set_sale_prices` · `POST /v1/promotion/sale-prices`
- 요약: 회원 등급·기간 한정 판매가를 상품별로 설정한다. **정가(list price)는 바꾸지 않는다.** 상품마다 schema.org `UnitPriceSpecification`(`price_type: "sale_price"`, `valid_for_member_tier`, `valid_from`, `valid_through`)을 쓰고, 이를 하나의 프로모션으로 묶는다. 판매가 = 정가 × (1 − percent_off / 100), 센트 단위 반올림. schema.org로는 `Offer` 가격에 대한 `UpdateAction`.
- consumes → produces: `product_id[]`, `tier` → `promotion_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | name | Text | ✓ | `"VIP weekend"` |
  | valid_for_member_tier | Text | ✓ | `"vip"` 또는 `"everyone"` (`list_tiers`에서) |
  | valid_from | DateTime | ✓ | `2026-11-07T00:00:00` |
  | valid_through | DateTime | ✓ | `2026-11-08T23:59:00` |
  | items | object[] | ✓ | `[{product_id: "prd_windbreaker", percent_off: 20}, {product_id: "prd_overshirt", percent_off: 15}]` (`search_products`에서) |
  | preview | Boolean | ✓ | true 먼저 |
- 출력(프리뷰): ([plan.md §3.1](../plan.md))
```jsonc
{"preview_id": "pv_8f2c",
 "changes": [{"op": "update", "entity": "price", "id": "prd_windbreaker", "from": 89.00, "to": 71.20}],
 "checks": [{"rule": "max_discount", "passed": true, "detail": "Highest discount 20%, limit 30%", "fix_tool": null},
            {"rule": "price_floor", "passed": true, "detail": "Overshirt moved to 15% to stay at cost + 15%", "fix_tool": null},
            {"rule": "min_stock", "passed": true, "detail": "Low-stock chore coat and rain parka left out", "fix_tool": null},
            {"rule": "promotion_overlap", "passed": true, "detail": "Checked 4 running promotions", "fix_tool": null}],
 "needs_approval": true,
 "approval_reason": "Changes 40 prices"}
```
- 출력(실행): `{"ok": true, "id": "promo_1042", "undo": {"tool": "undo_change_set", "until": "2026-11-09T23:59:00"}}`. 실행 후 상품마다 저장되는 값:
```jsonc
{"@type": "UnitPriceSpecification", "price": 71.20, "price_currency": "USD", "price_type": "sale_price",
 "valid_for_member_tier": "vip", "valid_from": "2026-11-07T00:00:00", "valid_through": "2026-11-08T23:59:00",
 "acom:percent_off": 20}
```
- 프리뷰: 검사 4가지 — `max_discount`(할인율 상한), `price_floor`(판매가 ≥ 원가 + 마진 하한), `min_stock`(재고 부족 상품 제외), `promotion_overlap`(같은 기간·등급의 다른 프로모션, `list_active_promotions`). 실패한 검사는 `passed: false`와 `fix_tool`(예 `set_sale_prices`, `search_products`)을 담는다. 대화로 수정하면 프리뷰를 다시 만들고 변경셋 버전이 오른다(v1 → v2).
- 정책: [promotion.yml](policies/promotion.yml) `max_discount` · `min_stock` · `promotion_overlap`, [catalog.yml](policies/catalog.yml) `price_floor`, [approval.yml](policies/approval.yml)
- 에러: `below_price_floor` → set_sale_prices · `over_max_discount` → get_policy · `low_stock` → search_products · `promotion_overlap` → list_active_promotions · `unknown_tier` → list_tiers · `product_not_found` → search_products
- 멱등성: `Idempotency-Key`. `preview_id`는 승인 후 실행기가 한 번만 실행하며, 같은 `preview_id`로 다시 보내면 첫 결과를 반환.

---

## coupon

### `create_coupon` — Kind: W$
- 논리주소/REST: `promotion.coupon.create` · `POST /v1/promotion/coupons`
- 요약: 프로모션에 연결된 쿠폰 생성. 쿠폰은 체크아웃에서 **그 프로모션 혜택을 주문에 기록하는 수단이며, 두 번째 할인을 더하지 않는다.** 장바구니에는 한 줄 "VIP weekend −$17.80"만 보인다.
- consumes → produces: `promotion_id`, `tier` → `coupon_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | code | Text | ✓ | `"VIPWEEKEND20"` |
  | promotion_id | Text | ✓ | `promo_1042` (`set_sale_prices`에서) |
  | valid_for_member_tier | Text | ✓ | 등급 id(`list_tiers`, 예 `"vip"`) 또는 `"everyone"` |
  | auto_apply | Boolean | ✓ | true면 체크아웃에서 자동 적용 |
  | stacking | Text | ✓ | `"no_tier_coupons"` — 등급 쿠폰과 함께 쓰지 않음 |
  | preview | Boolean | ✓ | true 먼저 |
- 출력(프리뷰): `{"preview_id": "pv_8f31", "changes": [{"op": "create", "entity": "coupon", "id": "cpn_vipweekend20", "from": null, "to": {"code": "VIPWEEKEND20", "valid_for_member_tier": "vip", "auto_apply": true}}], "checks": [{"rule": "coupon_stacking", "passed": true, "detail": "Not combined with tier coupons", "fix_tool": null}], "needs_approval": true, "approval_reason": "Issues a coupon to 1,284 VIP members"}`
- 출력(실행): `{"ok": true, "id": "cpn_vipweekend20", "undo": {"tool": "undo_change_set", "until": "2026-11-09T23:59:00"}}`
- 프리뷰: 검사 `coupon_stacking`.
- 정책: [promotion.yml](policies/promotion.yml) `coupon_stacking` · `coupon`(`tier_match`: 발급 등급 = 사용 등급)
- 에러: `promotion_not_found` → list_active_promotions · `code_taken` → search_coupons · `tier_mismatch` → list_tiers
- 멱등성: `Idempotency-Key`. `code`는 스토어 안에서 하나.

### `issue_coupon` — Kind: W$
- 논리주소/REST: `promotion.coupon.issue` · `POST /v1/promotion/coupons/{coupon_id}/issue`
- 요약: 자동 적용이 아닌 쿠폰을 회원에게 발급(고객에게 닿으므로 항상 승인). 방식: 다운로드 / 조건부 자동 / 수동.
- consumes → produces: `coupon_id`, `member_id[]` 또는 `segment_id` → 발급 수
- 입력: coupon_id(✓), acom:issue_method(`download` \| `conditional_auto` \| `manual`,✓), member_ids(Text[]) 또는 segment_id(Text), preview(✓)
- 출력(프리뷰): `{preview_id, changes: [{op: "create", entity: "member_coupon", id: "cpn_birthday", from: 0, to: 320}], checks: [{rule: "per_member_limit", passed: true, ...}], needs_approval: true, approval_reason: "Reaches 320 customers"}`; (실행): `{ok: true, id: "cpn_birthday", undo: {tool: "revoke_coupon", until: null}}`
- 정책: `coupon.per_member_limit` · `coupon.total_pool_limit`, [approval.yml](policies/approval.yml)
- 에러: `pool_exhausted` → (한도 상향 요청) · `tier_mismatch` → preview_segment
- 멱등성: `(coupon_id, member_id)` 한 번 발급.

### `redeem_coupon` — Kind: W
- 논리주소/REST: `promotion.coupon.redeem` · `POST /v1/promotion/coupons/{coupon_id}/redeem`
- 요약: 주문하기 시점에 쿠폰 사용 처리(내부 호출). 상태 `available → used`. 등급 일치·최소 주문·상한 검증.
- 입력: coupon_id(✓), order_id(✓). 출력: `{ok: true, id: "cpn_vipweekend20", acom:benefit_recorded: -17.80}`. 에러: `not_eligible` · `expired` · `min_order_not_met` → (없음). 멱등성: `(coupon_id, order_id)` 한 번.

### `restore_coupon` — Kind: W
- 논리주소/REST: `promotion.coupon.restore` · `POST /v1/promotion/coupons/{coupon_id}/restore`
- 요약: 취소·반품 시 복원(`used → restored`). 만료된 쿠폰은 유예일 추가. 전체 취소는 전부, 부분 취소는 상품 쿠폰만([promotion.yml](policies/promotion.yml) `coupon.restore_on_cancel`). 에러: `invalid_state` → get_coupon.

### `revoke_coupon` — Kind: W
- 논리주소/REST: `promotion.coupon.revoke` · `POST /v1/promotion/coupons/{coupon_id}/revoke`
- 요약: 사용하지 않은 쿠폰 회수(`→ revoked`, 실적 제외). 에러: `already_used` → get_coupon.

### `search_coupons` / `get_coupon` / `list_member_coupons` — Kind: R
- 논리주소/REST: `GET /v1/promotion/coupons?code=&promotion_id=&limit=&offset=` · `GET /v1/promotion/coupons/{coupon_id}` · `GET /v1/promotion/members/{member_id}/coupons`
- 요약: 쿠폰 목록 `{items: acom:Coupon[], total}` / 상세 / 회원별 보유 쿠폰.

---

## benefit

### `calculate_benefits` — Kind: R (단일 기준)
- 논리주소/REST: `promotion.benefit.calculate` · `POST /v1/promotion/benefits/calculate`
- 요약: 장바구니 + 고객 + 쿠폰 + 적립금으로 **할인 계층 전부를 근거와 함께 계산**. `get_cart`, `prepare_order`(서버 가격 재계산 검사), `create_promotion`이 공통으로 부른다. 부작용이 없어 POST여도 읽기.
- consumes → produces: `cart_id` → benefit breakdown
- 입력:
  | 필드 | 타입 | 필수 | 설명 |
  |------|------|:--:|------|
  | cart_id | Text | ✓ | 라인·단가 |
  | customer_id | Text | ✓ | 등급·세그먼트를 서버가 조회 |
  | coupon_ids | Text[] | | 적용할 쿠폰 |
  | spend_loyalty | Number | | 사용할 적립금(예 5.00) |
- 출력:
```jsonc
{"tier1": [{"line_id": "ln_1", "adjust": -17.80, "reason": "promo_1042 VIP weekend 20%", "recorded_by": "cpn_vipweekend20"}],
 "tier2": null,
 "tier3": null,
 "loyalty_applied": 0.00,
 "total": {"subtotal": 89.00, "discount": -17.80, "loyalty": 0.00, "shipping": 0.00, "payable": 71.20, "price_currency": "USD"},
 "rule_violations": [],
 "below_price_floor_items": []}
```
- 정책: [promotion.yml](policies/promotion.yml) `stacking`(tier2는 택1, tier3는 위에 중첩, 합산 상한) · `margin_protection` · `loyalty`
- 에러: 없음(읽기). 위반은 `rule_violations`로 표현.
- 멱등성: 읽기.

---

## conditional_gift

### `create_conditional_gift` — Kind: W
- 논리주소/REST: `promotion.conditional_gift.create` · `POST /v1/promotion/conditional-gifts`
- 요약: 구매 금액 조건 사은품(구간별). 입력: acom:trigger_threshold(Number,✓, 예 120.00), acom:gift_product_id(✓), acom:tiers_by_amount[], valid_for_member_tier, acom:channel[], preview(✓). 출력: `acom:ConditionalGift`. 정책: `conditional_gift`. 에러: `product_not_found` → search_products.

### `list_conditional_gifts` — Kind: R
- 논리주소/REST: `promotion.conditional_gift.list` · `GET /v1/promotion/conditional-gifts?limit=&offset=` → `{items, total}`.

---

## loyalty

### `get_loyalty_balance` — Kind: R
- 논리주소/REST: `promotion.loyalty.get_balance` · `GET /v1/promotion/members/{member_id}/loyalty`
- 요약: `{value: 12.40, price_currency: "USD", acom:expiring_soon: [{value: 2.00, expires: "2026-11-30"}]}` (원장 집계).

### `list_loyalty_transactions` — Kind: R
- 논리주소/REST: `promotion.loyalty.list_transactions` · `GET /v1/promotion/members/{member_id}/loyalty/transactions?limit=&offset=` → `{items: acom:LoyaltyTransaction[], total}`.

### `grant_loyalty` — Kind: W$
- 논리주소/REST: `promotion.loyalty.grant` · `POST /v1/promotion/members/{member_id}/loyalty/grant`
- 요약: 수동 지급(원장에 추가). 입력: member_id(✓), amount(✓, 예 5.00), acom:accrual_type, reason(✓), preview(✓). 프리뷰 후 항상 승인([approval.yml](policies/approval.yml)). 에러: `over_limit` → (승인 한도 상향 요청). 멱등성: `Idempotency-Key`.

### `spend_loyalty` — Kind: W
- 논리주소/REST: `promotion.loyalty.spend` · `POST /v1/promotion/members/{member_id}/loyalty/spend`
- 요약: 주문하기 시점의 적립금 차감(원장에 추가, 내부 호출). 부분 사용 허용. 정책: `loyalty.spend`. 에러: `insufficient_balance` → get_loyalty_balance. 멱등성: `(member_id, order_id)` 한 번.

### `expire_loyalty` — Kind: W
- 논리주소/REST: `promotion.loyalty.expire` · 배치 만료(원장에 추가). 정책: `loyalty.expiry`.

---

## event

> 이벤트는 **응모 방식**(entry mechanic)과 **당첨 방식**(selection method)의 조합이고, 보상은 **보상 종류**(reward type)와 **지급 방식**(fulfillment)의 조합이다. 셋 다 고정 목록이 아니라 스토어가 늘리고 끌 수 있는 레지스트리다([promotion.yml](policies/promotion.yml) `event.entry_mechanics` · `event.selection_methods` · `event.reward_types`). 세금·신고·법적 요구는 금액을 박아 두지 않고 스토어가 관할별로 정의하는 `event.compliance` 규칙으로 건다. 아래 값은 모두 **예시**다.

### `list_event_types` — Kind: R
- 논리주소/REST: `promotion.event.list_types` · `GET /v1/promotion/event-types`
- 요약: 이 스토어에서 켜진 응모 방식과 당첨 방식, 허용 조합. `create_event` 전에 읽는다. 스토어 정의(`custom`) 항목은 응모 데이터 스키마(`acom:entry_payload_schema`)를 함께 준다.
- consumes → produces: → `entry_mechanic`, `selection_method`
- 출력(예시):
```jsonc
{"items": [
  {"kind": "entry_mechanic", "id": "purchase_triggered", "name": "Buy to enter", "enabled": true, "builtin": true},
  {"kind": "entry_mechanic", "id": "check_in", "name": "Daily check-in", "enabled": true, "builtin": true},
  {"kind": "entry_mechanic", "id": "photo_styling_contest", "name": "Styling photo", "enabled": true, "builtin": false,
   "acom:entry_payload_schema": {"image": "ImageObject", "caption": "Text"}},
  {"kind": "selection_method", "id": "random_draw", "name": "Random draw", "enabled": true, "builtin": true},
  {"kind": "selection_method", "id": "instant_win", "name": "Instant win", "enabled": true, "builtin": true}],
 "acom:allowed_combinations": "any",
 "total": 12}
```
- 정책: `event.entry_mechanics` · `event.selection_methods`
- 멱등성: 읽기.

### `list_reward_types` — Kind: R
- 논리주소/REST: `promotion.event.list_reward_types` · `GET /v1/promotion/reward-types`
- 요약: 켜진 보상 종류와 각 종류의 기본 지급 방식(`ship` \| `issue` \| `grant` \| `email` \| `manual`), 가치 입력 여부.
- consumes → produces: → `reward_type`
- 출력(예시): `{"items": [{"id": "product", "acom:fulfillment": "ship", "acom:has_value": true}, {"id": "coupon", "acom:fulfillment": "issue", "acom:has_value": true}, {"id": "loyalty_points", "acom:fulfillment": "grant", "acom:has_value": true}, {"id": "digital_code", "acom:fulfillment": "email", "acom:has_value": true}], "total": 7}`
- 정책: `event.reward_types`
- 멱등성: 읽기.

### `search_events` / `get_event` — Kind: R
- 논리주소/REST: `promotion.event.search` · `GET /v1/promotion/events?status=&entry_mechanic=&selection_method=&valid_for_member_tier=&limit=&offset=` · `promotion.event.get` · `GET /v1/promotion/events/{event_id}`
- 요약: 이벤트 목록 `{items: Event[], total}` / 상세. 상세는 응모 방식·당첨 방식·보상 등수·자격·한도·추첨 기록 요약(`acom:draw_record_id`)·보상 지급 현황을 담는다.
- 에러: `event_not_found` → search_events.

### `create_event` — Kind: W
- 논리주소/REST: `promotion.event.create` · `POST /v1/promotion/events`
- 요약: 이벤트를 `draft`로 만든다. 응모 방식·당첨 방식은 `list_event_types`, 보상 종류는 `list_reward_types`에서 고른다.
- consumes → produces: `entry_mechanic`, `selection_method`, `reward_type`, `tier`?, `segment_id`? → `event_id`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명·예(예시) |
  |------|----------------------|:--:|--------|
  | name | Text | ✓ | `"Fall check-in week"` |
  | start_date / end_date | DateTime | ✓ | 응모 가능 기간 |
  | acom:display_period | object | | `{from, through}` 노출 기간(기본 = 응모 기간) |
  | acom:entry_mechanic | Text | ✓ | 레지스트리 id, 예 `check_in` |
  | acom:selection_method | Text | ✓ | 레지스트리 id, 예 `random_draw` |
  | acom:entry_payload_schema | object | | 설문 문항·UGC 형식 등. `custom`이면 필수 |
  | acom:reward_tiers | object[] | ✓ | 등수별 `{rank, reward_type, reward_ref?, acom:reward_value?: {value, currency}, winner_count, waitlist_count?, odds?}`. 예 `[{rank: 1, reward_type: "product", reward_ref: "prd_wool_overcoat", reward_value: {value: 240.00, currency: "USD"}, winner_count: 1, waitlist_count: 2}, {rank: 2, reward_type: "loyalty_points", reward_value: {value: 5.00, currency: "USD"}, winner_count: 100}]`. `instant_win`은 `odds`, `check_in`은 회차별 보상 `round` 허용 |
  | acom:eligibility | object | | `{valid_for_member_tier, segment_id, member_types, purchase_condition: {min_subtotal, window, scope{product_ids[], categories[], brands[], exclude_*[]}}, channels[]}` — `valid_for_member_tier`는 `list_tiers`의 id 또는 `"everyone"` |
  | acom:entry_limit | object | | `{count, period: day \| week \| event \| unlimited}`. 예 `{count: 1, period: "day"}` |
  | acom:schedule | object | | `{draw_at?, announce_at?, dispatch_at?}` — `draw_at` ≥ `end_date`, `dispatch_at` ≥ `draw_at` |
  | acom:compliance_jurisdictions | Text[] | | 적용할 스토어 정의 관할 id. 없으면 `event.compliance.jurisdiction_from`으로 결정 |
  | preview | Boolean | ✓ | true 먼저 |
- 출력(프리뷰): `{preview_id, changes: [{op: "create", entity: "event", ...}], checks: [{rule: "type_enabled", ...}, {rule: "combination_allowed", ...}, {rule: "reward_type_enabled", ...}, {rule: "instant_win_odds", ...}, {rule: "schedule_order", ...}, {rule: "compliance_rules", passed: true, detail: "Rank 1 reward triggers 1 store rule: collect winner info", fix_tool: null}], needs_approval: false, approval_reason: null}`
- 출력(실행): `{ok: true, id: "evt_0311", undo: {tool: "undo_change_set", until: "..."}}`
- 정책: `event.*`, 보상이 쿠폰·적립금이면 `coupon` · `loyalty`도 확인
- 에러: `type_disabled` → list_event_types · `combination_not_allowed` → list_event_types · `reward_type_disabled` → list_reward_types · `odds_exceed_one` → update_event · `unknown_tier` → list_tiers · `invalid_schedule` → update_event
- 멱등성: `Idempotency-Key`.

### `update_event` — Kind: W
- 논리주소/REST: `promotion.event.update` · `PATCH /v1/promotion/events/{event_id}` (프리뷰 먼저)
- 요약: 이벤트 수정·상태 전이(`scheduled`, `paused`, `cancelled`). 응모가 시작된 뒤에는 자격을 좁히거나 보상을 줄이는 변경을 막는다(`changes_after_entry`). 에러: `invalid_state` → get_event · `changes_after_entry` → get_event. 멱등성: `Idempotency-Key`.

### `enter_event` — Kind: W
- 논리주소/REST: `promotion.event.enter` · `POST /v1/promotion/events/{event_id}/entries`
- 요약: 쇼퍼가 이벤트에 응모. 응모 방식에 맞는 데이터·자격·참여 한도·기간을 검증한다. `instant_win` · `first_come` · `everyone_qualifies`는 응답에 판정 결과가 바로 오고, `random_draw` · `ranked`는 `run_draw`로 분리된다. `purchase_triggered`는 주문 완료 시 내부 호출도 가능.
- consumes → produces: `event_id` → `entry_id`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명 |
  |------|----------------------|:--:|------|
  | event_id | Text | ✓ | 대상 이벤트 |
  | acom:entry_payload | object | | 응모 데이터(설문 응답, 댓글·이미지, 코드 등). 이벤트의 `entry_payload_schema`를 따름 |
  | acom:order_id | Text | | `purchase_triggered`일 때 근거 주문 |
  | acom:entry_consent | object | ✓ | `event.eligibility.consent_required` 항목별 동의, 예 `{entry_terms: true, winner_contact: true}` |
  | preview | Boolean | ✓ | true 먼저 |
- 출력(프리뷰): `{preview_id, changes: [{op: "create", entity: "event_entry", id: null, from: null, to: {event_id}}], checks: [{rule: "eligibility", passed: true, ...}, {rule: "entry_limit", passed: true, detail: "1 of 1 entries today", ...}, {rule: "period_active", passed: true, ...}, {rule: "payload_valid", passed: true, ...}], needs_approval: false, approval_reason: null}`
- 출력(실행): `{ok: true, id: "ent_5521", acom:entry_status: "entered" | "won" | "lost", acom:reward?: {rank, reward_type}, undo: null}`
- 정책: `event.eligibility` · `event.entry_limit`
- 에러: `not_eligible` → get_event · `entry_limit_exceeded` → list_my_entries · `event_not_active` → get_event · `invalid_payload` → get_event · `consent_required` → (동의 요청) · `sold_out` → get_event
- 멱등성: `(event_id, member_id, period_bucket)` — 기간당 한도 1이면 같은 기간 재전송은 첫 결과.

### `list_my_entries` — Kind: R
- 논리주소/REST: `promotion.event.list_my_entries` · `GET /v1/promotion/events/entries/mine?event_id=&status=&limit=&offset=`
- 요약: 쇼퍼 본인의 응모·당첨 내역. 출력: `{items: [{entry_id, event_id, acom:entry_status (entered \| invalid \| won \| lost \| waitlist \| cancelled), acom:reward?: {rank, reward_type, acom:reward_status}, acom:compliance_pending?: ["legal_name", "address"], entered_at}], total}`. 컴플라이언스 요구가 남았으면 무엇을 내야 하는지 함께 보인다.

### `run_draw` — Kind: W$
- 논리주소/REST: `promotion.event.run_draw` · `POST /v1/promotion/events/{event_id}/draw`
- 요약: 추첨·재추첨·당첨 취소·예비 승격. `random_draw` · `ranked` 이벤트에 쓴다. 실행마다 추첨 기록(`get_draw_record`)이 남는다. 고객에게 닿으므로 항상 승인.
- consumes → produces: `event_id` → `draw_record_id`, `entry_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 |
  |------|------|:--:|------|
  | event_id | Text | ✓ | |
  | acom:mode | enum | ✓ | `draw` \| `redraw` \| `cancel_winner` \| `promote_waitlist` \| `add_waitlist` \| `manual_select` |
  | acom:rank | Integer | | 대상 등수(없으면 전 등수) |
  | winner_count / waitlist_count | Integer | | 없으면 이벤트의 등수별 값 |
  | acom:entry_ids | Text[] | | `cancel_winner` · `manual_select` 대상 |
  | acom:reason | Text | | `cancel_winner` · `manual_select` · `redraw`에 필수 |
  | preview | Boolean | ✓ | true 먼저 |
- 출력(프리뷰): `{preview_id, changes: [{op: "update", entity: "event_entry", id: "ent_5521", from: "entered", to: "won"}, ...], checks: [{rule: "entries_closed", ...}, {rule: "exclude_previous_winners", ...}, {rule: "seed_committed", passed: true, detail: "Seed hash published before close", ...}, {rule: "compliance_rules", passed: true, detail: "2 winners need winner info", ...}], needs_approval: true, approval_reason: "Picks 101 winners"}`
- 출력(실행): `{ok: true, id: "drw_0311_01", undo: null}` — 발표 후에는 되돌리지 않고 `cancel_winner` · `promote_waitlist`로 고친다.
- 정책: `event.draw`(모드·기존 당첨자 제외·취소 사유) · `event.draw.fairness` · `event.draw.announcement` · `event.compliance`
- 에러: `entries_open` → get_event · `not_draw_event` → get_event · `reason_required` → run_draw · `waitlist_empty` → run_draw(`add_waitlist`)
- 멱등성: `Idempotency-Key`. 같은 시드·같은 응모자 스냅샷이면 같은 결과(재현 가능).

### `get_draw_record` — Kind: R
- 논리주소/REST: `promotion.event.get_draw_record` · `GET /v1/promotion/events/{event_id}/draws/{draw_record_id}`
- 요약: 추첨 감사 기록. 고치지 않고 추가만 한다. 운영자·감사자용이며 쇼퍼에게는 공개 항목(방식, 시드 해시, 발표 시각)만.
- 출력(예시):
```jsonc
{"items": [{"draw_record_id": "drw_0311_01", "event_id": "evt_0311", "acom:mode": "draw",
  "acom:method": "seeded_shuffle", "acom:seed_commitment": "sha256:9c1e…", "acom:seed": "…",
  "acom:entrant_snapshot_digest": "sha256:41ab…", "acom:entrant_count": 3812,
  "acom:excluded": {"previous_winners": 0, "invalid_entries": 14},
  "acom:results": [{"rank": 1, "winners": ["ent_5521"], "waitlist": ["ent_6002", "ent_0919"]}],
  "agent": "op_mina", "acom:approved_by": "op_lee", "start_time": "2026-11-16T10:00:00"}],
 "total": 1}
```
- 정책: `event.draw.fairness.record` · `event.draw.fairness.retention_days`
- 에러: `draw_record_not_found` → get_event. 멱등성: 읽기.

### `dispatch_reward` — Kind: W$
- 논리주소/REST: `promotion.event.dispatch_reward` · `POST /v1/promotion/events/{event_id}/rewards`
- 요약: 당첨 보상을 보상 종류의 지급 방식대로 처리한다 — `ship`(배송 요청, 기본 배송지), `issue`(쿠폰 발급, `issue_coupon`과 같은 경로), `grant`(적립금·스토어 크레딧, 원장에 추가), `email`(디지털 코드·안내 발송), `manual`(운영자 처리 기록). 컴플라이언스 규칙이 걸린 당첨자는 요구(정보 제출, 원천징수, 당첨자 납부 등)가 채워질 때까지 `compliance_pending`으로 보류한다. 즉시형 이벤트의 `issue` · `grant` 보상은 정책(`event.reward_types.auto_dispatch`)상 자동 지급된다.
- consumes → produces: `event_id`, `entry_id[]`? → `reward_id[]`
- 입력: event_id(✓), acom:entry_ids(Text[], 없으면 지급 대기 당첨자 전체), acom:rank(Integer), acom:notify(Boolean, 기본 true), preview(✓)
- 출력(프리뷰): `{preview_id, changes: [{op: "create", entity: "reward", id: null, from: null, to: {entry_id: "ent_5521", reward_type: "product", fulfillment: "ship"}}, ...], checks: [{rule: "compliance_satisfied", passed: false, detail: "1 winner has not submitted required info; held", fix_tool: null}, {rule: "ship_address_present", ...}], needs_approval: true, approval_reason: "Sends rewards to 100 winners"}`
- 출력(실행): `{ok: true, id: "rwd_batch_0311", undo: null}`
- 정책: `event.reward_types` · `event.compliance` · [approval.yml](policies/approval.yml)
- 에러: `compliance_pending` → list_my_entries · `missing_address` → (당첨자에게 주소 요청) · `pool_exhausted` → (쿠폰 한도 상향 요청)
- 멱등성: `(entry_id, rank)` 한 번 지급.

---

## segment

### `define_segment` — Kind: W
- 논리주소/REST: `promotion.segment.define` · `POST /v1/promotion/segments`
- 요약: 조건 기반 대상 정의(버전 관리). 입력: name(✓), acom:condition_rules(object,✓: purchase_history / member_tier / last_order_days / exclusions), preview(✓). 출력: `acom:Segment`.

### `preview_segment` — Kind: R
- 논리주소/REST: `promotion.segment.preview` · `POST /v1/promotion/segments/preview` → `{estimated_size, sample_members}` (부작용 없음).

### `list_segments` — Kind: R
- 논리주소/REST: `promotion.segment.list` · `GET /v1/promotion/segments?limit=&offset=` → `{items, total}`.
