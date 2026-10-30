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
  | valid_for_member_tier | Text | | `"vip"` 또는 `"everyone"`. 없으면 전체 |
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
  | valid_for_member_tier | Text | | `"vip"` 또는 `"everyone"`(기본) |
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
  | valid_for_member_tier | Text | ✓ | `"vip"` 또는 `"everyone"` |
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

### `search_events` / `get_event` — Kind: R
- 논리주소/REST: `GET /v1/promotion/events?status=&limit=&offset=` · `GET /v1/promotion/events/{event_id}` → `{items: Event[], total}` / `Event`.

### `create_event` — Kind: W
- 논리주소/REST: `promotion.event.create` · `POST /v1/promotion/events`
- 요약: 이벤트 생성. 입력: name(✓), acom:event_type(`draw_*` / `instant_*`,✓), start_date / end_date(✓), acom:reward_set[] (`prize` \| `gift` \| `loyalty_points` \| `coupon`), valid_for_member_tier, acom:participation_limit, preview(✓). 출력: `Event` + `acom:DrawEvent`. 정책: `event`.

### `update_event` — Kind: W
- 논리주소/REST: `promotion.event.update` · `PATCH /v1/promotion/events/{event_id}` (프리뷰 먼저). 에러: `invalid_state` → get_event.

### `run_draw` — Kind: W$
- 논리주소/REST: `promotion.event.run_draw` · `POST /v1/promotion/events/{event_id}/draw`
- 요약: 추첨 실행(당첨·예비 선정). 입력: winner_count(✓), waitlist_count, preview(✓). 출력: 당첨자 집합. 정책: `event.tax_withholding`. 항상 승인.

### `dispatch_reward` — Kind: W$
- 논리주소/REST: `promotion.event.dispatch_reward` · `POST /v1/promotion/events/{event_id}/rewards`
- 요약: 당첨 보상 발송(경품 배송, 적립금·쿠폰 지급). 원천징수 기준을 넘는 경품은 안내 포함. 항상 승인.

### `enter_event` — Kind: W
- 논리주소/REST: `promotion.event.enter` · `POST /v1/promotion/events/{event_id}/entries`
- 요약: 쇼퍼가 이벤트에 응모. 자격·중복 참여 한도·기간 검증. 개인정보 수집·당첨 연락 동의가 있어야 하며, 추첨은 `run_draw`로 분리.
- consumes → produces: `event_id` → `entry_id`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명 |
  |------|----------------------|:--:|------|
  | event_id | Text | ✓ | 대상 이벤트 |
  | acom:answers | object | | 응모 설문·참여 데이터 |
  | acom:entry_consent | Boolean | ✓(=true) | 개인정보 수집·당첨 연락 동의 |
  | preview | Boolean | ✓ | true 먼저 |
- 출력(프리뷰): `{preview_id, changes: [{op: "create", entity: "event_entry", id: null, from: null, to: {event_id}}], checks: [{rule: "eligible_member_tier", passed: true, ...}, {rule: "participation_limit", passed: true, ...}, {rule: "period_active", passed: true, ...}], needs_approval: false, approval_reason: null}`
- 출력(실행): `{ok: true, id: "ent_5521", undo: null}`
- 정책: `event.participation_limit`, 응모 동의.
- 에러: `not_eligible` → get_event · `participation_limit_exceeded` → list_my_entries · `event_not_active` → get_event · `consent_required` → (동의 요청)
- 멱등성: `(event_id, member_id)` — 한도가 1이면 upsert.

### `list_my_entries` — Kind: R
- 논리주소/REST: `promotion.event.list_my_entries` · `GET /v1/promotion/events/entries/mine?event_id=&status=&limit=&offset=`
- 요약: 쇼퍼 본인의 응모·당첨 내역. 출력: `{items: [{entry_id, event_id, acom:entry_number, acom:entry_status (entered \| won \| lost \| waitlist), acom:reward, entered_at}], total}`.

---

## segment

### `define_segment` — Kind: W
- 논리주소/REST: `promotion.segment.define` · `POST /v1/promotion/segments`
- 요약: 조건 기반 대상 정의(버전 관리). 입력: name(✓), acom:condition_rules(object,✓: purchase_history / member_tier / last_order_days / exclusions), preview(✓). 출력: `acom:Segment`.

### `preview_segment` — Kind: R
- 논리주소/REST: `promotion.segment.preview` · `POST /v1/promotion/segments/preview` → `{estimated_size, sample_members}` (부작용 없음).

### `list_segments` — Kind: R
- 논리주소/REST: `promotion.segment.list` · `GET /v1/promotion/segments?limit=&offset=` → `{items, total}`.
