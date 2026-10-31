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
  | `items` | object[] | ✓ | 외부 상품 행. 각 행 `{external_id, name, category, price{value, currency}, cost{value, currency}, variants[{external_id, size, color, inventory_level, availability}]}`. `external_id` 말고는 방식마다 필요한 것만: `upsert`는 `import.required_fields`(새 상품의 `name`·`category`·`size`, 그리고 `price`), `price_only`는 `price`, `stock_only`는 변형의 `inventory_level`, `status_only`는 변형의 `availability`(schema.org `ItemAvailability` 단어). 한 행은 한 색 |
  | `acom:category_mapping` | object[] | | 외부 카테고리 단어 → 표준 카테고리 `[{external_category, category_id}]`(닫힌 스키마라 맵 대신 짝의 목록). 없는 단어는 같은 피드의 이전 가져오기가 저장한 매핑, 그다음 표준 카테고리 이름으로 맞춤 |
- 출력(프리뷰):
  ```jsonc
  {
    "preview_id": "pv_imp31",
    "expires_at": "2026-11-06T11:28:00-05:00",
    "changes": [{"op": "create", "entity": "product", "id": null, "from": null,
                 "to": {"external_id": "SKU-1182", "name": "Merino trail tee", "price": {"value": 58.00, "currency": "USD"}}}],
    "checks": [
      {"rule": "price_floor", "passed": true, "detail": "30 of 30 at or above cost + 15%", "fix_tool": null},
      {"rule": "import.category_mapping", "passed": false, "detail": "2 rows have no matching category", "fix_tool": "import_products"},
      {"rule": "import.required_fields", "passed": true, "detail": "All rows have name, price, category and size", "fix_tool": null}
    ],
    "needs_approval": true,
    "approval_reason": "Creates 28 products and changes 2 prices",
    "change_set_id": "cs_1044"
  }
  ```
- 출력(실행): `{"ok": true, "id": "imp_1107", "undo": {"by": "person", "target": "cs_1044", "until": "2026-11-07T23:59:00-05:00"}}` — 건별 결과(수락·거부와 사유)는 `get_import_status`
- 처리: 새 상품은 카탈로그의 초안 경로(`batch_import`의 공급사 시트 규칙: 카테고리 맞춤, SKU 유일, 초안 이름, 미해결 필드)로 초안이 되고(쇼퍼에게 보이지 않음, 게시는 `approve_product`), 판매가는 피드의 가격을 쓰되 가격 하한을 검사합니다. 이미 있는 상품(같은 피드의 이전 가져오기가 받은 코드, 또는 카탈로그의 변형 SKU)은 카탈로그의 가격 하한·재고·판매 상태 규칙으로 바뀝니다. 거부된 행은 프리뷰 `changes`에 `exclude`로 사유와 함께 나옵니다. 되돌리기는 가격·재고·판매 상태를 되돌리고 초안을 지우며, 이미 제출·게시된 초안이나 단종(`discontinued`)은 되돌리지 않습니다
- 정책: [integration.yml](../../policies/integration.yml) `import.max_items_per_batch`, `import.partial_success`, `import.required_fields`, `import.category_mapping`, [catalog.yml](../../policies/catalog.yml) `price_floor`, [approval.yml](../../policies/approval.yml) `thresholds.bulk_catalog_change`
- 에러: `seller_not_active` → `set_seller_status` · `category_unmapped` → `list_categories`(찾은 `cat_…`을 `acom:category_mapping`에 넣어 다시 가져옴. `map_category`는 표준 → 전시 매핑이라 외부 단어를 맞추지 못함) · `below_price_floor` → `set_price` · `batch_too_large`(행을 나눠 다시 보냄) → `import_products`. 행별 거부는 같은 코드와 `missing_required_field`·`duplicate_sku`로 `get_import_status`의 `rows[].error`에 남습니다. 모든 행이 거부되거나 `import.partial_success: all_or_nothing`에서 한 행이라도 거부되면 첫 행의 에러로 실패합니다
- 판매자 상태: `partner`와 `integration`은 서로 의존하지 않으므로, `partner`가 읽기 함수를 내놓으면 레지스트리가 `integration::service::for_partner::provide_seller_status`로 넣습니다. 넣기 전에는 판매자 상태를 확인하지 않고, 프리뷰가 그렇다고 말합니다
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 같은 `external_id`의 행은 상품을 중복 생성하지 않고 갱신합니다.

