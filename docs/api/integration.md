# API 명세 — `integration` (외부 채널 연계·대량 가져오기·이벤트 수신)

> 계층: `integration.{resource}.{operation}`. 공통 규약(응답 형식·프리뷰/실행·멱등성·헤더)은 [conventions.md](conventions.md)와 [README.md](README.md) §3, 에러는 [errors.md](errors.md), 값 목록은 [enums.md](enums.md), 데이터 타입은 [data-model.md](data-model.md), 정책은 [integration.yml](../../policies/integration.yml) · [approval.yml](../../policies/approval.yml).
> **범위**: 데모 시나리오([../scenarios.md](../scenarios.md))는 외부 채널·택배사·결제사와 연결하지 않습니다. 이 도메인은 그 연결을 위한 명세입니다.
> **설계 축**: ① 이 도메인은 어댑터입니다. 코어 도메인(`catalog`, `sales`, `fulfillment`)을 외부 시스템과 잇기만 하고, 비즈니스 규칙은 코어가 가집니다. ② 판매자에 속한 연계는 `seller_id`(`partner`)로 범위를 정합니다. ③ 대량 처리와 상태 동기화는 멱등이며, 부분 성공을 건별로 보고합니다. ④ 인바운드 이벤트는 서명을 검증하고, 같은 이벤트가 다시 와도 한 번만 반영합니다. ⑤ 쓰기는 모두 프리뷰 → 실행이고, 가격·노출·대량 변경과 외부 자격 등록은 W$입니다. ⑥ 외부 시스템의 id는 우리 id에 넣지 않고 `external_id`·`external_order_ref`에 둡니다(conventions.md §9).

