# API 명세 — `fulfillment` (클레임[반품·취소·교환]·검수·환불·배송·배송비)

> 계층: `fulfillment.{resource}.{operation}`. 공통 규약(응답 형식·에러·프리뷰/실행·멱등·헤더)은 [README.md](README.md) §3, 데이터 타입은 [data-model.md](data-model.md), 정책은 [fulfillment.yml](policies/fulfillment.yml) · [approval.yml](policies/approval.yml). MVP 툴의 시그니처는 [plan.md §3.3·§3.4](../plan.md)와 같습니다.
> **설계 축**: ① 반품 요청(`ReturnAction`)은 운영 에이전트가 **정책 근거와 함께 묶어 검토**하고, 환불·고객 메시지는 한 변경셋으로 사람이 승인, ② 반품 요청을 **보내는 것은 쇼퍼 본인**(사람 전용 엔드포인트, 툴 아님), ③ 환불은 원결제수단으로, 상태는 결제 이벤트로 확정(조용한 실패 없음), ④ 검수 결과가 클레임 상태·비용·재고 복원을 구동, ⑤ 귀책 → 비용 규칙은 정책 데이터, ⑥ 상태 변화는 이벤트 테이블에 append.

> ⑦ 취소·교환·반품은 같은 모양: 쇼퍼 에이전트는 **선택지 툴(R)** 로 대상·사유·비용·환불 예상을 준비하고, 보내기는 쇼퍼 버튼(사람 전용), 승인이 필요한 건은 운영 큐의 클레임 → `approve_claim`(변경셋).

