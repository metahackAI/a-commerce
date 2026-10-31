# API 명세 — `partner` (입점 판매자·계약·등급)

> 계층: `partner.{resource}.{operation}`. 공통 규약(응답 형식·프리뷰/실행·멱등성·헤더)은 [conventions.md](conventions.md)와 [README.md](README.md) §3, 에러는 [errors.md](errors.md), 값 목록은 [enums.md](enums.md), 데이터 타입은 [data-model.md](data-model.md), 정책은 [partner.yml](../../policies/partner.yml) · [approval.yml](../../policies/approval.yml).
> **범위**: 여러 판매자가 입점하는 구조를 위한 도메인입니다. 데모 스토어에는 입점 판매자 셋이 시드돼 있지만(Fieldstone Boot Co.와 Ridgeline Outdoor Inc.는 `active`, Harbor Goods LLC는 `under_review`, `seed/demo/11_partner.yml`) 데모 장면([../scenarios.md](../scenarios.md) §6)은 이 도메인을 쓰지 않습니다. 바깥에 닿는 일(사업자 등록 조회)은 결정적 스텁이고 결과에 그렇게 적습니다(`onboard_seller`의 `registry_check`).
> **설계 축**: ① 입점 판매자는 schema.org `Organization`(+ `acom:Seller`, id `slr_…`)입니다. 판매자가 파는 상품은 partner의 연결(`partner.seller_product_group`, `schema:offeredBy`, 상품 하나에 판매자 하나)로 정합니다. ② 신청 → 심사 → 승인 → 활성의 상태 전이(enums.md §1.16 판매자). ③ 계약(수수료·정산 조건)은 돈이 움직이므로 W$이고 버전으로 쌓습니다(id는 그대로, `version` 필드가 오름). ④ 등급은 배송 준수율·이행률 실적으로 서버가 계산하고, 사람은 확정과 예외 조정만 합니다. ⑤ `seller_id`는 partner·integration에서 같은 필터 축입니다. 주문은 아직 판매자를 기록하지 않아 `search_orders`의 `seller_id`는 `invalid_filter`입니다.