### `get_import_status` — Kind: R
- 논리 주소 · REST: `integration.product_import.get_import_status` · `GET /v1/integration/product-imports/{import_id}`
- 요약: 가져오기 배치의 진행 상태와 건별 결과·오류. 재전송 판단의 근거.
- consumes → produces: `import_id` → `product_id[]`
- 입력: `import_id`(✓), `fields?[]`
- 출력: `{"id": "imp_1107", "action_status": "completed", "accepted": 28, "rejected": 2, "end_time": "2026-11-06T11:40:00-05:00", "rows": [{"position": 1, "external_id": "SKU-1182", "product_id": "prd_merino_tee", "acom:item_status": "accepted", "error": null}, {"position": 2, "external_id": "SKU-1190", "product_id": null, "acom:item_status": "rejected", "error": {"code": "category_unmapped", "message": "No category matches 'Trail tops'.", "next_tool": "list_categories"}}]}` (그 밖에 `seller_id`, `acom:import_mode`, `start_time`) — `action_status`: `potential` \| `active` \| `completed` \| `failed`, `acom:item_status`: `accepted` \| `rejected`
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
- 출력: `{"items": [{"external_order_ref": "MK-55102", "channel_id": "chn_marketplace_a", "ordered_item": [{"external_id": "SKU-1182", "order_quantity": 1}], "acom:external_status": "paid_waiting_shipment", "acom:order_status": "processing", "acom:shippable": true, "order_id": "ord_20455"}], "total": 1, "acom:source": {"channel_id": "chn_marketplace_a", "connected": false, "note": "…"}}` — `order_id`는 매핑된 내부 주문, 없으면 `null`. `acom:order_status`는 채널 매핑으로 읽은 우리 단어, `acom:shippable`은 그 단어가 `processing`일 때. 이 스토어는 외부 채널과 연결하지 않으므로 채널이 마지막으로 보낸 사본(`integration.external_order`)을 읽고 `acom:source`에 그렇게 적습니다. 마켓플레이스·외부 물류 채널만 받습니다(`invalid_input` → `list_channels`), 연결이 끊긴(`disconnected`) 채널은 `external_lookup_failed`
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
- 에러: `status_unmapped`(채널의 매핑을 보강) → `register_channel` · `invalid_transition` → `lookup_external_order` · `order_not_mapped` → `lookup_external_order` · `channel_not_registered` → `register_channel`
- 처리: inbound는 `sales`가 허용하는 이동만 합니다(결제·배송·완료 방향으로, 어느 열린 상태에서든 `problem`으로, `problem`에서 다시; `cancelled`·`returned`는 끝). outbound는 채널에 알릴 단어가 지금 주문 상태와 같아야 하며, 연결된 채널이 없으므로 보내지 않고 우리 사본에 기록한 뒤 실행 결과의 `acom:outside_system`에 그렇게 적습니다
- 멱등성: 같은 채널·주문·외부 상태의 재수신은 변경 없이 성공.

---

## claim

