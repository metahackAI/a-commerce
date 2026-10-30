# API 명세 — `fulfillment` (클레임[반품·취소·교환]·검수·환불·배송·배송비)

> 계층: `fulfillment.{resource}.{operation}`. 공통 규약(응답 형식·에러·프리뷰/실행·멱등·헤더)은 [README.md](README.md) §3, 데이터 타입은 [data-model.md](data-model.md), 정책은 [fulfillment.yml](policies/fulfillment.yml) · [approval.yml](policies/approval.yml). MVP 툴의 시그니처는 [plan.md §3.3·§3.4](../plan.md)와 같습니다.
> **설계 축**: ① 반품 요청(`ReturnAction`)은 운영 에이전트가 **정책 근거와 함께 묶어 검토**하고, 환불·고객 메시지는 한 변경셋으로 사람이 승인, ② 반품 요청을 **보내는 것은 쇼퍼 본인**(사람 전용 엔드포인트, 툴 아님), ③ 환불은 원결제수단으로, 상태는 결제 이벤트로 확정(조용한 실패 없음), ④ 검수 결과가 클레임 상태·비용·재고 복원을 구동, ⑤ 귀책 → 비용 규칙은 정책 데이터, ⑥ 상태 변화는 이벤트 테이블에 append.

리소스: [claim](#claim) · [inspection](#inspection) · [refund](#refund) · [parcel_delivery](#parcel_delivery) · [shipping_rate](#shipping_rate) · [사람 전용 엔드포인트](#사람-전용-엔드포인트)

---

## 상태 요약

**claim — 반품 요청(`ReturnAction`)** (MVP 백오피스 큐의 `status`)

```
(쇼퍼) POST /returns ──▶ waiting ──refund_return 승인·실행──▶ approved ──▶ picked_up ──▶ inspected ──▶ refunded
                           │
                           ├─send_customer_message(kind="ask_for_photos")──▶ photos_requested ──사진 도착──▶ waiting
                           └─send_customer_message(kind="decline") / reject_claim──▶ declined
```

**claim — 취소·교환** (출고 전 = 취소, 출고 후 = 반품; 교환 = 회수 + 재배송)

```
cancel:    requested ──approve_claim──▶ approved ──▶ completed(+refund)
               └─reject_claim──▶ declined           (출고 후에는 반품으로)

exchange:  requested ──approve_claim──▶ recovery(반품 흐름) ──▶ reshipment(배송 흐름)
               대체 상품 품절·쇼퍼 거절 ──convert_exchange_to_return──▶ 반품 + 환불
```

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
- 정책: `return_window_days`, `defect_refund`, `changed_mind_return_shipping`(변심이면 `you_pay`에 반품 배송비), `size_exchange_days`, `refund_without_person_max`(초과 금액이면 `confirmed_by`가 사람)
- 에러: `order_not_found` → `get_my_orders` · `line_not_found` → `get_order` · `outside_return_window` → (선택지 없음, 사유 안내) · `not_your_order` → `get_my_orders`
- 멱등성: 읽기, 부작용 없음.

### `search_claims` — Kind: R
- 논리주소/REST: `fulfillment.claim.search` · `GET /v1/fulfillment/claims`
- 요약: 반품·취소·교환 클레임 전체 검색. 반품 요청 큐만 볼 때는 `search_return_requests`를 씁니다.
- consumes → produces: → `claim_id[]`
- 입력: `type`(`return` \| `cancel` \| `exchange`), `status`, `order_id`, `customer_id`, `created_from`, `created_through`, `sort`(`newest` \| `amount_desc`), `fields?[]`, `limit`(기본 20), `offset`(기본 0)
- 출력: `{"items": [{"claim_id": "clm_3301", "type": "exchange", "status": "requested", "order_id": "ord_20388", "amount": 128.00, "price_currency": "USD"}], "total": 7}`
- 정책: 없음(읽기).
- 에러: `invalid_filter` → `search_claims`
- 멱등성: 읽기.

### `get_claim` — Kind: R
- 논리주소/REST: `fulfillment.claim.get` · `GET /v1/fulfillment/claims/{claim_id}`
- 요약: 클레임 한 건(상태 이력·증빙·검수·비용·연결된 환불). 반품 요청 id(`ret_…`)도 받습니다.
- consumes → produces: `claim_id` → claim
- 입력: `claim_id`(✓), `fields?[]`
- 출력: `{"items": [{"claim_id": "ret_20402", "type": "return", "object": {"order_id": "ord_20402", "line_id": "line_1"}, "reason": "defect", "acom:liability": "seller_fault", "evidence": [...], "inspection": null, "refund_id": null, "status_history": [{"status": "waiting", "at": "2026-11-06T10:12:00"}]}], "total": 1}`
- 정책: 없음(읽기).
- 에러: `claim_not_found` → `search_claims`
- 멱등성: 읽기.

### `approve_claim` — Kind: W$
- 논리주소/REST: `fulfillment.claim.approve` · `POST /v1/fulfillment/claims/{claim_id}/approve`
- 요약: 취소·교환 클레임(및 환불 없이 진행할 반품의 회수 지시)을 승인. 승인되면 회수 지시·재배송·환불 개시가 이어집니다. 반품 환불은 `refund_return`으로 합니다.
- consumes → produces: `claim_id[]` → `claim_id[]`(approved)
- 입력: `claim_id[]`(✓), `acom:liability`(`seller_fault` \| `customer_fault`, 미지정 시 접수값·검수값), `acom:cost_override`(배송비 조정, `note` 필수), `preview`(✓)
- 출력(프리뷰):
  ```jsonc
  {"preview_id": "pv_c41a",
   "changes": [{"op": "update", "entity": "claim", "id": "clm_3301", "from": "requested", "to": "approved"}],
   "checks": [{"rule": "liability_cost", "passed": true, "detail": "Seller fault: store pays both shipping legs", "fix_tool": null},
              {"rule": "inspection_before_restock", "passed": true, "detail": "Stock restores only after a normal inspection", "fix_tool": "assess_return_condition"}],
   "needs_approval": true, "approval_reason": "Approves 1 exchange"}
  ```
- 출력(실행): `{"ok": true, "id": "clm_3301", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 프리뷰: `checks` `liability_cost`(귀책 ↔ 비용 일관), `inspection_before_restock`. 항상 변경셋을 거쳐 사람이 승인.
- 정책: `liability`, `exchange`, `cancel` ([fulfillment.yml](policies/fulfillment.yml)), [approval.yml](policies/approval.yml)
- 에러: `liability_cost_conflict` → `approve_claim`(귀책·비용 다시 지정) · `inspection_pending` → `assess_return_condition` · `invalid_state` → `get_claim`
- 멱등성: `Idempotency-Key = hash(claim_id[] + decision)` — 중복 승인 없음.

### `reject_claim` — Kind: W
- 논리주소/REST: `fulfillment.claim.reject` · `POST /v1/fulfillment/claims/{claim_id}/reject`
- 요약: 클레임 반려(사유 필수). `requested`/`waiting` → `declined`. 쇼퍼에게 알리는 문구는 `send_customer_message(kind="decline")`로 따로 보내며 그쪽이 승인을 받습니다.
- consumes → produces: `claim_id` → `claim_id`
- 입력: `claim_id`(✓), `reason`(✓, `outside_return_window` \| `not_returnable` \| `inspection_failed`), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_c52b", "changes": [{"op": "update", "entity": "claim", "id": "ret_20290", "from": "waiting", "to": "declined"}], "checks": [{"rule": "return_window_days", "passed": true, "detail": "Delivered Sep 28, 39 days ago (limit 30)", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "ret_20290", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 프리뷰: 반려 사유가 정책과 맞는지 check. 같은 요청의 거절 메시지와 한 변경셋으로 묶이면 그 변경셋 승인을 따릅니다.
- 정책: `return_window_days`, `return.non_returnable_when`
- 에러: `invalid_state` → `get_claim` · `reason_required` → `reject_claim`
- 멱등성: `Idempotency-Key = hash(claim_id + reason)`

### `convert_exchange_to_return` — Kind: W
- 논리주소/REST: `fulfillment.claim.convert_exchange_to_return` · `POST /v1/fulfillment/claims/{claim_id}/convert-to-return`
- 요약: 교환을 반품으로 전환(대체 상품 품절, 쇼퍼 거절). 재배송비는 환불 대상에 포함됩니다.
- consumes → produces: `claim_id` → `claim_id`(return)
- 입력: `claim_id`(✓), `reason`(✓, `replacement_out_of_stock` \| `customer_declines`), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_c60d", "changes": [{"op": "update", "entity": "claim", "id": "clm_3301", "from": "exchange", "to": "return"}], "checks": [{"rule": "exchange_fallback", "passed": true, "detail": "Replacement size M is out of stock", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "clm_3301", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: `exchange.fallback_to_return_when`, `exchange.on_fallback`
- 에러: `not_an_exchange` → `get_claim` · `invalid_state` → `get_claim`
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
- 입력: `claim_id`(✓), `condition`(✓, `normal` \| `damaged` \| `missing_parts`), `acom:liability`(`seller_fault` \| `customer_fault`), `next`(`approve` \| `hold` \| `decline`), `note`, `preview`(✓)
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

쇼퍼 본인이 UI 버튼으로만 부르는 HTTP 엔드포인트입니다. **툴이 아니며 모델은 보지 못하므로**, 어떤 프롬프트로도 에이전트가 반품·취소·교환 요청을 대신 보낼 수 없습니다. 쇼퍼 에이전트는 `get_return_options`로 선택지만 보여줍니다([plan.md §3.6](../plan.md), [README.md §2](README.md)).

| 엔드포인트 | 누가 | 무엇 |
|---|---|---|
| `POST /returns` `{order_id, line_id, option}` | 쇼퍼 | 반품 요청 보내기. `option`은 `get_return_options`의 `"replacement"` 또는 `"refund"`. 백오피스에 `waiting` 반품 요청(`ret_…`)이 생김 |
| `POST /orders/{id}/cancel` `{line_ids?, reason}` | 쇼퍼 | 출고 전 주문 취소 요청. 결제 대기 주문은 바로 취소, 결제된 주문은 `requested` 클레임이 되어 `approve_claim` 대기. 출고 후면 `already_shipped`로 거부하고 반품 안내 |
| `POST /exchanges` `{order_id, line_id, replacement_product_id, reason}` | 쇼퍼 | 교환 요청. 대체 상품은 같은 가격·판매 중·재고 있음이어야 함. `requested` 클레임이 되어 `approve_claim` 대기 |

- 공통: 쇼퍼 세션의 본인 주문만, `Idempotency-Key` 필수(같은 키 재전송 시 첫 결과). 응답 `{"ok": true, "id": "ret_20402"}`, 에러는 공통 형식(`outside_return_window`, `already_shipped`, `not_your_order`, `replacement_unavailable`).
- schema.org: 쇼퍼의 `ReturnAction`, `CancelAction`(취소), 교환은 `ReturnAction` + 재배송 `ParcelDelivery`.