리소스: [seller](#seller) · [contract](#contract) · [grade](#grade) · [dashboard](#dashboard)

---

## seller

### `search_sellers` — Kind: R
- 논리 주소 · REST: `partner.seller.search_sellers` · `POST /v1/partner/sellers/search` (부작용 없는 POST 읽기)
- 요약: 조건으로 입점 판매자 검색. 결과의 `id`는 상품·주문의 `seller_id` 필터에 그대로 씁니다.
- consumes → produces: → `seller_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `query` | Text | | 상호 또는 사업자 번호 일부. 예 `"Harbor"` |
  | `acom:seller_status` | enum | | `applied` \| `under_review` \| `approved` \| `active` \| `suspended` \| `terminated` |
  | `acom:seller_type` | enum | | `marketplace` \| `consignment` \| `direct_supply` |
  | `acom:grade_level` | enum | | `platinum` \| `gold` \| `silver` \| `bronze` \| `watch` |
  | `sort` | enum | | `newest` \| `name` \| `grade` |
  | `limit` / `cursor` | Integer / Text | | 기본 20, 최대 100 / 이전 응답의 `next_cursor` |
- 출력: `{"items": [{"id": "slr_harbor_goods", "legal_name": "Harbor Goods LLC", "tax_id": "***-**-4410", "acom:seller_status": "active", "acom:seller_type": "marketplace", "acom:grade_level": "gold"}], "total": 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음(읽기). `tax_id`는 기본 가림([partner.yml](../../policies/partner.yml) `privacy`: 스토어 운영자에게는 끝 네 자리만, 판매사 조직에 속한 판매자 본인 직원에게는 그대로). 판매자 직원은 자기 판매자만 봅니다
- 에러: `invalid_filter` → `search_sellers`
- 멱등성: 읽기(부작용 없음)

### `get_seller` — Kind: R
- 논리 주소 · REST: `partner.seller.get_seller` · `GET /v1/partner/sellers/{seller_id}`
- 요약: 판매자 하나의 법인 정보·담당자·계약 요약·등급. `fields`로 응답 축소.
- consumes → produces: `seller_id` → seller
- 입력: `seller_id`(✓, `^slr_[a-z0-9_]+$`), `fields?[]`
- 출력: `Organization {id, legal_name, tax_id, address(PostalAddress), contact_point(ContactPoint[]), acom:seller_status, acom:seller_type, acom:contract_summary{contract_id, version, valid_through}, acom:grade_level}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음(읽기)
- 에러: `seller_not_found` → `search_sellers`
- 멱등성: 읽기(부작용 없음)

### `onboard_seller` — Kind: W$
- 논리 주소 · REST: `partner.seller.onboard_seller` · `POST /v1/partner/sellers`
- 요약: 입점 신청을 등록하고 심사를 시작합니다. 사업자 검증과 계약 초안 생성으로 이어지며, 정산 주체가 생기므로 항상 승인이 필요합니다.
- consumes → produces: → `seller_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `legal_name` | Text | ✓ | 법인명. 예 `"Harbor Goods LLC"` |
  | `tax_id` | Text | ✓ | 사업자 등록 번호(검증 대상) |
  | `acom:seller_type` | enum | ✓ | `marketplace` \| `consignment` \| `direct_supply` |
  | `address` | PostalAddress | ✓ | 사업장 주소 |
  | `contact_point` | ContactPoint[] | ✓ | 정산·고객 응대·상품 담당자 |
  | `acom:business_category` | Text[] | ✓ | 취급 카테고리 id(`cat_…`, 금지 업종 검사) |
  | `acom:documents` | MediaObject[] | ✓ | 사업자 등록증, 정산 계좌 증빙(업로드 id `upl_…`) |
- 출력(프리뷰): `{"preview_id": "pv_s01a", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "create", "entity": "seller", "id": null, "from": null, "to": {"legal_name": "Harbor Goods LLC", "acom:seller_status": "under_review"}}], "checks": [{"rule": "tax_id_unique", "passed": true, "detail": "No other seller uses this tax id", "fix_tool": null}, {"rule": "onboarding.prohibited_categories", "passed": true, "detail": "No prohibited category", "fix_tool": null}, {"rule": "onboarding.required_documents", "passed": true, "detail": "Both documents attached", "fix_tool": null}], "needs_approval": true, "approval_reason": "Adds a new seller", "change_set_id": "cs_1046"}`
- 출력(실행): `{"ok": true, "id": "slr_harbor_goods", "undo": {"by": "person", "target": "cs_1046", "until": "2026-11-07T23:59:00-05:00"}}` — 상태는 `under_review`
- 정책: [partner.yml](../../policies/partner.yml) `onboarding.required_documents`, `onboarding.prohibited_categories`, `tax_id_unique`
- 에러: `duplicate_tax_id` → `get_seller` · `prohibited_category`(금지 카테고리를 빼고 다시) → `onboard_seller` · `missing_document` → `onboard_seller` · `invalid_tax_id` → `onboard_seller`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 같은 `tax_id`로 다시 신청하면 `duplicate_tax_id`.

### `update_seller` — Kind: W
- 논리 주소 · REST: `partner.seller.update_seller` · `PATCH /v1/partner/sellers/{seller_id}`
- 요약: 판매자 프로필(담당자·주소·취급 카테고리) 수정. 계약·정산 조건은 `set_contract`, 상태는 `set_seller_status`로 분리.
- consumes → produces: `seller_id` → `seller_id`
- 입력: `seller_id`(✓), `contact_point?`, `address?`, `acom:business_category?[]`
- 출력: 프리뷰(`changes` = `update` `seller`, `needs_approval: false`) → 실행 `{"ok": true, "id": "slr_harbor_goods", "undo": {"by": "person", "target": "cs_1046", "until": "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: checks `profile.editable_fields`
- 정책: [partner.yml](../../policies/partner.yml) `profile.editable_fields`
- 에러: `seller_not_found` → `search_sellers` · `immutable_field`(계약 필드는 여기서 못 바꿈) → `set_contract` · `invalid_state` → `set_seller_status`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과.

### `set_seller_status` — Kind: W$
- 논리 주소 · REST: `partner.seller.set_seller_status` · `POST /v1/partner/sellers/{seller_id}/status`
- 요약: 판매자 상태 전이(승인·활성·정지·해지). 노출·판매·정산에 바로 닿으므로 항상 승인. 허용되지 않은 전이는 거부.
- consumes → produces: `seller_id` → `seller_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `seller_id` | Text | ✓ | `"slr_harbor_goods"` |
  | `acom:target_status` | enum | ✓ | `approved` \| `active` \| `suspended` \| `terminated` |
  | `reason` | Text | ✓ | 전이 사유(감사 기록) |
  | `valid_from` | DateTime | | 예약 전이 시각. 없으면 실행 즉시 |
- 출력(프리뷰): `changes`에 `update` `seller`(`from`/`to` 상태)와 영향(숨겨질 상품 수, 진행 중 주문 수, 미정산 금액 `{value, currency}`), checks `lifecycle.allowed_transitions`, `suspend.hide_products`, `needs_approval: true`, `approval_reason: "Suspends a seller with 18 open orders"`
- 출력(실행): `{"ok": true, "id": "slr_harbor_goods", "undo": {"by": "person", "target": "cs_1046", "until": "2026-11-07T23:59:00-05:00"}}` — 해지(`terminated`)는 끝 상태라 되돌리기 대상이 아닙니다.
- 정책: [partner.yml](../../policies/partner.yml) `lifecycle.allowed_transitions`, `suspend.hide_products`. 숨김은 상품을 판매 중지(`active` → `draft`)로 옮기고 활성화 때 그 상품만 되돌립니다. 해지도 상품을 숨기며, 진행 중 주문이 있거나(`open_orders_exist`) 미정산 금액이 남으면(`pending_settlement`) 거부합니다. 정산 금액은 외부 지급 시스템 대신 계약 조건으로 계산한 추정치입니다. `valid_from`이 미래면 변경을 예약하고(`changes[].op: schedule`) 그 시각에 작업 `partner.change_seller_status`가 실행합니다
- 에러: `invalid_state` → `get_seller` · `open_orders_exist` → `search_orders` · `pending_settlement` → `get_seller_overview`
- 멱등성: 같은 목표 상태로의 재시도는 성공으로 처리.

---

## contract

### `get_contract` — Kind: R
- 논리 주소 · REST: `partner.contract.get_contract` · `GET /v1/partner/sellers/{seller_id}/contract`
- 요약: 판매자 계약(수수료율, 정산 주기와 방식, 유효 기간)과 버전 이력.
- consumes → produces: `seller_id` → `contract_id`
- 입력: `seller_id`(✓), `version?`(Integer, 없으면 현재 버전), `fields?[]`
- 출력: `acom:SellerContract {id: "ctr_harbor_goods", version: 2, acom:commission_rate[{category_id, rate}], acom:settlement_cycle, acom:settlement_method, acom:payout_account(마스킹), valid_from, valid_through, acom:contract_status}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음(읽기). 정산 계좌는 가림(`privacy`, `tax_id`와 같은 규칙). 출력의 `acom:commission_rate`는 기본 수수료율(`category_id: null`)을 먼저 두고, `acom:versions[]`에 모든 버전의 기간과 상태를 붙입니다
- 에러: `contract_not_found` → `set_contract` · `seller_not_found` → `search_sellers`
- 멱등성: 읽기(부작용 없음)

### `set_contract` — Kind: W$
- 논리 주소 · REST: `partner.contract.set_contract` · `POST /v1/partner/sellers/{seller_id}/contract`
- 요약: 계약 생성 또는 개정. 정산 금액을 정하므로 항상 승인. 기존 버전은 바꾸지 않고 새 버전을 추가합니다.
- consumes → produces: `seller_id` → `contract_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `seller_id` | Text | ✓ | `"slr_harbor_goods"` |
  | `acom:commission_rate` | object[] | ✓ | 기본과 카테고리별 수수료율(0–1). `category_id`가 없는 항목이 기본 수수료율(하나만)이고, 개정에서 빼면 현재 버전의 기본을 이어 씁니다(첫 계약은 필수). 예 `[{"category_id": "cat_outerwear", "rate": 0.12}]` |
  | `acom:settlement_cycle` | enum | ✓ | `weekly` \| `biweekly` \| `monthly` |
  | `acom:settlement_method` | enum | ✓ | `on_delivery` \| `on_confirmation` |
  | `acom:payout_account_ref` | Text | ✓ | 저장된 정산 계좌 참조(계좌 번호 원문은 받지 않음) |
  | `valid_from` / `valid_through` | DateTime | ✓ | 계약 기간 |
- 출력(프리뷰): `{"preview_id": "pv_s02b", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "create", "entity": "seller_contract", "id": "ctr_harbor_goods", "from": {"version": 1}, "to": {"version": 2, "default_rate": 0.12}}], "checks": [{"rule": "commission", "passed": true, "detail": "12% is within 0–40%", "fix_tool": null}, {"rule": "contract.no_period_overlap", "passed": true, "detail": "Version 1 ends the day before", "fix_tool": null}], "needs_approval": true, "approval_reason": "Sets commission and payout terms", "change_set_id": "cs_1046"}`
- 출력(실행): `{"ok": true, "id": "ctr_harbor_goods", "version": 2, "undo": {"by": "person", "target": "cs_1046", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: [partner.yml](../../policies/partner.yml) `commission.min_rate`, `commission.max_rate`, `contract.no_period_overlap`
- 에러: `commission_out_of_bounds` → `set_contract` · `contract_overlap` → `get_contract` · `seller_not_active` → `set_seller_status`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 계약 버전은 실행마다 하나만 생깁니다.

---

## grade

### `get_seller_grade` — Kind: R
- 논리 주소 · REST: `partner.grade.get_seller_grade` · `GET /v1/partner/sellers/{seller_id}/grade`
- 요약: 판매자 등급과 계산 근거(배송 준수율, 주문 이행률, 클레임률, 고객 응대 점수). 서버 집계 값이며 월·일 단위 상세를 지원합니다.
- consumes → produces: `seller_id` → grade
- 입력: `seller_id`(✓), `valid_from?` / `valid_through?`(Date, 기간. 없으면 지난 달), `acom:granularity?`(`month` \| `day`, 기본 `month`)
- 출력: `acom:SellerGrade {seller_id, acom:grade_level, acom:shipping_compliance_rate, acom:order_fulfillment_rate, acom:claim_rate, acom:service_score, acom:graded_period{valid_from, valid_through}, acom:next_review_date}` — 비율은 0–1
- 프리뷰: 해당 없음(읽기)
- 정책: [partner.yml](../../policies/partner.yml) `grade.computed_metrics`
- 에러: `seller_not_found` → `search_sellers` · `no_data_for_period`(기간에 주문이 없음) → `get_seller_grade`
- 멱등성: 읽기(부작용 없음)

### `set_seller_grade` — Kind: W
- 논리 주소 · REST: `partner.grade.set_seller_grade` · `POST /v1/partner/sellers/{seller_id}/grade`
- 요약: 계산된 등급을 확정하거나 예외로 조정. 계산값과 다르면 사유가 필수이며, 프리뷰가 수수료·노출 영향을 보여 줍니다.
- consumes → produces: `seller_id` → `seller_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `seller_id` | Text | ✓ | `"slr_harbor_goods"` |
  | `acom:grade_level` | enum | ✓ | `platinum` \| `gold` \| `silver` \| `bronze` \| `watch` |
  | `acom:adjustment_reason` | Text | | 계산값과 다를 때 필수 |
  | `valid_from` / `valid_through` | DateTime | ✓ | 적용 기간 |
- 출력(프리뷰): `changes`(`update` `seller_grade`, 수수료 구간·노출 가중치 영향), `checks: [{"rule": "grade.override_requires_reason", "passed": true, "detail": "Reason given for moving gold → platinum", "fix_tool": null}]`, `needs_approval: false`
- 출력(실행): `{"ok": true, "id": "slr_harbor_goods", "undo": {"by": "person", "target": "cs_1046", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: [partner.yml](../../policies/partner.yml) `grade.computed_metrics`, `grade.override_requires_reason`, `grade.benefits`. 계산 등급은 적용 기간이 시작하는 달의 앞 달 실적(`grade.levels`)이고, 어느 시점의 등급은 그 시점을 덮는 가장 최근에 확정한 등급입니다
- 에러: `override_not_allowed` → `get_seller_grade` · `invalid_level` → `set_seller_grade`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과.

---

## dashboard

### `get_seller_overview` — Kind: R
- 논리 주소 · REST: `partner.dashboard.get_seller_overview` · `GET /v1/partner/sellers/{seller_id}/overview`
- 요약: 판매자 운영 요약(판매, 정산 예정, 배송 준수, 클레임, 등급, 미처리 작업)을 한 번에. 판매자 본인과 운영자가 함께 쓰며, 역할에 따라 보이는 지표가 다릅니다.
- consumes → produces: `seller_id` → overview
- 입력: `seller_id`(✓), `period`(`last_30_days` \| `last_7_days` \| `custom`, 기본 `last_30_days`), `valid_from?` / `valid_through?`(`custom`일 때 ✓), `fields?[]`
- 출력:
  ```jsonc
  {
    "seller_id": "slr_harbor_goods",
    "period": {"valid_from": "2026-10-07", "valid_through": "2026-11-05"},
    "sales": {"order_count": 1240, "revenue": {"value": 48200.00, "currency": "USD"}},
    "settlement": {"pending_amount": {"value": 3120.00, "currency": "USD"}, "next_payout_date": "2026-11-15", "acom:settlement_cycle": "monthly"},
    "fulfillment": {"acom:shipping_compliance_rate": 0.98, "acom:order_fulfillment_rate": 0.99, "late_shipments": 7},
    "claims": {"open_claims": 3, "acom:claim_rate": 0.012},
    "grade": {"acom:grade_level": "gold", "acom:next_review_date": "2026-12-01"},
    "tasks": {"waiting_for_approval": 2, "unanswered_inquiries": 5}
  }
  ```
- 프리뷰: 해당 없음(읽기)
- 정책: [partner.yml](../../policies/partner.yml) `dashboard.visible_metrics_by_role`. 역할은 판매사 조직의 운영자면 `seller`, 스토어 운영자면 `operator`. `fields`는 구역 이름(`sales` … `tasks`). `tasks.waiting_for_approval`은 판매자 상품 중 검수(`pending_review`)를 기다리는 수, `unanswered_inquiries`는 답이 없는 상품 문의 수입니다(partner는 catalog·sales·fulfillment만 읽음). 정산 구역은 외부 지급 시스템 대신 계약 조건으로 계산한 추정치이고 그렇게 적습니다
- 에러: `seller_not_found` → `search_sellers` · `forbidden`(역할 밖의 지표를 요청) → `get_seller_overview`
- 멱등성: 읽기(부작용 없음)