### `lookup_external_claim` — Kind: R
- 논리 주소 · REST: `integration.claim.lookup_external_claim` · `POST /v1/integration/claims/lookup` (부작용 없는 POST 읽기)
- 요약: 외부 채널의 취소·반품·교환 요청을 읽습니다. 내부 클레임은 `fulfillment`가 다룹니다.
- consumes → produces: `channel_id` → `external_claim_ref[]`
- 입력: `channel_id`(✓), `external_claim_ref?`, `claim_type?`(`cancel` \| `return` \| `exchange`), `requested_from?` / `requested_through?`(DateTime), `limit`(기본 20), `cursor?`
- 출력: `{"items": [{"external_claim_ref": "MK-R-2210", "external_order_ref": "MK-54980", "channel_id": "chn_marketplace_a", "claim_type": "return", "acom:external_status": "return_requested", "acom:claim_status": "requested", "ordered_item": [{"external_id": "SKU-1182", "order_quantity": 1}], "claim_id": null}], "total": 1, "acom:source": {...}}` — `claim_id`는 매핑된 내부 클레임, 없으면 `null`. 읽는 곳과 `acom:source`는 `lookup_external_order`와 같습니다
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
- 에러: `status_unmapped` → `register_channel` · `invalid_transition` → `lookup_external_claim` · `claim_not_mapped` → `lookup_external_claim` · `channel_not_registered` → `register_channel`
- 처리: inbound는 `fulfillment`의 클레임 생애(접수 → 사진 요청 → 승인·반려 → 회수 지시 → 회수 → 검수 → 완료, 교환은 재발송 요청 → 완료, 회수 전 철회)만 따르고, 반품은 환불이 나간 뒤에만 `completed`가 됩니다. `claim_type`은 매핑된 클레임의 종류와 같아야 합니다(`invalid_input`). outbound는 `sync_order_status`와 같습니다
- 멱등성: 같은 채널·클레임·외부 상태의 재수신은 변경 없이 성공. 환불은 이 툴이 하지 않고 `refund_return`을 거칩니다.

---

## channel

### `list_channels` — Kind: R
- 논리 주소 · REST: `integration.channel.list_channels` · `GET /v1/integration/channels`
- 요약: 등록된 외부 채널(마켓플레이스, 택배사, 결제사, 물류)과 연결 상태.
- consumes → produces: → `channel_id[]`
- 입력: `acom:channel_type?`(`marketplace` \| `carrier` \| `payment` \| `fulfillment`), `acom:connection_status?`(`connected` \| `degraded` \| `disconnected`), `limit`(기본 20), `cursor?`, `fields?[]`
- 출력: `{"items": [{"id": "chn_carrier_ups", "name": "UPS", "acom:channel_type": "carrier", "url": "https://api.ups.example/v1", "acom:connection_status": "connected", "acom:last_synced_at": "2026-11-06T10:55:00-05:00", "seller_id": null, "acom:status_mapping": {}, "acom:signs_events": true}], "total": 3}` — 자격 증명과 서명 키는 돌려주지 않고, 서명 키가 있는지만(`acom:signs_events`) 알려 줍니다
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
  | `acom:credential_ref` | Text | ✓ | 비밀 저장소 참조(평문은 받지도 돌려주지도 않음). 스키마에 `writeOnly`라 프리뷰 저장소에 남지 않고, SHA-256 해시만 보관 |
  | `acom:status_mapping` | object | ✓ | 외부 상태 → 내부 상태 단어. 닫힌 스키마라 `{"order": [{"external_status": "shipped", "order_status": "in_transit"}], "claim": [{"external_status": "return_requested", "claim_status": "requested"}]}`처럼 짝의 목록(우리 단어는 enum). 저장은 `{"order": {"shipped": "in_transit"}, "claim": {...}}` |
  | `acom:signing_secret_ref` | Text | | 인바운드 이벤트 서명 비밀(`writeOnly`, 해시만 보관). 없으면 실행 때 서버가 만들어 그 응답에서 한 번만 보여 줌(`shown_once`, 변경셋이면 승인 응답) |
  | `seller_id` | Text | | 판매자에 속한 채널일 때 |
