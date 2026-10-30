# API 명세 — `fulfillment` (클레임[반품·취소·교환]·검수·환불·배송·배송비)

> 계층: `fulfillment.{resource}.{operation}`. 공통 규약(응답 형식·프리뷰/실행·멱등·헤더)은 [conventions.md](conventions.md)와 [README.md](README.md) §3, 상태·사유 값은 [enums.md](enums.md), 에러 분류는 [errors.md](errors.md), 데이터 타입은 [data-model.md](data-model.md), 정책은 [fulfillment.yml](../../policies/fulfillment.yml) · [approval.yml](../../policies/approval.yml). 예시 값은 데모 시나리오([../scenarios.md](../scenarios.md) B2·F5)와 같습니다.
> **설계 축**: ① 반품 요청(`claim_type: return`인 클레임, schema.org `ReturnAction`)은 운영 에이전트가 **정책 근거와 함께 묶어 검토**하고, 환불·고객 메시지는 한 변경셋으로 사람이 승인, ② 반품·취소·교환 요청을 **보내는 것은 쇼퍼 본인**(사람 전용 엔드포인트, 툴 아님), ③ 환불은 원결제수단으로, 상태는 결제 이벤트로 확정(조용한 실패 없음), ④ 검수 결과가 클레임 상태·비용·재고 복원을 구동, ⑤ 귀책 → 비용 규칙은 정책 데이터, ⑥ 상태 변화는 이벤트 테이블에 append.
>
> ⑦ 취소·교환·반품은 같은 모양: 쇼퍼 에이전트는 **선택지 툴(R)** 로 대상·사유·비용·환불 예상을 준비하고, 보내기는 쇼퍼 버튼(사람 전용), 승인이 필요한 건은 운영 큐의 클레임 → `approve_claim`(변경셋).

