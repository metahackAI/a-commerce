# API 명세 — `customer` (회원·등급·주소·동의·기억(선호)·위시리스트·임직원·상담·보상·FAQ/공지)

> 계층: `customer.{resource}.{operation}`. 공통 규약(응답 형식·에러·프리뷰/실행·멱등·헤더)은 [README.md](README.md) §3, 데이터 타입은 [data-model.md](data-model.md), 정책은 [customer.yml](policies/customer.yml) · [approval.yml](policies/approval.yml). MVP 툴의 시그니처는 [plan.md §3.2–§3.4](../plan.md)와 같습니다.
> **설계 축**: ① 개인정보는 기본 가림, 가림 해제는 사유를 액션 로그에 남김, ② 마케팅 발송은 동의(opt-in) 게이트, ③ 탈퇴·비밀번호 재설정은 **회원 본인만**(사람 전용 엔드포인트, 툴 아님), ④ **보이고 고칠 수 있는 기억** — 에이전트가 기억한 선호는 쇼퍼에게 한 줄로 보이고 한 번에 되돌릴 수 있음, ⑤ 보상은 귀책 분류 + 단계별 승인(현금은 지급 승인자 별도), ⑥ 동의·탈퇴·보상 이력은 append만.

리소스: [person](#person) · [membership](#membership) · [address](#address) · [consent](#consent) · [preference](#preference) · [wishlist](#wishlist) · [employee](#employee) · [inquiry](#inquiry) · [remedy](#remedy) · [faq_notice](#faq_notice) · [사람 전용 엔드포인트](#사람-전용-엔드포인트)

- schema.org: 회원 `Person` + `ProgramMembership`, 등급 `MemberProgramTier`, 주소 `PostalAddress`, 기억 `acom:Preference`(`PropertyValue`의 확장), 상담 `acom:CustomerInquiry`(`Message`의 확장).
- id 예: 회원 `cus_jamie`, `cus_alex`, 등급 `tier_vip`, 기억 `mem_12`.

---

## person

### `get_my_profile` — Kind: R
- 논리주소/REST: `customer.person.get_my_profile` · `GET /v1/customer/people/me`
- 요약: 로그인한 쇼퍼 본인의 프로필. 쇼퍼 에이전트가 사이즈·선호를 알고 답하기 위해 씁니다. 카드는 끝 4자리만, 그 밖의 결제 정보는 돌려주지 않습니다. 쇼퍼 툴, MVP([plan.md §3.4](../plan.md)).
- consumes → produces: → profile
- 입력: 없음(세션의 쇼퍼). `fields?[]`
- 출력:
  ```jsonc
  {"items": [{"customer_id": "cus_jamie", "name": "Jamie", "tier": "vip",
              "sizes": {"tops": "M"},
              "preferences": [{"memory_id": "mem_07", "key": "fabric", "value": "natural fabrics", "text": "Prefers natural fabrics", "source": "conversation"},
                              {"memory_id": "mem_12", "key": "travel", "value": "light", "text": "Travels light", "source": "conversation"}],
              "address": {"label": "Home", "street_address": "12 Harbor St, Apt 3"},
              "card": {"last4": "4821"}}],
   "total": 1}
  ```
- 정책: `privacy.masking_default`(카드는 끝 4자리만)
- 에러: `not_signed_in` → (쇼퍼 로그인 필요, 게스트는 프로필 없음)
- 멱등성: 읽기, 부작용 없음.

### `get_return_history` — Kind: R
- 논리주소/REST: `customer.person.get_return_history` · `GET /v1/customer/people/{customer_id}/returns?days=90`
- 요약: 한 고객이 기간 안에 한 반품. 운영 에이전트가 반품 요청을 검토할 때 "90일 안에 4번 반품"(#20431) 같은 check를 위해 씁니다. MVP 툴([plan.md §3.3](../plan.md)).
- consumes → produces: `customer_id` → history
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | customer_id | Text | ✓ | `"cus_4431"` |
  | days | Integer | | 기본 `90` |
- 출력: `{"items": [{"return_request_id": "ret_20431", "order_id": "ord_20431", "item": "Wool overcoat", "amount": 289.00, "price_currency": "USD", "reason": "not_as_described", "status": "waiting", "requested_at": "2026-11-05T16:40:00"}], "total": 4}`
- 정책: 없음(읽기). 기본 가림 적용.
- 에러: `customer_not_found` → `search_members`
- 멱등성: 읽기.

### `search_members` — Kind: R
- 논리주소/REST: `customer.person.search` · `GET /v1/customer/people`
- 요약: 조건으로 회원 검색. 이름·연락처·이메일은 기본 가림.
- consumes → produces: → `customer_id[]`
- 입력: `query`, `type`(`individual` \| `business`), `state`(`active` \| `dormant` \| `withdrawn` \| `blocked`), `tier`, `segment_id`, `joined_from`, `joined_through`, `unmask`(Boolean, `X-Access-Reason` 헤더 필수), `fields?[]`, `limit`(기본 20), `offset`(기본 0)
- 출력: `{"items": [{"customer_id": "cus_alex", "name": "A*** K***", "email": "a***@example.com", "tier": "member", "state": "active"}], "total": 3120}`
- 정책: `privacy.masking_default`, `privacy.access_reason_logging`
- 에러: `access_reason_required` → `search_members`(사유와 함께 재호출) · `unmask_forbidden` → `list_roles`
- 멱등성: 읽기.

### `get_member` — Kind: R
- 논리주소/REST: `customer.person.get` · `GET /v1/customer/people/{customer_id}`
- 요약: 회원 한 명(회원 정보, `member_of` 등급, 주소·동의 요약). 기본 가림.
- consumes → produces: `customer_id` → person
- 입력: `customer_id`(✓), `unmask`, `fields?[]`
- 출력: `{"items": [{"customer_id": "cus_alex", "name": "A*** K***", "member_of": {"program": "northwind_rewards", "tier": "member", "membership_number": "NW-008812"}, "state": "active", "consent": {"marketing_email": true}}], "total": 1}`
- 정책: `privacy.masking_default`, `privacy.access_reason_logging`
- 에러: `customer_not_found` → `search_members` · `access_reason_required` → `get_member`
- 멱등성: 읽기.

### `register_member` — Kind: W
- 논리주소/REST: `customer.person.register` · `POST /v1/customer/people`
- 요약: 회원 가입(개인·사업자). 본인 확인 토큰과 필수 동의를 함께 받습니다.
- consumes → produces: → `customer_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | type | Text(enum) | ✓ | `individual` \| `business` |
  | name | Text | ✓ | `"Alex Kim"` |
  | email | Text | ✓ | 로그인 id 겸용 |
  | telephone | Text | ✓ | 본인 확인 번호 |
  | password | Text | ✓ | 8–30자, 문자·숫자·기호 |
  | legal_name, tax_id | Text | business | 사업자 상호·사업자 번호 |
  | verification_token | Text | ✓ | `verify_identity` 결과 |
  | consents | Object[] | ✓ | `[{consent_type: "terms", granted: true}, {consent_type: "privacy", granted: true}]` |
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력(프리뷰): `{"preview_id": "pv_m01a", "changes": [{"op": "create", "entity": "person", "id": "cus_new", "from": null, "to": "active"}], "checks": [{"rule": "identity_verification", "passed": true, "detail": "Phone verified 2 min ago", "fix_tool": "verify_identity"}, {"rule": "consent_mandatory", "passed": true, "detail": "Terms and privacy accepted", "fix_tool": "set_consent"}, {"rule": "member_password", "passed": true, "detail": "12 characters, letters, digits and a symbol", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "cus_8812", "undo": null}`
- 정책: `identity_verification.required_for`, `member.password`, `consent.mandatory`, `member.id_reuse_after_withdrawal_days`
- 에러: `identity_not_verified` → `verify_identity` · `mandatory_consent_missing` → `set_consent` · `duplicate_member` → `check_availability` · `password_policy` → `register_member` · `id_reuse_blocked` → `check_availability`
- 멱등성: `Idempotency-Key = hash(email + telephone)`

### `update_member` — Kind: W
- 논리주소/REST: `customer.person.update` · `PATCH /v1/customer/people/{customer_id}`
- 요약: 회원 정보 수정. 연락처(`telephone`, `email`)를 바꾸면 본인 확인을 다시 합니다.
- consumes → produces: `customer_id` → `customer_id`
- 입력: `customer_id`(✓), `name`, `telephone`, `email`, `verification_token`(연락처 변경 시 ✓), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_m02b", "changes": [{"op": "update", "entity": "person", "id": "cus_alex", "from": "a***@example.com", "to": "a***@example.org"}], "checks": [{"rule": "identity_verification", "passed": true, "detail": "Verified for contact change", "fix_tool": "verify_identity"}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "cus_alex", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: `identity_verification.required_for(contact_change)`
- 에러: `reverify_required` → `verify_identity` · `customer_not_found` → `search_members`
- 멱등성: `Idempotency-Key`

### `block_member` — Kind: W$
- 논리주소/REST: `customer.person.block` · `POST /v1/customer/people/{customer_id}/block`
- 요약: 회원 이용 정지(`active`/`dormant` → `blocked`). 고객에게 닿는 제재이므로 항상 사람 승인.
- consumes → produces: `customer_id` → `customer_id`
- 입력: `customer_id`(✓), `reason`(✓), `block_until`(DateTime), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_m03c", "changes": [{"op": "update", "entity": "person", "id": "cus_7710", "from": "active", "to": "blocked"}], "checks": [{"rule": "member_block", "passed": true, "detail": "Reason recorded: repeated chargebacks", "fix_tool": null}], "needs_approval": true, "approval_reason": "Blocks a member until 2026-12-31"}`
- 출력(실행): `{"ok": true, "id": "cus_7710", "undo": {"tool": "unblock_member", "until": "2026-12-31T23:59:00"}}`
- 정책: [approval.yml](policies/approval.yml) `member_block`(항상), `member.states`
- 에러: `already_blocked` → `get_member` · `customer_not_found` → `search_members`
- 멱등성: `Idempotency-Key = hash(customer_id + "block")`

### `unblock_member` — Kind: W$
- 논리주소/REST: `customer.person.unblock` · `POST /v1/customer/people/{customer_id}/unblock`
- 요약: 정지 해제(`blocked` → `active`). `member_block`과 같은 승인 규칙.
- consumes → produces: `customer_id` → `customer_id`
- 입력: `customer_id`(✓), `reason`(✓), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_m04d", "changes": [{"op": "update", "entity": "person", "id": "cus_7710", "from": "blocked", "to": "active"}], "checks": [], "needs_approval": true, "approval_reason": "Unblocks a member"}`
- 출력(실행): `{"ok": true, "id": "cus_7710", "undo": {"tool": "block_member", "until": null}}`
- 정책: [approval.yml](policies/approval.yml) `member_block`
- 에러: `not_blocked` → `get_member`
- 멱등성: `Idempotency-Key = hash(customer_id + "unblock")`

### `verify_identity` — Kind: W
- 논리주소/REST: `customer.person.verify_identity` · `POST /v1/customer/people/verify-identity`
- 요약: 가입·연락처 변경·탈퇴·비밀번호 재설정 전에 하는 본인 확인(휴대폰 일회용 코드). 성공하면 짧게 유효한 1회용 토큰을 줍니다. 승인 대상이 아니라 신원 확인 절차입니다.
- consumes → produces: → `verification_token`
- 입력: `name`(✓), `telephone`(✓), `method`(✓, `sms_code`), `code`(두 번째 호출에서 ✓), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_v01a", "changes": [{"op": "create", "entity": "identity_verification", "id": "ivf_201", "from": null, "to": "code_sent"}], "checks": [{"rule": "verification_rate_limit", "passed": true, "detail": "1 code per minute", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "ivf_201", "undo": null}` — 코드 확인 후 `get`으로 `{verification_token, expires_in: 300}`
- 정책: `identity_verification`
- 에러: `code_mismatch` → `verify_identity` · `verification_expired` → `verify_identity`
- 멱등성: 코드 발송은 `(telephone, 1분)` 제한, 토큰은 1회용.

### `check_availability` — Kind: R
- 논리주소/REST: `customer.person.check_availability` · `GET /v1/customer/people/availability?field=email&value=…`
- 요약: 가입 전 이메일·로그인 id 사용 가능 여부. 비회원도 호출 가능.
- consumes → produces: → availability
- 입력: `field`(✓, `email` \| `login_id`), `value`(✓)
- 출력: `{"items": [{"field": "email", "value": "alex@example.com", "available": false, "reason": "taken"}], "total": 1}` — `reason`: `taken` \| `reserved` \| `invalid_format`
- 정책: `member.id_reuse_after_withdrawal_days`
- 에러: `invalid_format` → `check_availability`
- 멱등성: 읽기.

### `verify_business` — Kind: W$
- 논리주소/REST: `customer.person.verify_business` · `POST /v1/customer/people/{customer_id}/verify-business`
- 요약: 사업자 회원의 사업자 번호 확인. 확인되면 사업자 가격·세금 영수증 발급 자격이 켜지므로 사람 승인을 거칩니다.
- consumes → produces: `customer_id` → `customer_id`
- 입력: `customer_id`(✓), `tax_id`(✓), `legal_name`(✓), `registration_document`(MediaObject), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_b01c", "changes": [{"op": "update", "entity": "person", "id": "cus_biz_31", "from": "business_unverified", "to": "business_verified"}], "checks": [{"rule": "business_registry_match", "passed": true, "detail": "Tax id and legal name match the registry", "fix_tool": null}], "needs_approval": true, "approval_reason": "Turns on business pricing and tax receipts"}`
- 출력(실행): `{"ok": true, "id": "cus_biz_31", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: [approval.yml](policies/approval.yml), `identity_verification`; 세금 영수증 발급은 `sales` 도메인의 `issue_receipt`
- 에러: `registry_mismatch` → `verify_business` · `already_verified` → `get_member` · `not_business_member` → `get_member`
- 멱등성: `Idempotency-Key = hash(customer_id + tax_id)`

---

## membership

### `list_tiers` — Kind: R
- 논리주소/REST: `customer.membership.list_tiers` · `GET /v1/customer/tiers`
- 요약: 회원 등급과 등급별 회원 수. 운영 에이전트가 "VIP 회원"을 `set_sale_prices`·`create_coupon`의 `valid_for_member_tier`로 넘길 때 씁니다. MVP 툴([plan.md §3.2](../plan.md)). schema.org `MemberProgramTier`.
- consumes → produces: → `tier`
- 입력: 없음
- 출력:
  ```jsonc
  {"items": [{"tier": "vip", "name": "VIP", "member_count": 1284},
             {"tier": "gold", "name": "Gold", "member_count": 4210},
             {"tier": "member", "name": "Member", "member_count": 18930}],
   "total": 3}
  ```
- 정책: `tier`
- 에러: 없음(읽기, 빈 목록 가능).
- 멱등성: 읽기.

### `get_member_tier` — Kind: R
- 논리주소/REST: `customer.membership.get_tier` · `GET /v1/customer/people/{customer_id}/tier`
- 요약: 회원 한 명의 현재 등급과 다음 산정일. 등급은 구매 이력으로 매달 산정한 값(읽기).
- consumes → produces: `customer_id` → `tier`
- 입력: `customer_id`(✓)
- 출력: `{"items": [{"customer_id": "cus_jamie", "tier": "vip", "since": "2026-03-01", "next_review": "2026-12-01", "has_tier_benefit": ["Free shipping", "Early access"]}], "total": 1}`
- 정책: `tier.recalc`, `tier.basis_months`
- 에러: `customer_not_found` → `search_members`
- 멱등성: 읽기.

---

## address

### `list_addresses` — Kind: R
- 논리주소/REST: `customer.address.list` · `GET /v1/customer/people/{customer_id}/addresses`
- 요약: 저장된 주소 목록(`usage`: 배송·청구·반품, 대표 주소 1개). 기본 가림.
- consumes → produces: `customer_id` → `address_id[]`
- 입력: `customer_id`(✓), `limit`, `offset`
- 출력: `{"items": [{"address_id": "adr_home", "label": "Home", "street_address": "12 Harbor St, Apt 3", "address_locality": "Portland", "address_region": "ME", "postal_code": "04101", "address_country": "US", "usage": ["shipping"], "is_primary": true}], "total": 1}`
- 정책: `privacy.masking_default`
- 에러: `customer_not_found` → `search_members`
- 멱등성: 읽기.

### `add_address` — Kind: W
- 논리주소/REST: `customer.address.add` · `POST /v1/customer/people/{customer_id}/addresses`
- 요약: 주소 추가. `is_primary: true`면 기존 대표 주소는 해제.
- consumes → produces: `customer_id` → `address_id`
- 입력: `customer_id`(✓), `label`, `street_address`(✓), `address_locality`(✓), `address_region`, `postal_code`(✓), `address_country`(✓), `usage[]`(✓), `is_primary`, `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_a01a", "changes": [{"op": "create", "entity": "postal_address", "id": "adr_work", "from": null, "to": "200 Commercial St"}], "checks": [{"rule": "postal_code_valid", "passed": true, "detail": "04101 is in ME", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "adr_work", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: 대표 주소 1개
- 에러: `address_limit_exceeded` → `list_addresses` · `invalid_postal_code` → `add_address`
- 멱등성: `Idempotency-Key`

### `update_address` — Kind: W
- 논리주소/REST: `customer.address.update` · `PATCH /v1/customer/people/{customer_id}/addresses/{address_id}`
- 요약: 주소 수정.
- consumes → produces: `address_id` → `address_id`
- 입력: `address_id`(✓), 바꿀 필드, `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_a02b", "changes": [{"op": "update", "entity": "postal_address", "id": "adr_home", "from": "Apt 3", "to": "Apt 4"}], "checks": [], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "adr_home", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: 없음.
- 에러: `address_not_found` → `list_addresses`
- 멱등성: `Idempotency-Key`

### `set_primary_address` — Kind: W
- 논리주소/REST: `customer.address.set_primary` · `POST /v1/customer/people/{customer_id}/addresses/{address_id}/primary`
- 요약: 대표 배송지 지정(기존 대표는 자동 해제).
- consumes → produces: `address_id` → `address_id`
- 입력: `address_id`(✓), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_a03c", "changes": [{"op": "update", "entity": "postal_address", "id": "adr_work", "from": false, "to": true}], "checks": [], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "adr_work", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: 대표 주소 1개
- 에러: `address_not_found` → `list_addresses`
- 멱등성: 같은 주소 재지정은 성공으로 수렴.

---

## consent

### `get_consent` — Kind: R
- 논리주소/REST: `customer.consent.get` · `GET /v1/customer/people/{customer_id}/consent`
- 요약: 필수·선택 동의 상태와 변경 이력. 이력은 바뀌지 않습니다. 마케팅 발송 전 게이트가 이 값을 읽습니다.
- consumes → produces: `customer_id` → consent
- 입력: `customer_id`(✓)
- 출력: `{"items": [{"consent_type": "marketing_email", "granted": true, "at": "2026-04-02T09:00:00"}, {"consent_type": "terms", "granted": true, "at": "2025-12-10T18:22:00"}], "total": 5}`
- 정책: `consent`
- 에러: `customer_not_found` → `search_members`
- 멱등성: 읽기.

### `set_consent` — Kind: W
- 논리주소/REST: `customer.consent.set` · `POST /v1/customer/people/{customer_id}/consent`
- 요약: 동의 설정·철회. 필수 동의(`terms`, `privacy`)는 철회할 수 없습니다. 기존 기록을 덮어쓰지 않고 새 기록을 append합니다.
- consumes → produces: `customer_id` → `consent_id`
- 입력: `customer_id`(✓), `consent_type`(✓, `terms` \| `privacy` \| `marketing_email` \| `marketing_sms` \| `marketing_push`), `granted`(✓), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_c01a", "changes": [{"op": "create", "entity": "consent", "id": "cns_551", "from": true, "to": false}], "checks": [{"rule": "consent_mandatory", "passed": true, "detail": "marketing_email is optional", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "cns_551", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: `consent.mandatory`, `consent.history`, `consent.marketing_gate`
- 에러: `cannot_revoke_mandatory` → (철회는 탈퇴로만) · `unknown_consent_type` → `list_enums`
- 멱등성: `(customer_id, consent_type, granted)` — 같은 값이면 새 기록 없음.

---

## preference

> **보이고 고칠 수 있는 기억.** 에이전트가 대화에서 기억한 선호는 쇼퍼에게 한 줄("Remembered: weight and packed size matter to you when traveling")로 보이고, 쇼퍼가 **Undo**를 누르면 사람 전용 `POST /memories/{id}/undo`로 지워집니다. 기억은 쇼퍼 본인에 대한 것만 저장하며, **선물 받는 사람의 정보(사이즈·취향·주소)는 저장하지 않습니다.** 결제 정보·건강 정보 같은 민감 정보도 저장하지 않습니다.

### `update_my_preferences` — Kind: W(승인 없음)
- 논리주소/REST: `customer.preference.update_my_preferences` · `POST /v1/customer/people/me/preferences`
- 요약: 대화에서 알게 된 쇼퍼 본인의 선호를 더하거나 뺌. 정책상 승인이 필요 없으므로 프리뷰 후 바로 실행되고, 결과로 쇼퍼에게 보일 기억 한 줄과 되돌리기 id를 돌려줍니다. 쇼퍼 툴, MVP([plan.md §3.4](../plan.md)). schema.org `acom:Preference`.
- consumes → produces: → `memory_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | add | Object[] | | `[{key: "travel", value: "weight and packed size matter", source: "conversation"}]` |
  | remove | Object[] | | `[{key: "layout", value: "detailed lists", source: "conversation"}]` |
  | preview | Boolean | ✓ | `true` 먼저 |
  - 각 항목 `{key, value, source}`. `source`는 MVP에서 `"conversation"`만. `add`·`remove` 중 하나는 있어야 합니다.
- 출력(프리뷰): `{"preview_id": "pv_p12a", "changes": [{"op": "create", "entity": "preference", "id": "mem_12", "from": null, "to": "weight and packed size matter"}], "checks": [{"rule": "preference_about_shopper", "passed": true, "detail": "About the shopper, not a gift recipient", "fix_tool": null}, {"rule": "no_sensitive_data", "passed": true, "detail": "No payment or health data", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행):
  ```jsonc
  {"ok": true, "id": "mem_12",
   "memory": {"memory_id": "mem_12", "text": "Remembered: weight and packed size matter to you when traveling"},
   "undo": {"tool": "POST /memories/mem_12/undo", "until": null}}
  ```
  - `memory.text`는 UI의 `memory_note` 블록에 그대로 쓰입니다. 되돌리기는 쇼퍼 버튼만 부르는 사람 전용 엔드포인트입니다.
- 정책: 기억은 쇼퍼에게 항상 보이고 되돌릴 수 있음, 선물 받는 사람 정보 저장 안 함, 민감 정보 저장 안 함 ([customer.yml](policies/customer.yml) `privacy`)
- 에러: `gift_recipient_data` → (저장하지 않음, 이번 대화에서만 사용) · `sensitive_data` → (저장하지 않음) · `not_signed_in` → (게스트는 기억 없음) · `nothing_to_change` → `get_preferences`
- 멱등성: `(customer_id, key, value)` — 같은 선호를 다시 더해도 하나로 수렴.

### `get_preferences` — Kind: R
- 논리주소/REST: `customer.preference.get` · `GET /v1/customer/people/{customer_id}/preferences`
- 요약: 에이전트가 한 회원에 대해 기억하는 것 전부. 쇼퍼 본인은 `me`로 같은 목록을 봅니다.
- consumes → produces: `customer_id` → `memory_id[]`
- 입력: `customer_id`(✓, 쇼퍼는 `me`), `key?`, `limit`, `offset`
- 출력: `{"items": [{"memory_id": "mem_12", "key": "travel", "value": "weight and packed size matter", "text": "Travels light", "source": "conversation", "created_at": "2026-11-07T14:21:00"}], "total": 2}`
- 정책: 기억은 본인에게 항상 보임
- 에러: `customer_not_found` → `search_members`
- 멱등성: 읽기.

---

## wishlist

### `list_wishlist` — Kind: R
- 논리주소/REST: `customer.wishlist.list` · `GET /v1/customer/people/{customer_id}/wishlist`
- 요약: 관심 상품(지금 가격·재고 반영).
- consumes → produces: `customer_id` → `product_id[]`
- 입력: `customer_id`(✓), `category?`, `limit`, `offset`
- 출력: `{"items": [{"product_id": "prd_field_jacket", "name": "Linen blend field jacket", "price": 128.00, "price_currency": "USD", "availability": "in_stock", "added_at": "2026-10-20T19:02:00"}], "total": 4}`
- 정책: 없음(읽기).
- 에러: `customer_not_found` → `search_members`
- 멱등성: 읽기.

### `add_to_wishlist` — Kind: W
- 논리주소/REST: `customer.wishlist.add` · `POST /v1/customer/people/{customer_id}/wishlist`
- 요약: 관심 상품 추가.
- consumes → produces: `product_id` → `product_id`
- 입력: `customer_id`(✓), `product_id`(✓), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_w01a", "changes": [{"op": "create", "entity": "wishlist_item", "id": "prd_field_jacket", "from": null, "to": "added"}], "checks": [], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "prd_field_jacket", "undo": {"tool": "remove_from_wishlist", "until": null}}`
- 정책: 없음.
- 에러: `product_not_found` → `search_products`
- 멱등성: `(customer_id, product_id)` — 이미 있으면 성공으로 수렴.

### `remove_from_wishlist` — Kind: W
- 논리주소/REST: `customer.wishlist.remove` · `DELETE /v1/customer/people/{customer_id}/wishlist/{product_id}`
- 요약: 관심 상품 삭제.
- consumes → produces: `product_id` → `product_id`
- 입력: `customer_id`(✓), `product_id`(✓), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_w02b", "changes": [{"op": "delete", "entity": "wishlist_item", "id": "prd_field_jacket", "from": "added", "to": null}], "checks": [], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "prd_field_jacket", "undo": {"tool": "add_to_wishlist", "until": null}}`
- 정책: 없음.
- 에러: 없음(없으면 성공으로 간주).
- 멱등성: 삭제 멱등.

---

## employee

### `register_employee` — Kind: W
- 논리주소/REST: `customer.employee.register` · `POST /v1/customer/employees`
- 요약: 임직원 회원 등록. 임직원 할인은 다른 회원 혜택과 함께 쓰지 않습니다(주문당 회원 혜택 1개).
- consumes → produces: `customer_id` → `employee_id`
- 입력: `customer_id`(✓), `employee_number`(✓), `discount_limit`(예 `500.00`/연), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_e01a", "changes": [{"op": "create", "entity": "employee", "id": "emp_041", "from": null, "to": "active"}], "checks": [{"rule": "member_benefits_per_order", "passed": true, "detail": "Employee discount replaces tier coupons", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "emp_041", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: `member_benefits_per_order`
- 에러: `duplicate_employee_number` → `get_member` · `customer_not_found` → `register_member`
- 멱등성: `Idempotency-Key = hash(employee_number)`

### `set_discount_limit` — Kind: W
- 논리주소/REST: `customer.employee.set_discount_limit` · `PATCH /v1/customer/employees/{employee_id}/discount-limit`
- 요약: 임직원 할인 한도(기간·금액).
- consumes → produces: `employee_id` → `employee_id`
- 입력: `employee_id`(✓), `discount_limit`(✓, 예 `500.00`), `period`(✓, `month` \| `year`), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_e02b", "changes": [{"op": "update", "entity": "employee", "id": "emp_041", "from": 500.00, "to": 750.00}], "checks": [], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "emp_041", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: 없음.
- 에러: `employee_not_found` → `register_employee`
- 멱등성: `Idempotency-Key`

---

## inquiry

### `search_inquiries` — Kind: R
- 논리주소/REST: `customer.inquiry.search` · `GET /v1/customer/inquiries`
- 요약: 고객 상담 검색. 기본 가림.
- consumes → produces: → `inquiry_id[]`
- 입력: `inquiry_type`(`general` \| `direct_message` \| `product_question` \| `callback`), `status`(`received` \| `in_progress` \| `complete`), `agent_id`, `order_id`, `created_from`, `created_through`, `limit`, `offset`
- 출력: `{"items": [{"inquiry_id": "inq_771", "inquiry_type": "direct_message", "status": "received", "about": "ord_20402", "created_at": "2026-11-06T09:14:00"}], "total": 18}`
- 정책: `privacy.masking_default`
- 에러: `invalid_filter` → `search_inquiries`
- 멱등성: 읽기.

### `get_inquiry` — Kind: R
- 논리주소/REST: `customer.inquiry.get` · `GET /v1/customer/inquiries/{inquiry_id}`
- 요약: 상담 한 건과 대화 스레드(`Comment[]`), 관련 주문.
- consumes → produces: `inquiry_id` → inquiry
- 입력: `inquiry_id`(✓), `fields?[]`
- 출력: `{"items": [{"inquiry_id": "inq_771", "status": "received", "about": "ord_20402", "thread": [{"author": "customer", "text": "The sole is coming apart.", "at": "2026-11-06T09:14:00"}]}], "total": 1}`
- 정책: `privacy.masking_default`
- 에러: `inquiry_not_found` → `search_inquiries`
- 멱등성: 읽기.

### `create_inquiry` — Kind: W
- 논리주소/REST: `customer.inquiry.create` · `POST /v1/customer/inquiries`
- 요약: 상담 생성(`received`).
- consumes → produces: → `inquiry_id`
- 입력: `inquiry_type`(✓), `text`(✓), `order_id?`, `customer_id`(✓), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_q01a", "changes": [{"op": "create", "entity": "inquiry", "id": "inq_772", "from": null, "to": "received"}], "checks": [], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "inq_772", "undo": {"tool": "close_inquiry", "until": null}}`
- 정책: `customer_service.inquiry_types`
- 에러: `invalid_inquiry_type` → `list_enums`
- 멱등성: `Idempotency-Key`

### `assign_inquiry` — Kind: W
- 논리주소/REST: `customer.inquiry.assign` · `POST /v1/customer/inquiries/{inquiry_id}/assign`
- 요약: 상담원 배정(자동 순환 또는 지정). `received` → `in_progress`.
- consumes → produces: `inquiry_id` → `inquiry_id`
- 입력: `inquiry_id`(✓), `mode`(✓, `auto` \| `manual`), `agent_id`(manual 시 ✓), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_q02b", "changes": [{"op": "update", "entity": "inquiry", "id": "inq_771", "from": "received", "to": "in_progress"}], "checks": [{"rule": "agent_available", "passed": true, "detail": "Dana has 3 open inquiries", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "inq_771", "undo": {"tool": "undo_change_set", "until": "2026-11-06T23:59:00"}}`
- 정책: `customer_service.assignment`
- 에러: `no_available_agent` → `assign_inquiry`(manual)
- 멱등성: `Idempotency-Key`

### `respond_inquiry` — Kind: W$
- 논리주소/REST: `customer.inquiry.respond` · `POST /v1/customer/inquiries/{inquiry_id}/respond`
- 요약: 상담 답변을 기록하고 고객에게 보냄. 고객에게 닿는 쓰기이므로 `send_customer_message`와 같이 항상 승인.
- consumes → produces: `inquiry_id` → `comment_id`
- 입력: `inquiry_id`(✓), `text`(✓), `channel`(✓, `email` \| `in_app`), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_q03c", "changes": [{"op": "create", "entity": "comment", "id": "cmt_902", "from": null, "to": "Sorry about the sole. A refund is being reviewed today."}], "checks": [{"rule": "customer_messages", "passed": true, "detail": "Shown as the customer will read it", "fix_tool": null}], "needs_approval": true, "approval_reason": "Sends a reply to a customer"}`
- 출력(실행): `{"ok": true, "id": "cmt_902", "undo": null}` — 보낸 답변은 되돌릴 수 없음.
- 정책: `customer_messages: always_ask`, `customer_service.notify_member_on_complete`
- 에러: `inquiry_closed` → `create_inquiry` · `inquiry_not_found` → `search_inquiries`
- 멱등성: `Idempotency-Key` — 중복 발송 없음.

### `close_inquiry` — Kind: W
- 논리주소/REST: `customer.inquiry.close` · `POST /v1/customer/inquiries/{inquiry_id}/close`
- 요약: 상담 종료(`in_progress` → `complete`).
- consumes → produces: `inquiry_id` → `inquiry_id`
- 입력: `inquiry_id`(✓), `resolution`, `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_q04d", "changes": [{"op": "update", "entity": "inquiry", "id": "inq_771", "from": "in_progress", "to": "complete"}], "checks": [], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "inq_771", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: `customer_service.states`
- 에러: `invalid_state` → `get_inquiry`
- 멱등성: 종료 재요청은 성공으로 수렴.

---

## remedy

> 보상은 귀책(`store_fault` \| `seller_fault` \| `customer_claim`)을 분류하고 단계별로 승인합니다: 상담원 요청 → 보상 승인자 → (현금이면) 지급 승인자.

### `request_remedy` — Kind: W
- 논리주소/REST: `customer.remedy.request` · `POST /v1/customer/remedies`
- 요약: 보상 요청 생성(`requested`). 돈은 아직 움직이지 않습니다. 반려된 요청은 새 요청으로만 다시 냅니다.
- consumes → produces: `inquiry_id` \| `order_id` → `remedy_id`
- 입력: `inquiry_id` 또는 `order_id`(✓), `remedy_type`(✓, `loyalty_points` \| `coupon` \| `cash`), `liability`(✓), `amount`(✓, 예 `15.00`), `reason`(✓), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_y01a", "changes": [{"op": "create", "entity": "remedy", "id": "rmd_310", "from": null, "to": "requested"}], "checks": [{"rule": "remedy_liability", "passed": true, "detail": "Store fault: late delivery", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "rmd_310", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: `remedy.liability`, `remedy.types`, `remedy.resubmit_on_reject`
- 에러: `invalid_liability` → `list_enums` · `target_not_found` → `search_inquiries`
- 멱등성: `Idempotency-Key = hash(target + remedy_type + amount)`

### `approve_remedy` — Kind: W$
- 논리주소/REST: `customer.remedy.approve` · `POST /v1/customer/remedies/{remedy_id}/approve`
- 요약: 보상 승인(`requested` → `approved`, 현금은 → `payout_pending`). 현금 보상은 지급 승인자 단계가 하나 더 있습니다.
- consumes → produces: `remedy_id` → `remedy_id`
- 입력: `remedy_id`(✓), `decision`(✓, `approve` \| `reject`), `reason`(reject 시 ✓), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_y02b", "changes": [{"op": "update", "entity": "remedy", "id": "rmd_310", "from": "requested", "to": "approved"}], "checks": [{"rule": "remedy_cash", "passed": true, "detail": "Points, no payout step", "fix_tool": null}], "needs_approval": true, "approval_reason": "Approves a $15.00 remedy"}`
- 출력(실행): `{"ok": true, "id": "rmd_310", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: [approval.yml](policies/approval.yml) `remedy_cash`, `remedy.approval_levels`
- 에러: `wrong_approver` → (지급 승인자에게) · `already_decided` → `approve_remedy`
- 멱등성: `Idempotency-Key` — 재시도 시 첫 결정 반환.

### `pay_remedy` — Kind: W$
- 논리주소/REST: `customer.remedy.pay` · `POST /v1/customer/remedies/{remedy_id}/pay`
- 요약: 승인된 보상 지급. 포인트는 `grant_loyalty`, 쿠폰은 `issue_coupon`과 연결되고, 현금은 원결제수단 또는 확인된 계좌로 지급(`payout_pending` → `paid` \| `failed`).
- consumes → produces: `remedy_id` → `remedy_id`
- 입력: `remedy_id`(✓), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_y03c", "changes": [{"op": "update", "entity": "remedy", "id": "rmd_310", "from": "approved", "to": "paid"}], "checks": [{"rule": "remedy_payout_approved", "passed": true, "detail": "Approved by Son SeongJun", "fix_tool": "approve_remedy"}], "needs_approval": true, "approval_reason": "Pays 1,500 points ($15.00)"}`
- 출력(실행): `{"ok": true, "id": "rmd_310", "undo": null}`
- 정책: `remedy.cash_requires`, [approval.yml](policies/approval.yml) `remedy_cash`
- 에러: `payout_not_approved` → `approve_remedy` · `already_paid` → (첫 결과 반환)
- 멱등성: `Idempotency-Key` — 중복 지급 없음.

---

## faq_notice

### `list_faqs` — Kind: R
- 논리주소/REST: `customer.faq_notice.list_faqs` · `GET /v1/customer/faqs`
- 요약: FAQ 목록(schema.org `FAQPage`). 비회원도 가능.
- consumes → produces: → `faq_id[]`
- 입력: `category?`, `query?`, `limit`, `offset`
- 출력: `{"items": [{"faq_id": "faq_returns", "name": "How long do I have to return?", "category": "returns"}], "total": 24}`
- 정책: 없음(읽기).
- 에러: 없음.
- 멱등성: 읽기.

### `get_faq` — Kind: R
- 논리주소/REST: `customer.faq_notice.get_faq` · `GET /v1/customer/faqs/{faq_id}`
- 요약: FAQ 한 건(`Question` + `accepted_answer`).
- consumes → produces: `faq_id` → faq
- 입력: `faq_id`(✓)
- 출력: `{"items": [{"faq_id": "faq_returns", "name": "How long do I have to return?", "accepted_answer": "30 days from delivery."}], "total": 1}`
- 정책: 없음(읽기).
- 에러: `faq_not_found` → `list_faqs`
- 멱등성: 읽기.

### `list_notices` — Kind: R
- 논리주소/REST: `customer.faq_notice.list_notices` · `GET /v1/customer/notices`
- 요약: 공지 목록(schema.org `NewsArticle`).
- consumes → produces: → `notice_id[]`
- 입력: `category?`, `published_from`, `published_through`, `limit`, `offset`
- 출력: `{"items": [{"notice_id": "ntc_holiday", "headline": "Holiday shipping cutoffs", "date_published": "2026-11-02"}], "total": 9}`
- 정책: 없음(읽기).
- 에러: 없음.
- 멱등성: 읽기.

### `get_notice` — Kind: R
- 논리주소/REST: `customer.faq_notice.get_notice` · `GET /v1/customer/notices/{notice_id}`
- 요약: 공지 한 건.
- consumes → produces: `notice_id` → notice
- 입력: `notice_id`(✓)
- 출력: `{"items": [{"notice_id": "ntc_holiday", "headline": "Holiday shipping cutoffs", "article_body": "…", "date_published": "2026-11-02"}], "total": 1}`
- 정책: 없음(읽기).
- 에러: `notice_not_found` → `list_notices`
- 멱등성: 읽기.

### `publish_notice` — Kind: W
- 논리주소/REST: `customer.faq_notice.publish_notice` · `POST /v1/customer/notices/{notice_id}/publish`
- 요약: 공지 게시(`draft` → `published`).
- consumes → produces: `notice_id` → `notice_id`
- 입력: `notice_id`(✓), `date_published`, `pinned`, `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_n01a", "changes": [{"op": "update", "entity": "notice", "id": "ntc_holiday", "from": "draft", "to": "published"}], "checks": [], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "ntc_holiday", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: 없음.
- 에러: `already_published` → `get_notice`
- 멱등성: `Idempotency-Key`

---

## 사람 전용 엔드포인트

회원 본인이 UI 버튼으로만 부르는 HTTP 엔드포인트입니다. **툴이 아니며 모델은 보지 못합니다**([plan.md §3.6](../plan.md), [README.md §2](README.md)).

| 엔드포인트 | 누가 | 무엇 |
|---|---|---|
| `POST /memories/{id}/undo` | 쇼퍼 | 기억한 선호 삭제. `memory_note` 블록의 Undo 버튼. 응답 `{"ok": true, "id": "mem_12"}` |
| `POST /me/withdraw` `{verification_token, reason?}` | 회원 본인 | 탈퇴. 운영자·에이전트는 할 수 없음. 적립금 잔액·진행 중인 주문/클레임/상담이 있으면 거부(`loyalty_balance_positive`, `open_orders`, `open_claims`, `open_inquiries`). 통과하면 개인정보를 즉시 가리고 90일 보관 후 파기, 주문·클레임 기록은 법적 보존 기간 동안 유지 |
| `POST /password-reset` `{verification_token, email, password}` | 회원 본인 | 비밀번호 재설정. `verify_identity` 토큰 필수, 모든 세션 토큰 폐기. 에러 `identity_not_verified`, `password_policy` |

- 공통: `Idempotency-Key` 필수, 에러는 공통 형식. 정책 `withdrawal`, `identity_verification`, `member.password` ([customer.yml](policies/customer.yml)).
