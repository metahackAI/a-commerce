# API 명세 — `customer` (회원·등급·주소·동의·기억(선호)·위시리스트·임직원·상담·보상·FAQ/공지)

> 계층: `customer.{resource}.{operation}`. 공통 규약(응답 형식·에러·프리뷰/실행·멱등·헤더)은 [README.md](README.md) §3, 데이터 타입은 [data-model.md](data-model.md), 정책은 [customer.yml](../../policies/customer.yml) · [approval.yml](../../policies/approval.yml). 식별자·시간·금액·페이지는 [conventions.md](conventions.md), 값 목록은 [enums.md](enums.md), 에러는 [errors.md](errors.md), 데모 스토어의 사람·등급은 [../scenarios.md](../scenarios.md) §2를 따릅니다.
> **설계 축**: ① 개인정보는 기본 가림, 가림 해제는 사유를 액션 로그에 남김, ② 마케팅 발송은 동의(opt-in) 게이트, ③ 탈퇴·비밀번호는 **회원 본인만**(사람 전용 엔드포인트, 툴 아님). 에이전트는 비밀번호를 받지도 보지도 않고, 탈퇴는 자격 확인(읽기)까지만, ④ **등급은 코드가 아니라 데이터** — 등급 이름·개수·요건·혜택은 스토어가 정의, ⑤ **보이고 고칠 수 있는 기억** — 에이전트가 기억한 선호는 쇼퍼에게 한 줄로 보이고 한 번에 되돌릴 수 있음, ⑥ 보상은 귀책 분류 + 단계별 승인(현금은 지급 승인자 별도), ⑦ 동의·탈퇴·보상·등급 이력은 append만, ⑧ 본인 확인은 교체 가능한 방법(일회용 코드·링크·외부 제공자) 목록.