리소스: [product_import](#product_import) · [order](#order) · [claim](#claim) · [channel](#channel) · [event](#event)

---

## product_import

### `import_products` — Kind: W$
- 논리 주소 · REST: `integration.product_import.import_products` · `POST /v1/integration/product-imports`
- 요약: 외부 피드(판매자·공급사)의 상품·변형·가격·재고·상태를 한 배치로 가져옵니다. 내부적으로 `create_product`, `update_product`, `set_variants`, `set_price`, `set_variant_stock`, `discontinue_product`와 같은 계획을 한 배치로 만듭니다. 가격과 노출이 대량으로 바뀌므로 항상 승인. 프리뷰가 가격 하한, 카테고리 매핑, 필수 정보를 검사합니다.
- consumes → produces: → `import_id`, `product_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `seller_id` | Text | | 판매자 피드일 때. 예 `"slr_harbor_goods"` |
  | `acom:import_mode` | enum | ✓ | `upsert` \| `price_only` \| `stock_only` \| `status_only` |
  | `items` | object[] | ✓ | 외부 상품 행. 각 행 `{external_id, name, category, price{value, currency}, cost{value, currency}, variants[{external_id, size, color, inventory_level}]}` |
  | `acom:category_mapping` | object | | 외부 카테고리 → 내부 `cat_…`. 없으면 서버가 저장된 매핑을 씀 |
- 출력(프리뷰):
  ```jsonc
  {
    "preview_id": "pv_imp31",
    "expires_at": "2026-11-06T11:28:00-05:00",
    "changes": [{"op": "create", "entity": "product", "id": null, "from": null,
                 "to": {"external_id": "SKU-1182", "name": "Merino trail tee", "price": {"value": 58.00, "currency": "USD"}}}],
    "checks": [
      {"rule": "price_floor", "passed": true, "detail": "30 of 30 at or above cost + 15%", "fix_tool": null},
      {"rule": "import.category_mapping", "passed": false, "detail": "2 rows have no matching category", "fix_tool": "map_category"},
      {"rule": "import.required_fields", "passed": true, "detail": "All rows have name, price, category and size", "fix_tool": null}
    ],
    "needs_approval": true,
    "approval_reason": "Creates 28 products and changes 2 prices",
    "change_set_id": "cs_1044"
  }
  ```
- 출력(실행): `{"ok": true, "id": "imp_1107", "undo": {"by": "person", "target": "cs_1044", "until": "2026-11-07T23:59:00-05:00"}}` — 건별 결과(수락·거부와 사유)는 `get_import_status`
- 정책: [integration.yml](../../policies/integration.yml) `import.max_items_per_batch`, `import.partial_success`, `import.required_fields`, `import.category_mapping`, [catalog.yml](../../policies/catalog.yml) `price_floor`, [approval.yml](../../policies/approval.yml) `thresholds.bulk_catalog_change`
- 에러: `seller_not_active` → `set_seller_status` · `category_unmapped` → `map_category` · `below_price_floor` → `set_price` · `batch_too_large`(행을 나눠 다시 보냄) → `import_products`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 같은 `external_id`의 행은 상품을 중복 생성하지 않고 갱신합니다.

### `get_import_status` — Kind: R
- 논리 주소 · REST: `integration.product_import.get_import_status` · `GET /v1/integration/product-imports/{import_id}`
- 요약: 가져오기 배치의 진행 상태와 건별 결과·오류. 재전송 판단의 근거.
- consumes → produces: `import_id` → `product_id[]`
- 입력: `import_id`(✓), `fields?[]`
- 출력: `{"id": "imp_1107", "action_status": "completed", "accepted": 28, "rejected": 2, "end_time": "2026-11-06T11:40:00-05:00", "rows": [{"external_id": "SKU-1182", "product_id": "prd_merino_tee", "acom:item_status": "accepted", "error": null}, {"external_id": "SKU-1190", "product_id": null, "acom:item_status": "rejected", "error": {"code": "category_unmapped", "message": "No category matches 'Trail tops'.", "next_tool": "map_category"}}]}` — `action_status`: `potential` \| `active` \| `completed` \| `failed`, `acom:item_status`: `accepted` \| `rejected`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음(읽기)
- 에러: `import_not_found` → `import_products`
- 멱등성: 읽기(부작용 없음)

---

## order

### `lookup_external_order` — Kind: R
- 논리 주소 · REST: `integration.order.lookup_external_order` · `POST /v1/integration/orders/lookup` (부작용 없는 POST 읽기)
- 요약: 외부 채널(마켓플레이스 등)의 주문과 출고 가능 여부를 읽습니다. 내부 주문은 `sales`가 다루고, 이 툴은 외부 기록을 비춰 볼 뿐입니다.
- consumes → produces: `channel_id` → `external_order_ref[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `channel_id` | Text | ✓ | 외부 채널. 예 `"chn_marketplace_a"` |
  | `external_order_ref` | Text | | 한 건 조회 |
  | `order_date_from` / `order_date_through` | DateTime | | 기간 |
  | `acom:external_status` | Text | | 외부 상태 |
  | `limit` / `cursor` | Integer / Text | | 기본 20 / 이전 응답의 `next_cursor` |
- 출력: `{"items": [{"external_order_ref": "MK-55102", "channel_id": "chn_marketplace_a", "ordered_item": [{"external_id": "SKU-1182", "order_quantity": 1}], "acom:external_status": "paid_waiting_shipment", "acom:shippable": true, "order_id": "ord_20455"}], "total": 1}` — `order_id`는 매핑된 내부 주문, 없으면 `null`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음(읽기)
- 에러: `channel_not_registered` → `register_channel` · `external_lookup_failed`(연결 상태 확인) → `list_channels`
- 멱등성: 읽기(부작용 없음)

### `sync_order_status` — Kind: W
- 논리 주소 · REST: `integration.order.sync_order_status` · `POST /v1/integration/orders/sync-status`
- 요약: 외부 주문 상태를 내부로 가져오거나(inbound) 내부 상태를 외부로 보냅니다(outbound). 채널별 상태 매핑으로 schema.org `OrderStatus` 단어(`processing`, `in_transit`, `delivered` …, enums.md §1.1)로 맞춘 뒤 `sales`·`fulfillment`의 상태 전이 규칙을 그대로 따릅니다.
- consumes → produces: `channel_id`, `external_order_ref` → `order_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `channel_id` | Text | ✓ | `"chn_marketplace_a"` |
  | `external_order_ref` | Text | ✓ | `"MK-55102"` |
  | `acom:external_status` | Text | ✓ | 외부 상태 값(서버가 의미 있는 단어로 매핑) |
  | `acom:direction` | enum | ✓ | `inbound` \| `outbound` |
- 출력(프리뷰): `changes: [{"op": "update", "entity": "order", "id": "ord_20455", "from": "processing", "to": "in_transit"}]`, `checks: [{"rule": "status_mapping.unmapped", "passed": true, ...}, {"rule": "status_mapping.transitions_from_core", "passed": true, ...}]`, `needs_approval: false`
- 출력(실행): `{"ok": true, "id": "ord_20455", "undo": null}` — 외부에서 이미 일어난 사실을 기록하므로 되돌리지 않습니다.
- 정책: [integration.yml](../../policies/integration.yml) `status_mapping.unmapped`, `status_mapping.transitions_from_core`, [order.yml](../../policies/order.yml) `order_status`
- 에러: `status_unmapped`(채널의 매핑을 보강) → `register_channel` · `invalid_transition` → `lookup_external_order` · `order_not_mapped` → `lookup_external_order`
- 멱등성: 같은 채널·주문·외부 상태의 재수신은 변경 없이 성공.

---

## claim

### `lookup_external_claim` — Kind: R
- 논리 주소 · REST: `integration.claim.lookup_external_claim` · `POST /v1/integration/claims/lookup` (부작용 없는 POST 읽기)
- 요약: 외부 채널의 취소·반품·교환 요청을 읽습니다. 내부 클레임은 `fulfillment`가 다룹니다.
- consumes → produces: `channel_id` → `external_claim_ref[]`
- 입력: `channel_id`(✓), `external_claim_ref?`, `claim_type?`(`cancel` \| `return` \| `exchange`), `requested_from?` / `requested_through?`(DateTime), `limit`(기본 20), `cursor?`
- 출력: `{"items": [{"external_claim_ref": "MK-R-2210", "channel_id": "chn_marketplace_a", "claim_type": "return", "acom:external_status": "return_requested", "ordered_item": [{"external_id": "SKU-1182", "order_quantity": 1}], "claim_id": null}], "total": 1}` — `claim_id`는 매핑된 내부 클레임, 없으면 `null`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음(읽기)
- 에러: `channel_not_registered` → `register_channel` · `external_lookup_failed` → `list_channels`
- 멱등성: 읽기(부작용 없음)

### `sync_claim_status` — Kind: W
- 논리 주소 · REST: `integration.claim.sync_claim_status` · `POST /v1/integration/claims/sync-status`
- 요약: 외부 클레임 상태를 내부 `fulfillment` 클레임 상태(`acom:claim_status`, enums.md §1.5)로 맞추거나 반대로 보냅니다(접수·승인·반려·완료).
- consumes → produces: `channel_id`, `external_claim_ref` → `claim_id`
- 입력: `channel_id`(✓), `external_claim_ref`(✓), `claim_type`(✓, `cancel` \| `return` \| `exchange`), `acom:external_status`(✓), `acom:direction`(✓, `inbound` \| `outbound`)
- 출력(프리뷰): `changes: [{"op": "update", "entity": "claim", "id": "clm_7731", "from": "requested", "to": "approved"}]`, `checks: [{"rule": "status_mapping.unmapped", ...}, {"rule": "status_mapping.transitions_from_core", ...}]`, `needs_approval: false`
- 출력(실행): `{"ok": true, "id": "clm_7731", "undo": null}`
- 정책: [integration.yml](../../policies/integration.yml) `status_mapping.unmapped`, `status_mapping.transitions_from_core`, [fulfillment.yml](../../policies/fulfillment.yml) `return.states`
- 에러: `status_unmapped` → `register_channel` · `invalid_transition` → `lookup_external_claim` · `claim_not_mapped` → `lookup_external_claim`
- 멱등성: 같은 채널·클레임·외부 상태의 재수신은 변경 없이 성공. 환불은 이 툴이 하지 않고 `refund_return`을 거칩니다.

---

## channel

### `list_channels` — Kind: R
- 논리 주소 · REST: `integration.channel.list_channels` · `GET /v1/integration/channels`
- 요약: 등록된 외부 채널(마켓플레이스, 택배사, 결제사, 물류)과 연결 상태.
- consumes → produces: → `channel_id[]`
- 입력: `acom:channel_type?`(`marketplace` \| `carrier` \| `payment` \| `fulfillment`), `acom:connection_status?`(`connected` \| `degraded` \| `disconnected`), `limit`(기본 20), `cursor?`, `fields?[]`
- 출력: `{"items": [{"id": "chn_carrier_ups", "name": "UPS", "acom:channel_type": "carrier", "acom:connection_status": "connected", "acom:last_synced_at": "2026-11-06T10:55:00-05:00", "seller_id": null}], "total": 3}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음(읽기)
- 에러: `invalid_filter` → `list_channels`
- 멱등성: 읽기(부작용 없음)

### `register_channel` — Kind: W$
- 논리 주소 · REST: `integration.channel.register_channel` · `POST /v1/integration/channels`
- 요약: 외부 채널 연결 등록과 재구성(엔드포인트, 자격 증명, 상태 매핑, 이벤트 서명 키). 자격 증명을 보관하고 외부와 통신하므로 항상 승인. 프리뷰가 연결 테스트와 매핑 완전성을 검사합니다.
- consumes → produces: → `channel_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `name` | Text | ✓ | 표시 이름. 예 `"UPS"` |
  | `acom:channel_type` | enum | ✓ | `marketplace` \| `carrier` \| `payment` \| `fulfillment` |
  | `url` | URL | ✓ | 외부 API 주소 |
  | `acom:credential_ref` | Text | ✓ | 비밀 저장소 참조(평문은 받지도 돌려주지도 않음) |
  | `acom:status_mapping` | object | ✓ | 외부 상태 → 내부 상태 단어. 예 `{"order": {"shipped": "in_transit"}, "claim": {"return_requested": "requested"}}` |
  | `acom:signing_secret_ref` | Text | | 인바운드 이벤트 서명 검증 키 참조 |
  | `seller_id` | Text | | 판매자에 속한 채널일 때 |
- 출력(프리뷰): `changes: [{"op": "create", "entity": "channel", "id": null, ...}]`, `checks: [{"rule": "channel.connection_test_required", "passed": true, "detail": "Test call answered in 320 ms", "fix_tool": null}, {"rule": "status_mapping.must_cover", "passed": true, "detail": "All order and claim states mapped", "fix_tool": null}]`, `needs_approval: true`, `approval_reason: "Stores credentials for an outside system"`
- 출력(실행): `{"ok": true, "id": "chn_carrier_ups", "undo": {"by": "person", "target": "cs_1044", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: [integration.yml](../../policies/integration.yml) `channel.credential_vaulting`, `channel.connection_test_required`, `status_mapping.must_cover`
- 에러: `connection_failed`(주소·자격 확인) → `register_channel` · `mapping_incomplete` → `register_channel` · `duplicate_channel` → `list_channels`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 같은 이름과 주소로 다시 등록하면 `duplicate_channel`.

---

## event

외부 시스템이 호출하는 인바운드 이벤트입니다. 에이전트가 고르는 툴이 아니며(모델의 툴 매니페스트에서 빠짐, 시스템 주체 전용), 시스템이 받아 기록하고 코어 도메인의 상태 전이 규칙을 통과한 것만 반영합니다. 프리뷰 단계가 없습니다.

### `receive_carrier_event` — Kind: W
- 논리 주소 · REST: `integration.event.receive_carrier_event` · `POST /v1/integration/events/carrier`
- 요약: 택배사의 운송 이벤트(송장 등록, 집화, 배송 중, 배송 완료, 배송 실패)를 받습니다. 서명을 검증한 뒤 `fulfillment`의 `ParcelDelivery`(송장·추적·배송 완료, enums.md §1.3)에 반영합니다.
- consumes → produces: → `delivery_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `channel_id` | Text | ✓ | 택배사 채널. 예 `"chn_carrier_ups"` |
  | `acom:signature` | Text | ✓ | 본문 서명 |
  | `acom:event_type` | enum | ✓ | `label_created` \| `picked_up` \| `in_transit` \| `delivered` \| `delivery_failed` |
  | `tracking_number` | Text | ✓ | 송장 번호 |
  | `external_order_ref` | Text | ✓ | 외부 주문 참조(내부 주문으로 매핑) |
  | `start_time` | DateTime | ✓ | 외부에서 일어난 시각 |
- 출력: `{"ok": true, "id": "whk_c_90211", "undo": null}` — `id`는 받은 웹훅 기록. 반영 대상이 아직 없으면 저장만 하고 나중에 반영
- 프리뷰: 없음(외부 사실의 기록)
- 정책: [integration.yml](../../policies/integration.yml) `events.signature_required`, `events.replay_window`
- 에러: `invalid_signature`(class `unauthenticated`) → `null` · `unknown_channel` → `null` · `order_not_mapped` → `null` — 호출자가 외부 시스템이라 다음 툴이 없습니다.
- 멱등성: 채널·이벤트 유형·송장 번호·발생 시각으로 중복 제거. 재전송은 저장만 하고 다시 반영하지 않습니다.

### `receive_payment_event` — Kind: W
- 논리 주소 · REST: `integration.event.receive_payment_event` · `POST /v1/integration/events/payment`
- 요약: 결제사의 이벤트(승인, 입금 확인, 취소, 환불 완료, 환불 실패)를 받습니다. 서명을 검증한 뒤 `sales`의 결제 상태(enums.md §1.4)와 `fulfillment`의 환불 상태(§1.6)에 반영합니다. 돈의 상태를 바꾸지만 외부에서 이미 일어난 사실을 기록하는 것이므로 W이고, 내부 확정은 코어 도메인의 규칙이 합니다.
- consumes → produces: → `payment_id` 또는 `refund_id`
- 입력: `channel_id`(✓), `acom:signature`(✓), `acom:event_type`(✓, `authorized` \| `deposit_confirmed` \| `cancelled` \| `refund_completed` \| `refund_failed`), `acom:payment_ref`(✓), `amount`(✓, `{"value": 312.00, "currency": "USD"}`), `external_order_ref`(✓), `start_time`(✓)
- 출력: `{"ok": true, "id": "whk_p_55102", "undo": null}`
- 프리뷰: 없음(외부 사실의 기록)
- 정책: [integration.yml](../../policies/integration.yml) `events.signature_required`, `events.replay_window`, [fulfillment.yml](../../policies/fulfillment.yml) `refund`
- 에러: `invalid_signature`(class `unauthenticated`) → `null` · `unknown_channel` → `null` · `amount_mismatch` → `null` · `order_not_mapped` → `null`
- 멱등성: 채널·이벤트 유형·결제 참조·발생 시각으로 중복 제거. 같은 이벤트로 두 번 청구하거나 두 번 환불하지 않습니다.