- 출력(프리뷰): `changes: [{"op": "create", "entity": "channel", "id": null, ...}]`, `checks: [{"rule": "channel.connection_test_required", "passed": true, "detail": "No outside system is connected in this store: api.ups.example and the credential reference were checked for form only, no call was made", "fix_tool": null}, {"rule": "status_mapping.must_cover", "passed": true, "detail": "Order and claim states mapped (order: 6 words, claim: 10 words)", "fix_tool": null}]`, `needs_approval: true`, `approval_reason: "Stores credentials for an outside system"` — 연결된 외부 시스템이 없으므로 연결 테스트는 주소(https와 호스트)와 자격 증명 참조의 형식만 보고 호출하지 않습니다. `status_mapping.must_cover`는 주문·클레임을 가진 채널(마켓플레이스·외부 물류)에 적용되고, 택배사·결제사는 고정된 이벤트 단어를 보내므로 매핑이 필요 없습니다. 채널 id는 `chn_<종류>_<이름>`
- 출력(실행): `{"ok": true, "id": "chn_carrier_ups", "undo": {"by": "person", "target": "cs_1044", "until": "2026-11-07T23:59:00-05:00"}}` — 서명 비밀을 서버가 만들었으면 `shown_once.signing_secret`(한 번만). 되돌리기는 채널을 지우며, 그 채널의 주문·클레임 사본이나 이벤트가 이미 있으면 거부합니다
- 재구성: 입력에 채널 id가 없으므로 이 툴은 새 채널만 만듭니다(같은 이름·주소면 `duplicate_channel`)
- 정책: [integration.yml](../../policies/integration.yml) `channel.credential_vaulting`, `channel.connection_test_required`, `status_mapping.must_cover`
- 에러: `connection_failed`(주소·자격 확인) → `register_channel` · `mapping_incomplete` → `register_channel` · `duplicate_channel` → `list_channels`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 같은 이름과 주소로 다시 등록하면 `duplicate_channel`.

---

## event

외부 시스템이 호출하는 인바운드 이벤트입니다. 에이전트가 고르는 툴이 아니며(모델의 툴 매니페스트에서 빠짐, `audience: internal`, 시스템 주체 전용), 시스템이 받아 기록하고 코어 도메인의 상태 전이 규칙을 통과한 것만 반영합니다. 사람이 볼 프리뷰 단계가 없습니다: 모든 쓰기처럼 프리뷰 → 실행이지만, 시스템 주체가 받은 즉시 프리뷰하고(`needs_approval: false`) 바로 실행합니다.

- **서명.** `acom:signature`는 각 툴이 정한 필드를 줄바꿈(`\n`)으로 이은 문자열의 HMAC-SHA256(소문자 16진수, 앞의 `sha256=`는 있어도 됨)이고, 키는 채널의 서명 비밀의 SHA-256(16진수)입니다. 스토어는 이 해시만 가지며 비밀 자체는 저장하지 않습니다(`events.signature_required`).
- **한 번만.** 채널·이벤트 유형·송장 번호 또는 결제 참조·발생 시각으로 같은 이벤트를 알아보고, 재전송에는 첫 기록의 `id`를 돌려줄 뿐 아무것도 다시 기록하거나 반영하지 않습니다.
- **너무 오래된 것.** 받았을 때 발생 시각이 `events.replay_window.minutes`보다 오래됐으면 기록만 하고 반영하지 않습니다(`failed`, 사유 `outside_replay_window`).
- **아직 대상이 없는 것.** 송장의 배송, 결제, 환불이 아직 기록되지 않았으면 `potential`로 기다리고, 작업 `integration.apply_event`가 `events.pending.retry_every_minutes`마다 다시 시도하다가 `events.pending.give_up_after_minutes`가 지나면 `failed`로 남깁니다.
- **주문.** `external_order_ref`는 채널이 매핑한 마켓플레이스 주문 참조, 우리 주문 id, 또는 주문 번호(`20402`, `#20402`)입니다. 어느 것도 아니면 `order_not_mapped`.
- 실행 결과: `{"ok": true, "id": "whk_…", "undo": null, "action_status": "completed" | "potential" | "failed", "delivery_id" | "payment_id" | "refund_id": …, "error": {code, message}?}`. 재전송이면 `acom:duplicate: true`.

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
- 프리뷰: 받는 즉시 시스템 주체가 만들고 실행(외부 사실의 기록). `changes`는 기록할 이벤트와 반영할 배송 상태 이동, `checks`는 `events.signature_required`, `events.replay_window`
- 서명 필드: `channel_id`, `acom:event_type`, `tracking_number`, `external_order_ref`, `start_time`(본문에 쓴 그대로)
- 처리: 송장 번호로 찾은 배송을 `fulfillment`의 운송 규칙으로만 옮깁니다(앞으로만, 배송 완료·취소 뒤에는 없음). 집화·운송 중이면 품목을 출고로, 모든 품목이 나가면 주문을 `in_transit`로, 배송 완료면 품목과(남은 품목이 모두 배송되면) 주문을 `delivered`로
- 정책: [integration.yml](../../policies/integration.yml) `events.signature_required`, `events.replay_window`, `events.pending`
- 에러: `invalid_signature`(class `unauthenticated`) → `null` · `unknown_channel` → `null` · `order_not_mapped` → `null` — 호출자가 외부 시스템이라 다음 툴이 없습니다.
- 멱등성: 채널·이벤트 유형·송장 번호·발생 시각으로 중복 제거. 재전송은 저장만 하고 다시 반영하지 않습니다.