리소스: [claim](#claim) · [inspection](#inspection) · [refund](#refund) · [parcel_delivery](#parcel_delivery) · [shipping_rate](#shipping_rate) · [사람 전용 엔드포인트](#사람-전용-엔드포인트)

| resource | operations |
|---|---|
| claim | **`search_return_requests`** R · **`evaluate_return_request`** R · **`get_return_options`** R · `get_cancel_options` R · `get_exchange_options` R · `search_claims` R · `get_claim` R · `approve_claim` W$ · `reject_claim` W · `cancel_order_lines` W$ · `convert_exchange_to_return` W · `upload_evidence` W · 사람 전용 `POST /v1/fulfillment/returns`, `POST /v1/fulfillment/orders/{order_id}/cancel`, `POST /v1/fulfillment/exchanges`, `POST /v1/fulfillment/claims/{claim_id}/pay-charge`, `POST /v1/fulfillment/claims/{claim_id}/withdraw` |
| inspection | `assess_return_condition` W |
| refund | **`refund_return`** W · `get_refund` R · `retry_refund` W$ |
| parcel_delivery | **`get_delivery_dates`** R · `get_delivery` R · `track_shipment` R · `schedule_shipment` W · `confirm_shipment` W · `register_shortage` W |
| shipping_rate | `list_shipping_zones` R · `manage_shipping_zone` W · `set_shipping_rate` W |

- id 예: 클레임(취소·반품·교환 공통) `clm_20402`, 주문 품목 `oln_1`, 변형 `var_chelsea_boots_9_tan`, 환불 `rfd_20402`, 배송 `dlv_20402`, 증빙 `evd_901`, 검수 `ins_4402`, 배송 지역 `zone_us_contiguous`. 반품 요청은 따로 접두를 두지 않고 `claim_type: return`인 클레임입니다.

---

## 상태 요약

클레임 상태는 세 종류 모두 `acom:claim_status`([enums.md](enums.md) §1.5) 하나의 목록을 씁니다. 품목의 출고 단계는 `acom:fulfillment_stage`(§1.2), 배송은 `acom:delivery_state`(§1.3)입니다.

**claim — 반품** (`claim_type: return`)

```
(쇼퍼) POST /v1/fulfillment/returns ──▶ requested ──refund_return 실행──▶ approved ──회수 지시──▶ pickup_instructed
          ──택배 이벤트──▶ picked_up ──assess_return_condition──▶ inspected ──환불 completed──▶ completed
requested ─send_customer_message(kind=ask_for_photos)─▶ photos_requested ─upload_evidence─▶ requested
requested ─reject_claim (+ send_customer_message(kind=decline))─▶ declined
```
- 차감할 환불액보다 쇼퍼가 낼 비용(반품 배송비·무료 배송 회수분)이 크면 요청은 `charge_due`로 시작하고, `POST /v1/fulfillment/claims/{claim_id}/pay-charge` 뒤 `requested`가 됩니다.
- 환불이 시작되는 시점은 [fulfillment.yml](../../policies/fulfillment.yml) `refund.timing.return`(기본 `on_approval`: `refund_return` 실행 때)입니다.

**claim — 취소** (`claim_type: cancel`, 출고 전만. 품목 상태가 경로를 정함, `cancel`)

| 품목 상태 | 쇼퍼 | 결과 |
|---|---|---|
| `payment_due`(결제 전) | 직접 | 즉시 `completed`. 환불 없음, 재고 복원 |
| `processing`, `awaiting_stock`(창고 인계 전·재고 대기) | 직접 | 추가 비용이 없으면 자동 `approved` → 환불 확정 시 `completed`. 추가 비용이 환불액보다 크면 `charge_due` |
| `ship_instructed`, `packed`(피킹·포장 중) | 요청만 | `requested` 클레임 → 운영 큐 → `approve_claim`(출고를 멈출 수 있을 때) 또는 `reject_claim(already_handed_over)` |
| `shipped`, `delivered` | 불가 | `already_shipped` 에러 → `get_return_options`(반품 경로) |

```
(쇼퍼) POST /v1/fulfillment/orders/{order_id}/cancel
   ├─ 결제 전 ───────────────────────────────────────────────▶ completed
   ├─ 직접 취소 상태 · 추가 비용 0 ──(자동 승인)──▶ approved ──환불 확정──▶ completed
   ├─ 직접 취소 상태 · 추가 비용 > 환불액 ──▶ charge_due ──pay-charge──▶ approved ─▶ …
   └─ 운영자 전용 상태 ──▶ requested ──approve_claim(W$)──▶ approved ──환불 확정──▶ completed
                              └─reject_claim──▶ declined (출고됨 → 반품 안내)
(운영자) cancel_order_lines(W$, 품절·직권) ──승인──▶ approved ──▶ completed
```
- 취소는 철회할 수 없습니다(`cancel.revocable: false`). 환불 실패는 클레임을 `approved`에 두고 환불을 수동 큐로 보냅니다(`refund.on_failure`).

**claim — 교환** (`claim_type: exchange`, 출고 후. **회수 단계 → 재배송 단계**가 순서대로, 맞교환 없음, `exchange`)

```
(쇼퍼) POST /v1/fulfillment/exchanges
   ├─ 쇼퍼 부담 비용 있음 ──▶ charge_due ──pay-charge──┐
   └─ 비용 없음 ───────────────────────────────────────┴─▶ requested
회수:   requested ──(자동 또는 approve_claim)──▶ pickup_instructed ──택배 이벤트──▶ picked_up
          ──assess_return_condition──▶ inspected
재배송: inspected(as_new) ──▶ reship_requested ──schedule_shipment──▶ (재배송 배송 ship_instructed → packed → label_created → … → delivered) ──▶ completed
분기:   charge_due · requested · pickup_instructed(회수 송장 전) ──POST …/withdraw──▶ withdrawn (품목은 delivered로)
        pickup_instructed(회수 송장 후) ~ reship_requested(재배송 출고 전) ──convert_exchange_to_return──▶ converted + 새 반품 클레임
        inspected(damaged, 쇼퍼 귀책) ──reject_claim(inspection_failed)──▶ declined (반송 협의)
```
- 교환품의 배송 진행은 클레임이 아니라 재배송 `parcel_delivery`의 `delivery_state`에 있습니다.

**parcel_delivery — 배송** (품목별 출고 단계, 송장 여러 개 가능, 모든 품목이 `shipped`면 주문이 `in_transit`)

```
schedule_shipment ──▶ ship_instructed ──(창고 포장)──▶ packed ──confirm_shipment(송장)──▶ label_created
   ──택배 이벤트──▶ picked_up ──▶ in_transit ──▶ delivered | delivery_failed
   └─ 재고 부족 ──register_shortage──▶ awaiting_stock ──(입고)──▶ ship_instructed
```

- 불가능한 전이는 `invalid_state` 에러. 상태 변화는 `claim_event`, `parcel_delivery_event`에 append되고 덮어쓰지 않습니다.
- 택배사 추적 이벤트 수신은 이 도메인의 툴이 아닙니다. `integration`의 `receive_carrier_event`가 받아 배송 상태를 전이합니다.

---

## claim

### `search_return_requests` — Kind: R
- 논리주소/REST: `fulfillment.claim.search_return_requests` · `GET /v1/fulfillment/return-requests` (필터는 쿼리 파라미터, 금액은 `min_amount.value`·`min_amount.currency`)
- 요약: 처리를 기다리는 반품 요청(`claim_type: return`) 목록. 운영 에이전트가 "$200 넘는 반품 12건"처럼 큐를 묶어 검토할 때 첫 호출입니다. MVP 툴.
- consumes → produces: → `claim_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | status | Text[] (enum `acom:claim_status`) | | 이 상태의 반품만. 값: `charge_due`, `requested`, `photos_requested`, `approved`, `pickup_instructed`, `picked_up`, `inspected`, `reship_requested`, `completed`, `declined`, `withdrawn`, `converted`. 기본 `["requested"]` | `["requested"]` |
  | min_amount | MonetaryAmount | | 반품 금액이 이 값 이상인 요청만. 통화는 스토어 통화 | `{"value": 200.00, "currency": "USD"}` |
  | reason | Text[] (enum `acom:claim_reason`) | | 사유로 거르기. 값은 [enums.md](enums.md) §2.2 | `["defect", "damaged_in_transit"]` |
  | sort | Text (enum) | | `requested_at`: 오래 기다린 순(기본); `-amount`: 큰 금액 먼저 | `"requested_at"` |
  | limit | Integer | | 1–100, 기본 20 | `20` |
  | cursor | Text | | 이전 응답의 `next_cursor` | `"c_2"` |
- 출력: 항목은 검토 큐의 한 줄(클레임 + 주문·고객 요약)입니다. `photos`는 붙은 사진 증빙 수, `days_since_delivery`는 `return_window_days`와 비교할 값입니다.
  ```jsonc
  {"items": [
     {"claim_id": "clm_20402", "order_id": "ord_20402", "order_number": "20402", "customer_id": "cus_alex",
      "order_item_id": "oln_1", "item": "Leather chelsea boots", "quantity": 1,
      "amount": {"value": 312.00, "currency": "USD"},
      "reason": "defect", "reason_text": "The sole is coming apart after a week.",
      "photos": 2, "delivered_at": "2026-10-31", "days_since_delivery": 6,
      "requested_at": "2026-11-06T09:52:00-05:00", "status": "requested"},
     {"claim_id": "clm_20366", "order_id": "ord_20366", "order_number": "20366", "customer_id": "cus_riley",
      "order_item_id": "oln_1", "item": "Light rain parka", "quantity": 2,
      "amount": {"value": 236.00, "currency": "USD"},
      "reason": "damaged_in_transit", "reason_text": "Both arrived torn.",
      "photos": 0, "delivered_at": "2026-11-04", "days_since_delivery": 2,
      "requested_at": "2026-11-05T18:10:00-05:00", "status": "requested"}],
   "total": 12,
   "next_cursor": "c_2"}
  ```
- 정책: 없음(읽기). 값의 비교 기준은 `return_window_days`, `damage_claim_needs_photo`.
- 에러: `invalid_status`(허용 값은 `detail.allowed`) → `search_return_requests` · `currency_mismatch`(`min_amount` 통화가 스토어 통화와 다름) → `search_return_requests`
- 멱등성: 읽기, 부작용 없음.

### `evaluate_return_request` — Kind: R
- 논리주소/REST: `fulfillment.claim.evaluate_return_request` · `GET /v1/fulfillment/return-requests/{claim_id}/evaluation`
- 요약: 반품 요청 한 건에 정책을 적용해 처리안을 제안합니다. [fulfillment.yml](../../policies/fulfillment.yml) `return_suggestions`의 순서(`decline` → `ask_for_photos` → `refund`)로 처음 맞는 규칙을 고르고, 근거 정책 키를 `rule`로 돌려줍니다. 제안일 뿐 아무것도 바꾸지 않으며, 실행은 `refund_return` 또는 `send_customer_message`로 합니다. schema.org `AssessAction`. MVP 툴.
- consumes → produces: `claim_id` → suggestion
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | claim_id | Text (`^clm_[a-z0-9_]+$`) | ✓ | 평가할 반품 클레임. `search_return_requests`에서 | `"clm_20402"` |
- 출력:
  ```jsonc
  {"items": [{"claim_id": "clm_20402", "order_number": "20402",
              "suggested": "refund",                    // refund | ask_for_photos | decline
              "rule": "defect_refund",                  // 근거 정책 키
              "rule_text": "Defects are refunded in full, return shipping free",
              "liability": "store_fault",               // acom:liability, liability.claim_reason_default
              "refund_amount": {"value": 312.00, "currency": "USD"},   // decline·ask_for_photos면 null
              "return_fees": "free_return",             // free_return | return_fees_customer_responsibility | return_shipping_fees
              "needs_person": true,                     // 환불액이 approval.yml thresholds.refund($200.00)를 넘음
              "flags": []}],
   "total": 1}
  ```
  | 항목 | 제안 | `rule` | 메모 |
  |---|---|---|---|
  | `clm_20402` 불량, 사진 2, 6일 | `refund` $312.00 | `defect_refund` | `return_fees: free_return` |
  | `clm_20431` 색상 차이(`not_as_described`), 4일 | `refund` $289.00 | `defect_refund` | `flags: [{"rule": "return_suggestions.flag_return_history", "detail": "4 returns in the last 90 days"}]` |
  | `clm_20377` 단순 변심, 9일 | `refund` $217.00 | `changed_mind_return_shipping` | `return_fees: return_fees_customer_responsibility` |
  | `clm_20366` 배송 중 파손, 사진 0 | `ask_for_photos` | `damage_claim_needs_photo` | `refund_amount: null` |
  | `clm_20290` 배송 후 39일 | `decline` | `return_window_days` | 30일 초과. 스토어 크레딧은 정책 밖 |
- 정책: `return_suggestions`, `return_window_days`, `defect_refund`, `changed_mind_return_shipping`, `damage_claim_needs_photo`, `liability.claim_reason_default`, `refund.coupon_deducted`(쿠폰 쓴 주문의 환불액), approval.yml `thresholds.refund`(needs_person 판정)
- 에러: `claim_not_found` → `search_return_requests` · `invalid_state`(이미 처리된 클레임) → `get_claim` · `not_a_return`(반품이 아닌 클레임) → `get_claim`
- 멱등성: 읽기, 부작용 없음.

### `get_return_options` — Kind: R
- 논리주소/REST: `fulfillment.claim.get_return_options` · `GET /v1/fulfillment/orders/{order_id}/items/{order_item_id}/return-options?reason=defect`
- 요약: 쇼퍼 에이전트가 쇼퍼 본인의 주문 품목에 대해 고를 수 있는 선택지를 보여줍니다. 선택지마다 시점, 쇼퍼가 내는 비용, 누가 확정하는지, 보내는 방법을 함께 돌려주고, 웹은 이것을 [chat.md](chat.md) `return_options` 블록으로 그립니다. 요청을 **보내는 것은** 쇼퍼의 버튼(`POST /v1/fulfillment/returns`)뿐입니다. MVP 툴.
- consumes → produces: `order_id`, `order_item_id` → options
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | order_id | Text (`^ord_[a-z0-9_]+$`) | ✓ | 쇼퍼 본인 주문. `get_my_orders`에서 | `"ord_20402"` |
  | order_item_id | Text (`^oln_[a-z0-9_]+$`) | ✓ | 반품할 주문 품목 | `"oln_1"` |
  | reason | Text (enum `acom:claim_reason`) | ✓ | `defect`, `damaged_in_transit`, `not_as_described`, `wrong_item_sent`, `wrong_option_sent`, `missing_parts`: 스토어 귀책; `wrong_size`, `different_option_wanted`, `changed_mind`: 쇼퍼 귀책 | `"defect"` |
  | quantity | Integer | | 반품할 수량(1 이상). 생략하면 품목 전체. 사은품이 딸린 품목은 전체만(`return.free_gift_items`) | `1` |
- 출력:
  ```jsonc
  {"items": [
     {"kind": "replacement", "title": "New pair, same size",
      "variant_id": "var_chelsea_boots_9_tan", "available_quantity": 6,
      "ships_on": "2026-11-06", "arrives_by": "2026-11-10",
      "you_pay": {"value": 0.00, "currency": "USD"},
      "confirmed_by": "store_automatic",                       // store_automatic | store_person
      "send_back": {"methods": ["pickup", "drop_off"], "return_fees": "free_return",
                    "cost": {"value": 0.00, "currency": "USD"},
                    "pickup_window": {"start": "2026-11-09T09:00:00-05:00", "end": "2026-11-09T12:00:00-05:00"}},
      "facts": ["Ships today", "Arrives Tue Nov 10", "No cost to you"]},
     {"kind": "refund", "title": "Full refund",
      "refund_amount": {"value": 312.00, "currency": "USD"}, "refund_to": "card ending 0937",
      "money_back_business_days": {"min": 3, "max": 5},
      "you_pay": {"value": 0.00, "currency": "USD"},
      "confirmed_by": "store_person",                          // $200.00를 넘어 사람이 확인
      "send_back": {"methods": ["pickup", "drop_off"], "return_fees": "free_return",
                    "cost": {"value": 0.00, "currency": "USD"},
                    "pickup_window": {"start": "2026-11-09T09:00:00-05:00", "end": "2026-11-09T12:00:00-05:00"}},
      "facts": ["$312.00 back to card ending 0937", "A person at the store confirms refunds over $200", "Money back 3–5 days after it is confirmed"]}],
   "total": 2}
  ```
  - `replacement`는 같은 변형의 새 상품으로 바꾸는 교환입니다(`return.replacement_option: creates_exchange`). 다른 사이즈·색이면 `get_exchange_options`.
  - 쇼퍼 귀책 반품은 반품 배송비와 무료 배송 회수분(`lost_free_shipping`)을 환불액에서 먼저 뺍니다. 뺄 환불액보다 크면 항목에 `charge_due`(MonetaryAmount)가 붙고, 요청은 결제 후 접수됩니다.
  - `facts`는 블록이 그대로 보여 주는 문장이고, 숫자는 모두 같은 항목의 필드에서 옵니다.
- 정책: `return_window_days`, `return.non_returnable_when`, `defect_refund`, `changed_mind_return_shipping`, `damage_claim_needs_photo`, `return.replacement_option`, `return.send_back_methods`, `return.scope`, `liability.pays`, approval.yml `thresholds.refund`(넘으면 `confirmed_by: store_person`)
- 에러: `order_not_found` → `get_my_orders` · `order_item_not_found` → `get_order` · `not_your_order` → `get_my_orders` · `outside_return_window`(반품 기간이 지남. 선택지 없이 이유만 안내) → null · `not_returnable`(최종 세일·상품권 등) → null · `not_shipped_yet` → `get_cancel_options` · `claim_in_progress` → `get_claim`
- 멱등성: 읽기, 부작용 없음. 견적은 `POST /v1/fulfillment/returns` 때 다시 계산됩니다.

### `get_cancel_options` — Kind: R
- 논리주소/REST: `fulfillment.claim.get_cancel_options` · `GET /v1/fulfillment/orders/{order_id}/cancel-options?items=oln_1:1&items=oln_2:2&reason=changed_mind`
- 요약: 쇼퍼 에이전트가 취소를 **준비**하는 툴. 쇼퍼 본인 주문에서 취소할 수 있는 품목·수량, 고를 수 있는 사유(귀책 포함), 환불 예상과 결제수단별 내역, 복원되는 쿠폰·적립금, 배송비 재계산으로 생기는 추가 비용, 누가 확정하는지를 돌려줍니다. 아무것도 바꾸지 않으며, 취소를 **보내는 것은** 쇼퍼의 버튼(`POST /v1/fulfillment/orders/{order_id}/cancel`)뿐입니다. schema.org `CancelAction`(`potential`).
- consumes → produces: `order_id` → cancel options
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | order_id | Text (`^ord_`) | ✓ | 쇼퍼 본인 주문 | `"ord_20455"` |
  | items | {order_item_id, quantity}[] | | 취소해 볼 품목과 수량. 생략하면 **전체 취소**로 계산. 쿼리에서는 `items=oln_1:1` 반복 | `[{"order_item_id": "oln_1", "quantity": 1}]` |
  | reason | Text (enum `acom:cancel_reason`) | | 사유를 주면 그 귀책으로 배송비·복원을 계산. 생략 시 `changed_mind`(쇼퍼 귀책, 가장 보수적인 견적) | `"changed_mind"` |
- 출력:
  ```jsonc
  {"items": [
     {"order_item_id": "oln_1", "name": "Packable windbreaker, M", "ordered": 1, "cancellable_quantity": 1,
      "order_item_status": "processing", "fulfillment_stage": null,
      "path": "self_service",                        // self_service | store_review | not_allowed
      "unit_paid": {"value": 71.20, "currency": "USD"}, "whole_quantity_only": false},
     {"order_item_id": "oln_2", "name": "Merino socks, 2-pack", "ordered": 2, "cancellable_quantity": 2,
      "order_item_status": "processing", "fulfillment_stage": null, "path": "self_service",
      "unit_paid": {"value": 16.00, "currency": "USD"}, "whole_quantity_only": false}],
   "reasons": [{"reason": "changed_mind", "liability": "customer_fault"},
               {"reason": "wrong_option_ordered", "liability": "customer_fault"},
               {"reason": "found_lower_price", "liability": "customer_fault"},
               {"reason": "duplicate_order", "liability": "customer_fault"},
               {"reason": "product_info_inaccurate", "liability": "store_fault", "needs_store_review": true}],
   "estimate": {                                    // 요청한 items·reason 기준
     "scope": "partial",                            // full | partial
     "cancelled_amount": {"value": 71.20, "currency": "USD"},
     "extra_charge": [{"kind": "lost_free_shipping", "amount": {"value": 6.00, "currency": "USD"},
                       "why": "What stays ($32.00) is under the $80.00 free-shipping minimum"}],
     "discount_recalculated": [],                   // 주문 쿠폰 조건 미달로 회수되는 할인
     "refund_total": {"value": 65.20, "currency": "USD"},
     "refund_breakdown": [{"to": "card ending 4821", "method": "credit_card", "amount": {"value": 65.20, "currency": "USD"}},
                          {"to": "loyalty points", "method": "loyalty_points", "amount": {"value": 0.00, "currency": "USD"}}],
     "money_back_business_days": {"min": 3, "max": 5},
     "coupons_restored": [],
     "points_pending_cancelled": {"value": 0.71, "currency": "USD"},
     "charge_due": {"value": 0.00, "currency": "USD"},    // 환불액으로 못 덮는 추가 비용(먼저 결제)
     "needs_refund_destination": false,             // 원결제 되돌리기 불가 시 true → refund_to 필요
     "confirmed_by": "store_automatic",             // 운영자 전용 상태가 섞이면 store_person
     "revocable": false},
   "not_cancellable": [],                           // 예 [{"order_item_id": "oln_3", "why": "shipped", "next_tool": "get_return_options"}]
   "total": 2}
  ```
  - 같은 주문 전체 취소(`items` 생략): `refund_total` $103.20 = 카드 $93.20 + 적립금 $10.00(현금성 수단 먼저, 적립금은 나머지, `refund.split`), `coupons_restored: [{"coupon_id": "cpn_socks4", "amount": {"value": 4.00, "currency": "USD"}}]`, 배송비 재계산 없음.
  - 사은품이 딸린 품목은 `whole_quantity_only: true`이고 `cancellable_quantity`는 주문 수량 그대로이며, 사은품도 함께 취소됩니다.
- 정책: `cancel.self_service_states`, `cancel.operator_only_states`, `cancel.auto_approve_when`, `cancel.reasons`, `cancel.shopper_selectable_store_reasons`, `cancel.restore`, `cancel.shipping_recalculation`, `cancel.extra_charge`, `cancel.free_gift_items`, `refund.priority`, `refund.split`, `refund.reversal_window_days`, `free_shipping_min_subtotal`([order.yml](../../policies/order.yml))
- 에러: `order_not_found` → `get_my_orders` · `not_your_order` → `get_my_orders` · `order_item_not_found` → `get_order` · `quantity_over_cancellable` → `get_cancel_options` · `whole_quantity_only`(전체 수량으로 다시) → `get_cancel_options` · `already_shipped` → `get_return_options` · `already_cancelled` → `get_claim` · `claim_in_progress` → `get_claim` · `reason_not_selectable`(허용 사유는 `detail.allowed`) → `get_cancel_options`
- 멱등성: 읽기, 부작용 없음. 견적은 `POST /v1/fulfillment/orders/{order_id}/cancel` 때 다시 계산되며, 달라지면 그쪽이 `estimate_changed`로 거부합니다.

### `get_exchange_options` — Kind: R
- 논리주소/REST: `fulfillment.claim.get_exchange_options` · `GET /v1/fulfillment/orders/{order_id}/items/{order_item_id}/exchange-options?reason=wrong_size&quantity=1`
- 요약: 쇼퍼 에이전트가 교환을 **준비**하는 툴. 같은 상품의 바꿀 수 있는 변형(재고·가격 차이), 회수 일정, 재배송 예상일, 쇼퍼가 내는 비용과 결제 시점, 교환이 안 될 때의 대안(반품)을 돌려줍니다. 보내기는 쇼퍼의 버튼(`POST /v1/fulfillment/exchanges`)뿐입니다.
- consumes → produces: `order_id`, `order_item_id` → exchange options
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | order_id | Text (`^ord_`) | ✓ | 쇼퍼 본인 주문 | `"ord_20447"` |
  | order_item_id | Text (`^oln_`) | ✓ | 교환할 품목 | `"oln_2"` |
  | reason | Text (enum `acom:claim_reason`) | ✓ | 쇼퍼 귀책 `wrong_size`, `different_option_wanted`; 스토어 귀책 `defect`, `damaged_in_transit`, `missing_parts`, `wrong_item_sent`, `wrong_option_sent` | `"wrong_size"` |
  | quantity | Integer | | 교환할 수량. 생략 시 품목 전체 | `1` |
  | pickup_address_id | Text (`^adr_`) | | 회수지. 생략 시 배송지. 지역 추가비가 다시 계산됨 | `"adr_home"` |
  | delivery_address_id | Text (`^adr_`) | | 재배송지. 생략 시 배송지 | `"adr_home"` |
- 출력:
  ```jsonc
  {"items": [
     {"variant_id": "var_trail_runner_10_slate", "size": "10", "color": "Slate", "availability": "in_stock",
      "available_quantity": 7, "price": 128.00, "price_currency": "USD",
      "price_difference": {"value": 0.00, "currency": "USD"}, "selectable": true},
     {"variant_id": "var_trail_runner_10_5_slate", "size": "10.5", "color": "Slate", "availability": "out_of_stock",
      "available_quantity": 0, "price": 128.00, "price_currency": "USD",
      "price_difference": {"value": 0.00, "currency": "USD"}, "selectable": false, "why": "out_of_stock",
      "next_tool": "subscribe_restock"},
     {"variant_id": "var_trail_runner_10_ember", "size": "10", "color": "Ember", "availability": "in_stock",
      "available_quantity": 3, "price": 138.00, "price_currency": "USD",
      "price_difference": {"value": 10.00, "currency": "USD"}, "selectable": false, "why": "price_differs"}],
   "order_item": {"order_item_id": "oln_2", "name": "Trail runner, 9, Slate", "exchangeable_quantity": 1,
                  "days_since_delivery": 5, "window_days": 30, "path": "self_service"},   // 기간 후면 store_review
   "liability": "customer_fault",
   "pickup": {"window": {"start": "2026-11-09T09:00:00-05:00", "end": "2026-11-09T12:00:00-05:00"},
              "address": "12 Harbor St, Apt 3", "cost": {"value": 0.00, "currency": "USD"}},
   "reship": {"ships_after": "inspected", "ships_on": "2026-11-12", "arrives_by": "2026-11-14",
              "cost": {"value": 0.00, "currency": "USD"}},
   "you_pay": {"value": 0.00, "currency": "USD"},   // wrong_size → size_exchange_fee(0.00). different_option_wanted면 왕복 $12.00
   "pay_when": null,                                 // 비용이 있으면 before_accept
   "confirmed_by": "store_automatic",
   "if_not_possible": {"kind": "return", "next_tool": "get_return_options",
                       "why": "Used if the new size runs out before it ships, or you decline it"},
   "total": 3}
  ```
  - 스토어 귀책(`defect` 등)이면 같은 변형(새 상품)도 `selectable: true`이고 회수·재배송비는 스토어가 냅니다(`liability.pays`).
  - 바꿀 변형이 하나도 없으면 `items`의 `selectable`이 모두 `false`이고, 에이전트는 `if_not_possible`의 반품 경로를 보여줍니다.
- 정책: `exchange.eligible_states`, `exchange.window`, `exchange.not_exchangeable_when`, `exchange.replacement`, `exchange.shipping_cost`, `exchange.pickup`, `exchange.reship_after`, `size_exchange_fee`, `delivery.remote_area_surcharge`
- 에러: `order_not_found` → `get_my_orders` · `not_your_order` → `get_my_orders` · `order_item_not_found` → `get_order` · `not_shipped_yet`(출고 전이면 취소 후 다시 주문) → `get_cancel_options` · `outside_exchange_window` → `get_return_options` · `not_exchangeable` → `get_return_options` · `replacement_unavailable`(바꿀 변형 없음) → `get_return_options` · `claim_in_progress` → `get_claim`
- 멱등성: 읽기, 부작용 없음. 재고는 `POST /v1/fulfillment/exchanges` 때 예약됩니다(`exchange.replacement.reserve_stock_on`).

### `search_claims` — Kind: R
- 논리주소/REST: `fulfillment.claim.search` · `GET /v1/fulfillment/claims`
- 요약: 반품·취소·교환 클레임 전체 검색. 반품 검토 큐만 볼 때는 `search_return_requests`를 씁니다.
- consumes → produces: → `claim_id[]`
- 입력: `claim_type`(`return` \| `cancel` \| `exchange`), `status[]`(`acom:claim_status`, [enums.md](enums.md) §1.5), `liability`(`acom:liability`), `order_id`, `customer_id`, `created_from`, `created_through`, `sort`(`-created_at`(기본) \| `-amount`), `fields?[]`, `limit`(기본 20), `cursor`
- 출력: `{"items": [{"id": "clm_3301", "claim_type": "exchange", "status": "requested", "order_id": "ord_20447", "amount": {"value": 128.00, "currency": "USD"}, "created_at": "2026-11-05T14:12:00-05:00"}], "total": 7}`
- 정책: 없음(읽기).
- 에러: `invalid_filter` → `search_claims`
- 멱등성: 읽기.

### `get_claim` — Kind: R
- 논리주소/REST: `fulfillment.claim.get` · `GET /v1/fulfillment/claims/{claim_id}`
- 요약: 클레임 한 건(상태 이력·증빙·검수·비용·연결된 환불). 반품·취소·교환 모두 같은 `clm_` id입니다.
- consumes → produces: `claim_id` → claim
- 입력: `claim_id`(✓), `fields?[]`
- 출력: `{"id": "clm_20402", "claim_type": "return", "status": "requested", "order_id": "ord_20402", "items": [{"order_item_id": "oln_1", "quantity": 1}], "reason": "defect", "liability": "store_fault", "evidence": [{"id": "evd_877", "evidence_type": "product_defect"}, {"id": "evd_878", "evidence_type": "product_defect"}], "inspection": null, "refund_id": null, "status_history": [{"status": "requested", "at": "2026-11-06T09:52:00-05:00"}]}`
- 정책: 없음(읽기).
- 에러: `claim_not_found` → `search_claims`
- 멱등성: 읽기.

### `approve_claim` — Kind: W$
- 논리주소/REST: `fulfillment.claim.approve` · `POST /v1/fulfillment/claims/{claim_id}/approve`
- 요약: 운영 큐에서 `requested` 상태인 취소·교환 클레임(및 환불 없이 진행할 반품의 회수 지시)을 승인. 자동 승인되지 않은 건만 여기에 옵니다: 피킹·포장 중인 품목의 취소 요청, 스토어 귀책 사유를 쇼퍼가 고른 취소, 기간 후·사진 필요·가격 차이 교환. 승인되면 취소는 출고 중단·재고 복원·쿠폰/적립금 복원·환불 개시, 교환은 회수 지시가 이어집니다. 반품 환불은 `refund_return`으로 합니다.
- consumes → produces: `claim_id[]` → `claim_id[]`
- 입력: `claim_ids[]`(✓, 같은 요청에서 함께 승인할 클레임), `liability`(`acom:liability`, 미지정 시 접수값·검수값. 바꾸면 배송비·복원을 다시 계산), `cost_override`(`{amount: MonetaryAmount, note}`, 배송비 조정, `note` 필수)
- 출력(프리뷰, 포장 중 취소 1건):
  ```jsonc
  {"preview_id": "pv_c39e", "expires_at": "2026-11-06T10:32:00-05:00", "change_set_id": "cs_1047",
   "changes": [{"op": "update", "entity": "claim", "id": "clm_4410", "from": "requested", "to": "approved"},
               {"op": "update", "entity": "parcel_delivery", "id": "dlv_20455", "from": "packed", "to": "cancelled"},
               {"op": "update", "entity": "inventory_level", "id": "var_windbreaker_m_navy", "from": 11, "to": 12},
               {"op": "create", "entity": "refund", "id": null, "from": null,
                "to": {"claim_id": "clm_4410", "amount": {"value": 65.20, "currency": "USD"}, "to": "card ending 4821"}}],
   "checks": [{"rule": "cancel.operator_only_states", "passed": true, "detail": "Item is being packed and not yet with the carrier", "fix_tool": "reject_claim"},
              {"rule": "cancel.shipping_recalculation", "passed": true, "detail": "Customer fault: $6.00 shipping taken from the refund (what stays is under $80.00)", "fix_tool": "approve_claim"},
              {"rule": "cancel.restore", "passed": true, "detail": "No coupon on this item; $0.71 pending points cancelled", "fix_tool": null},
              {"rule": "refund.to", "passed": true, "detail": "$65.20 back to card ending 4821", "fix_tool": null},
              {"rule": "change_set.irreversible", "passed": true, "detail": "The refund can't be undone once it is sent", "fix_tool": null}],
   "needs_approval": true, "approval_reason": "Cancels 1 item and refunds $65.20"}
  ```
- 출력(프리뷰, 교환): `changes` `{"op": "update", "entity": "claim", "id": "clm_3301", "from": "requested", "to": "pickup_instructed"}`, 대체 변형 `inventory_level` 예약 1. `checks` `liability.pays`("Store fault: store pays both shipping legs"), `exchange.replacement`("Size 10 in stock, same price"), `return.inventory_restore_when`.
- 출력(실행): 환불이 따르는 취소는 `{"ok": true, "id": "clm_4410", "undo": null}`(되돌릴 수 없음). 교환 승인은 `{"ok": true, "id": "clm_3301", "undo": {"by": "person", "target": "cs_1047", "until": "2026-11-07T23:59:00-05:00"}}`(회수 송장이 생기기 전까지만 실제로 되돌려짐).
- 프리뷰: `checks` `liability.pays`(귀책 ↔ 비용 일관), `cancel.operator_only_states`(아직 택배사에 넘기지 않았는지), `cancel.shipping_recalculation`, `cancel.restore`, `refund.to`, `exchange.replacement`(재고·가격), `return.inventory_restore_when`, 환불이 있으면 `change_set.irreversible`. 항상 변경셋을 거쳐 사람이 승인.
- 정책: `liability`, `exchange`, `cancel`, `refund` ([fulfillment.yml](../../policies/fulfillment.yml)), approval.yml `thresholds.claim_approve`
- 에러: `liability_cost_conflict`(귀책과 비용이 맞지 않음, 귀책·비용 다시 지정) → `approve_claim` · `already_handed_over`(이미 택배사 인계) → `reject_claim` · `replacement_unavailable` → `convert_exchange_to_return` · `inspection_pending` → `assess_return_condition` · `charge_unpaid` → `get_claim` · `invalid_state` → `get_claim`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 서버는 `(claim_id, decision)`으로도 중복 승인을 막습니다.

### `reject_claim` — Kind: W
- 논리주소/REST: `fulfillment.claim.reject` · `POST /v1/fulfillment/claims/{claim_id}/reject`
- 요약: 클레임 반려(사유 필수). `requested`·`photos_requested`·`inspected` → `declined`. 쇼퍼에게 알리는 문구는 `send_customer_message(kind="decline")`로 따로 보내며 그쪽이 승인을 받습니다.
- consumes → produces: `claim_id` → `claim_id`
- 입력: `claim_id`(✓), `reason`(✓, 클레임 거절 사유 [enums.md](enums.md) §2.4: `outside_return_window` \| `not_returnable` \| `inspection_failed` \| `already_handed_over`(취소: 이미 택배사 인계 → 쇼퍼에게 반품 안내) \| `outside_exchange_window`)
- 출력(프리뷰): `{"preview_id": "pv_c52b", "expires_at": "2026-11-06T11:30:00-05:00", "change_set_id": "cs_1043", "changes": [{"op": "update", "entity": "claim", "id": "clm_20290", "from": "requested", "to": "declined"}], "checks": [{"rule": "return_window_days", "passed": true, "detail": "Delivered Sep 28, 39 days ago (limit 30)", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "clm_20290", "undo": {"by": "person", "target": "cs_1043", "until": "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: 반려 사유가 정책과 맞는지 check. 같은 요청의 거절 메시지와 한 변경셋으로 묶이면 그 변경셋 승인을 따릅니다.
- 정책: `return_window_days`, `return.non_returnable_when`, `exchange.window`
- 에러: `invalid_state` → `get_claim` · `reason_required` → `reject_claim`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 이미 `declined`인 클레임은 같은 결과.

### `cancel_order_lines` — Kind: W$
- 논리주소/REST: `fulfillment.claim.cancel_order_lines` · `POST /v1/fulfillment/orders/{order_id}/cancellations`
- 요약: **스토어가 먼저** 출고 전 품목을 취소(품절·결품 지속, 사은품 소진, 판매자 사정, 상담원 직권). 쇼퍼의 취소 요청이 아니라 운영 에이전트가 제안하고 사람이 승인합니다. 귀책은 스토어이므로 배송비 재계산·추가 비용이 없고, 쿠폰은 복원, 적립 예정분은 취소, 환불은 원결제수단으로. 쇼퍼 안내 메시지(`send_customer_message`)와 한 변경셋으로 묶입니다.
- consumes → produces: `order_id` → `claim_id`(claim_type `cancel`)
- 입력: `order_id`(✓), `items[]`(✓, `{order_item_id, quantity}`), `reason`(✓, 스토어 취소 사유 `out_of_stock` \| `free_gift_unavailable` \| `seller_cancelled` \| `store_cancelled` \| `shipping_delay`), `note`
- 출력(프리뷰): `{"preview_id": "pv_c71b", "expires_at": "2026-11-06T10:45:00-05:00", "change_set_id": "cs_1047", "changes": [{"op": "create", "entity": "claim", "id": null, "from": null, "to": {"claim_type": "cancel", "status": "approved", "order_id": "ord_20461"}}, {"op": "update", "entity": "parcel_delivery", "id": "dlv_20461", "from": "awaiting_stock", "to": "cancelled"}, {"op": "create", "entity": "refund", "id": null, "from": null, "to": {"amount": {"value": 54.00, "currency": "USD"}}}], "checks": [{"rule": "cancel.not_cancellable_states", "passed": true, "detail": "Item is waiting for stock, not shipped", "fix_tool": null}, {"rule": "cancel.restore", "passed": true, "detail": "Store fault: 1 coupon restored, no shipping charge", "fix_tool": null}, {"rule": "change_set.irreversible", "passed": true, "detail": "The refund can't be undone once it is sent", "fix_tool": null}], "needs_approval": true, "approval_reason": "Store-initiated cancel refunds $54.00"}`
- 출력(실행): `{"ok": true, "id": "clm_4433", "undo": null}`
- 정책: `cancel.reasons`, `cancel.restore`, `cancel.not_cancellable_states`, `refund`, [approval.yml](../../policies/approval.yml)
- 에러: `already_shipped` → `get_delivery` · `order_item_not_found` → `get_order` · `reason_not_store_fault` → `cancel_order_lines` · `whole_quantity_only` → `cancel_order_lines` · `claim_in_progress` → `get_claim`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 서버는 `cancel.dedupe_on`(주문·품목·수량)으로 중복 취소를 막습니다.

### `convert_exchange_to_return` — Kind: W
- 논리주소/REST: `fulfillment.claim.convert_exchange_to_return` · `POST /v1/fulfillment/claims/{claim_id}/convert-to-return`
- 요약: 교환을 반품으로 전환(대체 변형 품절·단종, 쇼퍼가 받지 않겠다고 함). 회수 송장이 잡힌 뒤부터 재배송 출고 전까지 가능합니다(`exchange.conversion_window`). 교환 클레임은 `converted`로 닫히고, 품목·수량·사유·회수지·회수 송장·검수 결과를 **승계한** 새 반품 클레임이 `requested`로 생겨 이후 반품 흐름(`refund_return`)을 탑니다. 쇼퍼가 낸 교환 배송비는 자동 환불되고, 예약한 대체 재고는 풀립니다. 스토어 사정(품절·단종)이면 귀책이 스토어로 바뀌어 반품 비용이 없습니다. 전환 전에 쇼퍼 안내(`send_customer_message`)가 같은 변경셋에 있어야 합니다.
- consumes → produces: `claim_id` → `claim_id`(새 반품 클레임)
- 입력: `claim_id`(✓), `reason`(✓, `replacement_out_of_stock` \| `replacement_discontinued` \| `customer_declines`)
- 출력(프리뷰): `{"preview_id": "pv_c60d", "expires_at": "2026-11-06T11:40:00-05:00", "change_set_id": "cs_1047", "changes": [{"op": "update", "entity": "claim", "id": "clm_3301", "from": "picked_up", "to": "converted"}, {"op": "create", "entity": "claim", "id": null, "from": null, "to": {"claim_type": "return", "status": "requested", "liability": "store_fault"}}, {"op": "update", "entity": "inventory_level", "id": "var_trail_runner_10_slate", "from": {"reserved": 1}, "to": {"reserved": 0}}], "checks": [{"rule": "exchange.fallback_to_return_when", "passed": true, "detail": "Replacement size 10 is out of stock", "fix_tool": null}, {"rule": "exchange.on_fallback", "passed": true, "detail": "Liability moves to store fault: no return shipping charged", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "clm_4471", "undo": {"by": "person", "target": "cs_1047", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: `exchange.fallback_to_return_when`, `exchange.allowed_conversion_states`, `exchange.conversion_window`, `exchange.on_fallback`
- 에러: `not_an_exchange` → `get_claim` · `invalid_state`(이미 재배송 출고) → `get_claim` · `customer_not_notified` → `send_customer_message`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 교환 클레임 하나는 한 번만 전환됩니다.

### `upload_evidence` — Kind: W
- 논리주소/REST: `fulfillment.claim.upload_evidence` · `POST /v1/fulfillment/claims/{claim_id}/evidence`
- 요약: 클레임에 증빙(파손 사진·배송 사진·영수증) 첨부. 검수·승인·귀책 판단의 근거가 됩니다. append만 하며 지우지 않고 무효 표시만 합니다. 쇼퍼가 사진 요청을 받고 올린 사진도 여기로 들어와 클레임을 `photos_requested` → `requested`로 되돌립니다.
- consumes → produces: `claim_id` → `evidence_id[]`
- 입력: `claim_id`(✓), `upload_ids[]`(✓, 업로드 id `upl_…`, schema.org `MediaObject`), `evidence_type`(✓, `damage_photo` \| `delivery_photo` \| `product_defect` \| `receipt` \| `other`), `note`
- 출력(프리뷰): `{"preview_id": "pv_e11f", "expires_at": "2026-11-06T12:05:00-05:00", "changes": [{"op": "create", "entity": "evidence", "id": null, "from": null, "to": {"evidence_type": "damage_photo", "files": 2}}, {"op": "update", "entity": "claim", "id": "clm_20366", "from": "photos_requested", "to": "requested"}], "checks": [{"rule": "evidence.media_types", "passed": true, "detail": "2 JPEG photos", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "evd_901", "undo": null}` — 증빙은 지우지 않으므로 되돌리기가 없습니다(무효 표시는 운영자 화면에서).
- 정책: `evidence.media_types`, `evidence.max_files_per_claim`, `evidence.append_only`, `evidence.mask_payment_details`
- 에러: `claim_not_found` → `search_claims` · `unsupported_media_type` → `upload_evidence` · `evidence_limit_exceeded` → null · `invalid_state`(종결된 클레임) → `get_claim`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 서버는 `(claim_id, 파일 해시)`로 같은 파일의 중복 첨부를 막습니다.

---

## inspection

### `assess_return_condition` — Kind: W
- 논리주소/REST: `fulfillment.inspection.assess` · `POST /v1/fulfillment/claims/{claim_id}/inspection`
- 요약: 회수품 검수 결과 기록. 결과가 클레임 상태·비용·재고 복원을 정합니다. `as_new`일 때만 재고를 복원합니다(`return.inventory_restore_when`).
- consumes → produces: `claim_id` → `inspection_id`
- 입력: `claim_id`(✓), `condition`(✓, `acom:inspection_condition`: `as_new` \| `damaged` \| `missing_parts`), `liability`(`acom:liability`), `next`(`approve` \| `hold` \| `decline`), `note`
- 출력(프리뷰): `{"preview_id": "pv_i20a", "expires_at": "2026-11-09T15:30:00-05:00", "changes": [{"op": "update", "entity": "claim", "id": "clm_20418", "from": "picked_up", "to": "inspected"}, {"op": "update", "entity": "inventory_level", "id": "var_field_jacket_s_sand", "from": 9, "to": 11}], "checks": [{"rule": "return.inventory_restore_when", "passed": true, "detail": "Condition as new: 2 units back to stock", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "ins_4402", "undo": {"by": "person", "target": "cs_1051", "until": "2026-11-10T23:59:00-05:00"}}`
- 정책: `return.inventory_restore_when`, `liability`
- 에러: `not_picked_up` → `get_claim` · `invalid_state` → `get_claim`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 클레임 하나에 검수 결과는 하나(다시 기록하면 새 프리뷰가 필요).

---

## refund

### `refund_return` — Kind: W
- 논리주소/REST: `fulfillment.refund.create` · `POST /v1/fulfillment/refunds`
- 요약: 반품 클레임 한 건을 환불합니다. 환불액은 입력이 아니라 서버가 클레임에서 계산합니다(`evaluate_return_request`의 `refund_amount`와 같은 계산, 쿠폰 차감 포함). 사람 승인 여부는 [approval.yml](../../policies/approval.yml) `thresholds.refund`가 정합니다: **환불액이 $200.00를 넘으면 `needs_approval: true`**(변경셋에서 사람이 승인한 뒤 실행기가 `preview_id`로 실행), **$200.00 이하면 `needs_approval: false`**(사람 없이 실행). 환불은 원결제수단으로만 가고, 돈이 나간 뒤에는 되돌릴 수 없습니다. schema.org `PayAction`(결과는 `MoneyTransfer` + `additionalType: acom:Refund`), 결과는 `acom:Refund`. MVP 툴.
- consumes → produces: `claim_id` → `refund_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | claim_id | Text (`^clm_[a-z0-9_]+$`) | ✓ | 환불할 반품 클레임(`claim_type: return`, 상태 `requested`). `search_return_requests`·`evaluate_return_request`에서 | `"clm_20402"` |
- 출력(프리뷰): 변경셋 `cs_1043`에 붙은 프리뷰
  ```jsonc
  {"preview_id": "pv_r312",
   "expires_at": "2026-11-06T11:22:00-05:00",
   "change_set_id": "cs_1043",
   "changes": [{"op": "create", "entity": "refund", "id": null, "from": null,
                "to": {"claim_id": "clm_20402", "amount": {"value": 312.00, "currency": "USD"},
                       "refund_method": "original_payment", "to": "card ending 0937"}},
               {"op": "update", "entity": "claim", "id": "clm_20402", "from": "requested", "to": "approved"}],
   "checks": [{"rule": "refund.to", "passed": true, "detail": "$312.00 back to card ending 0937", "fix_tool": null},
              {"rule": "refund.coupon_deducted", "passed": true, "detail": "No coupon on order 20402", "fix_tool": null},
              {"rule": "change_set.irreversible", "passed": true, "detail": "A refund can't be undone once it is sent", "fix_tool": null}],
   "needs_approval": true,
   "approval_reason": "Refund $312.00 is over the $200.00 limit for refunds without a person"}
  ```
  - `refund.coupon_deducted`: 쿠폰을 쓴 주문은 쿠폰 할인분을 뺀 금액만 환불되는지 확인합니다(B2에서는 #20352, #20388, #20395). 맞지 않으면 `passed: false`, `fix_tool: "evaluate_return_request"`.
  - $200.00 이하 환불(예 $189.00)은 같은 모양에 `"needs_approval": false, "approval_reason": null`이고, 런타임이 바로 실행합니다.
- 출력(실행): `{"ok": true, "id": "rfd_20402", "undo": null}` — 환불은 되돌릴 수 없으므로 `undo`는 `null`. 결제 이벤트로 확정되면 `get_refund`의 `refund_status`가 `completed`가 됩니다.
- 프리뷰: `checks` `refund.to`, `refund.coupon_deducted`, `change_set.irreversible`. 여러 건을 한 요청으로 처리하면 건마다 프리뷰를 만들어 같은 변경셋에 붙입니다(B2: `cs_1043`에 환불 9건 $2,463.00 + `send_customer_message` 3건, 버튼 "Approve 9 refunds and send 3 messages"). 되돌릴 수 없는 항목이 있는 변경셋은 통째로 되돌릴 수 없습니다(`irreversible_change`).
- 정책: approval.yml `thresholds.refund`(승인 기준), `change_set.irreversible`, `refund.to`, `refund.coupon_deducted`, `refund.timing`, `refund.settlement`, `refund.on_failure`, `defect_refund`, `changed_mind_return_shipping` ([fulfillment.yml](../../policies/fulfillment.yml))
- 에러: `claim_not_found` → `search_return_requests` · `not_a_return`(반품이 아닌 클레임) → `get_claim` · `invalid_state`(이미 거절·완료된 클레임) → `get_claim` · `already_refunded` → `get_refund` · `amount_over_payment`(계산한 환불액이 결제액보다 큼) → `evaluate_return_request` · `coupon_not_deducted` → `evaluate_return_request` · `payment_gateway_error`(결제사 실패, `upstream_unavailable`) → `retry_refund`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 클레임 하나에 환불 하나 — 다른 프리뷰로 다시 와도 `already_refunded`로 중복 환불을 막습니다.

### `get_refund` — Kind: R
- 논리주소/REST: `fulfillment.refund.get` · `GET /v1/fulfillment/refunds/{refund_id}`
- 요약: 환불 한 건(상태, 결제 이벤트 확정 여부, 결제수단별 금액).
- consumes → produces: `refund_id` → refund
- 입력: `refund_id`(✓), `fields?[]`
- 출력: `{"id": "rfd_20402", "claim_id": "clm_20402", "amount": {"value": 312.00, "currency": "USD"}, "refund_method": "original_payment", "to": "card ending 0937", "refund_status": "requested", "events": [{"refund_status": "requested", "at": "2026-11-06T11:20:00-05:00"}]}` — `refund_status`: `requested` \| `completed` \| `failed`
- 정책: 없음(읽기).
- 에러: `refund_not_found` → `search_claims`
- 멱등성: 읽기.

### `retry_refund` — Kind: W$
- 논리주소/REST: `fulfillment.refund.retry` · `POST /v1/fulfillment/refunds/{refund_id}/retry`
- 요약: 실패한 환불 재시도(`failed` → `requested`). 결제 이벤트가 오지 않거나 게이트웨이 오류일 때. 실패는 수동 큐에 남고 조용히 사라지지 않습니다. 돈이 움직이므로 항상 승인.
- consumes → produces: `refund_id` → `refund_id`
- 입력: `refund_id`(✓)
- 출력(프리뷰): `{"preview_id": "pv_r9a1", "expires_at": "2026-11-06T15:30:00-05:00", "changes": [{"op": "update", "entity": "refund", "id": "rfd_20377", "from": "failed", "to": "requested"}], "checks": [{"rule": "refund.to", "passed": true, "detail": "$217.00 back to card ending 5512", "fix_tool": null}, {"rule": "refund.max_retries", "passed": true, "detail": "Attempt 2 of 3", "fix_tool": null}, {"rule": "change_set.irreversible", "passed": true, "detail": "A refund can't be undone once it is sent", "fix_tool": null}], "needs_approval": true, "approval_reason": "Retries a $217.00 refund"}`
- 출력(실행): `{"ok": true, "id": "rfd_20377", "undo": null}`
- 정책: `refund.on_failure`, `refund.manual_queue_sla_hours`, `refund.max_retries`
- 에러: `not_failed` → `get_refund` · `max_retries_reached`(수동 처리) → `get_refund`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 시도 번호마다 한 번.

---

## parcel_delivery

### `get_delivery_dates` — Kind: R
- 논리주소/REST: `fulfillment.parcel_delivery.get_delivery_dates` · `GET /v1/fulfillment/delivery-dates?product_id=prd_windbreaker&address_mode=saved&address_id=adr_home&by=2026-11-14`
- 요약: 상품을 이 주소로 받을 수 있는 가장 이른 날짜. `by`를 주면 그날까지 도착 가능한지도 답합니다. 쇼퍼 툴, MVP. 출고일은 `delivery.estimate`(출고 마감 시각·출고 요일), 운송 일수는 배송 지역 데이터에서 계산합니다. schema.org `ParcelDelivery.expected_arrival_until`.
- consumes → produces: `product_id` → delivery estimate
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | product_id | Text (`^prd_[a-z0-9_]+$`) | ✓ | 상품(사람이 말하는 상품) | `"prd_windbreaker"` |
  | variant_id | Text (`^var_[a-z0-9_]+$`) | | 사이즈·색을 골랐으면 그 변형(재고 확인이 정확해짐) | `"var_windbreaker_m_slate"` |
  | address_mode | Text (enum) | ✓ | `saved`: 쇼퍼가 저장한 주소(`address_id` 필요); `one_time`: 이번만 쓸 주소(`delivery_address` 필요, 저장하지 않음 — 선물 받는 사람 주소 등) | `"saved"` |
  | address_id | Text (`^adr_[a-z0-9_]+$`) | `saved`일 때 | 저장한 배송지 | `"adr_home"` |
  | delivery_address | PostalAddress `{street_address, address_locality, address_region, postal_code, address_country}` | `one_time`일 때 | 배송지 | `{"street_address": "48 Pine Rd", "address_locality": "Lakeview", "address_region": "NY", "postal_code": "14085", "address_country": "US"}` |
  | by | Date | | 이 날짜까지 받아야 함 | `"2026-11-14"` |
- 출력:
  ```jsonc
  {"items": [{"product_id": "prd_windbreaker", "variant_id": "var_windbreaker_m_slate",
              "method": "standard",                              // 배송 방법(enums.md §3)
              "ships_on": "2026-11-07", "earliest_arrival": "2026-11-10",
              "shipping": {"value": 0.00, "currency": "USD"},
              "shipping_note": "Free over $80.00 before member benefits",
              "remote_area_surcharge": {"value": 0.00, "currency": "USD"},
              "by": "2026-11-14", "arrives_by_requested_date": true}],
   "total": 1}
  ```
  - 예: Sat Nov 7 10:00, Jamie의 `adr_home`(Brooklyn) → 그날 출고, Tue Nov 10 도착. 배송비는 이 상품 하나 기준이고 장바구니 전체 배송비는 `get_cart`가 계산합니다.
- 정책: `delivery.estimate`, `delivery.methods`, `delivery.remote_area_surcharge`, `delivery.no_ship_for`, `free_shipping_min_subtotal`([order.yml](../../policies/order.yml))
- 에러: `product_not_found` → `search_products` · `address_not_found` → `get_my_profile` · `address_not_deliverable` → `get_my_profile` · `out_of_stock` → `get_product`
- 멱등성: 읽기.

### `get_delivery` — Kind: R
- 논리주소/REST: `fulfillment.parcel_delivery.get` · `GET /v1/fulfillment/deliveries/{delivery_id}`
- 요약: 배송 한 건(상태 이력, 송장, 지역 추가 배송비).
- consumes → produces: `delivery_id` → delivery
- 입력: `delivery_id`(✓), `fields?[]`
- 출력: `{"id": "dlv_20402", "order_id": "ord_20402", "delivery_address": "12 Harbor St, Apt 3", "tracking_number": "1Z999", "carrier": "UPS", "delivery_state": "delivered", "expected_arrival_until": "2026-10-31", "events": [{"delivery_state": "delivered", "at": "2026-10-31T14:02:00-04:00"}]}`
- 정책: 없음(읽기).
- 에러: `delivery_not_found` → `get_order`
- 멱등성: 읽기.

### `track_shipment` — Kind: R
- 논리주소/REST: `fulfillment.parcel_delivery.track` · `GET /v1/fulfillment/deliveries/{delivery_id}/tracking`
- 요약: 택배사 추적 이벤트. 이벤트는 `integration`의 `receive_carrier_event`가 받아 쌓은 것입니다.
- consumes → produces: `delivery_id` → tracking events
- 입력: `mode`(✓, `delivery`: `delivery_id`로 \| `tracking_number`: 송장 번호로), `delivery_id`(mode `delivery`일 때), `tracking_number`(mode `tracking_number`일 때), `limit`, `cursor`
- 출력: `{"items": [{"at": "2026-10-31T14:02:00-04:00", "delivery_state": "delivered", "location": "Brooklyn, NY"}], "total": 5}`
- 정책: `delivery.tracking`
- 에러: `tracking_not_found` → `get_delivery`
- 멱등성: 읽기.

### `schedule_shipment` — Kind: W
- 논리주소/REST: `fulfillment.parcel_delivery.schedule` · `POST /v1/fulfillment/deliveries`
- 요약: 출고 지시(배송 그룹 단위, 부분 출고 가능). 재고가 있으면 `ship_instructed`, 부족하면 `awaiting_stock`. 포장 완료(`packed`)는 창고 연동이 기록하고, 송장은 `confirm_shipment`에서 확정합니다.
- consumes → produces: `order_id` → `delivery_id`
- 입력: `order_id`(✓), `order_item_ids?[]`(부분 출고), `shipping_group`, `delivery_method`(`standard`), `carrier`
- 출력(프리뷰): `{"preview_id": "pv_d10c", "expires_at": "2026-11-06T09:40:00-05:00", "changes": [{"op": "create", "entity": "parcel_delivery", "id": null, "from": null, "to": {"order_id": "ord_20455", "delivery_state": "ship_instructed", "order_item_ids": ["oln_1", "oln_2"]}}], "checks": [{"rule": "delivery.shortage_state", "passed": true, "detail": "All 2 items in stock", "fix_tool": "register_shortage"}, {"rule": "delivery.no_ship_for", "passed": true, "detail": "No gift cards in this order", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "dlv_20455", "undo": {"by": "person", "target": "cs_1048", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: `delivery.shipping_group_by`, `delivery.remote_area_surcharge`, `delivery.no_ship_for`, `delivery.shortage_state`
- 에러: `no_ship_item`(배송하지 않는 상품, 그 품목을 빼고 다시) → `schedule_shipment` · `out_of_stock` → `register_shortage` · `invalid_state` → `get_delivery`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 서버는 `(order_id, shipping_group, order_item_ids)`로 중복 출고 지시를 막습니다.

### `confirm_shipment` — Kind: W
- 논리주소/REST: `fulfillment.parcel_delivery.confirm` · `POST /v1/fulfillment/deliveries/{delivery_id}/confirm`
- 요약: 출고 확정(송장번호 기재, 배송 `packed` → `label_created`, 품목의 출고 단계 `shipped`). 모든 품목이 `shipped`면 주문 상태가 `in_transit`이 됩니다.
- consumes → produces: `delivery_id` → `delivery_id`
- 입력: `delivery_id`(✓), `tracking_number`(✓), `carrier`(✓)
- 출력(프리뷰): `{"preview_id": "pv_d11a", "expires_at": "2026-11-06T16:30:00-05:00", "changes": [{"op": "update", "entity": "parcel_delivery", "id": "dlv_20455", "from": "packed", "to": "label_created"}], "checks": [{"rule": "delivery.tracking", "passed": true, "detail": "Tracking number matches the UPS format", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "dlv_20455", "undo": {"by": "person", "target": "cs_1048", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: `delivery.tracking`
- 에러: `invalid_state` → `get_delivery` · `tracking_required` → `confirm_shipment`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 같은 송장 번호는 한 배송에만.

### `register_shortage` — Kind: W
- 논리주소/REST: `fulfillment.parcel_delivery.register_shortage` · `POST /v1/fulfillment/deliveries/{delivery_id}/shortage`
- 요약: 재고 부족 등록(`awaiting_stock`). 입고되면 다시 `ship_instructed`.
- consumes → produces: `delivery_id` → `delivery_id`
- 입력: `delivery_id`(✓), `order_item_ids[]`(✓), `reason`
- 출력(프리뷰): `{"preview_id": "pv_d12e", "expires_at": "2026-11-06T10:10:00-05:00", "changes": [{"op": "update", "entity": "parcel_delivery", "id": "dlv_20455", "from": "ship_instructed", "to": "awaiting_stock"}], "checks": [{"rule": "delivery.shortage_state", "passed": true, "detail": "1 item short; customer can cancel it themselves while waiting", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "dlv_20455", "undo": {"by": "person", "target": "cs_1048", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: `delivery.shortage_state`, `cancel.self_service_states`
- 에러: `invalid_state` → `get_delivery`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과.

---

## shipping_rate

### `list_shipping_zones` — Kind: R
- 논리주소/REST: `fulfillment.shipping_rate.list_zones` · `GET /v1/fulfillment/shipping-zones`
- 요약: 배송 지역과 요율(지역 추가 배송비, 조건부 무료 기준, 운송 일수 포함).
- consumes → produces: → `zone_id[]`
- 입력: `fields?[]`, `limit`(기본 20), `cursor`
- 출력: `{"items": [{"id": "zone_us_contiguous", "name": "Contiguous US", "zone_type": "standard", "fee_type": "free_over_min_subtotal", "shipping_rate": {"value": 6.00, "currency": "USD"}, "free_over": {"value": 80.00, "currency": "USD"}, "surcharge": {"value": 0.00, "currency": "USD"}, "transit_business_days": 2}], "total": 3}`
- 정책: 없음(읽기).
- 에러: `invalid_filter` → `list_shipping_zones`
- 멱등성: 읽기.

### `manage_shipping_zone` — Kind: W
- 논리주소/REST: `fulfillment.shipping_rate.manage_zone` · `POST /v1/fulfillment/shipping-zones`
- 요약: 배송 지역 정의·수정(원거리 지역 등). schema.org `OfferShippingDetails`, `shipping_destination`(`DefinedRegion[]`).
- consumes → produces: → `zone_id`
- 입력: `zone_id`(수정 시), `name`(✓), `shipping_destination[]`(✓), `surcharge`(MonetaryAmount), `transit_business_days`, `zone_type`(`standard` \| `remote`)
- 출력(프리뷰): `{"preview_id": "pv_z01b", "expires_at": "2026-11-06T13:30:00-05:00", "changes": [{"op": "create", "entity": "shipping_zone", "id": null, "from": null, "to": {"name": "Alaska and Hawaii", "zone_type": "remote", "surcharge": {"value": 12.00, "currency": "USD"}}}], "checks": [{"rule": "delivery.zone_overlap", "passed": true, "detail": "No region is in two zones", "fix_tool": "list_shipping_zones"}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "zone_us_remote", "undo": {"by": "person", "target": "cs_1049", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: `delivery.remote_area_surcharge`, `delivery.zone_overlap`
- 에러: `overlapping_zone` → `list_shipping_zones`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과.

### `set_shipping_rate` — Kind: W
- 논리주소/REST: `fulfillment.shipping_rate.set` · `POST /v1/fulfillment/shipping-zones/{zone_id}/rate`
- 요약: 지역별 배송비 설정(유료·무료·조건부 무료 기준). schema.org `ShippingRateSettings`.
- consumes → produces: `zone_id` → `zone_id`
- 입력: `zone_id`(✓), `shipping_rate`(✓, MonetaryAmount, 예 `{"value": 6.00, "currency": "USD"}`), `fee_type`(✓, `paid` \| `free` \| `free_over_min_subtotal`), `free_over`(MonetaryAmount, `free_over_min_subtotal`일 때, 예 `{"value": 80.00, "currency": "USD"}`)
- 출력(프리뷰): `{"preview_id": "pv_z02c", "expires_at": "2026-11-06T13:40:00-05:00", "changes": [{"op": "update", "entity": "shipping_zone", "id": "zone_us_contiguous", "from": {"shipping_rate": {"value": 6.00, "currency": "USD"}}, "to": {"shipping_rate": {"value": 7.00, "currency": "USD"}}}], "checks": [{"rule": "free_shipping_min_subtotal", "passed": true, "detail": "Free over $80.00 kept", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "zone_us_contiguous", "undo": {"by": "person", "target": "cs_1049", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: `delivery.shipping_fee_types`, `free_shipping_min_subtotal`([order.yml](../../policies/order.yml))
- 에러: `invalid_threshold` → `set_shipping_rate`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과.

---

## 사람 전용 엔드포인트

쇼퍼 본인이 UI 버튼으로만 부르는 HTTP 엔드포인트입니다. **툴이 아니며 모델은 보지 못하므로**, 어떤 프롬프트로도 에이전트가 반품·취소·교환 요청을 대신 보낼 수 없습니다. 쇼퍼 에이전트는 선택지 툴(`get_cancel_options`, `get_exchange_options`, `get_return_options`)로 준비한 내용을 확인 카드로 보여주고, 쇼퍼가 그 카드의 `person` 버튼을 누릅니다([chat.md](chat.md) §3.2). 전체 목록은 [README.md](README.md) §2.

| 엔드포인트 | 스코프 | 누가 | 준비 툴 | 무엇 |
|---|---|---|---|---|
| `POST /v1/fulfillment/orders/{order_id}/cancel` | `human:cancel` | 쇼퍼 | `get_cancel_options` | 출고 전 취소(전체·품목·수량) |
| `POST /v1/fulfillment/exchanges` | `human:exchange` | 쇼퍼 | `get_exchange_options` | 같은 상품 다른 변형으로 교환(회수 → 재배송) |
| `POST /v1/fulfillment/returns` | `human:return` | 쇼퍼 | `get_return_options` | 반품(환불) 또는 같은 변형 교체 |
| `POST /v1/fulfillment/claims/{claim_id}/pay-charge` | `human:claim` | 쇼퍼 | `get_claim` | `charge_due` 클레임의 추가 비용 결제 → 접수 |
| `POST /v1/fulfillment/claims/{claim_id}/withdraw` | `human:claim` | 쇼퍼 | `get_claim` | 교환·반품 철회(클레임 단위). 취소는 철회 불가 |

**공통 규칙**
- 사람 세션(`Cookie: acom_session` + `X-CSRF-Token`)과 해당 `human:*` 스코프가 있어야 합니다. 채팅에서 발급된 토큰에는 `human:*`가 없으므로 에이전트는 부를 수 없습니다([../backend/gateway.md](../backend/gateway.md) §1–2).
- 쇼퍼 세션의 본인 주문만(`not_your_order`). `Idempotency-Key` 필수: 같은 키 재전송은 첫 결과를 돌려주고, 다른 키라도 같은 품목에 열린 클레임이 있으면 `claim_in_progress`.
- 서버가 보내는 순간 선택지를 **다시 계산**합니다. 쇼퍼가 본 견적(`expected_refund`·`expected_charge`)과 다르면 실행하지 않고 `estimate_changed`(새 견적은 `detail.new_preview`)로 거부해, 확인하지 않은 금액으로 처리되는 일이 없습니다.
- 응답은 `{ok, id, …}`에 클레임 상태와 견적을 더한 형식입니다. 에러는 공통 형식이고 **`next_tool`은 항상 `null`**입니다(사람이 화면에서 처리, [errors.md](errors.md) 규칙 1). 괄호 안의 설명은 화면이 보여 줄 안내입니다.
- 상태 변화는 클레임 이벤트로 append되고, 쇼퍼 알림은 `messaging`이 상태 이벤트를 받아 보냅니다.

### `POST /v1/fulfillment/orders/{order_id}/cancel`
- 스코프: `human:cancel`
- 요약: 출고 전 취소. 경로는 품목 상태가 정합니다(위 [상태 요약](#상태-요약)): 결제 전은 즉시 취소, 직접 취소 상태는 자동 승인, 피킹·포장 중은 운영 큐로 가는 요청. 출고 후는 거부하고 반품을 안내합니다. schema.org `CancelAction`.
- 본문:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | items | {order_item_id, quantity}[] | | 생략하면 전체 취소 | `[{"order_item_id": "oln_1", "quantity": 1}]` |
  | reason | Text (enum `acom:cancel_reason`) | ✓ | `get_cancel_options.reasons`의 값 | `"changed_mind"` |
  | reason_text | Text | | 쇼퍼가 쓴 말(500자) | `"Ordered the wrong color"` |
  | expected_refund | MonetaryAmount | ✓ | 쇼퍼가 확인한 `estimate.refund_total` | `{"value": 65.20, "currency": "USD"}` |
  | expected_charge | MonetaryAmount | | 확인한 `estimate.charge_due`(기본 0.00) | `{"value": 0.00, "currency": "USD"}` |
  | refund_to | {type, ref} | 조건부 | `needs_refund_destination: true`일 때만. `type` `bank_account` \| `store_credit`, `ref`는 저장된 계좌 참조(계좌번호 원문은 받지 않음) | `{"type": "store_credit", "ref": null}` |
- 응답:
  ```jsonc
  {"ok": true, "id": "clm_4410", "claim_type": "cancel", "status": "approved",   // completed | approved | charge_due | requested
   "order_id": "ord_20455", "items": [{"order_item_id": "oln_1", "quantity": 1}],
   "liability": "customer_fault",
   "refund": {"total": {"value": 65.20, "currency": "USD"},
              "breakdown": [{"to": "card ending 4821", "amount": {"value": 65.20, "currency": "USD"}}],
              "money_back_business_days": {"min": 3, "max": 5}},
   "extra_charge": [{"kind": "lost_free_shipping", "amount": {"value": 6.00, "currency": "USD"}, "collected": "deducted_from_refund"}],
   "coupons_restored": [], "points_pending_cancelled": {"value": 0.71, "currency": "USD"},
   "confirmed_by": "store_automatic", "undo": null}
  ```
  - `status: "requested"`면 `confirmed_by: "store_person"`이고, 승인 전까지 `refund`는 견적입니다. `status: "charge_due"`면 `pay_charge: {"amount": {…}, "endpoint": "POST /v1/fulfillment/claims/clm_4410/pay-charge"}`가 붙습니다.
- 에러: `already_shipped`(출고됨, 반품으로 안내) → null · `not_cancellable` → null · `quantity_over_cancellable` → null · `whole_quantity_only` → null · `reason_not_selectable` → null · `estimate_changed`(새 견적을 다시 보여 줌) → null · `refund_destination_required` → null · `already_cancelled` → null · `claim_in_progress` → null · `not_your_order` → null
- 멱등성: `Idempotency-Key` 필수. 서버는 `cancel.dedupe_on`(주문·품목·수량)으로도 중복을 막아, 같은 품목·수량을 두 번 취소하지 않습니다.

### `POST /v1/fulfillment/exchanges`
- 스코프: `human:exchange`
- 요약: 같은 상품의 다른 변형(스토어 귀책이면 같은 변형도)으로 교환. 요청 시 대체 재고를 예약하고, 쇼퍼 부담 비용이 있으면 `charge_due`로 결제를 먼저 받은 뒤 `requested`가 됩니다. 이후 회수 지시는 자동 또는 `approve_claim`, 재배송은 검수 후. schema.org `ReturnAction` + 재배송 `ParcelDelivery`.
- 본문:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | order_id | Text (`^ord_`) | ✓ | | `"ord_20447"` |
  | items | {order_item_id, quantity, replacement_variant_id}[] | ✓ | 품목별 교환. 수량은 바꿀 수 없고, 대체 변형은 `get_exchange_options`에서 `selectable: true`인 것 | `[{"order_item_id": "oln_2", "quantity": 1, "replacement_variant_id": "var_trail_runner_10_slate"}]` |
  | reason | Text (enum `acom:claim_reason`) | ✓ | `get_exchange_options`의 사유 | `"wrong_size"` |
  | reason_text | Text | | 쇼퍼가 쓴 말 | `"Runs half a size small"` |
  | evidence_ids | Text[] (`^evd_`) | 조건부 | `damaged_in_transit`·`defect`는 `damage_claim_needs_photo`에 따라 사진(`upload_evidence`) | `["evd_901"]` |
  | pickup_address_id | Text (`^adr_`) | | 생략 시 배송지 | `"adr_home"` |
  | delivery_address_id | Text (`^adr_`) | | 재배송지. 생략 시 배송지 | `"adr_home"` |
  | pickup_window | {start, end} | | `get_exchange_options.pickup.window` | `{"start": "2026-11-09T09:00:00-05:00", "end": "2026-11-09T12:00:00-05:00"}` |
  | expected_charge | MonetaryAmount | ✓ | 쇼퍼가 확인한 `you_pay` | `{"value": 0.00, "currency": "USD"}` |
- 응답:
  ```jsonc
  {"ok": true, "id": "clm_3301", "claim_type": "exchange", "status": "requested",   // charge_due | requested | pickup_instructed
   "liability": "customer_fault",
   "replacement": [{"order_item_id": "oln_2", "variant_id": "var_trail_runner_10_slate", "reserved": 1}],
   "pickup": {"window": {"start": "2026-11-09T09:00:00-05:00", "end": "2026-11-09T12:00:00-05:00"}, "cost": {"value": 0.00, "currency": "USD"}},
   "reship": {"ships_after": "inspected", "arrives_by": "2026-11-14", "cost": {"value": 0.00, "currency": "USD"}},
   "you_pay": {"value": 0.00, "currency": "USD"},
   "withdraw_until": "pickup_instructed",
   "confirmed_by": "store_automatic", "undo": null}
  ```
- 에러: `not_shipped_yet`(출고 전이면 취소) → null · `outside_exchange_window` → null · `not_exchangeable` → null · `replacement_unavailable`(방금 품절) → null · `price_differs` → null · `quantity_mismatch` → null · `whole_quantity_only` → null · `evidence_required`(사진을 먼저 올림) → null · `estimate_changed` → null · `claim_in_progress` → null · `not_your_order` → null
- 멱등성: `Idempotency-Key` 필수. 같은 품목에 열린 교환·반품이 있으면 새로 만들지 않습니다.

### `POST /v1/fulfillment/returns`
- 스코프: `human:return`
- 요약: 반품 요청 보내기. `kind: refund`는 반품 클레임(`claim_type: return`, `requested`)이 되고, `kind: replacement`(같은 변형 새 상품)는 교환 클레임으로 만들어집니다. 쇼퍼 귀책 반품의 반품 배송비·무료 배송 회수분은 환불에서 차감하며, 환불액보다 크면 `charge_due`. 본문은 아래 하나뿐입니다. [chat.md](chat.md) `return_options` 블록의 "Send refund request" 버튼이 이 엔드포인트를 부릅니다.
- 본문:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | order_id | Text (`^ord_`) | ✓ | | `"ord_20402"` |
  | items | {order_item_id, quantity}[] | ✓ | 반품할 품목과 수량 | `[{"order_item_id": "oln_1", "quantity": 1}]` |
  | reason | Text (enum `acom:claim_reason`) | ✓ | `get_return_options`에 준 사유 | `"defect"` |
  | kind | Text (enum) | ✓ | `refund` \| `replacement` (`get_return_options.items[].kind`) | `"refund"` |
  | reason_text | Text | | 쇼퍼가 쓴 말 | `"The sole is coming apart after a week."` |
  | evidence_ids | Text[] (`^evd_`) | 조건부 | 배송 중 파손이면 사진 필수(`damage_claim_needs_photo`) | `["evd_877", "evd_878"]` |
  | send_back_method | Text (enum) | ✓ | `pickup` \| `drop_off` (`return.send_back_methods`) | `"pickup"` |
  | pickup_window | {start, end} | `pickup`일 때 | `get_return_options`의 `send_back.pickup_window` | `{"start": "2026-11-09T09:00:00-05:00", "end": "2026-11-09T12:00:00-05:00"}` |
  | expected_refund | MonetaryAmount | `refund`일 때 | 쇼퍼가 확인한 `refund_amount` | `{"value": 312.00, "currency": "USD"}` |
  | expected_charge | MonetaryAmount | | 확인한 `you_pay`(기본 0.00) | `{"value": 0.00, "currency": "USD"}` |
  | refund_to | {type, ref} | 조건부 | 원결제 되돌리기 불가 시만 | `{"type": "store_credit", "ref": null}` |
- 응답: `{"ok": true, "id": "clm_20402", "claim_type": "return", "status": "requested", "refund_estimate": {"total": {"value": 312.00, "currency": "USD"}, "breakdown": [{"to": "card ending 0937", "amount": {"value": 312.00, "currency": "USD"}}]}, "send_back": {"method": "pickup", "cost": {"value": 0.00, "currency": "USD"}, "pickup_window": {"start": "2026-11-09T09:00:00-05:00", "end": "2026-11-09T12:00:00-05:00"}}, "confirmed_by": "store_person", "undo": null}` — 이 요청이 B2 검토 큐의 #20402가 됩니다.
- 에러: `outside_return_window`(기간이 지남, 사유 안내) → null · `not_returnable` → null · `not_shipped_yet` → null · `evidence_required`(사진을 먼저 올림) → null · `estimate_changed` → null · `claim_in_progress` → null · `not_your_order` → null
- 멱등성: `Idempotency-Key` 필수.

### `POST /v1/fulfillment/claims/{claim_id}/pay-charge`
- 스코프: `human:claim`
- 요약: `charge_due` 클레임의 추가 비용(취소의 환불 초과분, 교환 왕복 배송비, 반품 배송비 초과분) 결제. 결제 이벤트로 확정되면 클레임이 다음 상태(`approved` \| `requested`)로 갑니다.
- 본문: `amount`(✓, MonetaryAmount, 쇼퍼가 확인한 금액), `payment_method_ref`(✓, 저장된 결제 수단 참조)
- 응답: `{"ok": true, "id": "clm_3302", "status": "requested", "charged": {"value": 12.00, "currency": "USD"}, "undo": null}`
- 에러: `estimate_changed`(금액이 바뀜) → null · `invalid_state` → null · `payment_declined` → null · `not_your_order` → null
- 멱등성: `Idempotency-Key` 필수.

### `POST /v1/fulfillment/claims/{claim_id}/withdraw`
- 스코프: `human:claim`
- 요약: 교환·반품을 클레임 단위로 철회(일부 품목만 철회 불가). 회수 송장이 잡히기 전까지(`exchange.withdraw`, `return.withdraw`). 예약 재고 해제, 낸 추가 비용 자동 환불, 품목은 `delivered`로. 취소 클레임은 철회할 수 없습니다(`not_revocable`).
- 본문: `reason_text`(선택)
- 응답: `{"ok": true, "id": "clm_3301", "status": "withdrawn", "charge_refunded": {"value": 0.00, "currency": "USD"}, "undo": null}`
- 에러: `invalid_state`(회수 송장이 이미 생김) → null · `not_revocable`(취소 클레임) → null · `not_your_order` → null
- 멱등성: `Idempotency-Key` 필수. 이미 `withdrawn`이면 같은 결과.