리소스: [person](#person) · [membership](#membership) · [address](#address) · [consent](#consent) · [preference](#preference) · [wishlist](#wishlist) · [employee](#employee) · [inquiry](#inquiry) · [remedy](#remedy) · [faq_notice](#faq_notice) · [사람 전용 엔드포인트](#사람-전용-엔드포인트)

- schema.org: 회원 `Person` + `ProgramMembership`, 등급 프로그램 `MemberProgram`(`has_tiers`), 등급 `MemberProgramTier`(`has_tier_requirement`, `has_tier_benefit`, `membership_points_earned`), 주소 `PostalAddress`, 기억 `acom:Preference`(`PropertyValue`의 확장), 상담 `acom:CustomerInquiry`(`Message`의 확장), 본인 확인 `acom:IdentityVerification`, 등급 평가 `acom:TierEvaluation`.
- id 예: 회원 `cus_jamie`, `cus_alex`, 프로그램 `prg_rewards`, 등급 `vip`(데모 스토어의 tier id, 접두 없음, **예시**), 등급 평가 `tev_2612`, 본인 확인 `ivf_201`, 기억 `mem_12`, 주소 `adr_home`. 응답의 엔티티 자신의 id는 `id`, 다른 엔티티를 가리키는 필드는 `<entity>_id`(`customer_id`, `tier_id`, `claim_id`)입니다.
- 쓰기: 운영자 쓰기는 변경셋에 들어가 실행 응답의 `undo`가 변경셋을 가리킵니다(`{"by": "person", "target": "cs_…", "until": …}`, 기한은 [approval.yml](../../policies/approval.yml) `change_set`). 보낸 메시지·링크·답변과 쇼퍼가 변경셋 밖에서 하는 쓰기는 `undo: null`이고, 기억은 기억 id를 가리킵니다. 반대 동작은 별도 툴입니다(예 `block_member` ↔ `unblock_member`).
- 목록 읽기는 `limit`(기본 20, 최대 100)과 `cursor`를 받고 `{items, total, next_cursor?}`를 돌려줍니다. `get_*`은 목록 봉투 없이 엔티티 하나를 돌려줍니다.

---

## person

### `get_my_profile` — Kind: R
- 논리주소/REST: `customer.person.get_my_profile` · `GET /v1/customer/people/me`
- 요약: 로그인한 쇼퍼 본인의 프로필. 쇼퍼 에이전트가 사이즈·선호·배송지를 알고 답하기 위해 씁니다. 화면의 `profile_card` 블록([chat.md](chat.md) §3.3)이 같은 값을 보여 줍니다. 카드는 끝 4자리만, 그 밖의 결제 정보는 돌려주지 않습니다. 쇼퍼 툴.
- consumes → produces: → `customer_id`, `tier_id`, `address_id[]`, `memory_id[]`
- 입력: 세션의 쇼퍼(다른 사람의 프로필은 읽을 수 없음).
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |---|---|:--:|---|---|
  | fields | Text[] | | 돌려받을 필드만 고름. 값: `id` \| `name` \| `tier_id` \| `sizes` \| `preferences` \| `addresses` \| `card` | `["sizes", "preferences"]` |
- 출력(목록 봉투 없이 엔티티):
  ```jsonc
  {"id": "cus_jamie",
   "name": "Jamie",
   "tier_id": "vip",
   "sizes": {"tops": "M", "bottoms": null, "shoes": null},
   "preferences": [
     {"memory_id": "mem_03", "key": "size", "value": "M", "text": "Size M", "source": "conversation"},
     {"memory_id": "mem_07", "key": "fabric_preference", "value": "natural fabrics", "text": "Natural fabrics", "source": "conversation"},
     {"memory_id": "mem_09", "key": "compares_specs", "value": "yes", "text": "Compares specs", "source": "conversation"},
     {"memory_id": "mem_11", "key": "travel_priority", "value": "travels light", "text": "Travels light", "source": "conversation"}],
   "addresses": [
     {"id": "adr_home", "label": "Home", "street_address": "12 Harbor St, Apt 3", "address_locality": "Brooklyn",
      "address_region": "NY", "postal_code": "11201", "address_country": "US", "is_primary": true}],
   "card": {"label": "Card ending 4821", "last4": "4821"}}
  ```
  | 필드 | 타입 | 설명 |
  |---|---|---|
  | id | Text `^cus_[a-z0-9_]+$` | 쇼퍼 id |
  | name | Text | 표시 이름 |
  | tier_id | Text \| null | 등급 id(`list_tiers`). 등급 프로그램이 없으면 `null` |
  | sizes | `{tops, bottoms, shoes}`, 각 Text \| null | 사이즈. 의류 `XS`–`XL`, 바지 `28`–`36`, 신발 `7`–`12`. 모르면 `null` |
  | preferences[] | `{memory_id, key, value, text, source}` | 기억한 선호. `key`는 `update_my_preferences`의 키 어휘(`preference.keys`), `text`는 쇼퍼에게 보이는 한 줄, `source`는 `conversation` |
  | addresses[] | `PostalAddress` + `{id, label, is_primary}` | 저장된 배송지. `id`는 `adr_…` |
  | card | `{label, last4}` \| null | 저장된 카드. 끝 4자리만 |
- 정책: `privacy.payment_card_shown`(카드는 끝 4자리만), `preference.shown_to_shopper`
- 에러: `not_signed_in`(게스트는 프로필이 없음, 쇼퍼 로그인 필요) → null
- 멱등성: 읽기, 부작용 없음.

### `get_return_history` — Kind: R
- 논리주소/REST: `customer.person.get_return_history` · `GET /v1/customer/people/{customer_id}/returns?days=90`
- 요약: 한 고객이 기간 안에 요청한 반품 클레임 목록과 요약. 운영 에이전트가 반품 요청을 검토할 때 "90일 안에 4번 반품"(#20431, [../scenarios.md](../scenarios.md) B2) 같은 확인에 씁니다. `summary.flagged`는 [fulfillment.yml](../../policies/fulfillment.yml) `return_suggestions.flag_return_history` 기준으로 서버가 계산합니다. 운영자 툴.
- consumes → produces: `customer_id` → `claim_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |---|---|:--:|---|---|
  | customer_id | Text `^cus_[a-z0-9_]+$` | ✓ | 고객 id(`search_return_requests`의 `customer_id`) | `"cus_4431"` |
  | days | Integer 1–365 | | 오늘(스토어 시계)부터 거슬러 셀 일수. 기본은 `return_suggestions.flag_return_history.days`(90) | `90` |
  | limit | Integer 1–100 | | 기본 20 | `20` |
  | cursor | Text | | 이전 응답의 `next_cursor` | `"c_2"` |
- 출력:
  ```jsonc
  {"items": [
     {"id": "clm_20431", "claim_type": "return", "order_id": "ord_20431", "order_number": "20431",
      "item": "Wool overcoat", "amount": {"value": 289.00, "currency": "USD"},
      "reason": "not_as_described", "status": "requested", "requested_at": "2026-11-05T16:40:00-05:00"},
     {"id": "clm_20311", "claim_type": "return", "order_id": "ord_20311", "order_number": "20311",
      "item": "Cashmere sweater", "amount": {"value": 248.00, "currency": "USD"},
      "reason": "changed_mind", "status": "completed", "requested_at": "2026-10-19T12:05:00-04:00"}],
   "total": 4,
   "summary": {"count": 4, "days": 90, "threshold": 4, "flagged": true}}
  ```
  - `reason`은 [enums.md](enums.md) §2.2 `acom:claim_reason`, `status`는 §1.5 `acom:claim_status`.
  - `summary`: `count`는 기간 안의 반품 클레임 수(철회 `withdrawn` 제외), `threshold`는 정책의 `returns`, `flagged`는 `count ≥ threshold`.
- 정책: `return_suggestions.flag_return_history`(90일 안 4회), `privacy.masking_default`
- 에러: `customer_not_found` → `search_members` · `invalid_input`(`days` 범위 밖) → `get_return_history`
- 멱등성: 읽기.

### `search_members` — Kind: R
- 논리주소/REST: `customer.person.search` · `GET /v1/customer/people`
- 요약: 조건으로 회원 검색. 이름·연락처·이메일은 기본 가림.
- consumes → produces: → `customer_id[]`
- 입력: `query`, `type`(`individual` \| `business`), `state`(`active` \| `dormant` \| `blocked` \| `pending_withdrawal` \| `withdrawn`), `tier_id`, `segment_id`, `joined_from`, `joined_through`, `unmask`(Boolean), `access_reason`(Text, `unmask: true`일 때 ✓, 액션 로그에 남음), `fields?[]`, `limit`(기본 20, 최대 100), `cursor?`
- 출력: `{"items": [{"id": "cus_alex", "name": "A*** K***", "email": "a***@example.com", "tier_id": "member", "state": "active"}], "total": 3120, "next_cursor": "c_2"}`
- 정책: `privacy.masking_default`, `privacy.access_reason_logging`
- 에러: `access_reason_required`(`unmask`에는 사유가 필요, 사유와 함께 다시 호출) → `search_members` · `unmask_forbidden` → `list_roles` · `invalid_filter` → `search_members`
- 멱등성: 읽기.

### `get_member` — Kind: R
- 논리주소/REST: `customer.person.get` · `GET /v1/customer/people/{customer_id}`
- 요약: 회원 한 명(회원 정보, `member_of` 등급, 주소·동의 요약). 기본 가림.
- consumes → produces: `customer_id` → person
- 입력: `customer_id`(✓), `unmask`(Boolean), `access_reason`(`unmask: true`일 때 ✓), `fields?[]`
- 출력(목록 봉투 없이 엔티티): `{"id": "cus_alex", "name": "A*** K***", "type": "individual", "member_of": {"program_id": "prg_rewards", "tier_id": "member", "membership_number": "NW-008812"}, "state": "active", "acom:business_verification": null, "consent": {"marketing_email": true}}`
- 정책: `privacy.masking_default`, `privacy.access_reason_logging`
- 에러: `customer_not_found` → `search_members` · `access_reason_required`(가림 해제에는 사유 필요) → `get_member`
- 멱등성: 읽기.

### `register_member` — Kind: W
- 논리주소/REST: `customer.person.register` · `POST /v1/customer/people`
- 요약: 회원 가입(개인·사업자). 본인 확인 토큰과 필수 동의를 함께 받습니다. **비밀번호는 이 툴의 입력이 아닙니다**(입력 스키마는 닫혀 있고 `password` 필드가 없음). 가입이 실행되면 확인된 연락처로 비밀번호 설정 링크가 가고, 회원이 사람 전용 `POST /v1/customer/password-reset/confirm`으로 정합니다(규칙은 `password`). 가입 화면도 같은 순서를 씁니다. 연결 로그인(`linked_login`)만 쓰는 계정은 비밀번호가 없어도 됩니다.
- consumes → produces: → `customer_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | type | Text(enum) | ✓ | `individual` \| `business` |
  | name | Text | ✓ | `"Alex Kim"` |
  | email | Text | ✓ | 로그인 id 겸용 |
  | telephone | Text | | 본인 확인 방법이 `sms_code`면 ✓ |
  | legal_name, tax_id | Text | business | 사업자 상호·사업자 번호 |
  | verification_token | Text | ✓ | `verify_identity` 결과 |
  | consents | Object[] | ✓ | `[{consent_type: "terms", granted: true}, {consent_type: "privacy", granted: true}]` |
- 출력(프리뷰): `{"preview_id": "pv_m01a", "expires_at": "2026-11-07T10:30:00-05:00", "changes": [{"op": "create", "entity": "person", "id": null, "from": null, "to": "active"}], "checks": [{"rule": "identity_verification.required_for", "passed": true, "detail": "Phone verified 2 min ago", "fix_tool": "verify_identity"}, {"rule": "consent.mandatory", "passed": true, "detail": "Terms and privacy accepted", "fix_tool": "set_consent"}, {"rule": "withdrawal.rejoin.block_days", "passed": true, "detail": "No recent withdrawal matches this person", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "cus_8812", "undo": null}` — 새 회원은 프로그램의 entry tier에 들어갑니다(`tier.entry_tier`).
- 정책: `identity_verification.required_for.signup`, `consent.mandatory`, `withdrawal.rejoin`, `withdrawal.identifier_reuse`, `tier.entry_tier`
- 에러: `identity_not_verified` → `verify_identity` · `mandatory_consent_missing` → `set_consent` · `duplicate_member` → `check_availability` · `id_reuse_blocked` → `check_availability` · `rejoin_blocked`(보관 기간 뒤 다시 가입, `detail.until`에 가능 날짜) → null
- 멱등성: `Idempotency-Key = hash(email + telephone)`

### `update_member` — Kind: W
- 논리주소/REST: `customer.person.update` · `PATCH /v1/customer/people/{customer_id}`
- 요약: 회원 정보 수정. 연락처(`telephone`, `email`)를 바꾸면 새 연락처로 본인 확인을 다시 합니다. 비밀번호는 바꾸지 않습니다(사람 전용 `PUT /v1/customer/me/password`).
- consumes → produces: `customer_id` → `customer_id`
- 입력: `customer_id`(✓), `name`, `telephone`, `email`, `verification_token`(연락처 변경 시 ✓)
- 출력(프리뷰): `{"preview_id": "pv_m02b", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "update", "entity": "person", "id": "cus_alex", "from": "a***@example.com", "to": "a***@example.org"}], "checks": [{"rule": "identity_verification.required_for", "passed": true, "detail": "Verified for contact change", "fix_tool": "verify_identity"}], "needs_approval": false, "approval_reason": null, "change_set_id": "cs_1045"}`
- 출력(실행): `{"ok": true, "id": "cus_alex", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: `identity_verification.required_for.contact_change`
- 에러: `reverify_required` → `verify_identity` · `customer_not_found` → `search_members`
- 멱등성: `Idempotency-Key`

### `block_member` — Kind: W$
- 논리주소/REST: `customer.person.block` · `POST /v1/customer/people/{customer_id}/block`
- 요약: 회원 이용 정지(`active`/`dormant` → `blocked`). 고객에게 닿는 제재이므로 항상 사람 승인.
- consumes → produces: `customer_id` → `customer_id`
- 입력: `customer_id`(✓), `reason`(✓), `block_until`(DateTime)
- 출력(프리뷰): `{"preview_id": "pv_m03c", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "update", "entity": "person", "id": "cus_7710", "from": "active", "to": "blocked"}], "checks": [{"rule": "member.block_reason_required", "passed": true, "detail": "Reason recorded: repeated chargebacks", "fix_tool": null}], "needs_approval": true, "approval_reason": "Blocks a member until 2026-12-31", "change_set_id": "cs_1045"}`
- 출력(실행): `{"ok": true, "id": "cus_7710", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}` — 되돌리기 기한이 지난 뒤의 해제는 `unblock_member`.
- 정책: [approval.yml](../../policies/approval.yml) `thresholds.member_block`(항상), `member.states`, `member.block_reason_required`
- 에러: `already_blocked` → `get_member` · `customer_not_found` → `search_members`
- 멱등성: `Idempotency-Key = hash(customer_id + "block")`

### `unblock_member` — Kind: W$
- 논리주소/REST: `customer.person.unblock` · `POST /v1/customer/people/{customer_id}/unblock`
- 요약: 정지 해제(`blocked` → `active`). `block_member`와 같은 승인 규칙(`thresholds.member_block`).
- consumes → produces: `customer_id` → `customer_id`
- 입력: `customer_id`(✓), `reason`(✓)
- 출력(프리뷰): `{"preview_id": "pv_m04d", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "update", "entity": "person", "id": "cus_7710", "from": "blocked", "to": "active"}], "checks": [{"rule": "member.block_reason_required", "passed": true, "detail": "Reason recorded: chargeback resolved", "fix_tool": null}], "needs_approval": true, "approval_reason": "Unblocks a member", "change_set_id": "cs_1045"}`
- 출력(실행): `{"ok": true, "id": "cus_7710", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: [approval.yml](../../policies/approval.yml) `thresholds.member_block`, `member.block_reason_required`
- 에러: `not_blocked` → `get_member`
- 멱등성: `Idempotency-Key = hash(customer_id + "unblock")`

### `verify_identity` — Kind: W
- 논리주소/REST: `customer.person.verify_identity` · `POST /v1/customer/people/verify-identity`
- 요약: 가입·연락처 변경·사업자 확인 전에 하는 본인 확인. 방법은 정책에서 켠 **교체 가능한 방법** 중 하나입니다 — 일회용 코드(이메일·SMS), 매직 링크, 외부 신원 확인 제공자, 사업자 등록 조회. 특정 국가의 신원 체계에 묶이지 않고, 지역별 제공자는 `identity_verification.methods`에 플러그인으로 추가합니다. 성공하면 짧게 유효한 1회용 `verification_token`을 줍니다. 승인 대상이 아니라 신원 확인 절차입니다. 비밀번호 재설정·로그인 id 찾기·탈퇴 재인증도 같은 확인 엔진을 쓰지만 사람 전용 엔드포인트 안에서만 돕니다.
- consumes → produces: → `verification_token`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | purpose | Text(enum) | ✓ | `signup` \| `contact_change` \| `business_verification` |
  | method | Text | ✓ | 정책의 방법 id. 예 `email_code`, `sms_code`, `business_registry` |
  | destination | Text | 코드·링크 방법 | 이메일 또는 전화번호. 예 `"alex@example.com"` |
  | subject | Object | registry_lookup | 예 `{tax_id, legal_name}` |
  | code | Text | 두 번째 호출 | 받은 일회용 코드 `"482913"` |
- 출력(프리뷰): `{"preview_id": "pv_v01a", "expires_at": "2026-11-07T10:30:00-05:00", "changes": [{"op": "create", "entity": "identity_verification", "id": null, "from": null, "to": "code_sent"}], "checks": [{"rule": "identity_verification.required_for", "passed": true, "detail": "email_code is allowed for signup", "fix_tool": "get_policy"}, {"rule": "identity_verification.resend_cooldown_seconds", "passed": true, "detail": "1 code per minute, 5 per hour", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "ivf_201", "undo": null}` — 코드 확인 후 `{verification_token, expires_in: 300}`. `verification_token`은 이 확인의 id(`ivf_…`)입니다. 그 자체로는 비밀이 아니고, 확인한 목적·연락처에 한 번, 기한 안에만 쓰입니다(코드는 해시로만 저장, 출력·액션 로그에 남지 않음). 이메일·SMS 발송은 messaging에 연결되지 않아 지금은 스텁이며, 실행 결과의 `delivery`가 `delivered: false`로 그 사실을 말합니다. 사업자 등록 조회도 연결된 제공자가 없어 스텁입니다(`registry.stub: true`). 외부 제공자 방법이면 `{"ok": true, "id": "ivf_202", "redirect_url": "…"}`를 돌려주고, 제공자 콜백은 integration 도메인이 받아 같은 `ivf` id를 완료합니다.
- 정책: `identity_verification.methods`, `identity_verification.required_for`, `identity_verification.code`, `identity_verification.resend_cooldown_seconds`
- 에러: `method_not_enabled` → `get_policy` · `code_mismatch` → `verify_identity` · `verification_expired` → `verify_identity` · `too_many_attempts`(이 확인의 시도를 다 씀, 새 확인을 시작) → `verify_identity` · `rate_limited`(`detail.retry_after_seconds` 뒤에 다시) → null
- 멱등성: 코드 발송은 `(destination, resend_cooldown_seconds)` 제한, 토큰은 1회용.

### `check_availability` — Kind: R
- 논리주소/REST: `customer.person.check_availability` · `GET /v1/customer/people/availability?field=email&value=…`
- 요약: 가입 전 이메일·로그인 id 사용 가능 여부. 비회원도 호출 가능.
- consumes → produces: → availability
- 입력: `field`(✓, `email` \| `login_id`), `value`(✓)
- 출력(목록 봉투 없이 결과 하나): `{"field": "email", "value": "alex@example.com", "available": false, "reason": "taken"}` — `reason`: `taken` \| `reserved`(탈퇴 후 보관 기간 포함) \| `invalid_format`. 탈퇴한 계정의 것인지는 드러내지 않습니다.
- 정책: `withdrawal.identifier_reuse`
- 에러: `invalid_format` → `check_availability`
- 멱등성: 읽기.

### `verify_business` — Kind: W$
- 논리주소/REST: `customer.person.verify_business` · `POST /v1/customer/people/{customer_id}/verify-business`
- 요약: 사업자 회원의 사업자 번호 확인. 확인되면 사업자 가격·세금 영수증 발급 자격이 켜지므로 사람 승인을 거칩니다.
- consumes → produces: `customer_id` → `customer_id`
- 입력: `customer_id`(✓), `tax_id`(✓), `legal_name`(✓), `registration_document`(MediaObject)
- 출력(프리뷰): `{"preview_id": "pv_b01c", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "update", "entity": "person", "id": "cus_biz_31", "from": {"acom:business_verification": "unverified"}, "to": {"acom:business_verification": "verified"}}], "checks": [{"rule": "business_verification.match_fields", "passed": true, "detail": "Tax id and legal name match the registry", "fix_tool": null}], "needs_approval": true, "approval_reason": "Turns on business pricing and tax receipts", "change_set_id": "cs_1045"}`
- 출력(실행): `{"ok": true, "id": "cus_biz_31", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: [approval.yml](../../policies/approval.yml), `business_verification`, `identity_verification.required_for.business_verification`. 세금 영수증 발급 자체는 sales 도메인의 영수증 툴이 합니다.
- 에러: `registry_mismatch` → `verify_business` · `already_verified` → `get_member` · `not_business_member` → `get_member`
- 멱등성: `Idempotency-Key = hash(customer_id + tax_id)`

### `check_withdrawal_eligibility` — Kind: R
- 논리주소/REST: `customer.person.check_withdrawal_eligibility` · `GET /v1/customer/people/me/withdrawal-eligibility` (운영자: `GET /v1/customer/people/{customer_id}/withdrawal-eligibility`)
- 요약: 탈퇴하기 전에 **막는 것, 없어지는 것, 남는 것**을 한 번에 돌려줍니다. 쇼퍼 에이전트는 "탈퇴하면 어떻게 되나요?"에 이 결과로 답하고, 탈퇴 화면도 같은 결과를 보여 준 뒤 회원이 사람 전용 `POST /v1/customer/me/withdrawal`을 누릅니다. 운영 에이전트는 상담 응대를 위해 `customer_id`로 읽을 수 있지만(기본 가림) **탈퇴를 실행할 수는 없습니다**. 막힌 이유마다 해결할 다음 툴을 붙입니다.
- consumes → produces: (`customer_id`) → eligibility
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | customer_id | Text | | 기본 `me`(쇼퍼 본인). 운영자만 다른 id |
  | fields | Text[] | | 응답 축소. 예 `["eligible", "blockers"]` |
- 출력(목록 봉투 없이 결과 하나):
  ```jsonc
  {"customer_id": "cus_jamie", "eligible": false, "checked_at": "2026-11-07T14:30:00-05:00",
   "blockers": [
     {"type": "open_order", "id": "ord_20518", "detail": "Arrives Tue Nov 10; returns stay open 14 days after delivery", "clears_at": "2026-11-24T23:59:00-05:00", "next_tool": "get_order"},
     {"type": "open_inquiry", "id": "inq_771", "detail": "Inquiry still in progress", "clears_at": null, "next_tool": "get_inquiry"},
     {"type": "loyalty_points", "id": null, "detail": "12.40 USD in points must be used or expire first", "amount": {"value": 12.40, "currency": "USD"}, "next_tool": "get_loyalty_balance"}],
   "on_withdrawal": [
     {"type": "store_credit", "detail": "25.00 USD is paid back to the original payment method", "amount": {"value": 25.00, "currency": "USD"}, "action": "pay_out"},
     {"type": "coupons", "detail": "3 unused coupons are voided", "count": 3, "action": "void"},
     {"type": "linked_logins", "detail": "Sign-in with an external account is unlinked", "action": "unlink"},
     {"type": "active_subscriptions", "id": "sub_coffee", "detail": "Ends after the current period (Dec 1)", "action": "cancel_at_period_end"},
     {"type": "tier", "detail": "VIP status ends and does not carry over", "action": "remove"}],
   "data_handling": {
     "deleted_now": ["addresses", "preferences", "wishlist", "refund_accounts", "device_tokens", "linked_login_ids"],
     "masked_now": ["name", "telephone", "email", "birth_date"],
     "authored_content": "anonymize",
     "retained": [
       {"data": "order_and_payment_records", "days": 1825, "why": "tax_and_consumer_protection"},
       {"data": "claims_and_inquiries", "days": 1095, "why": "dispute_handling"},
       {"data": "consent_history", "days": 1825, "why": "proof_of_consent"},
       {"data": "rejoin_check_fingerprint", "days": 90, "why": "prevent_signup_benefit_abuse"}]},
   "rejoin_after_days": 90, "login_id_reusable": "after_hold",
   "cooling_off_days": 0,
   "reauthentication_methods": ["password", "email_code"]}
  ```
  - `blockers[].type`·`on_withdrawal[].type`은 정책 키 이름입니다: `withdrawal.blockers`의 `open_order` \| `open_claim` \| `open_inquiry` \| `pending_refund` \| `pending_remedy`, `withdrawal.balances`의 `loyalty_points` \| `store_credit`, `withdrawal.on_withdraw`의 `coupons` \| `active_subscriptions` \| `linked_logins` \| `employee_benefits` \| `tier`. 어떤 항목이 막는지(`block`)·알리기만 하는지(`warn`)·처리하는지(`forfeit`, `pay_out`, `void` …)는 정책이 정합니다.
  - `checked_at`은 탈퇴 요청의 `confirmation.eligibility_checked_at`에 그대로 씁니다.
- 정책: `withdrawal.blockers`, `withdrawal.balances`, `withdrawal.on_withdraw`, `withdrawal.data_handling`, `withdrawal.rejoin`, `withdrawal.identifier_reuse`, `withdrawal.cooling_off_days`, `privacy.masking_default`
- 에러: `not_signed_in`(쇼퍼 로그인 필요) → null · `already_withdrawn`(이미 탈퇴한 회원) → null · `customer_not_found` → `search_members`
- 멱등성: 읽기, 부작용 없음.

### `send_password_reset` — Kind: W$
- 논리주소/REST: `customer.person.send_password_reset` · `POST /v1/customer/people/{customer_id}/password-reset`
- 요약: 상담 중 회원이 로그인하지 못할 때, 운영 에이전트가 **회원의 확인된 연락처로** 재설정 링크(또는 코드)를 보냅니다. 회원에게 닿는 쓰기이므로 항상 승인. 임시 비밀번호를 만들지 않고, 새 연락처로 보낼 수 없으며, 비밀번호를 보거나 정하지 않습니다. 회원은 받은 링크로 사람 전용 `POST /v1/customer/password-reset/confirm`을 거칩니다. 로그인 잠금은 재설정이 끝나면 풀립니다.
- consumes → produces: `customer_id` → `recovery_request_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | customer_id | Text | ✓ | `"cus_alex"` |
  | method | Text | | `account_recovery`에서 허용한 방법 id. 기본은 `identity_verification.required_for.password_reset`의 첫 항목 |
  | reason | Text | ✓ | `"Locked out after 5 attempts, asked by phone"` |
- 출력(프리뷰): `{"preview_id": "pv_r01a", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "send", "entity": "recovery_request", "id": null, "from": null, "to": "link_sent"}], "checks": [{"rule": "account_recovery.verified_contact_only", "passed": true, "detail": "Sends to a***@example.com, verified 2026-04-02", "fix_tool": null}, {"rule": "account_recovery.rate_limits", "passed": true, "detail": "1 of 3 requests this hour", "fix_tool": null}], "needs_approval": true, "approval_reason": "Sends a password reset link to a customer", "change_set_id": "cs_1045"}`
- 출력(실행): `{"ok": true, "id": "rcv_88", "undo": null}` — 보낸 링크는 되돌릴 수 없고, 새 요청이 오면 이전 링크는 무효.
- 정책: `account_recovery.operator_initiated`, `account_recovery.verified_contact_only`, `account_recovery.rate_limits`, `password.lockout.unlock_by`
- 에러: `no_verified_contact` → `update_member` · `rate_limited`(`detail.retry_after_seconds` 뒤에 다시) → null · `member_not_active` → `get_member` · `customer_not_found` → `search_members`
- 멱등성: `Idempotency-Key = hash(customer_id + "password_reset" + hour)`

---

## membership

> **등급은 데이터입니다.** 스토어는 `MemberProgram`을 하나 이상 두고, 각 프로그램의 `has_tiers`에 등급(`MemberProgramTier`)을 원하는 만큼 둡니다. 등급 이름·개수·요건·혜택은 코드나 enum이 아니라 `define_tier_program`으로 저장한 정의입니다. 다른 도메인은 tier id만 참조합니다(`valid_for_member_tier`). 이 절의 `member`·`gold`·`vip`와 회원 수는 데모 스토어의 값([../scenarios.md](../scenarios.md) §2.1)이고, 요건 금액은 **예시**입니다.

**정의 형식** (`get_tier`가 돌려주고 `define_tier_program`이 받는 모양, 값은 데모 스토어 예시):
```jsonc
{"id": "prg_rewards", "name": "Northwind Rewards", "version": 4,
 "evaluation": {"schedule": {"every": "month", "on_day": 1, "at": "03:00"}, "on_event": ["order_delivered"], "settle_days": 14},
 "movement": {"upgrade": "immediate", "downgrade": "at_schedule", "max_steps_down": 1, "grace_periods": 1},
 "has_tiers": [
   {"id": "member", "name": "Member", "rank": 1, "has_tier_requirement": [],
    "has_tier_benefit": [{"type": "recurring_coupon", "coupon_id": "cpn_birthday", "every": "year"}]},
   {"id": "gold", "name": "Gold", "rank": 2,
    "requirement_combine": "any",
    "has_tier_requirement": [{"metric": "spend", "threshold": {"value": 300.00, "currency": "USD"},
                              "window": {"type": "rolling", "length": 6, "unit": "month"},
                              "counts": {"channels": ["web", "app"], "order_status": "delivered", "net_of_returns": true, "include_loyalty_spent": true}}],
    "has_tier_benefit": [{"type": "recurring_coupon", "coupon_id": "cpn_gold_10", "every": "month"}]},
   {"id": "vip", "name": "VIP", "rank": 3,
    "requirement_combine": "any",
    "has_tier_requirement": [{"metric": "spend", "threshold": {"value": 800.00, "currency": "USD"}, "retain_threshold": {"value": 600.00, "currency": "USD"}},
                             {"metric": "invite"}],
    "has_tier_benefit": [{"type": "member_price"}]}]}
```
- 데모 스토어의 상시 혜택: Member는 생일 쿠폰, Gold는 등급 쿠폰 "Gold tier 10%", VIP는 상시 혜택 없이 VIP 주말 세일 같은 한정 회원가(`member_price`)의 대상입니다.

| 요소 | 필드 | 설명 |
|---|---|---|
| 등급 | `id`, `name`, `rank` | id는 스토어가 정한 단어(접두 없음, `everyone`은 예약어라 쓸 수 없음)이고 스토어 안에서 유일. `rank`가 클수록 위, 중복 금지(`tier.rank`). 요건 없는 entry tier(최하위) 1개 필수(`tier.entry_tier`) |
| 요건 `has_tier_requirement[]` | `metric`([enums.md](enums.md) §1.13) | `spend` \| `order_count` \| `points_earned` \| `manual`(set_member_tier로만) \| `invite`(초대 수락) \| `custom`(`custom_metric`: analytics 지표 id) |
| | `threshold`, `retain_threshold` | spend는 `{value, currency}`, 나머지는 수. 같은 지표면 rank가 높을수록 커야 함(`tier.requirement.higher_rank_needs_more`). 유지 기준을 진입 기준보다 낮게 두면 경계에서 오르내림이 줄어듦 |
| | `window` | `{type: rolling \| calendar \| lifetime, length, unit}` |
| | `counts` | `channels[]`, `order_status`, `net_of_returns`, `include_loyalty_spent` — 실적에 무엇을 셀지 |
| | `requirement_combine` | 요건 여러 개를 `any` \| `all` |
| 혜택 `has_tier_benefit[]` | `type`([enums.md](enums.md) §1.13) | `member_price`(이 tier가 `valid_for_member_tier` 대상) \| `points_multiplier`(`value`) \| `free_shipping`(`min_subtotal?`) \| `free_returns` \| `coupon_on_entry`(`coupon_id`, `quantity`) \| `recurring_coupon`(`coupon_id`, `every`) \| `custom`(`name`, `description`) |
| 적립 | `membership_points_earned` | `{value, unit_text}`. `points_multiplier` 혜택과 같은 값 |
| 프로그램 | `evaluation`, `movement` | 평가 일정(정기·이벤트), 실적 확정 대기, 승급·강등·유예 규칙. 생략하면 [customer.yml](../../policies/customer.yml) `tier`의 기본값 |

schema.org 대응: `member_price` → `TierBenefitLoyaltyPrice`, `points_multiplier` → `TierBenefitLoyaltyPoints`, `free_shipping` → `TierBenefitLoyaltyShipping`, `free_returns` → `TierBenefitLoyaltyReturns`, 나머지는 `acom:`.

### `list_tiers` — Kind: R
- 논리주소/REST: `customer.membership.list_tiers` · `GET /v1/customer/tiers`
- 요약: 스토어가 정의한 등급과 등급별 회원 수. 운영 에이전트가 "VIP members" 같은 말을 tier id로 바꿔 `set_sale_prices`·`create_coupon`·`schedule_banner`의 `valid_for_member_tier`로 넘길 때 씁니다. 몇 개의 등급이 있든 같은 모양으로 돌려줍니다. schema.org `MemberProgramTier`.
- consumes → produces: → `tier_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |---|---|:--:|---|---|
  | program_id | Text `^prg_[a-z0-9_]+$` | | 이 프로그램의 등급만. 없으면 스토어의 모든 프로그램 | `"prg_rewards"` |
  | fields | Text[] | | 돌려받을 필드만. 값: `id` \| `name` \| `rank` \| `program_id` \| `member_count` | `["id", "member_count"]` |
  | limit | Integer 1–100 | | 기본 20 | `20` |
  | cursor | Text | | 이전 응답의 `next_cursor` | `"c_2"` |
- 출력(데모 스토어, [../scenarios.md](../scenarios.md) §2.1):
  ```jsonc
  {"items": [{"id": "vip", "name": "VIP", "rank": 3, "program_id": "prg_rewards", "member_count": 1284},
             {"id": "gold", "name": "Gold", "rank": 2, "program_id": "prg_rewards", "member_count": 2900},
             {"id": "member", "name": "Member", "rank": 1, "program_id": "prg_rewards", "member_count": 9816}],
   "total": 3}
  ```
  | 필드 | 타입 | 설명 |
  |---|---|---|
  | id | Text | 등급 id. 다른 도메인의 `valid_for_member_tier`·`tier_id`에 그대로 씀 |
  | name | Text | 사람에게 보이는 이름 |
  | rank | Integer ≥ 1 | 클수록 위. 응답은 `rank` 내림차순 |
  | program_id | Text | 속한 등급 프로그램 |
  | member_count | Integer | 지금 이 등급인 회원 수(탈퇴·차단 제외) |
  - 프로그램이 없으면 `{"items": [], "total": 0}`.
- 정책: `tier.tier_id_scope`, `tier.rank`
- 에러: `program_not_found` → `list_tiers`(`program_id` 없이 다시)
- 멱등성: 읽기.

### `get_tier` — Kind: R
- 논리주소/REST: `customer.membership.get_tier` · `GET /v1/customer/tiers/{tier_id}`
- 요약: 등급 하나의 전체 정의(요건·혜택·적립)와 속한 프로그램의 평가·이동 규칙, 회원 수, 이 등급을 가리키는 진행 중 프로모션·쿠폰 수. `define_tier_program`으로 바꾸기 전에 지금 값을 읽을 때 씁니다.
- consumes → produces: `tier_id` → tier definition
- 입력: `tier_id`(✓, 예 `"vip"`), `fields?[]`
- 출력(목록 봉투 없이 엔티티, 예시): `{"id": "vip", "name": "VIP", "rank": 3, "program": {"id": "prg_rewards", "version": 4, "evaluation": {"schedule": {"every": "month", "on_day": 1}}, "movement": {"downgrade": "at_schedule", "grace_periods": 1}}, "has_tier_requirement": [{"metric": "spend", "threshold": {"value": 800.00, "currency": "USD"}, "window": {"type": "rolling", "length": 6, "unit": "month"}}], "has_tier_benefit": [{"type": "member_price"}], "member_count": 1284, "referenced_by": {"active_promotions": 1, "active_coupons": 1, "display_targeting": 1}}` — `referenced_by`는 이 등급을 가리키는 진행 중·예정 프로모션·쿠폰·진열 대상 수(`tier.program_change.block_if_referenced_by`). 예시는 #1042 승인 뒤
- 정책: `tier`
- 에러: `tier_not_found` → `list_tiers`
- 멱등성: 읽기.

### `get_member_tier` — Kind: R
- 논리주소/REST: `customer.membership.get_member_tier` · `GET /v1/customer/people/{customer_id}/tier`
- 요약: 회원 한 명의 현재 등급, **왜 그 등급인지**, 다음 등급까지 얼마나 남았는지, 유지가 위태로운지, 다음 평가일. 쇼퍼는 `customer_id: "me"`로 자기 것을 봅니다(예: "Gold까지 300.00 USD 남았어요").
- consumes → produces: `customer_id` → `tier_id`
- 입력: `customer_id`(✓, 쇼퍼는 `me`), `program_id?`
- 출력(목록 봉투 없이 결과 하나, 예시 — Alex):
  ```jsonc
  {"customer_id": "cus_alex", "program_id": "prg_rewards", "tier_id": "member", "rank": 1, "since": "2025-12-10",
   "source": "entry",                       // evaluation | event | manual_override | entry
   "override": null,                        // 수동 지정이면 {reason, expires_at, after_expiry, by}
   "why": {"rule": "Entry tier, no requirement", "value": null, "evaluation_id": null},
   "progress": [{"to_tier_id": "gold", "metric": "spend",
                 "current": {"value": 0.00, "currency": "USD"}, "threshold": {"value": 300.00, "currency": "USD"},
                 "remaining": {"value": 300.00, "currency": "USD"},
                 "pending": {"value": 312.00, "currency": "USD", "detail": "Order 20402 counts after Nov 14 unless it is returned"}}],
   "retention": {"status": "on_track", "retain_threshold": null},   // on_track | at_risk | in_grace
   "next_evaluation": "2026-12-01T03:00:00-05:00",
   "has_tier_benefit": [{"type": "recurring_coupon", "coupon_id": "cpn_birthday", "every": "year"}]}
  ```
  - `pending`은 아직 실적에 들어가지 않은 금액입니다(`tier.evaluation.settle_days`, 반품 요청 중이면 반품 결과에 따라 빠짐).
- 정책: `tier.requirement`, `tier.evaluation`, `tier.movement`
- 에러: `customer_not_found` → `search_members` · `tier_not_found`(게스트·탈퇴 회원은 등급이 없음) → null
- 멱등성: 읽기.

### `define_tier_program` — Kind: W$
- 논리주소/REST: `customer.membership.define_tier_program` · `PUT /v1/customer/tier-programs/{program_id}` (없으면 만들고 있으면 새 버전)
- 요약: 등급 프로그램과 등급을 만들거나 바꿉니다(등급 추가·삭제·이름·순서·요건·혜택·평가 일정·이동 규칙). 회원의 등급과 혜택이 바뀌므로 항상 승인. 프리뷰는 **지금 데이터로 평가했을 때 회원이 어느 등급에서 어느 등급으로 몇 명 움직이는지**를 `changes`에 보여 줍니다. 정의는 버전으로 남고 되돌릴 수 있습니다.
- consumes → produces: (`program_id`, `coupon_id`) → `program_id`, `tier_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | program_id | Text | ✓ | `"prg_rewards"` |
  | name | Text | ✓ | `"Northwind Rewards"`(예시) |
  | has_tiers | Object[] | ✓ | 위 정의 형식의 등급 전체 목록. 빠진 등급은 삭제로 봄 |
  | evaluation | Object | | 생략 시 `tier.evaluation` 기본값 |
  | movement | Object | | 생략 시 `tier.movement` 기본값 |
  | has_tiers[].has_tier_requirement[] | Object[] | | `{metric, threshold?, threshold_count?, retain_threshold?, retain_threshold_count?, window?, counts?, custom_metric?}`. 툴 입력은 닫힌 스키마라 금액 기준(`spend`)은 `threshold`(`{value, currency}`), 수 기준(`order_count`, `points_earned`)은 `threshold_count`로 나눠 받고, 저장·`get_tier` 출력은 위 정의 형식처럼 `threshold` 하나입니다 |
  | tier_migrations | Object[] | 등급 삭제 시 ✓ | `[{from_tier_id: "silver", to_tier_id: "gold"}]` — 삭제할 등급의 회원이 갈 곳 |
  | takes_effect | Text(enum) | | `next_evaluation`(기본) \| `immediately`(승인과 함께 평가 실행) |
- 출력(프리뷰, 예시 — VIP 기준을 800 → 1000 USD로 올리고 Platinum 추가):
  ```jsonc
  {"preview_id": "pv_t01a", "expires_at": "2026-11-06T11:30:00-05:00",
   "changes": [{"op": "update", "entity": "member_program_tier", "id": "vip", "from": {"spend": {"value": 800.00, "currency": "USD"}}, "to": {"spend": {"value": 1000.00, "currency": "USD"}}},
               {"op": "create", "entity": "member_program_tier", "id": "platinum", "from": null, "to": {"rank": 4}},
               {"op": "update", "entity": "member_tier", "id": null, "from": "vip", "to": "gold", "count": 212},
               {"op": "update", "entity": "member_tier", "id": null, "from": "vip", "to": "platinum", "count": 96}],
   "checks": [{"rule": "tier.entry_tier", "passed": true, "detail": "member is the entry tier", "fix_tool": null},
              {"rule": "tier.rank", "passed": true, "detail": "Ranks 1–4, no duplicates", "fix_tool": null},
              {"rule": "tier.requirement.higher_rank_needs_more", "passed": true, "detail": "Higher ranks need more spend", "fix_tool": "define_tier_program"},
              {"rule": "tier.program_change.block_if_referenced_by", "passed": true, "detail": "No removed tier is used by promotions or coupons", "fix_tool": "search_promotions"},
              {"rule": "tier.program_change.benefit_coupons_active", "passed": true, "detail": "cpn_platinum_welcome is active", "fix_tool": "create_coupon"},
              {"rule": "tier.program_change.preview_impact", "passed": true, "detail": "212 move down to gold and 96 up to platinum at the next evaluation (Dec 1); 14 manual overrides stay", "fix_tool": "preview_tier_evaluation"}],
   "needs_approval": true,
   "approval_reason": "Changes the VIP requirement and adds a tier; 308 members would change tier",
   "change_set_id": "cs_1045"}
  ```
  - 여러 회원의 등급 이동처럼 한 줄로 묶은 변경은 `id: null`과 `count`를 씁니다.

- 출력(실행): `{"ok": true, "id": "prg_rewards", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}` — 되돌리면 이전 버전 정의로 돌아갑니다. 이미 실행된 평가는 `run_tier_evaluation`의 되돌리기로.
- 정책: `tier.program_change`, `tier.programs_max`, `tier.tiers_max`, `tier.entry_tier`, `tier.rank`, `tier.requirement.higher_rank_needs_more`, [approval.yml](../../policies/approval.yml)
- 에러: `tier_rank_conflict` → `define_tier_program` · `entry_tier_missing` → `define_tier_program` · `tier_in_use` → `search_promotions` · `migration_required` → `define_tier_program`(`tier_migrations`와 함께) · `coupon_not_found` → `create_coupon` · `tier_limit_exceeded` → `list_tiers`
- 멱등성: `Idempotency-Key = hash(program_id + 정의)` — 같은 정의면 새 버전 없음.

### `preview_tier_evaluation` — Kind: R
- 논리주소/REST: `customer.membership.preview_tier_evaluation` · `GET /v1/customer/tier-programs/{program_id}/evaluation-preview`
- 요약: 지금(또는 `as_of` 시점) 평가하면 누가 오르고 내리고 유예에 들어가는지. 아무것도 바꾸지 않습니다. `group_by: "move"`면 이동 경로별 인원만.
- consumes → produces: `program_id` → `customer_id[]`
- 입력: `program_id`(✓), `as_of`(DateTime, 기본 지금), `scope`(`all` \| `segment` \| `customers`, 기본 `all`), `segment_id`(`scope: segment`일 때 ✓), `customer_ids[]`(`scope: customers`일 때 ✓), `direction`(`up` \| `down` \| `grace` \| `any`), `group_by`(`member`(기본) \| `move`), `fields?[]`, `limit`(기본 20, 최대 100), `cursor?`
- 출력(예시): `{"items": [{"customer_id": "cus_4431", "name": "M*** R***", "from_tier_id": "gold", "to_tier_id": "vip", "direction": "up", "reason": "spend 812.40 >= 800.00 USD over 6 months", "benefits_on_entry": []}, {"customer_id": "cus_7710", "name": "R*** T***", "from_tier_id": "vip", "to_tier_id": "vip", "direction": "grace", "reason": "spend 540.00 below 600.00 retain threshold; 1 grace period left"}], "total": 518, "next_cursor": "c_2"}` — `group_by: "move"`면 `{"items": [{"from_tier_id": "gold", "to_tier_id": "vip", "direction": "up", "members": 310}], "total": 4}`
- 정책: `tier.evaluation.settle_days`, `tier.movement`, `tier.override.evaluation_respects_override`, `privacy.masking_default`
- 에러: `program_not_found` → `list_tiers` · `as_of_in_future` → `preview_tier_evaluation` · `invalid_input`(`scope`에 맞는 필드가 없음) → `preview_tier_evaluation`
- 멱등성: 읽기.

### `run_tier_evaluation` — Kind: W$
- 논리주소/REST: `customer.membership.run_tier_evaluation` · `POST /v1/customer/tier-programs/{program_id}/evaluations`
- 요약: 평가를 실제로 적용합니다 — 등급을 옮기고, 진입 혜택(쿠폰 등)을 발급하고, messaging으로 회원에게 알립니다. 정기·이벤트 평가는 승인된 프로그램 정의를 승인으로 삼아 스케줄러가 돌리고(`tier.evaluation.scheduled_runs`), 이 툴은 일정 밖에서 돌리는 임시 실행이라 항상 승인입니다.
- consumes → produces: `program_id` → `evaluation_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | program_id | Text | ✓ | `"prg_rewards"` |
  | as_of | DateTime | | 기본 지금. 미래 불가 |
  | scope | Text(enum) | | `all`(기본) \| `segment` \| `customers` |
  | segment_id | Text | `scope: segment` | `"seg_lapsed_gold"` |
  | customer_ids | Text[] | `scope: customers` | `["cus_alex"]` |
  | notify | Boolean | | 기본 `tier.notify.on`대로 |
- 출력(프리뷰, 예시):
  ```jsonc
  {"preview_id": "pv_t02b", "expires_at": "2026-11-06T11:30:00-05:00",
   "changes": [{"op": "update", "entity": "member_tier", "id": null, "from": "gold", "to": "vip", "count": 310},
               {"op": "update", "entity": "member_tier", "id": null, "from": "vip", "to": "gold", "count": 212},
               {"op": "grant", "entity": "member_coupon", "id": null, "from": null, "to": "cpn_vip_welcome", "count": 310},
               {"op": "send", "entity": "message", "id": null, "from": null, "to": "tpl_tier_change", "count": 522}],
   "checks": [{"rule": "tier.evaluation.settle_days", "passed": true, "detail": "Orders delivered after Oct 23 count next cycle", "fix_tool": null},
              {"rule": "tier.override.evaluation_respects_override", "passed": true, "detail": "14 manual overrides left as they are", "fix_tool": null},
              {"rule": "tier.evaluation.one_run_at_a_time", "passed": true, "detail": "No other evaluation is running", "fix_tool": null},
              {"rule": "tier.notify.message_kind", "passed": true, "detail": "Tier notices are transactional; promotional wording would need marketing consent", "fix_tool": "create_message_template"},
              {"rule": "change_set.irreversible", "passed": true, "detail": "522 notices cannot be unsent; tier moves and coupons can be undone", "fix_tool": null}],
   "needs_approval": true,
   "approval_reason": "Moves 310 members up and 212 down, issues 310 coupons, sends 522 messages",
   "change_set_id": "cs_1045"}
  ```

- 출력(실행): `{"ok": true, "id": "tev_2612", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}` — 되돌리면 등급을 되돌리고 이 실행이 발급한 미사용 쿠폰을 회수합니다. 보낸 메시지는 되돌릴 수 없습니다.
- 정책: `tier.evaluation`, `tier.movement`, `tier.override`, `tier.notify`, `tier.history`, `consent.marketing_gate`, [approval.yml](../../policies/approval.yml) `change_set.irreversible`
- 에러: `evaluation_in_progress` → `list_tier_history` · `program_not_found` → `list_tiers` · `as_of_in_future` → `run_tier_evaluation` · `coupon_unavailable` → `create_coupon`
- 멱등성: `Idempotency-Key = hash(program_id + as_of + scope)` — 같은 시점 재실행은 첫 결과 반환.

### `set_member_tier` — Kind: W$
- 논리주소/REST: `customer.membership.set_member_tier` · `POST /v1/customer/people/{customer_id}/tier`
- 요약: 회원 한 명의 등급을 손으로 지정합니다(보상·초대·오류 정정). 사유와 만료가 반드시 있고, 만료 전에는 평가가 바꾸지 않습니다. `metric: manual`인 등급은 이 툴로만 들어갑니다. `clear_override: true`면 수동 지정을 지우고 다음 평가(또는 즉시 재평가)로 돌아갑니다.
- consumes → produces: `customer_id`, `tier_id` → `customer_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | customer_id | Text | ✓ | `"cus_alex"` |
  | tier_id | Text | ✓(해제 시 제외) | `"gold"`(`list_tiers`) |
  | reason | Text | ✓ | `"Goodwill after a defective pair of boots, inq_771"` |
  | expires_at | DateTime | | 기본 지금 + `tier.override.max_days`, 그보다 길 수 없음 |
  | after_expiry | Text(enum) | | `re_evaluate`(기본) \| `revert_to_previous` |
  | issue_entry_benefits | Boolean | | 기본 `false` |
  | clear_override | Boolean | | `true`면 수동 지정 해제 |
- 출력(프리뷰): `{"preview_id": "pv_t03c", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "update", "entity": "member_tier", "id": "cus_alex", "from": "member", "to": "gold"}], "checks": [{"rule": "tier.override.reason_required", "passed": true, "detail": "Reason recorded", "fix_tool": null}, {"rule": "tier.override.max_days", "passed": true, "detail": "Expires 2027-03-31, within the 365-day limit; then re-evaluated", "fix_tool": "set_member_tier"}], "needs_approval": true, "approval_reason": "Sets a member's tier by hand until 2027-03-31", "change_set_id": "cs_1045"}`
- 출력(실행): `{"ok": true, "id": "cus_alex", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: `tier.override`, `tier.notify`, `tier.history`, [approval.yml](../../policies/approval.yml)
- 에러: `tier_not_found` → `list_tiers` · `reason_required` → `set_member_tier` · `override_too_long` → `set_member_tier` · `member_not_active` → `get_member` · `customer_not_found` → `search_members`
- 멱등성: `Idempotency-Key = hash(customer_id + tier + expires_at)`

### `list_tier_history` — Kind: R
- 논리주소/REST: `customer.membership.list_tier_history` · `GET /v1/customer/people/{customer_id}/tier-history`
- 요약: 한 회원의 등급 변경 이력(append만). 각 변경의 원인, 그때의 실적, 누가·왜 바꿨는지. 상담에서 "왜 등급이 내려갔나요?"에 답할 때 씁니다.
- consumes → produces: `customer_id` → history
- 입력: `customer_id`(✓, 쇼퍼는 `me`), `program_id?`, `at_from`, `at_through`(DateTime), `limit`(기본 20, 최대 100), `cursor?`
- 출력(예시 — Jamie): `{"items": [{"at": "2026-09-01T03:00:00-04:00", "from_tier_id": "gold", "to_tier_id": "vip", "cause": "evaluation", "evaluation_id": "tev_2609", "snapshot": {"metric": "spend", "value": {"value": 842.00, "currency": "USD"}}, "actor": "sys_scheduler", "reason": null}, {"at": "2026-04-01T03:00:00-04:00", "from_tier_id": "member", "to_tier_id": "gold", "cause": "evaluation", "evaluation_id": "tev_2604", "snapshot": {"metric": "spend", "value": {"value": 412.50, "currency": "USD"}}, "actor": "sys_scheduler", "reason": null}], "total": 3}` — 수동 지정이면 `actor`가 운영자(`adm_sj`)이고 `reason`·`expires_at`이 붙습니다. — `cause`: `entry` \| `evaluation` \| `event` \| `manual_override` \| `override_expired` \| `grace_started` \| `grace_ended` \| `program_change` \| `withdrawal`
- 정책: `tier.history`
- 에러: `customer_not_found` → `search_members`
- 멱등성: 읽기.

---

## address

### `list_addresses` — Kind: R
- 논리주소/REST: `customer.address.list` · `GET /v1/customer/people/{customer_id}/addresses`
- 요약: 저장된 주소 목록(`usage`: 배송·청구·반품, 대표 주소 1개). 운영자에게는 기본 가림, 쇼퍼 본인에게는 그대로.
- consumes → produces: `customer_id` → `address_id[]`
- 입력: `customer_id`(✓, 쇼퍼는 `me`), `limit`(기본 20, 최대 100), `cursor?`
- 출력: `{"items": [{"id": "adr_home", "label": "Home", "street_address": "12 Harbor St, Apt 3", "address_locality": "Brooklyn", "address_region": "NY", "postal_code": "11201", "address_country": "US", "usage": ["shipping"], "is_primary": true}], "total": 1}`
- 정책: `privacy.masking_default`, `address.usages`
- 에러: `customer_not_found` → `search_members`
- 멱등성: 읽기.

### `add_address` — Kind: W
- 논리주소/REST: `customer.address.add` · `POST /v1/customer/people/{customer_id}/addresses`
- 요약: 주소 추가. `is_primary: true`면 기존 대표 주소는 해제.
- consumes → produces: `customer_id` → `address_id`
- 입력: `customer_id`(✓), `label`, `street_address`(✓), `address_locality`(✓), `address_region`, `postal_code`(✓), `address_country`(✓), `usage[]`(✓, `shipping` \| `billing` \| `return`), `is_primary`
- 출력(프리뷰): `{"preview_id": "pv_a01a", "expires_at": "2026-11-07T10:30:00-05:00", "changes": [{"op": "create", "entity": "postal_address", "id": null, "from": null, "to": "200 Water St, Brooklyn, NY 11201"}], "checks": [{"rule": "address.postal_code_validation", "passed": true, "detail": "11201 is in NY", "fix_tool": null}, {"rule": "address.max_per_member", "passed": true, "detail": "2 of 10 addresses", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "adr_work", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: `address.usages`, `address.max_per_member`, `address.primary_per_member`, `address.postal_code_validation`
- 에러: `address_limit_exceeded` → `list_addresses` · `invalid_postal_code` → `add_address`
- 멱등성: `Idempotency-Key`

### `update_address` — Kind: W
- 논리주소/REST: `customer.address.update` · `PATCH /v1/customer/people/{customer_id}/addresses/{address_id}`
- 요약: 주소 수정.
- consumes → produces: `address_id` → `address_id`
- 입력: `address_id`(✓), 바꿀 필드
- 출력(프리뷰): `{"preview_id": "pv_a02b", "expires_at": "2026-11-07T10:30:00-05:00", "changes": [{"op": "update", "entity": "postal_address", "id": "adr_home", "from": "Apt 3", "to": "Apt 4"}], "checks": [{"rule": "address.postal_code_validation", "passed": true, "detail": "Postal code unchanged", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "adr_home", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}` — 쇼퍼가 자기 주소를 고치면 변경셋 밖이므로 `undo: null`.
- 정책: `address.postal_code_validation`
- 에러: `address_not_found` → `list_addresses`
- 멱등성: `Idempotency-Key`

### `set_primary_address` — Kind: W
- 논리주소/REST: `customer.address.set_primary` · `POST /v1/customer/people/{customer_id}/addresses/{address_id}/primary`
- 요약: 대표 배송지 지정(기존 대표는 자동 해제).
- consumes → produces: `address_id` → `address_id`
- 입력: `address_id`(✓)
- 출력(프리뷰): `{"preview_id": "pv_a03c", "expires_at": "2026-11-07T10:30:00-05:00", "changes": [{"op": "update", "entity": "postal_address", "id": "adr_work", "from": {"is_primary": false}, "to": {"is_primary": true}}, {"op": "update", "entity": "postal_address", "id": "adr_home", "from": {"is_primary": true}, "to": {"is_primary": false}}], "checks": [{"rule": "address.primary_per_member", "passed": true, "detail": "Home stops being the primary address", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "adr_work", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: `address.primary_per_member`
- 에러: `address_not_found` → `list_addresses`
- 멱등성: 같은 주소 재지정은 성공으로 수렴.

---

## consent

### `get_consent` — Kind: R
- 논리주소/REST: `customer.consent.get` · `GET /v1/customer/people/{customer_id}/consent`
- 요약: 필수·선택 동의 상태와 변경 이력. 이력은 바뀌지 않습니다. 마케팅 발송 전 게이트가 이 값을 읽습니다.
- consumes → produces: `customer_id` → consent
- 입력: `customer_id`(✓)
- 출력: `{"items": [{"id": "cns_412", "consent_type": "marketing_email", "granted": true, "at": "2026-04-02T09:00:00-04:00"}, {"id": "cns_101", "consent_type": "terms", "granted": true, "at": "2025-12-10T18:22:00-05:00"}], "total": 6}` — 동의 종류마다 가장 최근 기록 하나(종류는 [enums.md](enums.md) §1.13).
- 정책: `consent`
- 에러: `customer_not_found` → `search_members`
- 멱등성: 읽기.

### `set_consent` — Kind: W
- 논리주소/REST: `customer.consent.set` · `POST /v1/customer/people/{customer_id}/consent`
- 요약: 동의 설정·철회. 필수 동의(`terms`, `privacy`)는 철회할 수 없습니다. 기존 기록을 덮어쓰지 않고 새 기록을 append합니다.
- consumes → produces: `customer_id` → `consent_id`
- 입력: `customer_id`(✓, 쇼퍼는 `me`), `consent_type`(✓, `terms` \| `privacy` \| `marketing_email` \| `marketing_sms` \| `marketing_push` \| `marketing_in_app`), `granted`(✓)
- 출력(프리뷰): `{"preview_id": "pv_c01a", "expires_at": "2026-11-07T10:30:00-05:00", "changes": [{"op": "create", "entity": "consent_record", "id": null, "from": {"marketing_email": true}, "to": {"marketing_email": false}}], "checks": [{"rule": "consent.mandatory", "passed": true, "detail": "marketing_email is optional", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "cns_551", "undo": null}` — 동의는 덮어쓰지 않는 이력이라 되돌리기 대신 반대 값으로 `set_consent`를 다시 부릅니다.
- 정책: `consent.mandatory`, `consent.optional`, `consent.history`, `consent.marketing_gate`
- 에러: `cannot_revoke_mandatory`(필수 동의의 철회는 탈퇴로만) → `check_withdrawal_eligibility` · `unknown_consent_type` → `list_enums`
- 멱등성: `(customer_id, consent_type, granted)` — 같은 값이면 새 기록 없음.

---

## preference

> **보이고 고칠 수 있는 기억.** 에이전트가 대화에서 기억한 선호는 쇼퍼에게 한 줄("Remembered: weight and packed size matter to you when traveling")로 보이고([chat.md](chat.md) `memory.saved` 이벤트와 `memory_note` 블록), 쇼퍼가 **Undo**를 누르면 사람 전용 `POST /v1/customer/me/memories/{memory_id}/undo`로 지워집니다. 기억은 쇼퍼 본인에 대한 것만 저장하며, **선물 받는 사람의 정보(사이즈·취향·주소)는 저장하지 않습니다**([../scenarios.md](../scenarios.md) F4). 결제 정보·건강 정보 같은 민감 정보도 저장하지 않습니다.

### `update_my_preferences` — Kind: W(승인 없음)
- 논리주소/REST: `customer.preference.update_my_preferences` · `POST /v1/customer/people/me/preferences`
- 요약: 대화에서 알게 된 쇼퍼 본인의 선호를 더하거나 뺍니다. [approval.yml](../../policies/approval.yml) `no_approval`에 있어 프리뷰 후 바로 실행되고(변경셋 없음), 결과로 쇼퍼에게 보일 기억 한 줄과 되돌리기 대상(기억 id)을 돌려줍니다. 쇼퍼 툴. schema.org `acom:Preference`.
- consumes → produces: → `memory_id`
- 입력: 세션의 쇼퍼. `add`·`remove` 중 적어도 하나는 항목이 있어야 합니다(서버 검증, 위반은 `invalid_input`).
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |---|---|:--:|---|---|
  | add | Object[] (최대 5) | | 더할 기억. 각 항목은 닫힌 객체 `{key, value, source}` | `[{"key": "travel_priority", "value": "weight and packed size", "source": "conversation"}]` |
  | add[].key | Text(enum) | ✓ | 값: `size`(사이즈); `fit_note`(핏·겹쳐 입기 습관); `fabric_preference`(소재); `compares_specs`(스펙 비교를 좋아함); `layout_preference`(화면 형태 선호); `travel_priority`(여행 때 중요한 것); `other`(그 밖). 어휘는 `preference.keys` | `"travel_priority"` |
  | add[].value | Text (1–200자) | ✓ | 쇼퍼의 말로 된 값 | `"weight and packed size"` |
  | add[].source | Text(enum) | ✓ | 값: `conversation`(이 대화에서 알게 됨). 어휘는 `preference.sources` | `"conversation"` |
  | remove | Object[] (최대 5) | | 지울 기억. 각 항목은 `{memory_id}` 또는 `{key, value}` 중 하나(모드 필드 `match`로 구분) | `[{"match": "memory_id", "memory_id": "mem_07"}]` |
  | remove[].match | Text(enum) | ✓ | 값: `memory_id`(id로 지움); `key_value`(같은 키·값의 기억을 지움) | `"key_value"` |
  | remove[].memory_id | Text `^mem_[a-z0-9_]+$` | `match: memory_id` | 지울 기억 id(`get_preferences`) | `"mem_07"` |
  | remove[].key | Text(enum) | `match: key_value` | `add[].key`와 같은 값 | `"layout_preference"` |
  | remove[].value | Text | `match: key_value` | 지울 값 | `"detailed lists"` |
- 출력(프리뷰):
  ```jsonc
  {"preview_id": "pv_p12a", "expires_at": "2026-11-07T10:42:00-05:00",
   "changes": [{"op": "create", "entity": "preference", "id": null, "from": null,
                "to": {"key": "travel_priority", "value": "weight and packed size", "source": "conversation"}}],
   "checks": [{"rule": "preference.about_shopper_only", "passed": true, "detail": "About the shopper, not a gift recipient", "fix_tool": null},
              {"rule": "preference.no_sensitive_data", "passed": true, "detail": "No payment or health data", "fix_tool": null}],
   "needs_approval": false,
   "approval_reason": null}
  ```
- 출력(실행):
  ```jsonc
  {"ok": true, "id": "mem_12",
   "memory": {"memory_id": "mem_12", "text": "Remembered: weight and packed size matter to you when traveling"},
   "undo": {"by": "person", "target": "mem_12", "until": null}}
  ```
  - `memory`는 채팅의 `memory.saved` 이벤트(`{memory_id, text}`)와 `memory_note` 블록에 그대로 쓰입니다. 되돌리기는 쇼퍼 버튼만 부르는 사람 전용 엔드포인트이고 기한이 없습니다(`until: null`). 여러 항목을 더하면 `id`는 첫 기억, `memories[]`에 전부가 옵니다.
  - `remove`만 있으면 `changes`는 `{"op": "delete", "entity": "preference", "id": "mem_07", ...}`이고 실행 응답의 `undo`는 `null`(지운 기억을 되살리는 길은 다시 더하는 것)입니다.
- 정책: `preference.keys`, `preference.sources`, `preference.about_shopper_only`, `preference.no_sensitive_data`, `preference.shown_to_shopper`, `preference.undo`, [approval.yml](../../policies/approval.yml) `no_approval`
- 에러: `gift_recipient_data`(다른 사람의 정보라 저장하지 않음, 이번 대화에서만 씀) → null · `sensitive_data`(민감 정보라 저장하지 않음) → null · `not_signed_in`(게스트는 기억이 없음) → null · `nothing_to_change`(이미 같은 기억이 있거나 지울 것이 없음) → `get_preferences` · `invalid_input`(`add`·`remove`가 모두 비었거나 `match`에 맞는 필드가 없음) → `update_my_preferences`
- 멱등성: `(customer_id, key, value)` — 같은 선호를 다시 더해도 하나로 수렴하고 첫 `memory_id`를 돌려줍니다.

### `get_preferences` — Kind: R
- 논리주소/REST: `customer.preference.get` · `GET /v1/customer/people/{customer_id}/preferences`
- 요약: 에이전트가 한 회원에 대해 기억하는 것 전부. 쇼퍼 본인은 `me`로 같은 목록을 봅니다.
- consumes → produces: `customer_id` → `memory_id[]`
- 입력: `customer_id`(✓, 쇼퍼는 `me`), `key?`(`preference.keys`의 값), `limit`(기본 20, 최대 100), `cursor?`
- 출력: `{"items": [{"id": "mem_12", "key": "travel_priority", "value": "weight and packed size", "text": "Weight and packed size matter when traveling", "source": "conversation", "created_at": "2026-11-07T10:12:00-05:00"}, {"id": "mem_07", "key": "fabric_preference", "value": "natural fabrics", "text": "Natural fabrics", "source": "conversation", "created_at": "2026-09-14T19:40:00-04:00"}], "total": 5}`
- 정책: `preference.shown_to_shopper`
- 에러: `customer_not_found` → `search_members`
- 멱등성: 읽기.

---

## wishlist

### `list_wishlist` — Kind: R
- 논리주소/REST: `customer.wishlist.list` · `GET /v1/customer/people/{customer_id}/wishlist`
- 요약: 관심 상품(지금 가격·재고 반영).
- consumes → produces: `customer_id` → `product_id[]`
- 입력: `customer_id`(✓, 쇼퍼는 `me`), `category_id?`, `limit`(기본 20, 최대 100), `cursor?`
- 출력: `{"items": [{"product_id": "prd_field_jacket", "name": "Linen blend field jacket", "price": 128.00, "price_currency": "USD", "availability": "in_stock", "added_at": "2026-10-20T19:02:00-04:00"}], "total": 4}`
- 정책: 없음(읽기).
- 에러: `customer_not_found` → `search_members`
- 멱등성: 읽기.

### `add_to_wishlist` — Kind: W
- 논리주소/REST: `customer.wishlist.add` · `POST /v1/customer/people/{customer_id}/wishlist`
- 요약: 관심 상품 추가. 쇼퍼 본인의 쓰기라 변경셋 밖에서 바로 실행되고, 빼는 것은 `remove_from_wishlist`입니다.
- consumes → produces: `product_id` → `product_id`
- 입력: `customer_id`(✓), `product_id`(✓)
- 출력(프리뷰): `{"preview_id": "pv_w01a", "expires_at": "2026-11-07T10:30:00-05:00", "changes": [{"op": "create", "entity": "wishlist_item", "id": null, "from": null, "to": {"product_id": "prd_field_jacket"}}], "checks": [], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "prd_field_jacket", "undo": null}` — 위시리스트 항목은 상품 id로 가리킵니다.
- 정책: 없음.
- 에러: `product_not_found` → `search_products`
- 멱등성: `(customer_id, product_id)` — 이미 있으면 성공으로 수렴.

### `remove_from_wishlist` — Kind: W
- 논리주소/REST: `customer.wishlist.remove` · `DELETE /v1/customer/people/{customer_id}/wishlist/{product_id}`
- 요약: 관심 상품 삭제. 다시 넣는 것은 `add_to_wishlist`입니다.
- consumes → produces: `product_id` → `product_id`
- 입력: `customer_id`(✓), `product_id`(✓)
- 출력(프리뷰): `{"preview_id": "pv_w02b", "expires_at": "2026-11-07T10:30:00-05:00", "changes": [{"op": "delete", "entity": "wishlist_item", "id": "prd_field_jacket", "from": {"product_id": "prd_field_jacket"}, "to": null}], "checks": [], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "prd_field_jacket", "undo": null}`
- 정책: 없음.
- 에러: 없음(없으면 성공으로 간주).
- 멱등성: 삭제 멱등.

---

## employee

### `register_employee` — Kind: W
- 논리주소/REST: `customer.employee.register` · `POST /v1/customer/employees`
- 요약: 임직원 회원 등록. 임직원 할인은 다른 회원 혜택과 함께 쓰지 않습니다(주문당 회원 혜택 1개).
- consumes → produces: `customer_id` → `employee_id`
- 입력: `customer_id`(✓), `employee_number`(✓), `discount_limit`(MonetaryAmount, 예 `{"value": 500.00, "currency": "USD"}`), `period`(`month` \| `year`, 기본 `year`)
- 출력(프리뷰): `{"preview_id": "pv_e01a", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "create", "entity": "employee", "id": null, "from": null, "to": "active"}], "checks": [{"rule": "member_benefits_per_order", "passed": true, "detail": "Employee discount replaces tier coupons", "fix_tool": null}], "needs_approval": false, "approval_reason": null, "change_set_id": "cs_1045"}`
- 출력(실행): `{"ok": true, "id": "emp_041", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: [promotion.yml](../../policies/promotion.yml) `member_benefits_per_order`
- 에러: `duplicate_employee_number` → `get_member` · `customer_not_found` → `register_member`
- 멱등성: `Idempotency-Key = hash(employee_number)`

### `set_discount_limit` — Kind: W
- 논리주소/REST: `customer.employee.set_discount_limit` · `PATCH /v1/customer/employees/{employee_id}/discount-limit`
- 요약: 임직원 할인 한도(기간·금액).
- consumes → produces: `employee_id` → `employee_id`
- 입력: `employee_id`(✓), `discount_limit`(✓, MonetaryAmount, 예 `{"value": 750.00, "currency": "USD"}`), `period`(✓, `month` \| `year`)
- 출력(프리뷰): `{"preview_id": "pv_e02b", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "update", "entity": "employee", "id": "emp_041", "from": {"value": 500.00, "currency": "USD"}, "to": {"value": 750.00, "currency": "USD"}}], "checks": [], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "emp_041", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: 없음.
- 에러: `employee_not_found` → `register_employee`
- 멱등성: `Idempotency-Key`

---

## inquiry

### `search_inquiries` — Kind: R
- 논리주소/REST: `customer.inquiry.search` · `GET /v1/customer/inquiries`
- 요약: 고객 상담 검색. 기본 가림.
- consumes → produces: → `inquiry_id[]`
- 입력: `inquiry_type`(`order_status` \| `return` \| `product_question` \| `account` \| `callback` \| `other`), `status`(`received` \| `in_progress` \| `resolved`), `admin_user_id`(담당자 `adm_…`), `order_id`, `customer_id`, `created_from`, `created_through`, `limit`(기본 20, 최대 100), `cursor?`
- 출력: `{"items": [{"id": "inq_771", "inquiry_type": "return", "status": "received", "customer_id": "cus_alex", "order_id": "ord_20402", "created_at": "2026-11-06T09:14:00-05:00"}], "total": 18}`
- 정책: `privacy.masking_default`
- 에러: `invalid_filter` → `search_inquiries`
- 멱등성: 읽기.

### `get_inquiry` — Kind: R
- 논리주소/REST: `customer.inquiry.get` · `GET /v1/customer/inquiries/{inquiry_id}`
- 요약: 상담 한 건과 대화 스레드(`Comment[]`), 관련 주문. 목록 봉투 없이 엔티티를 돌려줍니다.
- consumes → produces: `inquiry_id` → inquiry
- 입력: `inquiry_id`(✓), `fields?[]`
- 출력: `{"id": "inq_771", "inquiry_type": "return", "status": "received", "customer_id": "cus_alex", "order_id": "ord_20402", "admin_user_id": null, "thread": [{"id": "cmt_901", "author_type": "customer", "text": "The sole is coming apart.", "at": "2026-11-06T09:14:00-05:00"}]}` — `author_type`: `customer` \| `staff` \| `agent`
- 정책: `privacy.masking_default`
- 에러: `inquiry_not_found` → `search_inquiries`
- 멱등성: 읽기.

### `create_inquiry` — Kind: W
- 논리주소/REST: `customer.inquiry.create` · `POST /v1/customer/inquiries`
- 요약: 상담 생성(`received`). 닫는 것은 `close_inquiry`입니다.
- consumes → produces: → `inquiry_id`
- 입력: `inquiry_type`(✓, `order_status` \| `return` \| `product_question` \| `account` \| `callback` \| `other`), `text`(✓), `order_id?`, `customer_id`(✓, 쇼퍼는 `me`)
- 출력(프리뷰): `{"preview_id": "pv_q01a", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "create", "entity": "customer_inquiry", "id": null, "from": null, "to": "received"}], "checks": [{"rule": "customer_service.inquiry_types", "passed": true, "detail": "return is an inquiry type", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "inq_772", "undo": null}`
- 정책: `customer_service.inquiry_types`
- 에러: `invalid_inquiry_type` → `list_enums`
- 멱등성: `Idempotency-Key`

### `assign_inquiry` — Kind: W
- 논리주소/REST: `customer.inquiry.assign` · `POST /v1/customer/inquiries/{inquiry_id}/assign`
- 요약: 상담 담당자(운영자 계정) 배정(자동 순환 또는 지정). `received` → `in_progress`.
- consumes → produces: `inquiry_id` → `inquiry_id`
- 입력: `inquiry_id`(✓), `mode`(✓, `auto_round_robin` \| `manual`), `admin_user_id`(`manual`일 때 ✓, `adm_…`)
- 출력(프리뷰): `{"preview_id": "pv_q02b", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "update", "entity": "customer_inquiry", "id": "inq_771", "from": {"status": "received", "admin_user_id": null}, "to": {"status": "in_progress", "admin_user_id": "adm_dana"}}], "checks": [{"rule": "customer_service.max_open_per_agent", "passed": true, "detail": "Dana has 3 of 20 open inquiries", "fix_tool": null}], "needs_approval": false, "approval_reason": null, "change_set_id": "cs_1045"}`
- 출력(실행): `{"ok": true, "id": "inq_771", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: `customer_service.assignment`, `customer_service.max_open_per_agent`
- 에러: `no_available_agent`(자동 배정할 담당자가 없음, `manual`로 다시) → `assign_inquiry` · `inquiry_not_found` → `search_inquiries`
- 멱등성: `Idempotency-Key`

### `respond_inquiry` — Kind: W$
- 논리주소/REST: `customer.inquiry.respond` · `POST /v1/customer/inquiries/{inquiry_id}/respond`
- 요약: 상담 답변을 기록하고 고객에게 보냄. 고객에게 닿는 쓰기이므로 `send_customer_message`와 같이 항상 승인.
- consumes → produces: `inquiry_id` → `comment_id`
- 입력: `inquiry_id`(✓), `text`(✓), `channel`(✓, `email` \| `in_app`)
- 출력(프리뷰): `{"preview_id": "pv_q03c", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "send", "entity": "comment", "id": null, "from": null, "to": "Sorry about the sole. A refund is being reviewed today."}], "checks": [{"rule": "customer_messages", "passed": true, "detail": "Shown as the customer will read it", "fix_tool": null}], "needs_approval": true, "approval_reason": "Sends a reply to a customer", "change_set_id": "cs_1045"}`
- 출력(실행): `{"ok": true, "id": "cmt_902", "undo": null}` — 보낸 답변은 되돌릴 수 없음.
- 정책: [approval.yml](../../policies/approval.yml) `customer_messages`(항상 승인), `change_set.irreversible`
- 에러: `inquiry_closed` → `create_inquiry` · `inquiry_not_found` → `search_inquiries`
- 멱등성: `Idempotency-Key` — 중복 발송 없음.

### `close_inquiry` — Kind: W
- 논리주소/REST: `customer.inquiry.close` · `POST /v1/customer/inquiries/{inquiry_id}/close`
- 요약: 상담 종료(`in_progress` → `resolved`). 정책이 켜져 있으면 회원에게 거래성 알림을 보냅니다.
- consumes → produces: `inquiry_id` → `inquiry_id`
- 입력: `inquiry_id`(✓), `resolution`
- 출력(프리뷰): `{"preview_id": "pv_q04d", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "update", "entity": "customer_inquiry", "id": "inq_771", "from": "in_progress", "to": "resolved"}], "checks": [{"rule": "customer_service.notify_member_on_resolve", "passed": true, "detail": "Alex gets a resolved notice by email", "fix_tool": null}], "needs_approval": false, "approval_reason": null, "change_set_id": "cs_1045"}`
- 출력(실행): `{"ok": true, "id": "inq_771", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: `customer_service.states`, `customer_service.notify_member_on_resolve`
- 에러: `invalid_state` → `get_inquiry`
- 멱등성: 종료 재요청은 성공으로 수렴.

---

## remedy

> 보상은 귀책([enums.md](enums.md) §2.1 `acom:liability`: `store_fault` \| `seller_fault` \| `customer_fault` \| `carrier_fault` \| `none`)을 분류하고 단계별로 승인합니다: 상담원 요청 → 보상 승인자 → (현금이면) 지급 승인자. 상태는 `requested` → `approved` \| `rejected`, 현금은 `approved` → `payout_pending` → `paid` \| `failed`([enums.md](enums.md) §1.13 `acom:remedy_status`).

### `request_remedy` — Kind: W
- 논리주소/REST: `customer.remedy.request` · `POST /v1/customer/remedies`
- 요약: 보상 요청 생성(`requested`). 돈은 아직 움직이지 않습니다. 반려된 요청은 새 요청으로만 다시 냅니다.
- consumes → produces: `inquiry_id` \| `order_id` → `remedy_id`
- 입력: `target`(✓, `inquiry` \| `order`), `inquiry_id`(`target: inquiry`일 때 ✓), `order_id`(`target: order`일 때 ✓), `remedy_type`(✓, `loyalty_points` \| `coupon` \| `cash`), `liability`(✓, `store_fault` \| `seller_fault` \| `customer_fault` \| `carrier_fault` \| `none`), `amount`(✓, MonetaryAmount, 예 `{"value": 15.00, "currency": "USD"}`), `reason`(✓)
- 출력(프리뷰): `{"preview_id": "pv_y01a", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "create", "entity": "remedy", "id": null, "from": null, "to": "requested"}], "checks": [{"rule": "remedy.liability", "passed": true, "detail": "Store fault: late delivery", "fix_tool": null}], "needs_approval": false, "approval_reason": null, "change_set_id": "cs_1045"}`
- 출력(실행): `{"ok": true, "id": "rmd_310", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: `remedy.liability`, `remedy.types`, `remedy.resubmit_on_reject`
- 에러: `invalid_liability` → `list_enums` · `inquiry_not_found` → `search_inquiries` · `order_not_found` → `search_orders`
- 멱등성: `Idempotency-Key`. 같은 대상·종류·금액의 열린 요청이 있으면 그 요청을 돌려줍니다.

### `approve_remedy` — Kind: W$
- 논리주소/REST: `customer.remedy.approve` · `POST /v1/customer/remedies/{remedy_id}/approve`
- 요약: 보상 승인(`requested` → `approved` | `rejected`). 현금 보상은 지급 승인자 단계가 하나 더 있고, `payout_pending`은 `pay_remedy`가 거칩니다(위 절의 상태 흐름과 `pay_remedy` 프리뷰 예시에 맞춤). 결정한 사람은 변경셋이 기록하고, 보상의 `decided_by_id`는 그 변경셋을 가리킵니다. 지급 승인자 역할(`approval.yml` `remedy_cash`의 `approver_role`)은 변경셋 엔진이 승인 때 확인합니다.
- consumes → produces: `remedy_id` → `remedy_id`
- 입력: `remedy_id`(✓), `decision`(✓, `approve` \| `reject`), `reason`(reject 시 ✓)
- 출력(프리뷰): `{"preview_id": "pv_y02b", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "update", "entity": "remedy", "id": "rmd_310", "from": "requested", "to": "approved"}], "checks": [{"rule": "remedy.approval_levels", "passed": true, "detail": "Points, no payout step", "fix_tool": null}], "needs_approval": true, "approval_reason": "Approves a $15.00 remedy", "change_set_id": "cs_1045"}`
- 출력(실행): `{"ok": true, "id": "rmd_310", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: [approval.yml](../../policies/approval.yml) `thresholds.remedy_cash`·`thresholds.loyalty_grant`, `remedy.approval_levels`, `remedy.resubmit_on_reject`(반려된 요청은 고치지 않고 새로 냄)
- 에러: `wrong_approver`(현금 지급은 지급 승인자 역할만) → null · `already_decided` → `approve_remedy`
- 멱등성: `Idempotency-Key` — 재시도 시 첫 결정 반환.

### `pay_remedy` — Kind: W$
- 논리주소/REST: `customer.remedy.pay` · `POST /v1/customer/remedies/{remedy_id}/pay`
- 요약: 승인된 보상 지급. 포인트는 `grant_loyalty`, 쿠폰은 `issue_coupon`과 연결되고, 현금은 원결제수단 또는 확인된 계좌로 지급(`payout_pending` → `paid` \| `failed`).
- consumes → produces: `remedy_id` → `remedy_id`
- 입력: `remedy_id`(✓)
- 출력(프리뷰, 현금 보상): `{"preview_id": "pv_y03c", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "create", "entity": "money_transfer", "id": null, "from": null, "to": {"amount": {"value": 15.00, "currency": "USD"}, "to": "card ending 0937"}}, {"op": "update", "entity": "remedy", "id": "rmd_310", "from": "approved", "to": "payout_pending"}], "checks": [{"rule": "remedy.approval_levels", "passed": true, "detail": "Approved by Son SeongJun (adm_sj)", "fix_tool": "approve_remedy"}, {"rule": "remedy.cash_requires", "passed": true, "detail": "Card ending 0937 verified, payout approver signed off", "fix_tool": null}], "needs_approval": true, "approval_reason": "Pays $15.00 in cash to card ending 0937", "change_set_id": "cs_1045"}` — 결제 이벤트로 지급이 확정되면 보상 상태가 `payout_pending`에서 지급 완료(enums.md §1.13)로 바뀝니다. 포인트 보상은 `changes`가 `grant` `loyalty_transaction` 한 줄이고 실행 즉시 지급 완료입니다.
- 출력(실행): `{"ok": true, "id": "rmd_310", "undo": null}`
- 정책: `remedy.cash_requires`, `remedy.approval_levels`, [approval.yml](../../policies/approval.yml) `thresholds.remedy_cash`
- 에러: `payout_not_approved` → `approve_remedy`
- 멱등성: `Idempotency-Key` — 중복 지급 없음. 이미 지급된 보상은 첫 결과를 돌려줍니다.

---

## faq_notice

### `list_faqs` — Kind: R
- 논리주소/REST: `customer.faq_notice.list_faqs` · `GET /v1/customer/faqs`
- 요약: FAQ 목록(schema.org `FAQPage`). 비회원도 가능.
- consumes → produces: → `faq_id[]`
- 입력: `category?`, `query?`, `limit`(기본 20, 최대 100), `cursor?`
- 출력: `{"items": [{"id": "faq_returns", "name": "How long do I have to return?", "category": "returns"}], "total": 24}`
- 정책: 없음(읽기).
- 에러: 없음.
- 멱등성: 읽기.

### `get_faq` — Kind: R
- 논리주소/REST: `customer.faq_notice.get_faq` · `GET /v1/customer/faqs/{faq_id}`
- 요약: FAQ 한 건(`Question` + `accepted_answer`).
- consumes → produces: `faq_id` → faq
- 입력: `faq_id`(✓)
- 출력(목록 봉투 없이 엔티티): `{"id": "faq_returns", "name": "How long do I have to return?", "accepted_answer": "30 days from delivery.", "category": "returns"}`
- 정책: 없음(읽기).
- 에러: `faq_not_found` → `list_faqs`
- 멱등성: 읽기.

### `list_notices` — Kind: R
- 논리주소/REST: `customer.faq_notice.list_notices` · `GET /v1/customer/notices`
- 요약: 공지 목록(schema.org `NewsArticle`).
- consumes → produces: → `notice_id[]`
- 입력: `category?`, `published_from`, `published_through`, `limit`(기본 20, 최대 100), `cursor?`
- 출력: `{"items": [{"id": "ntc_holiday", "headline": "Holiday shipping cutoffs", "date_published": "2026-11-02"}], "total": 9}`
- 정책: 없음(읽기).
- 에러: 없음.
- 멱등성: 읽기.

### `get_notice` — Kind: R
- 논리주소/REST: `customer.faq_notice.get_notice` · `GET /v1/customer/notices/{notice_id}`
- 요약: 공지 한 건.
- consumes → produces: `notice_id` → notice
- 입력: `notice_id`(✓)
- 출력(목록 봉투 없이 엔티티): `{"id": "ntc_holiday", "headline": "Holiday shipping cutoffs", "article_body": "…", "date_published": "2026-11-02", "status": "published"}`
- 정책: 없음(읽기).
- 에러: `notice_not_found` → `list_notices`
- 멱등성: 읽기.

### `publish_notice` — Kind: W
- 논리주소/REST: `customer.faq_notice.publish_notice` · `POST /v1/customer/notices/{notice_id}/publish`
- 요약: 공지 게시(`draft` → `published`). 쇼퍼 모두에게 보이므로 변경셋에 들어갑니다.
- consumes → produces: `notice_id` → `notice_id`
- 입력: `notice_id`(✓), `date_published`, `pinned`
- 출력(프리뷰): `{"preview_id": "pv_n01a", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "update", "entity": "notice", "id": "ntc_holiday", "from": "draft", "to": "published"}], "checks": [], "needs_approval": false, "approval_reason": null, "change_set_id": "cs_1045"}`
- 출력(실행): `{"ok": true, "id": "ntc_holiday", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: 없음.
- 에러: `already_published` → `get_notice`
- 멱등성: `Idempotency-Key`

---

## 사람 전용 엔드포인트

회원 본인(또는 로그인하지 못한 사람)이 UI 화면·버튼으로만 부르는 HTTP 엔드포인트입니다. **툴이 아니며 모델은 보지 못합니다.** 전체 목록과 스코프는 [README.md](README.md) §2, 버튼 형식은 [chat.md](chat.md) §3.2(`person` 버튼)입니다. 에이전트는 비밀번호를 받거나 보거나 정하지 않습니다.

| 엔드포인트 | 누가 | 스코프 | 무엇 |
|---|---|---|---|
| `POST /v1/customer/me/memories/{memory_id}/undo` | 쇼퍼 | `human:undo` | 기억한 선호 삭제. `memory_note` 블록의 Undo 버튼 |
| `POST /v1/customer/me/withdrawal` | 회원 본인 | `human:account` | 탈퇴. 아래 상세 |
| `POST /v1/customer/password-reset/requests` | 누구나(비로그인, 익명 세션) | `human:account` | 비밀번호 재설정 요청. 계정 유무와 관계없이 같은 응답 |
| `POST /v1/customer/password-reset/confirm` | 링크·코드를 받은 사람 | `human:account` | 새 비밀번호 설정, 세션 폐기 |
| `POST /v1/customer/account-recovery/login-id` | 누구나(비로그인, 익명 세션) | `human:account` | 로그인 id 찾기 |
| `PUT /v1/customer/me/password` | 회원 본인 | `human:account` | 로그인한 상태에서 비밀번호 변경 |

- 주소와 이름(레지스트리): `customer.person.withdraw`(`withdraw_account`), `customer.person.request_password_reset`, `customer.person.confirm_password_reset`, `customer.person.recover_login_id`, `customer.person.change_password`, `customer.preference.undo_memory`. 툴이 아니므로 매니페스트에 나오지 않습니다.
- 틀린 코드·비밀번호는 실패해도 셉니다: 그 에러의 `detail.attempt_recorded: true`이면 엔드포인트의 트랜잭션이 커밋되어 시도 횟수와 잠금(`password.lockout`, `account_recovery.confirm_max_attempts`)이 남습니다(acom-http). 로그인하지 못한 사람(게스트 세션)도 `human:account`를 가집니다(재설정 요청·로그인 id 찾기).
- "`202`"로 적힌 응답은 지금 HTTP `200`과 같은 본문으로 나갑니다(사람 전용 엔드포인트의 상태 코드는 공통 계층이 정함). 비밀번호 재설정 링크·코드와 로그인 id 안내는 확인된 연락처로 "보내는" 스텁이며 응답·액션 로그에 비밀이 없습니다.
- 공통: 사람이 직접 누른 요청(세션 + `X-CSRF-Token`)만 받고, 에이전트 위임 토큰은 `human:*` 스코프가 없어 `forbidden`입니다. `Idempotency-Key` 필수(재설정 요청은 선택). 에러는 공통 형식이고 **`next_tool`은 항상 `null`**입니다(화면이 다음 단계를 안내). 정책 `withdrawal`, `identity_verification`, `account_recovery`, `password`, `preference.undo` ([customer.yml](../../policies/customer.yml)).
- 비밀번호는 이 엔드포인트들의 본문으로만 오가고, 툴 입력·출력·액션 로그·변경셋 어디에도 남지 않습니다. 비밀번호 규칙은 화면이 `get_policy`(`password`)로 읽어 미리 보여 줍니다.

### `POST /v1/customer/me/memories/{memory_id}/undo` — 사람 전용
- 누가: 로그인한 쇼퍼 본인(`human:undo`). 기억은 본인 것만 지울 수 있습니다.
- 본문: 없음.
- 응답 `200`: `{"ok": true, "id": "mem_12", "undo": null}` — 기억을 지우고(`preference` 삭제) 액션 로그에 남깁니다. 지운 기억은 다시 더하기 전에는 쓰이지 않습니다.
- 에러: `memory_not_found`(없거나 이미 지움) → null · `forbidden`(다른 사람의 기억) → null
- 멱등성: 이미 지운 기억에 다시 보내면 첫 결과를 돌려줍니다.

### `POST /v1/customer/me/withdrawal` — 사람 전용
- 누가: 로그인한 회원 본인(`human:account`). 운영자·에이전트 토큰은 `forbidden_initiator`로 거부합니다(`withdrawal.initiator: self_only`).
- 흐름: 탈퇴 화면이 `check_withdrawal_eligibility` 결과(막는 것·없어지는 것·남는 것)를 보여 줌 → 회원이 이유를 고르고 데이터 처리 요약에 동의 → 재인증 → 버튼.
- 본문:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | reasons | Text[] | ✓ | `withdrawal.reasons`의 값. 예 `["not_using", "too_many_messages"]` |
  | reason_text | Text | | 자유 입력(선택, 500자) |
  | confirmation | Object | ✓ | `{acknowledged_data_handling: true, eligibility_checked_at: "2026-11-07T14:30:00-05:00"}` — 확인한 자격 결과의 `checked_at`. `withdrawal.confirmation_max_age_minutes`보다 오래되면 거부 |
  | reauthentication | Object | ✓ | `{method: "password", credential: "…"}`(현재 비밀번호, 화면에서만) 또는 `{method: "email_code", verification_token: "…"}`. 허용 방법은 `identity_verification.required_for.withdrawal_reauth` |
- 응답 `200`:
  ```jsonc
  {"ok": true, "id": "cus_jamie",
   "status": "withdrawn",                      // withdrawn | pending_cooling_off
   "effective_at": "2026-11-07T14:36:00-05:00",
   "cancel_until": null,                       // cooling_off_days > 0이면 그때까지 로그인하면 취소
   "settled": [{"type": "store_credit", "amount": {"value": 25.00, "currency": "USD"}, "action": "pay_out"},
               {"type": "coupons", "count": 3, "action": "void"}],
   "data_retention": [{"data": "order_and_payment_records", "until": "2031-11-06", "why": "tax_and_consumer_protection"},
                      {"data": "claims_and_inquiries", "until": "2029-11-06", "why": "dispute_handling"},
                      {"data": "consent_history", "until": "2031-11-06", "why": "proof_of_consent"},
                      {"data": "rejoin_check_fingerprint", "until": "2027-02-05", "why": "prevent_signup_benefit_abuse"}],
   "rejoin_after": "2027-02-05", "login_id_reusable_after": "2027-02-05"}
  ```
- 에러: `withdrawal_blocked`(`detail`에 막는 항목 목록, 화면은 `check_withdrawal_eligibility` 결과로 안내) → null · `reauthentication_failed` → null · `reauthentication_locked`(`password.lockout`) → null · `confirmation_required` → null · `estimate_changed`(확인 뒤 새 주문·상담이 생겨 자격이 바뀜, 다시 확인) → null · `already_withdrawn` → null · `forbidden_initiator` → null
- 탈퇴 후(모두 정책 데이터, [customer.yml](../../policies/customer.yml) `withdrawal`):
  - **상태**: 회원 상태 `withdrawn`, 세션·연결 로그인·기기 토큰 폐기, 마케팅 동의 철회, 등급 제거(`list_tier_history`에 `withdrawal`), 미사용 쿠폰 무효, 임직원 할인 한도 소멸, 구독은 정책대로 기간 끝에 해지. 완료 안내는 거래성 메시지로 한 번.
  - **개인정보 가림**: `data_handling.mask_now`의 필드는 즉시 가리고, `delete_now`의 데이터는 즉시 삭제. 리뷰·글·댓글은 `authored_content`(기본 익명화).
  - **보존**: 주문·결제·클레임·동의 기록은 `data_handling.retain`의 기간과 이유로 운영 데이터와 분리해 암호화 보관하고, 열람은 사유를 남깁니다. 재가입 확인용으로는 연락처·본인 확인 결과의 단방향 해시만 둡니다. 기간이 끝나면 `purge` 작업이 파기합니다. 기간은 스토어 관할 법령에 맞게 바꿉니다.
  - **재가입·id 재사용**: 같은 사람은 `rejoin.block_days` 동안 재가입할 수 없고, 재가입해도 가입 혜택·등급·적립금·쿠폰은 이어지지 않습니다. 로그인 id·이메일은 `identifier_reuse`(기본: 보관 기간 뒤 재사용 가능).
  - 막힌 시도는 사유와 함께 액션 로그에 남습니다(`on_withdraw.log_blocked_attempts`).
- 멱등성: `Idempotency-Key` — 재시도는 첫 결과 반환.

### `POST /v1/customer/password-reset/requests` — 사람 전용
- 누가: 로그인하지 못한 누구나. **계정이 있든 없든 같은 응답**을 비슷한 시간에 돌려줘 계정 존재를 드러내지 않습니다(`account_recovery.uniform_response`).
- 본문: `identifier`(✓, 이메일·로그인 id·전화번호 중 `account_recovery.identifiers`), `method?`(`identity_verification.required_for.password_reset` 중 하나, 기본 첫 항목 — 예 `email_link`, `sms_code`, 외부 제공자), `client_challenge?`(봇 확인 토큰)
- 응답 `202` (항상):
  ```jsonc
  {"status": "accepted", "request_id": "rcv_9f31",
   "message": "If an account matches, we sent instructions to its verified contact.",
   "retry_after_seconds": 60}
  ```
  - 계정이 있으면 확인된 연락처로만 링크(토큰) 또는 코드를 보냅니다. 비밀번호 없이 연결 로그인만 쓰는 계정이면 그 사실과 비밀번호 설정 링크를 보냅니다. 사업자 회원은 지정 담당자 연락처로. 새 요청은 이전 링크·코드를 무효로 합니다.
  - 계정이 없어도 `request_id`는 만들어 돌려줍니다(형태 동일).
- 에러: `invalid_identifier_format`(형식만 검사) → null · `rate_limited`(`account_recovery.rate_limits`, 계정 유무와 무관하게 같은 응답, `retry_after_seconds` 포함) → null
- 멱등성: 같은 식별자의 짧은 재시도는 같은 `request_id`.

### `POST /v1/customer/password-reset/confirm` — 사람 전용
- 누가: 링크 또는 코드를 받은 사람.
- 본문: `token`(링크 방식) 또는 `{request_id, code}`(코드 방식) 중 하나(✓), `new_password`(✓, `password` 규칙)
- 응답 `200`: `{"ok": true, "id": "cus_alex", "sessions_revoked": 4, "signed_in": false, "next": "sign_in"}` — `password.on_reset`대로 모든 세션 폐기, 로그인 잠금 해제, 완료 알림을 확인된 연락처로(`account_recovery.notify_on_reset`), 액션 로그에는 "재설정됨"만(비밀번호 없음).
- 에러: `invalid_or_expired`(없는 토큰·만료·사용됨을 구분하지 않음) → null · `too_many_attempts`(`account_recovery.confirm_max_attempts`를 넘으면 요청 무효, 새 요청 필요) → null · `password_policy`(`detail`에 어긴 규칙 `["min_length", "contains_login_id"]`) → null · `password_reused`(`password.history_block`) → null
- 멱등성: 토큰·코드는 1회용. 성공한 토큰의 재시도는 `invalid_or_expired`.

### `POST /v1/customer/account-recovery/login-id` — 사람 전용
- 누가: 로그인 id를 잊은 누구나.
- 본문: `contact_type`(✓, `email` \| `telephone`), `contact`(✓), `verification_token?`(그 연락처로 받은 본인 확인 토큰, `identity_verification.required_for.login_id_recovery`)
- 응답:
  - 토큰 없음 → `202` 항상 같은 응답 `{"status": "accepted", "message": "If an account uses this contact, we sent its sign-in id there."}`. 등록된 연락처면 그곳으로 id를 보냅니다.
  - 토큰 있음(연락처 소유가 확인됨) + `account_recovery.login_id_recovery.delivery: show_masked_after_verification` → `200` `{"items": [{"login_id": "ja***e", "joined_at": "2025-12", "login_methods": ["password", "linked_login"]}], "total": 1}` (일부 가림)
- 에러: `invalid_contact_format` → null · `verification_expired` → null · `rate_limited` → null
- 멱등성: 읽기와 같음(발송은 `rate_limits`로 제한).

### `PUT /v1/customer/me/password` — 사람 전용
- 누가: 로그인한 회원 본인.
- 본문: `current_password`(✓, 비밀번호가 아직 없는 계정은 대신 `verification_token`), `new_password`(✓)
- 응답 `200`: `{"ok": true, "id": "cus_alex", "sessions_revoked": 2}` — `password.on_change`(기본: 지금 세션 말고 모두 폐기). `password.rotation_reminder_days`가 있으면 그 날짜 계산을 다시 시작합니다(강제 변경은 없음).
- 에러: `current_password_wrong`(로그인 실패와 같이 `password.lockout`에 셈) → null · `password_policy` → null · `password_reused` → null
- 멱등성: `Idempotency-Key`.

---

## 구현 메모 (백엔드, 2026-11)

명세를 코드로 옮기며 고르거나 고친 것입니다.

- **다른 도메인에 닿는 일은 스텁입니다.** customer는 의존 방향상 promotion·messaging·sales를 부를 수 없습니다([../backend/README.md](../backend/README.md) §2.1). 그래서 (1) 이메일·SMS(본인 확인 코드, 재설정 링크, 문의 답변, 해결 안내, 등급 알림, 탈퇴 완료 안내)는 기록만 하고 결과에 `delivered: false`와 이유를 적고, (2) 보상 지급의 포인트·쿠폰과 등급 진입 쿠폰은 promotion의 `grant_loyalty`·`issue_coupon`을 `next_tool`로 이름만 대며, (3) 현금 지급은 결제사 스텁(`reference: stub_…`)이 즉시 확정합니다. 탈퇴의 쿠폰 무효·구독 해지·연결 로그인 해제·적립금·충전금도 `settled[].connected: false`로 남습니다.
- **주문만 대상인 보상**(`request_remedy`의 `target: order`)은 주문이 sales에 있어 확인할 수 없습니다. 같은 주문의 문의가 있으면 그 고객을 쓰고, 없으면 고객 없이 기록합니다.
- **탈퇴 자격**의 `pending_refund`, `loyalty_points`, `store_credit`은 읽을 길이 없어 `not_checked`에 이름을 올립니다(추측하지 않음). `open_order`·`open_claim`은 sales·fulfillment가 등록한 읽기로 봅니다. 정책이 `warn`인 항목은 `warnings`에 갑니다.
- **등급 평가**는 회원마다 sales의 주문 읽기를 부릅니다. 주문 읽기가 연결되지 않은 곳에서는 아무도 움직이지 않고 결과가 그렇게 말합니다(`measured: false`). 초대(`invite`)·수동처럼 주문으로 잴 수 없는 요건이 `any`로 묶인 등급은 유지됩니다. `scope: segment`는 세그먼트가 promotion 소유라 아직 받지 않습니다(`invalid_input`).
- **실패한 시도의 기록**: `verify_identity`의 틀린 코드는 프리뷰 중에 시도 횟수만 따로 커밋합니다(상태 변경이 아니라 보안 기록, 가림 해제 사유 기록과 같은 방식).
- **세션 폐기**(재설정·비밀번호 변경·탈퇴)는 게이트웨이의 `platform.session`에 `revoked_at`을 씁니다. commerce가 세션을 끝낼 다른 길이 없어 이 한 문장만 그 테이블에 닿습니다.
- `check_withdrawal_eligibility`의 운영자 경로는 같은 `GET /v1/customer/people/me/withdrawal-eligibility`에 `customer_id`를 쿼리로 줍니다.
