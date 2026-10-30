# API 명세 — `partner` (입점 판매자·계약·등급)

> 계층: `partner.{resource}.{operation}`. 공통 규약(응답 형식·에러·프리뷰/커밋·멱등성·헤더)은 [README.md](README.md) §3, 데이터 타입은 [data-model.md](data-model.md).
> **범위**: 해커톤 MVP 범위 밖입니다. 데모 스토어는 판매자가 하나인 단일 스토어이며, 이 도메인은 여러 판매자가 입점하는 목표 구조를 위한 명세입니다.
> **설계 축**: ① 입점 판매자는 schema.org `Organization`이며, 상품의 `Offer.seller`·주문의 `Order.seller`와 같은 id를 씁니다. ② 신청 → 심사 → 승인 → 활성의 상태 전이. ③ 계약(수수료·정산 조건)은 돈이 움직이므로 W$이고 버전으로 쌓습니다. ④ 등급은 배송 준수율·이행률 실적으로 서버가 계산하고, 사람은 확정과 예외 조정만 합니다. ⑤ `seller`는 모든 도메인에서 같은 필터 축입니다.

리소스: [seller](#seller) · [contract](#contract) · [grade](#grade) · [dashboard](#dashboard)

---

## seller

### `search_sellers` — Kind: R
- 논리 주소 · REST: `partner.seller.search_sellers` · `POST /v1/partner/sellers/search`
- 요약: 조건으로 입점 판매자 검색. 결과의 `seller_id`는 상품·주문의 `seller` 필터에 그대로 씁니다.
- consumes → produces: → `seller_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `query` | Text | | 상호 또는 사업자 번호 일부 |
  | `acom:seller_status` | enum | | `applied` \| `under_review` \| `approved` \| `active` \| `suspended` \| `terminated` |
  | `acom:seller_type` | enum | | `marketplace` \| `consignment` \| `direct_supply` |
  | `acom:grade_level` | enum | | `platinum` \| `gold` \| `silver` \| `bronze` \| `watch` |
  | `sort` | enum | | `newest` \| `name` \| `grade` |
  | `limit` / `offset` | Integer | | 기본 20 / 0 |
- 출력: `{"items": [Organization {identifier, legal_name, tax_id, acom:seller_status, acom:seller_type, acom:grade_level}], "total"}`
- 에러: `invalid_filter` → `search_sellers`

### `get_seller` — Kind: R
- 논리 주소 · REST: `partner.seller.get_seller` · `GET /v1/partner/sellers/{seller_id}`
- 요약: 판매자 하나의 법인 정보·담당자·계약 요약·등급. `fields`로 응답 축소.
- consumes → produces: `seller_id` → seller
- 입력: `seller_id`(✓), `fields?[]`
- 출력: `Organization {identifier, legal_name, tax_id, address(PostalAddress), contact_point(ContactPoint[]), acom:seller_status, acom:seller_type, acom:contract_summary, acom:grade_level}`
- 에러: `seller_not_found` → `search_sellers`

### `onboard_seller` — Kind: W$
- 논리 주소 · REST: `partner.seller.onboard_seller` · `POST /v1/partner/sellers`
- 요약: 입점 신청을 등록하고 심사를 시작합니다. 사업자 검증과 계약 초안 생성으로 이어지며, 정산 주체가 생기므로 항상 승인이 필요합니다.
- consumes → produces: → `seller_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `legal_name` | Text | ✓ | 법인명 |
  | `tax_id` | Text | ✓ | 사업자 등록 번호(검증 대상) |
  | `acom:seller_type` | enum | ✓ | `marketplace` \| `consignment` \| `direct_supply` |
  | `address` | PostalAddress | ✓ | 사업장 주소 |
  | `contact_point` | ContactPoint[] | ✓ | 정산·고객 응대·상품 담당자 |
  | `acom:business_category` | Text[] | ✓ | 취급 카테고리(금지 업종 검사) |
  | `acom:documents` | MediaObject[] | ✓ | 사업자 등록증, 정산 계좌 증빙 |
  | `preview` | Boolean | ✓ | `true` 먼저 |
- 출력(프리뷰): `{"preview_id", "changes": [{"op": "create", "entity": "seller"}], "checks": [{"rule": "tax_id_unique", ...}, {"rule": "prohibited_category", ...}, {"rule": "required_documents", ...}], "needs_approval": true, "approval_reason": "Adds a new seller"}`
- 출력(커밋): `{"ok": true, "id": "slr_harbor_goods", "undo": null}` — 상태는 `under_review`
- 정책 키: `partner.onboarding.required_documents`, `partner.onboarding.prohibited_categories`, `partner.tax_id_unique`
- 에러: `duplicate_tax_id` → `get_seller` · `prohibited_category` → `onboard_seller`(카테고리 조정) · `missing_document` → `onboard_seller` · `invalid_tax_id` → `onboard_seller`
- 멱등성: `Idempotency-Key`. 같은 `tax_id`와 `acom:seller_type`의 재시도는 첫 결과 반환.

### `update_seller` — Kind: W
- 논리 주소 · REST: `partner.seller.update_seller` · `PATCH /v1/partner/sellers/{seller_id}`
- 요약: 판매자 프로필(담당자·주소·취급 카테고리) 수정. 계약·정산 조건은 `set_contract`, 상태는 `set_seller_status`로 분리.
- consumes → produces: `seller_id` → `seller_id`
- 입력: `seller_id`(✓), `contact_point?`, `address?`, `acom:business_category?[]`, `preview`(✓)
- 출력: 프리뷰 → 커밋 `{"ok": true, "id": "slr_harbor_goods", "undo": null}`
- 정책 키: `partner.profile.editable_fields`
- 에러: `seller_not_found` → `search_sellers` · `immutable_field` → `set_contract` · `invalid_state` → `set_seller_status`
- 멱등성: `Idempotency-Key`

### `set_seller_status` — Kind: W$
- 논리 주소 · REST: `partner.seller.set_seller_status` · `POST /v1/partner/sellers/{seller_id}/status`
- 요약: 판매자 상태 전이(승인·활성·정지·해지). 노출·판매·정산에 바로 닿으므로 항상 승인. 허용되지 않은 전이는 거부.
- consumes → produces: `seller_id` → `seller_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 |
  |---|---|:--:|---|
  | `seller_id` | Text | ✓ | |
  | `acom:target_status` | enum | ✓ | `approved` \| `active` \| `suspended` \| `terminated` |
  | `reason` | Text | ✓ | 전이 사유(감사 기록) |
  | `valid_from` | DateTime | | 예약 전이 시각 |
  | `preview` | Boolean | ✓ | |
- 출력(프리뷰): `changes`에 상태 전이와 영향(숨겨질 상품 수, 진행 중 주문 수, 미정산 금액), `needs_approval: true`, `approval_reason: "Suspends a seller with 18 open orders"`
- 출력(커밋): `{"ok": true, "id": "slr_harbor_goods", "undo": {"tool": "set_seller_status", "until": null}}`
- 정책 키: `partner.lifecycle.allowed_transitions`, `partner.suspend.hide_products`
- 에러: `invalid_state` → `get_seller` · `open_orders_exist` → `search_orders` · `pending_settlement` → `get_seller_overview`
- 멱등성: `Idempotency-Key`. 같은 목표 상태로의 재시도는 멱등 성공.

---

## contract

### `get_contract` — Kind: R
- 논리 주소 · REST: `partner.contract.get_contract` · `GET /v1/partner/sellers/{seller_id}/contract`
- 요약: 판매자 계약(수수료율, 정산 주기와 방식, 유효 기간)과 버전 이력.
- consumes → produces: `seller_id` → contract
- 입력: `seller_id`(✓), `version?`, `fields?[]`
- 출력: `acom:SellerContract {acom:commission_rate[], acom:settlement_cycle, acom:settlement_method, acom:payout_account(마스킹), valid_from, valid_through, version, acom:contract_status}`
- 에러: `contract_not_found` → `set_contract` · `seller_not_found` → `search_sellers`

### `set_contract` — Kind: W$
- 논리 주소 · REST: `partner.contract.set_contract` · `POST /v1/partner/sellers/{seller_id}/contract`
- 요약: 계약 생성 또는 개정. 정산 금액을 정하므로 항상 승인. 기존 버전은 바꾸지 않고 새 버전을 추가합니다.
- consumes → produces: `seller_id` → `contract_version`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `seller_id` | Text | ✓ | |
  | `acom:commission_rate` | object[] | ✓ | 기본과 카테고리별 수수료율 `[{"category": "outerwear", "rate": 0.12}]` |
  | `acom:settlement_cycle` | enum | ✓ | `weekly` \| `biweekly` \| `monthly` |
  | `acom:settlement_method` | enum | ✓ | `on_delivery` \| `on_confirmation` |
  | `acom:payout_account` | acom:BankAccount | ✓ | 정산 입금 계좌 |
  | `valid_from` / `valid_through` | DateTime | ✓ | 계약 기간 |
  | `preview` | Boolean | ✓ | |
- 출력(프리뷰): `{"preview_id", "changes": [{"op": "create", "entity": "contract", ...}], "checks": [{"rule": "commission_bounds", ...}, {"rule": "contract_no_overlap", ...}], "needs_approval": true, "approval_reason": "Sets commission and payout terms"}`
- 출력(커밋): `{"ok": true, "id": "ctr_harbor_goods_v2", "undo": null}`
- 정책 키: `partner.commission.min_rate`, `partner.commission.max_rate`, `partner.contract.no_period_overlap`
- 에러: `commission_out_of_bounds` → `set_contract` · `contract_overlap` → `get_contract` · `seller_not_active` → `set_seller_status`
- 멱등성: `Idempotency-Key`. 같은 `seller_id`·`valid_from`·수수료율의 재시도는 첫 결과 반환.

---

## grade

### `get_seller_grade` — Kind: R
- 논리 주소 · REST: `partner.grade.get_seller_grade` · `GET /v1/partner/sellers/{seller_id}/grade`
- 요약: 판매자 등급과 계산 근거(배송 준수율, 주문 이행률, 클레임률, 고객 응대 점수). 서버 집계 값이며 월·일 단위 상세를 지원합니다.
- consumes → produces: `seller_id` → grade
- 입력: `seller_id`(✓), `period?`(`"2026-10"` 또는 `{valid_from, valid_through}`), `acom:granularity?`(`month` \| `day`)
- 출력: `acom:SellerGrade {acom:grade_level, acom:shipping_compliance, acom:order_fulfillment, acom:claim_rate, acom:service_score, acom:graded_period, acom:next_review_date}`
- 에러: `seller_not_found` → `search_sellers` · `no_data_for_period` → `get_seller_grade`

### `set_seller_grade` — Kind: W
- 논리 주소 · REST: `partner.grade.set_seller_grade` · `POST /v1/partner/sellers/{seller_id}/grade`
- 요약: 계산된 등급을 확정하거나 예외로 조정. 계산값과 다르면 사유가 필수이며, 프리뷰가 수수료·노출 영향을 보여 줍니다.
- consumes → produces: `seller_id` → `seller_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 |
  |---|---|:--:|---|
  | `seller_id` | Text | ✓ | |
  | `acom:grade_level` | enum | ✓ | `platinum` \| `gold` \| `silver` \| `bronze` \| `watch` |
  | `acom:adjustment_reason` | Text | | 계산값과 다를 때 필수 |
  | `valid_from` / `valid_through` | DateTime | ✓ | 적용 기간 |
  | `preview` | Boolean | ✓ | |
- 출력(프리뷰): `changes`(등급 변경, 수수료 구간·노출 가중치 영향), `checks: [{"rule": "grade_override_bounds", ...}]`, `needs_approval`
- 출력(커밋): `{"ok": true, "id": "slr_harbor_goods", "undo": {"tool": "set_seller_grade", "until": null}}`
- 정책 키: `partner.grade.computed_metrics`, `partner.grade.override_requires_reason`, `partner.grade.benefits`
- 에러: `override_not_allowed` → `get_seller_grade` · `invalid_level` → `set_seller_grade`
- 멱등성: `Idempotency-Key`

---

## dashboard

### `get_seller_overview` — Kind: R
- 논리 주소 · REST: `partner.dashboard.get_seller_overview` · `GET /v1/partner/sellers/{seller_id}/overview`
- 요약: 판매자 운영 요약(판매, 정산 예정, 배송 준수, 클레임, 등급, 미처리 작업)을 한 번에. 판매자 본인과 운영자가 함께 쓰며, 역할에 따라 보이는 지표가 다릅니다.
- consumes → produces: `seller_id` → overview
- 입력: `seller_id`(✓), `period?`(`last_30_days` 또는 `{valid_from, valid_through}`), `fields?[]`
- 출력:
  ```jsonc
  {
    "seller_id": "slr_harbor_goods",
    "period": "last_30_days",
    "sales": {"order_count": 1240, "revenue": 48200.00, "price_currency": "USD"},
    "settlement": {"pending_amount": 3120.00, "price_currency": "USD", "next_payout_date": "2026-11-15", "acom:settlement_cycle": "monthly"},
    "fulfillment": {"acom:shipping_compliance": 0.98, "acom:order_fulfillment": 0.99, "late_shipments": 7},
    "claims": {"open_claims": 3, "acom:claim_rate": 0.012},
    "grade": {"acom:grade_level": "gold", "acom:next_review_date": "2026-12-01"},
    "tasks": {"waiting_for_approval": 2, "unanswered_inquiries": 5}
  }
  ```
- 정책 키: `partner.dashboard.visible_metrics_by_role`
- 에러: `seller_not_found` → `search_sellers` · `forbidden_scope` → `get_seller_overview`(권한 안의 지표로)