### `receive_payment_event` — Kind: W
- 논리 주소 · REST: `integration.event.receive_payment_event` · `POST /v1/integration/events/payment`
- 요약: 결제사의 이벤트(승인, 입금 확인, 취소, 환불 완료, 환불 실패)를 받습니다. 서명을 검증한 뒤 `sales`의 결제 상태(enums.md §1.4)와 `fulfillment`의 환불 상태(§1.6)에 반영합니다. 돈의 상태를 바꾸지만 외부에서 이미 일어난 사실을 기록하는 것이므로 W이고, 내부 확정은 코어 도메인의 규칙이 합니다.
- consumes → produces: → `payment_id` 또는 `refund_id`
- 입력: `channel_id`(✓), `acom:signature`(✓), `acom:event_type`(✓, `authorized` \| `deposit_confirmed` \| `cancelled` \| `refund_completed` \| `refund_failed`), `acom:payment_ref`(✓), `amount`(✓, `{"value": 312.00, "currency": "USD"}`), `external_order_ref`(✓), `start_time`(✓)
- 출력: `{"ok": true, "id": "whk_p_55102", "undo": null}`
- 프리뷰: 받는 즉시 시스템 주체가 만들고 실행(외부 사실의 기록). `changes`는 기록할 이벤트와 반영할 결제·환불 상태 이동
- 서명 필드: `channel_id`, `acom:event_type`, `acom:payment_ref`, `<amount.value> <amount.currency>`(본문에 쓴 그대로), `external_order_ref`, `start_time`
- 처리: 승인·입금 확인은 기다리는 결제를 `payment_complete`로(입금을 기다리던 주문은 `processing`으로, 수령인을 기다리는 선물은 그대로), 취소는 `payment_declined`로(`sales`). 환불 완료·실패는 보낸(`requested`) 환불을 `completed`·`failed`로(`fulfillment`, 실패는 `retry_refund`로 다시). 결제는 참조(`external_id` 또는 우리 id)로, 아직 참조가 없으면 주문의 첫 결제로 찾고, 환불은 참조로, 없으면 그 금액을 기다리는 환불이 하나일 때 그것으로 찾습니다
- 정책: [integration.yml](../../policies/integration.yml) `events.signature_required`, `events.replay_window`, `events.pending`, [fulfillment.yml](../../policies/fulfillment.yml) `refund`
- 에러: `invalid_signature`(class `unauthenticated`) → `null` · `unknown_channel` → `null` · `amount_mismatch` → `null` · `order_not_mapped` → `null`
- 멱등성: 채널·이벤트 유형·결제 참조·발생 시각으로 중복 제거. 같은 이벤트로 두 번 청구하거나 두 번 환불하지 않습니다.