리소스: [claim](#claim) · [inspection](#inspection) · [refund](#refund) · [parcel_delivery](#parcel_delivery) · [shipping_rate](#shipping_rate) · [사람 전용 엔드포인트](#사람-전용-엔드포인트)

| resource | operations |
|---|---|
| claim | **`search_return_requests`** R · **`evaluate_return_request`** R · **`get_return_options`** R · `get_cancel_options` R · `get_exchange_options` R · `search_claims` R · `get_claim` R · `approve_claim` W$ · `reject_claim` W · `cancel_order_lines` W$ · `convert_exchange_to_return` W · `upload_evidence` W · 사람 전용 `POST /returns`, `POST /orders/{order_id}/cancel`, `POST /exchanges`, `POST /claims/{claim_id}/pay-charge`, `POST /claims/{claim_id}/withdraw` |
| inspection | `assess_return_condition` W |
| refund | **`refund_return`** W$ · `get_refund` R · `retry_refund` W$ |
| parcel_delivery | `get_delivery_dates` R · `get_delivery` R · `track_shipment` R · `schedule_shipment` W · `confirm_shipment` W · `register_shortage` W |
| shipping_rate | `list_shipping_zones` R · `manage_shipping_zone` W · `set_shipping_rate` W |

---

## 상태 요약

**claim — 반품 요청(`ReturnAction`)** (MVP 백오피스 큐의 `status`)

```
(쇼퍼) POST /returns ──▶ waiting ──refund_return 승인·실행──▶ approved ──▶ picked_up ──▶ inspected ──▶ refunded
                           │
                           ├─send_customer_message(kind="ask_for_photos")──▶ photos_requested ──사진 도착──▶ waiting
                           └─send_customer_message(kind="decline") / reject_claim──▶ declined
```

**claim — 취소(`CancelAction`)** (출고 전만. 라인 상태가 경로를 정함, [fulfillment.yml](policies/fulfillment.yml) `cancel`)

| 라인 상태 | 쇼퍼 | 결과 |
|---|---|---|
| `payment_due`(결제 전) | 직접 | 즉시 `completed`. 환불 없음, 재고 복원 |
| `processing`, `ship_wait`(창고 인계 전·재고 대기) | 직접 | 추가 비용이 없으면 자동 `approved` → 환불 확정 시 `completed`. 추가 비용이 환불액보다 크면 `charge_due` |
| `ship_instructed`, `ship_confirmed`(피킹·포장 중) | 요청만 | `requested` 클레임 → 운영 큐 → `approve_claim`(출고를 멈출 수 있을 때) 또는 `reject_claim(already_handed_over)` |
| `shipped`, `delivered` | 불가 | `already_shipped` 에러 → `get_return_options`(반품 경로) |

```
(쇼퍼) POST /orders/{order_id}/cancel
   ├─ 결제 전 ─────────────────────────────────────────────────────────────▶ completed
   ├─ 직접 취소 상태 · charge_due == 0 ──(자동 승인)──▶ approved ──환불 확정──▶ completed
   ├─ 직접 취소 상태 · 추가 비용 > 환불액 ──▶ charge_due ──POST /claims/{id}/pay-charge──▶ approved ─▶ …
   └─ 운영자 전용 상태 ──▶ requested ──approve_claim(W$)──▶ approved ──환불 확정──▶ completed
                              └─reject_claim──▶ declined (출고됨 → 반품 안내)
(운영자) cancel_order_lines(W$, 품절·직권) ──승인──▶ approved ──▶ completed
```
- 취소는 철회할 수 없습니다(`revocable: false`). 환불 실패는 클레임을 `approved`에 두고 환불을 수동 큐로 보냅니다(`refund.on_failure`).

**claim — 교환** (출고 후. **회수 단계 → 재배송 단계**가 순서대로, 맞교환 없음, [fulfillment.yml](policies/fulfillment.yml) `exchange`)

```
(쇼퍼) POST /exchanges
   ├─ 쇼퍼 부담 비용 있음 ──▶ charge_due ──pay-charge──┐
   └─ 비용 없음 ───────────────────────────────────────┴─▶ requested
회수:   requested ──(자동 또는 approve_claim)──▶ pickup_instructed ──회수 송장──▶ pickup_confirmed
          ──택배 이벤트──▶ picked_up ──assess_return_condition──▶ inspected
재배송: inspected(normal) ──▶ reship_requested ──schedule_shipment──▶ ship_instructed ──▶ ship_confirmed
          ──confirm_shipment──▶ shipped ──▶ delivered = completed
분기:   charge_due · requested · pickup_instructed ──POST /claims/{id}/withdraw──▶ withdrawn (라인은 delivered로)
        pickup_confirmed ~ ship_instructed ──convert_exchange_to_return──▶ 반품 클레임(회수·검수 정보 승계) ──▶ 환불
        inspected(damaged, 쇼퍼 귀책) ──reject_claim(inspection_failed)──▶ declined (반송 협의)
```

**claim — 반품** 직접 요청도 같은 모양입니다: `get_return_options` → (쇼퍼) `POST /returns` → `charge_due`(차감할 환불액보다 비용이 클 때만) 또는 `waiting` → 위 반품 흐름.

**parcel_delivery — 배송** (라인별 상태, 송장 여러 개 가능, 모든 라인이 `shipped`면 주문 완료)

```
ship_instructed ──schedule_shipment──▶ ship_confirmed ──confirm_shipment──▶ shipped ──택배 이벤트──▶ delivered
        └─ 재고 부족 ──register_shortage──▶ ship_wait ──(입고)──▶ ship_instructed
```

- 불가능한 전이는 `invalid_state` 에러. 상태 변화는 `return_action_event`, `parcel_delivery_event`에 append되고 덮어쓰지 않습니다.
- 택배사 추적 이벤트 수신은 이 도메인의 툴이 아닙니다. `integration`의 `receive_carrier_event`가 받아 배송 상태를 전이합니다.

---

## claim

### `search_return_requests` — Kind: R
- 논리주소/REST: `fulfillment.claim.search_return_requests` · `GET /v1/fulfillment/return-requests?status=waiting&min_amount=200.00&limit=20&offset=0`
- 요약: 처리를 기다리는 반품 요청 목록. 운영 에이전트가 "$200 넘는 반품 12건"처럼 큐를 묶어 검토할 때 첫 호출. MVP 툴([plan.md §3.3](../plan.md)).
- consumes → produces: → `return_request_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | status | Text(enum) | ✓ | `"waiting"` \| `"photos_requested"` \| `"approved"` \| `"refunded"` \| `"declined"` |
  | min_amount | Number | | 이 금액 이상만. 예 `200.00` |
  | limit | Integer | | 기본 20 |
  | offset | Integer | | 기본 0 |
- 출력:
  ```jsonc
  {"items": [{"return_request_id": "ret_20402", "order_id": "ord_20402", "order_number": "#20402",
              "customer_id": "cus_alex", "item": "Leather chelsea boots", "line_id": "line_1",
              "amount": 312.00, "price_currency": "USD", "reason": "defect", "reason_text": "Sole coming apart",
              "photos": 2, "days_since_delivery": 6, "status": "waiting"}],
   "total": 12}
  ```
- 정책: 없음(읽기). `days_since_delivery`는 `return_window_days`(30)와 비교할 값입니다.
- 에러: `invalid_status` → `search_return_requests`(허용 값 안내)
- 멱등성: 읽기, 부작용 없음.

### `evaluate_return_request` — Kind: R
- 논리주소/REST: `fulfillment.claim.evaluate_return_request` · `GET /v1/fulfillment/return-requests/{return_request_id}/evaluation`
- 요약: 반품 요청 한 건에 정책을 적용해 처리안을 제안. 결과는 제안일 뿐 아무것도 바꾸지 않으며, 실행은 `refund_return` 또는 `send_customer_message`로 합니다. schema.org `AssessAction`. MVP 툴.
- consumes → produces: `return_request_id` → suggestion
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | return_request_id | Text | ✓ | `"ret_20402"` |
- 출력:
  ```jsonc
  {"items": [{"return_request_id": "ret_20402",
              "suggested": "refund",                    // "refund" | "ask_for_photos" | "decline"
              "rule": "defect_refund",
              "rule_text": "Defects are refunded in full, return shipping free",
              "refund_amount": 312.00, "price_currency": "USD",
              "return_shipping_paid_by": "store"}],
   "total": 1}
  ```
  - 예: `ret_20366`(파손, 사진 없음) → `"ask_for_photos"`, rule `damage_claim_needs_photo`. `ret_20290`(배송 후 39일) → `"decline"`, rule `return_window_days`. 단순 변심 → `"refund"`, rule `changed_mind_return_shipping`(반품 배송비 고객 부담).
- 정책: `return_window_days`(30), `defect_refund`, `changed_mind_return_shipping`, `damage_claim_needs_photo` ([fulfillment.yml](policies/fulfillment.yml))
- 에러: `return_request_not_found` → `search_return_requests` · `invalid_state`(이미 처리됨) → `get_claim`
- 멱등성: 읽기, 부작용 없음.

### `get_return_options` — Kind: R
- 논리주소/REST: `fulfillment.claim.get_return_options` · `GET /v1/fulfillment/orders/{order_id}/lines/{line_id}/return-options?reason=defect`
- 요약: 쇼퍼 에이전트가 쇼퍼 본인의 주문 라인에 대해 고를 수 있는 선택지를 보여줌. 선택지마다 시점, 쇼퍼가 내는 비용, 누가 확정하는지를 함께 돌려줍니다. 요청을 **보내는 것은** 쇼퍼의 버튼(`POST /returns`)뿐입니다. MVP 툴(P1).
- consumes → produces: `order_id` → options
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | order_id | Text | ✓ | `"ord_20402"`(쇼퍼 본인 주문만) |
  | line_id | Text | ✓ | `"line_1"` |
  | quantity | Integer | | 반품할 수량. 생략 시 라인 전체. 사은품이 딸린 라인은 전체만(`free_gift_lines`) |
  | reason | Text(enum) | ✓ | `"defect"` \| `"damaged"` \| `"wrong_size"` \| `"not_as_described"` \| `"changed_mind"` |
- 출력:
  ```jsonc
  {"items": [
     {"option": "replacement", "title": "New pair, same size", "ships": "2026-11-06", "arrives": "2026-11-10",
      "you_pay": 0.00, "confirmed_by": "store, automatic"},
     {"option": "refund", "title": "Full refund", "amount": 312.00, "price_currency": "USD",
      "to": "card ending 0937", "confirmed_by": "a person at the store, usually within 1 business day",
      "money_back": "3–5 days after it is confirmed", "you_pay": 0.00}],
   "send_back": {"cost": 0.00, "why": "defect", "pickup": "2026-11-09T09:00/12:00"},
   "total": 2}
  ```
  - `replacement`는 같은 변형으로 바꾸는 교환입니다(`return.replacement_option: creates_exchange`). 다른 사이즈·색이면 `get_exchange_options`.
  - 변심 반품은 반품 배송비와 무료 배송 회수분(`lost_free_shipping`)을 환불액에서 먼저 뺍니다. 뺄 환불액보다 크면 `send_back.charge_due`가 생기고 요청은 결제 후 접수됩니다.
- 정책: `return_window_days`, `defect_refund`, `changed_mind_return_shipping`(변심이면 `you_pay`에 반품 배송비), `size_exchange_days`, `refund_without_person_max`(초과 금액이면 `confirmed_by`가 사람), `return.scope`, `return.restore_discounts_when`
- 에러: `order_not_found` → `get_my_orders` · `line_not_found` → `get_order` · `outside_return_window` → (선택지 없음, 사유 안내) · `not_your_order` → `get_my_orders` · `not_shipped_yet` → `get_cancel_options` · `claim_in_progress` → `get_claim`
- 멱등성: 읽기, 부작용 없음.

### `get_cancel_options` — Kind: R
- 논리주소/REST: `fulfillment.claim.get_cancel_options` · `GET /v1/fulfillment/orders/{order_id}/cancel-options?lines=line_1:1,line_2:2&reason=changed_mind`
- 요약: 쇼퍼 에이전트가 취소를 **준비**하는 툴. 쇼퍼 본인 주문에서 취소할 수 있는 라인·수량, 고를 수 있는 사유(귀책 포함), 환불 예상과 결제수단별 내역, 복원되는 쿠폰·적립금, 배송비 재계산으로 생기는 추가 비용, 누가 확정하는지를 돌려줍니다. 아무것도 바꾸지 않으며, 취소를 **보내는 것은** 쇼퍼의 버튼(`POST /orders/{order_id}/cancel`)뿐입니다. schema.org `CancelAction`(`potential`).
- consumes → produces: `order_id` → cancel options
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | order_id | Text | ✓ | `"ord_20455"`(쇼퍼 본인 주문만) |
  | lines | {line_id, quantity}[] | | 취소해 볼 라인과 수량. 생략하면 **전체 취소**로 계산. 예 `[{"line_id": "line_1", "quantity": 1}]` |
  | reason | Text(enum) | | 사유를 주면 그 귀책으로 배송비·복원을 계산. 생략 시 `changed_mind`(쇼퍼 귀책, 가장 보수적인 견적) |
- 출력:
  ```jsonc
  {"items": [
     {"line_id": "line_1", "name": "Packable windbreaker, M", "ordered": 1, "cancellable_quantity": 1,
      "line_status": "processing", "path": "self_service",        // "self_service" | "store_review" | "not_cancellable"
      "unit_paid": 71.20, "price_currency": "USD"},
     {"line_id": "line_2", "name": "Merino socks, 2-pack", "ordered": 2, "cancellable_quantity": 2,
      "line_status": "processing", "path": "self_service", "unit_paid": 16.00,
      "whole_quantity_only": false}],
   "reasons": [{"reason": "changed_mind", "acom:liability": "customer_fault"},
               {"reason": "wrong_option_ordered", "acom:liability": "customer_fault"},
               {"reason": "found_lower_price", "acom:liability": "customer_fault"},
               {"reason": "duplicate_order", "acom:liability": "customer_fault"},
               {"reason": "product_info_inaccurate", "acom:liability": "store_fault", "needs_store_review": true}],
   "estimate": {                                    // 요청한 lines·reason 기준
     "scope": "partial",                            // "full" | "partial"
     "cancelled_amount": 71.20,
     "extra_charge": [{"kind": "lost_free_shipping", "amount": 6.95,
                       "why": "What stays ($32.00) is under the $80.00 free-shipping minimum"}],
     "discount_recalculated": [],                   // 주문 쿠폰 조건 미달로 회수되는 할인
     "refund_total": 64.25,
     "refund_breakdown": [{"to": "card ending 4821", "method": "credit_card", "amount": 64.25,
                           "money_back": "3–5 business days after it is confirmed"},
                          {"to": "loyalty points", "method": "loyalty_points", "amount": 0.00}],
     "coupons_restored": [],
     "points_pending_cancelled": 0.71,
     "charge_due": 0.00,                            // 환불액으로 못 덮는 추가 비용(먼저 결제)
     "needs_refund_destination": false,             // 원결제 되돌리기 불가 시 true → refund_to 필요
     "confirmed_by": "store, automatic",            // 운영자 전용 상태가 섞이면 "a person at the store"
     "revocable": false},
   "not_cancellable": [],                           // 예 [{"line_id": "line_3", "why": "shipped", "next_tool": "get_return_options"}]
   "total": 2}
  ```
  - 같은 주문 전체 취소(`lines` 생략): `refund_total` 103.20 = card 93.20 + loyalty points 10.00(현금성 수단 먼저, 적립금은 나머지), `coupons_restored: [{"coupon_id": "cpn_socks4", "amount": 4.00}]`, 배송비 재계산 없음.
  - 사은품이 딸린 라인은 `whole_quantity_only: true`이고 `cancellable_quantity`는 주문 수량 그대로이며, 사은품도 함께 취소됩니다.
- 정책: `cancel.self_service_states`, `cancel.operator_only_states`, `cancel.auto_approve_when`, `cancel.reasons`, `cancel.shopper_selectable_store_reasons`, `cancel.restore`, `cancel.shipping_recalculation`, `cancel.extra_charge`, `cancel.free_gift_lines`, `refund.priority`, `refund.split`, `refund.reversal_window_days`, `free_shipping_min_subtotal`([order.yml](policies/order.yml))
- 에러: `order_not_found` → `get_my_orders` · `not_your_order` → `get_my_orders` · `line_not_found` → `get_order` · `quantity_over_cancellable` → `get_cancel_options` · `whole_quantity_only` → `get_cancel_options`(전체 수량으로) · `already_shipped` → `get_return_options` · `already_cancelled` → `get_claim` · `claim_in_progress` → `get_claim` · `reason_not_selectable` → `get_cancel_options`(허용 사유 안내)
- 멱등성: 읽기, 부작용 없음. 견적은 `POST /orders/{order_id}/cancel` 시점에 다시 계산되며, 달라지면 그쪽이 `estimate_changed`로 거부합니다.

### `get_exchange_options` — Kind: R
- 논리주소/REST: `fulfillment.claim.get_exchange_options` · `GET /v1/fulfillment/orders/{order_id}/lines/{line_id}/exchange-options?reason=wrong_size&quantity=1`
- 요약: 쇼퍼 에이전트가 교환을 **준비**하는 툴. 같은 상품의 바꿀 수 있는 변형(재고·가격 차이), 회수 일정, 재배송 예상일, 쇼퍼가 내는 비용과 결제 시점, 교환이 안 될 때의 대안(반품)을 돌려줍니다. 보내기는 쇼퍼의 버튼(`POST /exchanges`)뿐입니다.
- consumes → produces: `order_id`, `line_id` → exchange options
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | order_id | Text | ✓ | `"ord_20388"`(쇼퍼 본인 주문만) |
  | line_id | Text | ✓ | `"line_2"` |
  | reason | Text(enum) | ✓ | 쇼퍼 귀책 `"wrong_size"` \| `"different_option_wanted"`, 판매자 귀책 `"defect"` \| `"damaged"` \| `"missing_parts"` \| `"wrong_item_sent"` \| `"wrong_option_sent"` |
  | quantity | Integer | | 교환할 수량. 생략 시 라인 전체 |
  | pickup_address | Text \| PostalAddress | | 회수지. 생략 시 배송지. 지역 추가비가 다시 계산됨 |
  | delivery_address | Text \| PostalAddress | | 재배송지. 생략 시 배송지 |
- 출력:
  ```jsonc
  {"items": [
     {"variant_id": "prd_trail_runner_10_slate", "size": "10", "color": "Slate", "availability": "in_stock",
      "available_quantity": 7, "price": 128.00, "price_difference": 0.00, "price_currency": "USD", "selectable": true},
     {"variant_id": "prd_trail_runner_10_5_slate", "size": "10.5", "color": "Slate", "availability": "out_of_stock",
      "available_quantity": 0, "price": 128.00, "price_difference": 0.00, "selectable": false, "why": "out_of_stock",
      "next_tool": "subscribe_restock"},
     {"variant_id": "prd_trail_runner_10_ember", "size": "10", "color": "Ember", "availability": "in_stock",
      "price": 138.00, "price_difference": 10.00, "selectable": false, "why": "price_differs"}],
   "line": {"line_id": "line_2", "name": "Trail runner, 9, Slate", "exchangeable_quantity": 1,
            "days_since_delivery": 5, "window_days": 30, "path": "self_service"},   // 기간 후면 "store_review"
   "acom:liability": "customer_fault",
   "pickup": {"earliest": "2026-11-09T09:00/12:00", "address": "12 Harbor St, Apt 3", "cost": 0.00},
   "reship": {"ships_after": "inspected", "ships": "2026-11-12", "arrives": "2026-11-14", "cost": 0.00},
   "you_pay": 0.00,                   // wrong_size → size_exchange_fee(0.00). different_option_wanted면 왕복 13.90
   "pay_when": null,                  // 비용이 있으면 "before the request is accepted"
   "confirmed_by": "store, automatic",
   "if_not_possible": {"option": "return", "next_tool": "get_return_options",
                       "why": "Used if the new size runs out before it ships, or you decline it"},
   "total": 3}
  ```
  - 판매자 귀책(`defect` 등)이면 같은 변형(새 상품)도 `selectable: true`이고 회수·재배송비는 스토어가 냅니다.
  - 바꿀 변형이 하나도 없으면 `items`의 `selectable`이 모두 `false`이고, 에이전트는 `if_not_possible`의 반품 경로를 보여줍니다.
- 정책: `exchange.eligible_states`, `exchange.window`, `exchange.not_exchangeable_when`, `exchange.replacement`, `exchange.shipping_cost`, `exchange.pickup`, `exchange.reship_after`, `size_exchange_fee`, `delivery.remote_area_surcharge`
- 에러: `order_not_found` → `get_my_orders` · `not_your_order` → `get_my_orders` · `line_not_found` → `get_order` · `not_shipped_yet` → `get_cancel_options`(출고 전이면 취소 후 다시 주문) · `outside_exchange_window` → `get_return_options` · `not_exchangeable` → `get_return_options` · `no_replacement_available` → `get_return_options` · `claim_in_progress` → `get_claim`
- 멱등성: 읽기, 부작용 없음. 재고는 `POST /exchanges` 때 예약됩니다(`replacement.reserve_stock_on`).

### `search_claims` — Kind: R
- 논리주소/REST: `fulfillment.claim.search` · `GET /v1/fulfillment/claims`
- 요약: 반품·취소·교환 클레임 전체 검색. 반품 요청 큐만 볼 때는 `search_return_requests`를 씁니다.
- consumes → produces: → `claim_id[]`
- 입력: `type`(`return` \| `cancel` \| `exchange`), `status`(취소 `charge_due` \| `requested` \| `approved` \| `completed` \| `declined`, 교환 `charge_due` \| `requested` \| `pickup_instructed` \| `pickup_confirmed` \| `picked_up` \| `inspected` \| `reship_requested` \| `shipped` \| `completed` \| `withdrawn` \| `converted` \| `declined`), `acom:liability`, `order_id`, `customer_id`, `created_from`, `created_through`, `sort`(`newest` \| `amount_desc`), `fields?[]`, `limit`(기본 20), `offset`(기본 0)
- 출력: `{"items": [{"claim_id": "clm_3301", "type": "exchange", "status": "requested", "order_id": "ord_20388", "amount": 128.00, "price_currency": "USD"}], "total": 7}`
- 정책: 없음(읽기).
- 에러: `invalid_filter` → `search_claims`
- 멱등성: 읽기.

### `get_claim` — Kind: R
- 논리주소/REST: `fulfillment.claim.get` · `GET /v1/fulfillment/claims/{claim_id}`
- 요약: 클레임 한 건(상태 이력·증빙·검수·비용·연결된 환불). 반품 요청 id(`ret_…`)도 받습니다.
- consumes → produces: `claim_id` → claim
- 입력: `claim_id`(✓), `fields?[]`
- 출력: `{"items": [{"claim_id": "ret_20402", "type": "return", "object": {"order_id": "ord_20402", "line_id": "line_1"}, "reason": "defect", "acom:liability": "store_fault", "evidence": [...], "inspection": null, "refund_id": null, "status_history": [{"status": "waiting", "at": "2026-11-06T10:12:00"}]}], "total": 1}`
- 정책: 없음(읽기).
- 에러: `claim_not_found` → `search_claims`
- 멱등성: 읽기.

### `approve_claim` — Kind: W$
- 논리주소/REST: `fulfillment.claim.approve` · `POST /v1/fulfillment/claims/{claim_id}/approve`
- 요약: 운영 큐에서 `requested` 상태인 취소·교환 클레임(및 환불 없이 진행할 반품의 회수 지시)을 승인. 자동 승인되지 않은 건만 여기에 옵니다: 피킹·포장 중인 라인의 취소 요청, 판매자 귀책 사유를 쇼퍼가 고른 취소, 기간 후·사진 필요·가격 차이 교환. 승인되면 취소는 출고 중단·재고 복원·쿠폰/적립금 복원·환불 개시, 교환은 회수 지시가 이어집니다. 반품 환불은 `refund_return`으로 합니다.
- consumes → produces: `claim_id[]` → `claim_id[]`(approved \| pickup_instructed)
- 입력: `claim_id[]`(✓), `acom:liability`(`store_fault` \| `customer_fault`, 미지정 시 접수값·검수값. 바꾸면 배송비·복원을 다시 계산), `acom:cost_override`(배송비 조정, `note` 필수), `preview`(✓)
- 출력(프리뷰, 피킹 중 취소 1건):
  ```jsonc
  {"preview_id": "pv_c39e",
   "changes": [{"op": "update", "entity": "claim", "id": "clm_4410", "from": "requested", "to": "approved"},
               {"op": "update", "entity": "parcel_delivery", "id": "dlv_20455", "from": "ship_instructed", "to": "cancelled"},
               {"op": "update", "entity": "inventory", "id": "prd_windbreaker_m_navy", "from": 11, "to": 12},
               {"op": "create", "entity": "refund", "id": "clm_4410", "from": null, "to": 64.25}],
   "checks": [{"rule": "cancel.operator_only_states", "passed": true, "detail": "Line is being packed and not yet with the carrier", "fix_tool": "reject_claim"},
              {"rule": "cancel.shipping_recalculation", "passed": true, "detail": "Customer fault: $6.95 shipping taken from the refund (what stays is under $80.00)", "fix_tool": "approve_claim"},
              {"rule": "cancel.restore", "passed": true, "detail": "No coupon on this line; 0.71 pending points cancelled", "fix_tool": null},
              {"rule": "refund_to_original_payment", "passed": true, "detail": "$64.25 back to card ending 4821", "fix_tool": null}],
   "needs_approval": true, "approval_reason": "Cancels 1 line and refunds $64.25"}
  ```
- 출력(프리뷰, 교환): `changes` `clm_3301` `requested` → `pickup_instructed`, 대체 변형 재고 예약 1. `checks` `liability_cost`("Seller fault: store pays both shipping legs"), `exchange.replacement`("Size 10 in stock, same price"), `inspection_before_restock`.
- 출력(실행): `{"ok": true, "id": "clm_4410", "undo": null}` — 환불이 따르는 취소는 되돌릴 수 없어 `undo`가 `null`. 교환 승인은 회수 송장 전까지 `{"tool": "undo_change_set", "until": …}`.
- 프리뷰: `checks` `liability_cost`(귀책 ↔ 비용 일관), `cancel.operator_only_states`(아직 택배사에 넘기지 않았는지), `cancel.shipping_recalculation`, `cancel.restore`, `refund_to_original_payment`, `exchange.replacement`(재고·가격), `inspection_before_restock`. 항상 변경셋을 거쳐 사람이 승인.
- 정책: `liability`, `exchange`, `cancel`, `refund` ([fulfillment.yml](policies/fulfillment.yml)), [approval.yml](policies/approval.yml)
- 에러: `liability_cost_conflict` → `approve_claim`(귀책·비용 다시 지정) · `already_handed_over`(이미 택배사 인계) → `reject_claim` · `replacement_unavailable` → `convert_exchange_to_return` · `inspection_pending` → `assess_return_condition` · `charge_unpaid` → `get_claim` · `invalid_state` → `get_claim`
- 멱등성: `Idempotency-Key = hash(claim_id[] + decision)` — 중복 승인 없음.

### `reject_claim` — Kind: W
- 논리주소/REST: `fulfillment.claim.reject` · `POST /v1/fulfillment/claims/{claim_id}/reject`
- 요약: 클레임 반려(사유 필수). `requested`/`waiting` → `declined`. 쇼퍼에게 알리는 문구는 `send_customer_message(kind="decline")`로 따로 보내며 그쪽이 승인을 받습니다.
- consumes → produces: `claim_id` → `claim_id`
- 입력: `claim_id`(✓), `reason`(✓, `outside_return_window` \| `not_returnable` \| `inspection_failed` \| `already_handed_over`(취소: 이미 택배사 인계 → 쇼퍼에게 반품 안내) \| `outside_exchange_window`), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_c52b", "changes": [{"op": "update", "entity": "claim", "id": "ret_20290", "from": "waiting", "to": "declined"}], "checks": [{"rule": "return_window_days", "passed": true, "detail": "Delivered Sep 28, 39 days ago (limit 30)", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "ret_20290", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 프리뷰: 반려 사유가 정책과 맞는지 check. 같은 요청의 거절 메시지와 한 변경셋으로 묶이면 그 변경셋 승인을 따릅니다.
- 정책: `return_window_days`, `return.non_returnable_when`
- 에러: `invalid_state` → `get_claim` · `reason_required` → `reject_claim`
- 멱등성: `Idempotency-Key = hash(claim_id + reason)`

### `cancel_order_lines` — Kind: W$
- 논리주소/REST: `fulfillment.claim.cancel_order_lines` · `POST /v1/fulfillment/orders/{order_id}/cancellations`
- 요약: **스토어가 먼저** 출고 전 라인을 취소(품절·결품 지속, 사은품 소진, 판매자 사정, 상담원 직권). 쇼퍼의 취소 요청이 아니라 운영 에이전트가 제안하고 사람이 승인합니다. 귀책은 판매자이므로 배송비 재계산·추가 비용이 없고, 쿠폰은 복원, 적립 예정분은 취소, 환불은 원결제수단으로. 쇼퍼 안내 메시지(`send_customer_message`)와 한 변경셋으로 묶입니다.
- consumes → produces: `order_id` → `claim_id`(type `cancel`)
- 입력: `order_id`(✓), `lines[]`(✓, `{line_id, quantity}`), `reason`(✓, `out_of_stock` \| `free_gift_unavailable` \| `seller_cancelled` \| `store_cancelled` \| `shipping_delay`), `note`, `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_c71b", "changes": [{"op": "create", "entity": "claim", "id": "clm_4433", "from": null, "to": "approved"}, {"op": "update", "entity": "parcel_delivery", "id": "dlv_20461", "from": "ship_wait", "to": "cancelled"}, {"op": "create", "entity": "refund", "id": "clm_4433", "from": null, "to": 54.00}], "checks": [{"rule": "cancel.not_cancellable_states", "passed": true, "detail": "Line is waiting for stock, not shipped", "fix_tool": null}, {"rule": "cancel.restore", "passed": true, "detail": "Seller fault: 1 coupon restored, no shipping charge", "fix_tool": null}], "needs_approval": true, "approval_reason": "Store-initiated cancel refunds $54.00"}`
- 출력(실행): `{"ok": true, "id": "clm_4433", "undo": null}`
- 정책: `cancel.reasons.store_fault`, `cancel.restore`, `cancel.not_cancellable_states`, `refund`, [approval.yml](policies/approval.yml)
- 에러: `already_shipped` → `get_delivery` · `line_not_found` → `get_order` · `reason_not_store_fault` → `cancel_order_lines` · `whole_quantity_only` → `cancel_order_lines` · `claim_in_progress` → `get_claim`
- 멱등성: `Idempotency-Key = hash(order_id + sorted(line_id:quantity) + reason)`

### `convert_exchange_to_return` — Kind: W
- 논리주소/REST: `fulfillment.claim.convert_exchange_to_return` · `POST /v1/fulfillment/claims/{claim_id}/convert-to-return`
- 요약: 교환을 반품으로 전환(대체 변형 품절·단종, 쇼퍼가 받지 않겠다고 함). 회수 송장이 잡힌 뒤부터 재배송 출고 전까지 가능합니다. 교환 클레임은 `converted`로 닫히고, 라인·수량·사유·회수지·회수 송장·검수 결과를 **승계한** 반품 클레임(`ret_…`)이 생겨 이후 반품 흐름(`refund_return`)을 탑니다. 쇼퍼가 낸 교환 배송비는 자동 환불되고, 예약한 대체 재고는 풀립니다. 스토어 사정(품절·단종)이면 귀책이 판매자로 바뀌어 반품 비용이 없습니다. 전환 전에 쇼퍼 안내(`send_customer_message`)가 같은 변경셋에 있어야 합니다.
- consumes → produces: `claim_id` → `return_request_id`
- 입력: `claim_id`(✓), `reason`(✓, `replacement_out_of_stock` \| `replacement_discontinued` \| `customer_declines`), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_c60d", "changes": [{"op": "update", "entity": "claim", "id": "clm_3301", "from": "picked_up", "to": "converted"}, {"op": "create", "entity": "return_request", "id": "ret_20388", "from": null, "to": "waiting"}, {"op": "update", "entity": "inventory", "id": "prd_trail_runner_10_slate", "from": "reserved 1", "to": "released 1"}], "checks": [{"rule": "exchange.fallback_to_return_when", "passed": true, "detail": "Replacement size 10 is out of stock", "fix_tool": null}, {"rule": "exchange.on_fallback", "passed": true, "detail": "Liability moves to seller fault: no return shipping charged", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "ret_20388", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: `exchange.fallback_to_return_when`, `exchange.allowed_conversion_states`, `exchange.on_fallback`
- 에러: `not_an_exchange` → `get_claim` · `invalid_state`(이미 재배송 출고) → `get_claim` · `customer_not_notified` → `send_customer_message`
- 멱등성: `Idempotency-Key = hash(claim_id + "convert")`

### `upload_evidence` — Kind: W
- 논리주소/REST: `fulfillment.claim.upload_evidence` · `POST /v1/fulfillment/claims/{claim_id}/evidence`
- 요약: 클레임에 증빙(파손 사진·배송 사진·영수증) 첨부. 검수·승인·귀책 판단의 근거가 됩니다. append만 하며 지우지 않고 무효 표시만 합니다. 쇼퍼가 사진 요청을 받고 올린 사진도 여기로 들어와 요청을 `photos_requested` → `waiting`으로 되돌립니다.
- consumes → produces: `claim_id` → `evidence_id[]`
- 입력: `claim_id`(✓), `media[]`(✓, 업로드 ref, schema.org `MediaObject`), `evidence_type`(✓, `damage_photo` \| `delivery_photo` \| `product_defect` \| `receipt` \| `other`), `note`, `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_e11f", "changes": [{"op": "create", "entity": "evidence", "id": "evd_901", "from": null, "to": "damage_photo"}], "checks": [{"rule": "evidence_media_type", "passed": true, "detail": "2 JPEG photos", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "evd_901", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: 증빙 append-only, 결제정보가 보이는 증빙은 가림 처리.
- 에러: `claim_not_found` → `search_claims` · `unsupported_media_type` → `upload_evidence` · `evidence_limit_exceeded` → `upload_evidence` · `invalid_state`(종결된 클레임) → `get_claim`
- 멱등성: `Idempotency-Key = hash(claim_id + media_digest)` — 같은 파일 중복 첨부 없음.

---

## inspection

### `assess_return_condition` — Kind: W
- 논리주소/REST: `fulfillment.inspection.assess` · `POST /v1/fulfillment/claims/{claim_id}/inspection`
- 요약: 회수품 검수 결과 기록. 결과가 클레임 상태·비용·재고 복원을 정합니다. `normal`일 때만 재고를 복원합니다.
- consumes → produces: `claim_id` → `inspection_id`
- 입력: `claim_id`(✓), `condition`(✓, `normal` \| `damaged` \| `missing_parts`), `acom:liability`(`store_fault` \| `customer_fault`), `next`(`approve` \| `hold` \| `decline`), `note`, `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_i20a", "changes": [{"op": "update", "entity": "claim", "id": "ret_20402", "from": "picked_up", "to": "inspected"}, {"op": "update", "entity": "inventory", "id": "prd_chelsea_boots_9_tan", "from": 6, "to": 7}], "checks": [{"rule": "inspection_before_restock", "passed": true, "detail": "Condition normal: 1 unit back to stock", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "ins_4402", "undo": {"tool": "undo_change_set", "until": "2026-11-10T23:59:00"}}`
- 정책: `return.inventory_restore_when`, `liability`
- 에러: `not_picked_up` → `get_claim` · `invalid_state` → `get_claim`
- 멱등성: `Idempotency-Key = hash(claim_id + condition)`

---

## refund

### `refund_return` — Kind: W$
- 논리주소/REST: `fulfillment.refund.create` · `POST /v1/fulfillment/refunds`
- 요약: 반품 요청 한 건을 환불. 돈이 움직이므로 **항상** 프리뷰 → 변경셋 → 사람 승인 → 실행기가 `preview_id`로 실행합니다. 정책 `refund_without_person_max`($200.00)는 사람이 보지 않고 환불할 수 있는 상한이지만, [approval.yml](policies/approval.yml)에 따라 `refund_return`은 그 이하 금액도 변경셋을 거칩니다. 상한을 넘으면 `approval_reason`에 그 사실을 적습니다. 환불은 원결제수단으로만 갑니다. schema.org `acom:RefundAction`(`PayAction`의 확장), 결과는 `acom:Refund`. MVP 툴([plan.md §3.3](../plan.md)).
- consumes → produces: `return_request_id` → `refund_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | return_request_id | Text | ✓ | `"ret_20402"` |
  | amount | Number | ✓ | `312.00`(USD, 소수 둘째 자리). 보통 `evaluate_return_request`의 `refund_amount` |
  | preview | Boolean | ✓ | `true` 먼저. 실행은 실행기만 `preview_id`로 |
- 출력(프리뷰):
  ```jsonc
  {"preview_id": "pv_r312",
   "changes": [{"op": "create", "entity": "refund", "id": "ret_20402", "from": null, "to": 312.00},
               {"op": "update", "entity": "return_request", "id": "ret_20402", "from": "waiting", "to": "approved"}],
   "checks": [{"rule": "refund_to_original_payment", "passed": true, "detail": "$312.00 back to card ending 0937", "fix_tool": null},
              {"rule": "coupon_deducted", "passed": true, "detail": "No coupon on order #20402", "fix_tool": null}],
   "needs_approval": true,
   "approval_reason": "Refund $312.00 is over the $200.00 limit for refunds without a person"}
  ```
  - `coupon_deducted`: 쿠폰을 쓴 주문은 쿠폰 할인분을 뺀 금액만 환불되는지 확인(12건 중 3건). 금액이 맞지 않으면 `passed: false`, `fix_tool: "refund_return"`.
- 출력(실행): `{"ok": true, "id": "rfd_20402", "undo": null}` — 환불은 되돌릴 수 없으므로 `undo`는 `null`. 결제 이벤트로 확정되면 `get_refund`의 상태가 `completed`가 됩니다.
- 프리뷰: `checks` `refund_to_original_payment`, `coupon_deducted`. `needs_approval`은 항상 `true`. 여러 건을 한 요청으로 처리하면(예: 9건 $2,463.00) 하네스가 같은 요청의 `send_customer_message` 프리뷰와 함께 한 변경셋으로 묶습니다.
- 정책: `refund_without_person_max`, `defect_refund`, `changed_mind_return_shipping`, `refund`(원결제수단·결제 이벤트 확정·실패 시 수동 큐) ([fulfillment.yml](policies/fulfillment.yml)), [approval.yml](policies/approval.yml)
- 에러: `return_request_not_found` → `search_return_requests` · `amount_over_payment`(결제액 초과) → `evaluate_return_request` · `coupon_not_deducted` → `refund_return` · `already_refunded` → `get_refund` · `invalid_state` → `get_claim` · `payment_gateway_error` → `retry_refund`
- 멱등성: `Idempotency-Key = hash(return_request_id + amount)` — 재시도해도 중복 환불 없음, 첫 결과 반환.

### `get_refund` — Kind: R
- 논리주소/REST: `fulfillment.refund.get` · `GET /v1/fulfillment/refunds/{refund_id}`
- 요약: 환불 한 건(상태, 결제 이벤트 확정 여부, 결제수단별 금액).
- consumes → produces: `refund_id` → refund
- 입력: `refund_id`(✓), `fields?[]`
- 출력: `{"items": [{"refund_id": "rfd_20402", "return_request_id": "ret_20402", "amount": 312.00, "price_currency": "USD", "to": "card ending 0937", "status": "requested", "events": [{"status": "requested", "at": "2026-11-06T11:20:00"}]}], "total": 1}` — `status`: `requested` \| `completed` \| `failed`
- 정책: 없음(읽기).
- 에러: `refund_not_found` → `search_return_requests`
- 멱등성: 읽기.

### `retry_refund` — Kind: W$
- 논리주소/REST: `fulfillment.refund.retry` · `POST /v1/fulfillment/refunds/{refund_id}/retry`
- 요약: 실패한 환불 재시도(`failed` → `requested`). 결제 이벤트가 오지 않거나 게이트웨이 오류일 때. 실패는 수동 큐에 남고 조용히 사라지지 않습니다. 돈이 움직이므로 승인 필요.
- consumes → produces: `refund_id` → `refund_id`
- 입력: `refund_id`(✓), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_r9a1", "changes": [{"op": "update", "entity": "refund", "id": "rfd_20377", "from": "failed", "to": "requested"}], "checks": [{"rule": "refund_to_original_payment", "passed": true, "detail": "$217.00 back to card ending 5512", "fix_tool": null}], "needs_approval": true, "approval_reason": "Retries a $217.00 refund"}`
- 출력(실행): `{"ok": true, "id": "rfd_20377", "undo": null}`
- 정책: `refund.on_failure`, `refund.manual_queue_sla_hours`
- 에러: `not_failed` → `get_refund` · `max_retries_reached` → `get_refund`(수동 처리)
- 멱등성: `Idempotency-Key = hash(refund_id + attempt)`

---

## parcel_delivery

### `get_delivery_dates` — Kind: R
- 논리주소/REST: `fulfillment.parcel_delivery.get_delivery_dates` · `GET /v1/fulfillment/delivery-dates?product_id=prd_windbreaker&delivery_address=home`
- 요약: 상품을 이 주소로 받을 수 있는 가장 이른 날짜. `by`를 주면 그날까지 도착 가능한지도 답합니다. 쇼퍼 툴, MVP(P2). schema.org `ParcelDelivery.expected_arrival_until`.
- consumes → produces: `product_id` → delivery estimate
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | product_id | Text | ✓ | `"prd_windbreaker"` |
  | delivery_address | Text \| PostalAddress | ✓ | `"home"`(저장된 주소) 또는 한 번 쓰는 주소 |
  | by | Date | | `"2026-11-11"` — 이 날짜까지 필요 |
- 출력: `{"items": [{"product_id": "prd_windbreaker", "earliest_arrival": "2026-11-10", "method": "standard", "ships": "2026-11-07", "shipping": 0.00, "price_currency": "USD", "arrives_by_requested_date": true}], "total": 1}`
- 정책: `free_shipping_min_subtotal`($80.00), `delivery.regional_surcharge`
- 에러: `product_not_found` → `search_products` · `address_not_deliverable` → `get_my_profile` · `out_of_stock` → `get_product`
- 멱등성: 읽기.

### `get_delivery` — Kind: R
- 논리주소/REST: `fulfillment.parcel_delivery.get` · `GET /v1/fulfillment/deliveries/{delivery_id}`
- 요약: 배송 한 건(상태 이력, 송장, 지역 추가 배송비).
- consumes → produces: `delivery_id` → delivery
- 입력: `delivery_id`(✓), `fields?[]`
- 출력: `{"items": [{"delivery_id": "dlv_20402", "part_of_order": "ord_20402", "delivery_address": "12 Harbor St, Apt 3", "tracking_number": "1Z999", "carrier": "UPS", "status": "delivered", "expected_arrival_until": "2026-10-31", "events": [...]}], "total": 1}`
- 정책: 없음(읽기).
- 에러: `delivery_not_found` → `get_order`
- 멱등성: 읽기.

### `track_shipment` — Kind: R
- 논리주소/REST: `fulfillment.parcel_delivery.track` · `GET /v1/fulfillment/deliveries/{delivery_id}/tracking`
- 요약: 택배사 추적 이벤트. 이벤트는 `integration`의 `receive_carrier_event`가 받아 쌓은 것입니다.
- consumes → produces: `delivery_id` → tracking events
- 입력: `delivery_id` 또는 `tracking_number`(둘 중 하나 ✓), `limit`, `offset`
- 출력: `{"items": [{"at": "2026-10-31T14:02:00", "status": "delivered", "location": "Portland, ME"}], "total": 5}`
- 정책: 없음(읽기).
- 에러: `tracking_not_found` → `get_delivery`
- 멱등성: 읽기.

### `schedule_shipment` — Kind: W
- 논리주소/REST: `fulfillment.parcel_delivery.schedule` · `POST /v1/fulfillment/deliveries`
- 요약: 출고 지시(배송 그룹 단위, 부분 출고 가능). 재고가 있으면 `ship_instructed` → `ship_confirmed`, 부족하면 `ship_wait`. 송장은 `confirm_shipment`에서 확정.
- consumes → produces: `order_id` → `delivery_id`
- 입력: `order_id`(✓), `line_ids?[]`(부분 출고), `shipping_group`, `delivery_method`, `carrier`, `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_d10c", "changes": [{"op": "create", "entity": "parcel_delivery", "id": "dlv_20455", "from": null, "to": "ship_instructed"}], "checks": [{"rule": "stock_available", "passed": true, "detail": "All 2 lines in stock", "fix_tool": "register_shortage"}, {"rule": "no_ship_item", "passed": true, "detail": "No gift cards in this order", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "dlv_20455", "undo": {"tool": "undo_change_set", "until": "2026-11-06T23:59:00"}}`
- 정책: `delivery.shipping_group_by`, `delivery.regional_surcharge`, `delivery.no_ship_for`, `delivery.shortage_state`
- 에러: `no_ship_item` → `schedule_shipment`(해당 라인 제외) · `out_of_stock` → `register_shortage` · `invalid_state` → `get_delivery`
- 멱등성: `Idempotency-Key = hash(order_id + shipping_group + line_ids)`

### `confirm_shipment` — Kind: W
- 논리주소/REST: `fulfillment.parcel_delivery.confirm` · `POST /v1/fulfillment/deliveries/{delivery_id}/confirm`
- 요약: 출고 완료 확정(송장번호 기재, `ship_confirmed` → `shipped`). 모든 라인이 `shipped`면 주문 상태가 바뀝니다.
- consumes → produces: `delivery_id` → `delivery_id`
- 입력: `delivery_id`(✓), `tracking_number`(✓), `carrier`(✓), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_d11a", "changes": [{"op": "update", "entity": "parcel_delivery", "id": "dlv_20455", "from": "ship_confirmed", "to": "shipped"}], "checks": [{"rule": "tracking_number_format", "passed": true, "detail": "Matches UPS format", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "dlv_20455", "undo": {"tool": "undo_change_set", "until": "2026-11-06T23:59:00"}}`
- 정책: `delivery.tracking`
- 에러: `invalid_state` → `get_delivery` · `tracking_required` → `confirm_shipment`
- 멱등성: `Idempotency-Key = hash(delivery_id + tracking_number)`

### `register_shortage` — Kind: W
- 논리주소/REST: `fulfillment.parcel_delivery.register_shortage` · `POST /v1/fulfillment/deliveries/{delivery_id}/shortage`
- 요약: 재고 부족 등록(`ship_wait`). 입고되면 다시 `ship_instructed`.
- consumes → produces: `delivery_id` → `delivery_id`
- 입력: `delivery_id`(✓), `line_ids[]`(✓), `reason`, `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_d12e", "changes": [{"op": "update", "entity": "parcel_delivery", "id": "dlv_20455", "from": "ship_instructed", "to": "ship_wait"}], "checks": [], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "dlv_20455", "undo": {"tool": "undo_change_set", "until": "2026-11-06T23:59:00"}}`
- 정책: `delivery.shortage_state`
- 에러: `invalid_state` → `get_delivery`
- 멱등성: `Idempotency-Key = hash(delivery_id + line_ids)`

---

## shipping_rate

### `list_shipping_zones` — Kind: R
- 논리주소/REST: `fulfillment.shipping_rate.list_zones` · `GET /v1/fulfillment/shipping-zones`
- 요약: 배송 지역과 요율(지역 추가 배송비, 조건부 무료 기준 포함).
- consumes → produces: → `zone_id[]`
- 입력: `fields?[]`, `limit`(기본 20), `offset`(기본 0)
- 출력: `{"items": [{"zone_id": "zone_us_contiguous", "name": "Contiguous US", "shipping_rate": 6.95, "free_over": 80.00, "surcharge": 0.00, "price_currency": "USD"}], "total": 3}`
- 정책: 없음(읽기).
- 에러: `invalid_filter` → `list_shipping_zones`
- 멱등성: 읽기.

### `manage_shipping_zone` — Kind: W
- 논리주소/REST: `fulfillment.shipping_rate.manage_zone` · `POST /v1/fulfillment/shipping-zones`
- 요약: 배송 지역 정의·수정(원거리 지역 등). schema.org `OfferShippingDetails`, `shipping_destination`(`DefinedRegion[]`).
- consumes → produces: → `zone_id`
- 입력: `zone_id`(수정 시), `name`(✓), `shipping_destination[]`(✓), `surcharge`, `zone_type`(`standard` \| `remote`), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_z01b", "changes": [{"op": "create", "entity": "shipping_zone", "id": "zone_us_remote", "from": null, "to": "Alaska, Hawaii"}], "checks": [{"rule": "zone_overlap", "passed": true, "detail": "No region is in two zones", "fix_tool": "list_shipping_zones"}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "zone_us_remote", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: `delivery.regional_surcharge`
- 에러: `overlapping_zone` → `list_shipping_zones`
- 멱등성: `Idempotency-Key`

### `set_shipping_rate` — Kind: W
- 논리주소/REST: `fulfillment.shipping_rate.set` · `POST /v1/fulfillment/shipping-zones/{zone_id}/rate`
- 요약: 지역별 배송비 설정(유료·무료·조건부 무료 기준). schema.org `ShippingRateSettings`.
- consumes → produces: `zone_id` → `zone_id`
- 입력: `zone_id`(✓), `shipping_rate`(✓, 예 `6.95`), `fee_type`(`paid` \| `free` \| `conditional_free`), `free_over`(조건부 무료일 때, 예 `80.00`), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_z02c", "changes": [{"op": "update", "entity": "shipping_rate", "id": "zone_us_contiguous", "from": 6.95, "to": 7.95}], "checks": [{"rule": "free_shipping_min_subtotal", "passed": true, "detail": "Free over $80.00 kept", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "zone_us_contiguous", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: `delivery.shipping_fee_types`, `free_shipping_min_subtotal`
- 에러: `invalid_threshold` → `set_shipping_rate`
- 멱등성: `Idempotency-Key`

---

## 사람 전용 엔드포인트

쇼퍼 본인이 UI 버튼으로만 부르는 HTTP 엔드포인트입니다. **툴이 아니며 모델은 보지 못하므로**, 어떤 프롬프트로도 에이전트가 반품·취소·교환 요청을 대신 보낼 수 없습니다. 쇼퍼 에이전트는 선택지 툴(`get_cancel_options`, `get_exchange_options`, `get_return_options`)로 준비한 내용을 확인 카드로 보여주고, 쇼퍼가 그 카드의 버튼을 누릅니다([plan.md §3.6](../plan.md), [README.md §2](README.md)).

| 엔드포인트 | 누가 | 준비 툴 | 무엇 |
|---|---|---|---|
| `POST /orders/{order_id}/cancel` | 쇼퍼 | `get_cancel_options` | 출고 전 취소(전체·라인·수량) |
| `POST /exchanges` | 쇼퍼 | `get_exchange_options` | 같은 상품 다른 변형으로 교환(회수 → 재배송) |
| `POST /returns` | 쇼퍼 | `get_return_options` | 반품(환불) 또는 같은 변형 교체 |
| `POST /claims/{claim_id}/pay-charge` | 쇼퍼 | `get_claim` | `charge_due` 클레임의 추가 비용 결제 → 접수 |
| `POST /claims/{claim_id}/withdraw` | 쇼퍼 | `get_claim` | 교환·반품 철회(클레임 단위). 취소는 철회 불가 |

**공통 규칙**
- 쇼퍼 세션의 본인 주문만(`not_your_order`). `Idempotency-Key` 필수: 같은 키 재전송은 첫 결과를 돌려주고, 다른 키라도 같은 라인에 열린 클레임이 있으면 `claim_in_progress`.
- 서버가 보내는 순간 선택지를 **다시 계산**합니다. 쇼퍼가 본 견적(`expected_refund`·`expected_charge`)과 다르면 실행하지 않고 `estimate_changed`(새 견적 포함)로 거부해, 확인하지 않은 금액으로 처리되는 일이 없습니다.
- 응답은 `{ok, id, …}`에 클레임 상태와 견적을 더한 형식, 에러는 공통 형식(`{"error": {code, message, next_tool}}`; `next_tool`은 쇼퍼 에이전트가 다시 부를 선택지 툴).
- 상태 변화는 클레임 이벤트로 append되고, 쇼퍼 알림은 `messaging`이 상태 이벤트를 받아 보냅니다.

### `POST /orders/{order_id}/cancel`
- 요약: 출고 전 취소. 경로는 라인 상태가 정합니다(위 [상태 요약](#상태-요약)): 결제 전은 즉시 취소, 직접 취소 상태는 자동 승인, 피킹·포장 중은 운영 큐로 가는 요청. 출고 후는 거부하고 반품을 안내합니다. schema.org `CancelAction`.
- 본문:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | lines | {line_id, quantity}[] | | 생략하면 전체 취소. 예 `[{"line_id": "line_1", "quantity": 1}]` |
  | reason | Text(enum) | ✓ | `get_cancel_options.reasons`의 값. 예 `"changed_mind"` |
  | reason_text | Text | | 쇼퍼가 쓴 말(500자) |
  | expected_refund | Number | ✓ | 쇼퍼가 확인한 `estimate.refund_total`. 예 `64.25` |
  | expected_charge | Number | | 확인한 `estimate.charge_due`(기본 `0.00`) |
  | refund_to | {type, ref} | 조건부 | `needs_refund_destination: true`일 때만. `type` `bank_account` \| `store_credit`, `ref`는 저장된 계좌 참조(계좌번호 원문은 받지 않음) |
- 응답:
  ```jsonc
  {"ok": true, "id": "clm_4410", "type": "cancel", "status": "approved",   // "completed" | "approved" | "charge_due" | "requested"
   "order_id": "ord_20455", "lines": [{"line_id": "line_1", "quantity": 1}],
   "acom:liability": "customer_fault",
   "refund": {"total": 64.25, "price_currency": "USD",
              "breakdown": [{"to": "card ending 4821", "amount": 64.25}],
              "money_back": "3–5 business days after it is confirmed"},
   "extra_charge": [{"kind": "lost_free_shipping", "amount": 6.95, "collected": "deducted_from_refund"}],
   "coupons_restored": [], "points_pending_cancelled": 0.71,
   "confirmed_by": "store, automatic", "undo": null}
  ```
  - `status: "requested"`면 `confirmed_by: "a person at the store"`이고, 승인 전까지 `refund`는 견적입니다. `status: "charge_due"`면 `pay_charge: {"amount": …, "endpoint": "POST /claims/clm_…/pay-charge"}`가 붙습니다.
- 에러: `already_shipped` → `get_return_options` · `not_cancellable` → `get_cancel_options` · `quantity_over_cancellable` → `get_cancel_options` · `whole_quantity_only` → `get_cancel_options` · `reason_not_selectable` → `get_cancel_options` · `estimate_changed` → `get_cancel_options` · `refund_destination_required` → `get_cancel_options` · `already_cancelled` → `get_claim` · `claim_in_progress` → `get_claim` · `not_your_order` → `get_my_orders`
- 멱등성: `Idempotency-Key` 필수. 서버는 `hash(order_id + sorted(line_id:quantity))`로도 중복을 막아, 같은 라인·수량을 두 번 취소하지 않습니다.

### `POST /exchanges`
- 요약: 같은 상품의 다른 변형(판매자 귀책이면 같은 변형도)으로 교환. 요청 시 대체 재고를 예약하고, 쇼퍼 부담 비용이 있으면 `charge_due`로 결제를 먼저 받은 뒤 `requested`가 됩니다. 이후 회수 지시는 자동 또는 `approve_claim`, 재배송은 검수 후. schema.org `ReturnAction` + 재배송 `ParcelDelivery`.
- 본문:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | order_id | Text | ✓ | `"ord_20388"` |
  | lines | {line_id, quantity, replacement_variant_id}[] | ✓ | 라인별 교환. 수량은 바꿀 수 없고, 대체 변형은 `get_exchange_options`에서 `selectable: true`인 것. 예 `[{"line_id": "line_2", "quantity": 1, "replacement_variant_id": "prd_trail_runner_10_slate"}]` |
  | reason | Text(enum) | ✓ | `get_exchange_options`의 사유. 예 `"wrong_size"` |
  | reason_text | Text | | 쇼퍼가 쓴 말 |
  | evidence_ids | Text[] | 조건부 | `damaged`·`defect`는 `damage_claim_needs_photo`에 따라 사진(`upload_evidence`) |
  | pickup_address | Text \| PostalAddress | | 생략 시 배송지 |
  | delivery_address | Text \| PostalAddress | | 재배송지. 생략 시 배송지 |
  | pickup_slot | Text | | `get_exchange_options.pickup.earliest` 등. 예 `"2026-11-09T09:00/12:00"` |
  | expected_charge | Number | ✓ | 쇼퍼가 확인한 `you_pay`. 예 `0.00` |
- 응답:
  ```jsonc
  {"ok": true, "id": "clm_3301", "type": "exchange", "status": "requested",   // "charge_due" | "requested" | "pickup_instructed"
   "acom:liability": "customer_fault",
   "replacement": [{"line_id": "line_2", "variant_id": "prd_trail_runner_10_slate", "reserved": 1}],
   "pickup": {"slot": "2026-11-09T09:00/12:00", "cost": 0.00},
   "reship": {"ships_after": "inspected", "arrives": "2026-11-14", "cost": 0.00},
   "you_pay": 0.00, "price_currency": "USD",
   "withdraw_until": "pickup_instructed",
   "confirmed_by": "store, automatic"}
  ```
- 에러: `not_shipped_yet` → `get_cancel_options` · `outside_exchange_window` → `get_return_options` · `not_exchangeable` → `get_return_options` · `replacement_unavailable`(방금 품절) → `get_exchange_options` · `price_differs` → `get_exchange_options` · `quantity_mismatch` → `get_exchange_options` · `whole_quantity_only` → `get_exchange_options` · `evidence_required` → `upload_evidence` · `estimate_changed` → `get_exchange_options` · `claim_in_progress` → `get_claim` · `not_your_order` → `get_my_orders`
- 멱등성: `Idempotency-Key` 필수. 같은 라인에 열린 교환·반품이 있으면 새로 만들지 않습니다.

### `POST /returns`
- 요약: 반품 요청 보내기. `option` `refund`는 반품 요청(`ret_…`, `waiting`)이 되고, `replacement`(같은 변형 새 상품)는 교환 클레임으로 만들어집니다. 변심 반품의 반품 배송비·무료 배송 회수분은 환불에서 차감하며, 환불액보다 크면 `charge_due`. MVP 형식 `{order_id, line_id, option}`(라인 전체 수량)도 그대로 받습니다.
- 본문: `order_id`(✓), `lines[]`(✓, `{line_id, quantity}`; MVP는 `line_id`), `reason`(✓, `get_return_options`의 사유), `option`(✓, `refund` \| `replacement`), `reason_text`, `evidence_ids[]`(조건부, 파손 사진), `pickup_address`, `pickup_slot`, `expected_refund`(✓, MVP 형식 제외), `refund_to`(조건부, 원결제 되돌리기 불가 시)
- 응답: `{"ok": true, "id": "ret_20402", "type": "return", "status": "waiting", "refund_estimate": {"total": 312.00, "breakdown": [{"to": "card ending 0937", "amount": 312.00}]}, "send_back": {"cost": 0.00, "pickup": "2026-11-09T09:00/12:00"}, "confirmed_by": "a person at the store, usually within 1 business day"}`
- 에러: `outside_return_window` → (선택지 없음, 사유 안내) · `not_returnable` → `get_return_options` · `not_shipped_yet` → `get_cancel_options` · `evidence_required` → `upload_evidence` · `estimate_changed` → `get_return_options` · `claim_in_progress` → `get_claim` · `not_your_order` → `get_my_orders`
- 멱등성: `Idempotency-Key` 필수.

### `POST /claims/{claim_id}/pay-charge` · `POST /claims/{claim_id}/withdraw`
- `pay-charge` 본문 `{amount, payment_method_ref}` — `charge_due` 클레임의 추가 비용(취소의 환불 초과분, 교환 왕복 배송비, 반품 배송비 초과분) 결제. 결제 이벤트로 확정되면 클레임이 다음 상태(`approved` \| `requested` \| `waiting`)로. 에러 `amount_mismatch` → `get_claim` · `invalid_state` → `get_claim`.
- `withdraw` 본문 `{reason_text?}` — 교환·반품을 클레임 단위로 철회(일부 라인만 철회 불가). 회수 송장이 잡히기 전까지(`exchange.withdraw.allowed_states`, `return.withdraw.allowed_states`). 예약 재고 해제, 낸 추가 비용 자동 환불, 라인은 `delivered`로. 취소는 `not_revocable`로 거부.
- 둘 다 `Idempotency-Key` 필수.
