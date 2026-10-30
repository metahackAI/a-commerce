# API 명세 — `integration` (외부 채널 연계·대량 가져오기·이벤트 수신)

> 계층: `integration.{resource}.{operation}`. 공통 규약(응답 형식·에러·프리뷰/커밋·멱등성·헤더)은 [README.md](README.md) §3, 데이터 타입은 [data-model.md](data-model.md).
> **범위**: 해커톤 MVP 범위 밖입니다. 데모는 외부 채널·택배사·결제사와 연결하지 않습니다.
> **설계 축**: ① 이 도메인은 어댑터입니다. 코어 도메인(`catalog`, `sales`, `fulfillment`)을 외부 시스템과 잇기만 하고, 비즈니스 규칙은 코어가 가집니다. ② 판매자에 속한 연계는 `seller_id`(`partner`)로 범위를 정합니다. ③ 대량 처리와 상태 동기화는 멱등이며, 부분 성공을 건별로 보고합니다. ④ 인바운드 이벤트는 서명을 검증하고, 같은 이벤트가 다시 와도 한 번만 반영합니다. ⑤ 쓰기는 모두 프리뷰 → 커밋이고, 가격·노출·대량 변경과 외부 자격 등록은 W$입니다.

리소스: [product_import](#product_import) · [order](#order) · [claim](#claim) · [channel](#channel) · [event](#event)

---

## product_import

### `import_products` — Kind: W$
- 논리 주소 · REST: `integration.product_import.import_products` · `POST /v1/integration/product-imports`
- 요약: 외부 피드(판매자·공급사)의 상품·변형·가격·재고·상태를 한 배치로 가져옵니다. 내부적으로 `create_product`, `update_product`, `set_variants`, `set_price`, `set_variant_stock`, `discontinue_product`를 하나의 배치로 실행합니다. 가격과 노출이 대량으로 바뀌므로 항상 승인. 프리뷰가 가격 하한, 카테고리 매핑, 필수 정보를 검사합니다.
- consumes → produces: → `import_id`, `product_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `seller_id` | Text | | 판매자 피드일 때 |
  | `acom:import_mode` | enum | ✓ | `upsert` \| `price_only` \| `stock_only` \| `status_only` |
  | `items` | Product[] | ✓ | 외부 상품(`acom:external_id`, 변형, 가격, 재고 포함) |
  | `acom:category_mapping` | object | | 외부 → 내부 카테고리 매핑. 없으면 서버가 매핑 |
  | `preview` | Boolean | ✓ | `true` 먼저 |
- 출력(프리뷰):
  ```jsonc
  {
    "preview_id": "pv_imp_31",
    "changes": [{"op": "create", "entity": "product", "id": "ext_SKU-1182", "from": null, "to": {"name": "Merino trail tee", "price": 58.00}}],
    "checks": [
      {"rule": "price_floor", "passed": true, "detail": "30 of 30 at or above cost + 15%", "fix_tool": null},
      {"rule": "category_mapped", "passed": false, "detail": "2 rows have no matching category", "fix_tool": "map_category"},
      {"rule": "required_fields", "passed": true, "detail": "All rows have name, price and size chart", "fix_tool": null}
    ],
    "needs_approval": true,
    "approval_reason": "Creates 28 products and changes 2 prices"
  }
  ```
- 출력(커밋): `{"ok": true, "id": "imp_1107", "undo": null}` — 건별 결과(수락·거부와 사유)는 `get_import_status`
- 정책 키: `integration.import.max_items_per_batch`, `integration.import.partial_success`, [catalog.yml](policies/catalog.yml) `price_floor`
- 에러: `seller_not_active` → `set_seller_status` · `category_unmapped` → `map_category` · `below_price_floor` → `set_price` · `batch_too_large` → `import_products`(나눠서 전송)
- 멱등성: `Idempotency-Key`. 같은 피드 내용을 다시 보내면 첫 `import_id`의 결과를 반환하고 상품을 중복 생성하지 않습니다.

### `get_import_status` — Kind: R
- 논리 주소 · REST: `integration.product_import.get_import_status` · `GET /v1/integration/product-imports/{import_id}`
- 요약: 가져오기 배치의 진행 상태와 건별 결과·오류. 재전송 판단의 근거.
- consumes → produces: `import_id` → `product_id[]`
- 입력: `import_id`(✓), `fields?[]`
- 출력: `{"items": [{"acom:external_id", "product_id", "acom:item_status": "accepted" | "rejected", "error": {"code", "message", "next_tool"} | null}], "total", "action_status": "potential" | "active" | "completed" | "failed", "accepted": 28, "rejected": 2, "end_time"}`
- 에러: `import_not_found` → `import_products`

---

## order

### `lookup_external_order` — Kind: R
- 논리 주소 · REST: `integration.order.lookup_external_order` · `POST /v1/integration/orders/lookup`
- 요약: 외부 채널(마켓플레이스 등)의 주문과 출고 가능 여부를 읽습니다. 내부 주문은 `sales`가 다루고, 이 툴은 외부 기록을 비춰 볼 뿐입니다.
- consumes → produces: `channel_id` → `external_order_ref[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 |
  |---|---|:--:|---|
  | `channel_id` | Text | ✓ | 외부 채널 |
  | `external_order_ref` | Text | | 한 건 조회 |
  | `order_date_from` / `order_date_to` | DateTime | | 기간 |
  | `acom:external_status` | Text | | 외부 상태 |
  | `limit` / `offset` | Integer | | 기본 20 / 0 |
- 출력: `{"items": [{"external_order_ref", "channel_id", "ordered_item[]", "acom:external_status", "acom:shippable", "order_id"(매핑된 내부 주문, 없으면 null)}], "total"}`
- 에러: `channel_not_registered` → `register_channel` · `external_lookup_failed` → `list_channels`(연결 상태 확인)

### `sync_order_status` — Kind: W
- 논리 주소 · REST: `integration.order.sync_order_status` · `POST /v1/integration/orders/sync-status`
- 요약: 외부 주문 상태를 내부로 가져오거나(inbound) 내부 상태를 외부로 보냅니다(outbound). 채널별 상태 매핑으로 schema.org `OrderStatus` 단어(`processing`, `in_transit`, `delivered` …)로 맞춘 뒤 `sales`·`fulfillment`의 상태 전이 규칙을 그대로 따릅니다.
- consumes → produces: `channel_id`, `external_order_ref` → `order_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 |
  |---|---|:--:|---|
  | `channel_id` | Text | ✓ | |
  | `external_order_ref` | Text | ✓ | |
  | `acom:external_status` | Text | ✓ | 외부 상태 값(서버가 의미 있는 단어로 매핑) |
  | `acom:direction` | enum | ✓ | `inbound` \| `outbound` |
  | `preview` | Boolean | ✓ | |
- 출력(프리뷰): `changes: [{"op": "update", "entity": "order", "id": "ord_20455", "from": "processing", "to": "in_transit"}]`, `checks: [{"rule": "status_mapped", ...}, {"rule": "valid_transition", ...}]`, `needs_approval: false`
- 출력(커밋): `{"ok": true, "id": "ord_20455", "undo": null}`
- 정책 키: `integration.status_mapping`, [order.yml](policies/order.yml) `order_status`
- 에러: `status_unmapped` → `register_channel`(매핑 보강) · `invalid_transition` → `lookup_external_order` · `order_not_mapped` → `lookup_external_order`
- 멱등성: 같은 채널·주문·외부 상태의 재수신은 변경 없이 성공.

---

## claim

### `lookup_external_claim` — Kind: R
- 논리 주소 · REST: `integration.claim.lookup_external_claim` · `POST /v1/integration/claims/lookup`
- 요약: 외부 채널의 취소·반품·교환 요청을 읽습니다. 내부 클레임은 `fulfillment`가 다룹니다.
- consumes → produces: `channel_id` → `external_claim_ref[]`
- 입력: `channel_id`(✓), `external_claim_ref?`, `acom:claim_type?`(`cancel` \| `return` \| `exchange`), `date_from?`, `date_to?`, `limit`, `offset`
- 출력: `{"items": [{"external_claim_ref", "channel_id", "acom:claim_type", "acom:external_status", "ordered_item[]", "claim_id"(매핑된 내부 클레임, 없으면 null)}], "total"}`
- 에러: `channel_not_registered` → `register_channel` · `external_lookup_failed` → `list_channels`

### `sync_claim_status` — Kind: W
- 논리 주소 · REST: `integration.claim.sync_claim_status` · `POST /v1/integration/claims/sync-status`
- 요약: 외부 클레임 상태를 내부 `fulfillment` 클레임(schema.org `ReturnAction` 등)의 `action_status`로 맞추거나 반대로 보냅니다(접수·승인·반려·완료).
- consumes → produces: `channel_id`, `external_claim_ref` → `claim_id`
- 입력: `channel_id`(✓), `external_claim_ref`(✓), `acom:claim_type`(✓, `cancel` \| `return` \| `exchange`), `acom:external_status`(✓), `acom:direction`(✓, `inbound` \| `outbound`), `preview`(✓)
- 출력(프리뷰): `changes: [{"op": "update", "entity": "claim", ...}]`, `checks: [{"rule": "status_mapped", ...}, {"rule": "valid_transition", ...}]`, `needs_approval: false`
- 출력(커밋): `{"ok": true, "id": "clm_7731", "undo": null}`
- 정책 키: `integration.status_mapping`, [fulfillment.yml](policies/fulfillment.yml)
- 에러: `status_unmapped` → `register_channel` · `invalid_transition` → `lookup_external_claim` · `claim_not_mapped` → `lookup_external_claim`
- 멱등성: 같은 채널·클레임·외부 상태의 재수신은 변경 없이 성공. 환불은 이 툴이 하지 않고 `refund_return`(W$)을 거칩니다.

---

## channel

### `list_channels` — Kind: R
- 논리 주소 · REST: `integration.channel.list_channels` · `GET /v1/integration/channels`
- 요약: 등록된 외부 채널(마켓플레이스, 택배사, 결제사, 물류)과 연결 상태.
- consumes → produces: → `channel_id[]`
- 입력: `acom:channel_type?`(`marketplace` \| `carrier` \| `payment` \| `fulfillment`), `acom:connection_status?`(`connected` \| `degraded` \| `disconnected`), `limit`, `offset`, `fields?[]`
- 출력: `{"items": [{"identifier", "name", "acom:channel_type", "acom:connection_status", "acom:last_synced_at", "seller_id"}], "total"}`
- 에러: `invalid_filter` → `list_channels`

### `register_channel` — Kind: W$
- 논리 주소 · REST: `integration.channel.register_channel` · `POST /v1/integration/channels`
- 요약: 외부 채널 연결 등록과 재구성(엔드포인트, 자격 증명, 상태 매핑, 이벤트 서명 키). 자격 증명을 보관하고 외부와 통신하므로 항상 승인. 프리뷰가 연결 테스트와 매핑 완전성을 검사합니다.
- consumes → produces: → `channel_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `name` | Text | ✓ | 표시 이름 |
  | `acom:channel_type` | enum | ✓ | `marketplace` \| `carrier` \| `payment` \| `fulfillment` |
  | `url` | URL | ✓ | 외부 API 주소 |
  | `acom:credential_ref` | Text | ✓ | 비밀 저장소 참조(평문은 받지도 돌려주지도 않음) |
  | `acom:status_mapping` | object | ✓ | 외부 상태 ↔ 내부 상태 단어 |
  | `acom:signing_secret_ref` | Text | | 인바운드 이벤트 서명 검증 키 참조 |
  | `seller_id` | Text | | 판매자에 속한 채널일 때 |
  | `preview` | Boolean | ✓ | |
- 출력(프리뷰): `changes: [{"op": "create", "entity": "channel", ...}]`, `checks: [{"rule": "connection_test", ...}, {"rule": "status_mapping_complete", ...}]`, `needs_approval: true`, `approval_reason: "Stores credentials for an outside system"`
- 출력(커밋): `{"ok": true, "id": "chn_carrier_ups", "undo": null}`
- 정책 키: `integration.channel.credential_vaulting`, `integration.channel.connection_test_required`
- 에러: `connection_failed` → `register_channel`(주소·자격 확인) · `mapping_incomplete` → `register_channel` · `duplicate_channel` → `list_channels`
- 멱등성: `Idempotency-Key`. 같은 이름과 주소의 재시도는 첫 결과 반환.

---

## event

외부 시스템이 호출하는 인바운드 이벤트입니다. 에이전트가 고르는 툴이 아니며, 시스템이 받아 기록하고 코어 도메인의 상태 전이 규칙을 통과한 것만 반영합니다. 프리뷰 단계가 없습니다.

### `receive_carrier_event` — Kind: W
- 논리 주소 · REST: `integration.event.receive_carrier_event` · `POST /v1/integration/events/carrier`
- 요약: 택배사의 운송 이벤트(송장 등록, 집화, 배송 중, 배송 완료, 배송 실패)를 받습니다. 서명을 검증한 뒤 `fulfillment`의 `ParcelDelivery`(송장·추적·배송 완료)에 반영합니다.
- consumes → produces: → `delivery_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 |
  |---|---|:--:|---|
  | `channel_id` | Text | ✓ | 택배사 채널 |
  | `acom:signature` | Text | ✓ | 본문 서명 |
  | `acom:event_type` | enum | ✓ | `label_created` \| `picked_up` \| `in_transit` \| `delivered` \| `delivery_failed` |
  | `tracking_number` | Text | ✓ | 송장 번호 |
  | `part_of_order` | Text | ✓ | 연결된 주문 |
  | `start_time` | DateTime | ✓ | 외부에서 일어난 시각 |
- 출력: `{"ok": true, "id": "evt_c_90211", "undo": null}` — `id`는 중복 제거 키. 반영 대상이 아직 없으면 저장만 하고 나중에 반영
- 정책 키: `integration.events.signature_required`, `integration.events.replay_window`
- 에러: `invalid_signature`(401) → `list_channels` · `unknown_channel` → `register_channel` · `order_not_mapped` → `lookup_external_order`
- 멱등성: 채널·이벤트 유형·송장 번호·발생 시각으로 중복 제거. 재전송은 저장만 하고 다시 반영하지 않습니다.

### `receive_payment_event` — Kind: W
- 논리 주소 · REST: `integration.event.receive_payment_event` · `POST /v1/integration/events/payment`
- 요약: 결제사의 이벤트(승인, 입금 확인, 취소, 환불 완료, 환불 실패)를 받습니다. 서명을 검증한 뒤 `sales`의 결제 상태와 `fulfillment`의 환불 상태에 반영합니다. 돈의 상태를 바꾸지만 외부에서 이미 일어난 사실을 기록하는 것이므로 W이고, 내부 확정은 코어 도메인의 규칙이 합니다.
- consumes → produces: → `payment_id` 또는 `refund_id`
- 입력: `channel_id`(✓), `acom:signature`(✓), `acom:event_type`(✓, `authorized` \| `deposit_confirmed` \| `cancelled` \| `refund_completed` \| `refund_failed`), `acom:payment_ref`(✓), `amount`(✓, `{"value": 312.00, "currency": "USD"}`), `part_of_order`(✓), `start_time`(✓)
- 출력: `{"ok": true, "id": "evt_p_55102", "undo": null}`
- 정책 키: `integration.events.signature_required`, `integration.events.replay_window`, [fulfillment.yml](policies/fulfillment.yml) `refund`
- 에러: `invalid_signature`(401) → `list_channels` · `unknown_channel` → `register_channel` · `amount_mismatch` → `get_payment` · `order_not_mapped` → `lookup_external_order`
- 멱등성: 채널·이벤트 유형·결제 참조·발생 시각으로 중복 제거. 같은 이벤트로 두 번 청구하거나 두 번 환불하지 않습니다.
