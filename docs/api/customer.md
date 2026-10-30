# API 명세 — `customer` (회원·등급·주소·동의·기억(선호)·위시리스트·임직원·상담·보상·FAQ/공지)

> 계층: `customer.{resource}.{operation}`. 공통 규약(응답 형식·에러·프리뷰/실행·멱등·헤더)은 [README.md](README.md) §3, 데이터 타입은 [data-model.md](data-model.md), 정책은 [customer.yml](policies/customer.yml) · [approval.yml](policies/approval.yml). MVP 툴의 시그니처는 [plan.md §3.2–§3.4](../plan.md)와 같습니다.
> **설계 축**: ① 개인정보는 기본 가림, 가림 해제는 사유를 액션 로그에 남김, ② 마케팅 발송은 동의(opt-in) 게이트, ③ 탈퇴·비밀번호는 **회원 본인만**(사람 전용 엔드포인트, 툴 아님). 에이전트는 비밀번호를 받지도 보지도 않고, 탈퇴는 자격 확인(읽기)까지만, ④ **등급은 코드가 아니라 데이터** — 등급 이름·개수·요건·혜택은 스토어가 정의, ⑤ **보이고 고칠 수 있는 기억** — 에이전트가 기억한 선호는 쇼퍼에게 한 줄로 보이고 한 번에 되돌릴 수 있음, ⑥ 보상은 귀책 분류 + 단계별 승인(현금은 지급 승인자 별도), ⑦ 동의·탈퇴·보상·등급 이력은 append만, ⑧ 본인 확인은 교체 가능한 방법(일회용 코드·링크·외부 제공자) 목록.

리소스: [person](#person) · [membership](#membership) · [address](#address) · [consent](#consent) · [preference](#preference) · [wishlist](#wishlist) · [employee](#employee) · [inquiry](#inquiry) · [remedy](#remedy) · [faq_notice](#faq_notice) · [사람 전용 엔드포인트](#사람-전용-엔드포인트)

- schema.org: 회원 `Person` + `ProgramMembership`, 등급 프로그램 `MemberProgram`(`has_tiers`), 등급 `MemberProgramTier`(`has_tier_requirement`, `has_tier_benefit`, `membership_points_earned`), 주소 `PostalAddress`, 기억 `acom:Preference`(`PropertyValue`의 확장), 상담 `acom:CustomerInquiry`(`Message`의 확장), 본인 확인 `acom:IdentityVerification`, 등급 평가 `acom:TierEvaluation`.
- id 예: 회원 `cus_jamie`, `cus_alex`, 프로그램 `prg_rewards`, 등급 `vip`(데모 스토어의 tier id, **예시**), 등급 평가 `tev_2612`, 본인 확인 `ivf_201`, 기억 `mem_12`.

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
- 출력: `{"items": [{"customer_id": "cus_alex", "name": "A*** K***", "member_of": {"program": "prg_rewards", "tier": "member", "membership_number": "NW-008812"}, "state": "active", "consent": {"marketing_email": true}}], "total": 1}`
- 정책: `privacy.masking_default`, `privacy.access_reason_logging`
- 에러: `customer_not_found` → `search_members` · `access_reason_required` → `get_member`
- 멱등성: 읽기.

### `register_member` — Kind: W
- 논리주소/REST: `customer.person.register` · `POST /v1/customer/people`
- 요약: 회원 가입(개인·사업자). 본인 확인 토큰과 필수 동의를 함께 받습니다. **비밀번호는 이 툴의 입력이 아닙니다.** 가입 화면은 같은 REST에 `password`를 직접 보낼 수 있고(사람 전용 필드, 모델에 노출되지 않음, 규칙은 `password`), 에이전트 경로로 가입하면 확인된 연락처로 비밀번호 설정 링크가 가고 회원이 사람 전용 `POST /password-reset/confirm`으로 설정합니다. 연결 로그인(`linked_login`)만 쓰는 계정은 비밀번호가 없어도 됩니다.
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
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력(프리뷰): `{"preview_id": "pv_m01a", "changes": [{"op": "create", "entity": "person", "id": "cus_new", "from": null, "to": "active"}], "checks": [{"rule": "identity_verification", "passed": true, "detail": "Phone verified 2 min ago", "fix_tool": "verify_identity"}, {"rule": "consent_mandatory", "passed": true, "detail": "Terms and privacy accepted", "fix_tool": "set_consent"}, {"rule": "rejoin_block", "passed": true, "detail": "No recent withdrawal matches this person", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "cus_8812", "undo": null}` — 새 회원은 프로그램의 entry tier에 들어갑니다(`tier.entry_tier`).
- 정책: `identity_verification.required_for.signup`, `consent.mandatory`, `withdrawal.rejoin`, `withdrawal.identifier_reuse`, `tier.entry_tier`
- 에러: `identity_not_verified` → `verify_identity` · `mandatory_consent_missing` → `set_consent` · `duplicate_member` → `check_availability` · `id_reuse_blocked` → `check_availability` · `rejoin_blocked` → (보관 기간 뒤 다시 가입, 메시지에 가능 날짜)
- 멱등성: `Idempotency-Key = hash(email + telephone)`

### `update_member` — Kind: W
- 논리주소/REST: `customer.person.update` · `PATCH /v1/customer/people/{customer_id}`
- 요약: 회원 정보 수정. 연락처(`telephone`, `email`)를 바꾸면 새 연락처로 본인 확인을 다시 합니다. 비밀번호는 바꾸지 않습니다(사람 전용 `PUT /me/password`).
- consumes → produces: `customer_id` → `customer_id`
- 입력: `customer_id`(✓), `name`, `telephone`, `email`, `verification_token`(연락처 변경 시 ✓), `preview`(✓)
- 출력(프리뷰): `{"preview_id": "pv_m02b", "changes": [{"op": "update", "entity": "person", "id": "cus_alex", "from": "a***@example.com", "to": "a***@example.org"}], "checks": [{"rule": "identity_verification", "passed": true, "detail": "Verified for contact change", "fix_tool": "verify_identity"}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "cus_alex", "undo": {"tool": "undo_change_set", "until": "2026-11-07T23:59:00"}}`
- 정책: `identity_verification.required_for.contact_change`
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
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력(프리뷰): `{"preview_id": "pv_v01a", "changes": [{"op": "create", "entity": "identity_verification", "id": "ivf_201", "from": null, "to": "code_sent"}], "checks": [{"rule": "verification_method_allowed", "passed": true, "detail": "email_code is allowed for signup", "fix_tool": "get_policy"}, {"rule": "verification_rate_limit", "passed": true, "detail": "1 code per minute, 5 per hour", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "ivf_201", "undo": null}` — 코드 확인 후 `{verification_token, expires_in: 300}`. 외부 제공자 방법이면 `{"ok": true, "id": "ivf_202", "redirect_url": "…"}`를 돌려주고, 제공자 콜백은 integration 도메인이 받아 같은 `ivf` id를 완료합니다.
- 정책: `identity_verification.methods`, `identity_verification.required_for`, `identity_verification.code`, `identity_verification.resend_cooldown_seconds`
- 에러: `method_not_enabled` → `get_policy` · `code_mismatch` → `verify_identity` · `verification_expired` → `verify_identity` · `too_many_attempts` → (새 확인 시작) · `rate_limited` → (메시지의 대기 시간 뒤)
- 멱등성: 코드 발송은 `(destination, resend_cooldown_seconds)` 제한, 토큰은 1회용.

### `check_availability` — Kind: R
- 논리주소/REST: `customer.person.check_availability` · `GET /v1/customer/people/availability?field=email&value=…`
- 요약: 가입 전 이메일·로그인 id 사용 가능 여부. 비회원도 호출 가능.
- consumes → produces: → availability
- 입력: `field`(✓, `email` \| `login_id`), `value`(✓)
- 출력: `{"items": [{"field": "email", "value": "alex@example.com", "available": false, "reason": "taken"}], "total": 1}` — `reason`: `taken` \| `reserved`(탈퇴 후 보관 기간 포함) \| `invalid_format`. 탈퇴한 계정의 것인지는 드러내지 않습니다.
- 정책: `withdrawal.identifier_reuse`
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

### `check_withdrawal_eligibility` — Kind: R
- 논리주소/REST: `customer.person.check_withdrawal_eligibility` · `GET /v1/customer/people/me/withdrawal-eligibility` (운영자: `GET /v1/customer/people/{customer_id}/withdrawal-eligibility`)
- 요약: 탈퇴하기 전에 **막는 것, 없어지는 것, 남는 것**을 한 번에 돌려줍니다. 쇼퍼 에이전트는 "탈퇴하면 어떻게 되나요?"에 이 결과로 답하고, 탈퇴 화면도 같은 결과를 보여 준 뒤 회원이 사람 전용 `POST /me/withdrawal`을 누릅니다. 운영 에이전트는 상담 응대를 위해 `customer_id`로 읽을 수 있지만(기본 가림) **탈퇴를 실행할 수는 없습니다**. 막힌 이유마다 해결할 다음 툴을 붙입니다.
- consumes → produces: (`customer_id`) → eligibility
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | customer_id | Text | | 기본 `me`(쇼퍼 본인). 운영자만 다른 id |
  | fields | Text[] | | 응답 축소. 예 `["eligible", "blockers"]` |
- 출력:
  ```jsonc
  {"items": [{
     "customer_id": "cus_jamie", "eligible": false, "checked_at": "2026-11-07T14:30:00",
     "blockers": [
       {"type": "open_order", "id": "ord_20431", "detail": "Delivered Nov 3; returns stay open until Nov 17", "clears_at": "2026-11-17T23:59:00", "next_tool": "get_order"},
       {"type": "open_inquiry", "id": "inq_771", "detail": "Inquiry still in progress", "clears_at": null, "next_tool": "get_inquiry"},
       {"type": "loyalty_balance", "id": null, "detail": "12.40 USD in points must be used or expire first", "amount": 12.40, "price_currency": "USD", "next_tool": "get_loyalty_balance"}],
     "on_withdrawal": [
       {"type": "store_credit", "detail": "25.00 USD is paid back to the original payment method", "amount": 25.00, "price_currency": "USD", "action": "pay_out"},
       {"type": "coupons", "detail": "3 unused coupons are voided", "count": 3, "action": "void"},
       {"type": "linked_login", "detail": "Sign-in with an external account is unlinked", "action": "unlink"},
       {"type": "subscription", "id": "sub_coffee", "detail": "Ends after the current period (Dec 1)", "action": "cancel_at_period_end"},
       {"type": "tier", "detail": "Gold status ends and does not carry over", "action": "remove"}],
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
     "reauthentication_methods": ["password", "email_code"]}],
   "total": 1}
  ```
  - `blockers[].type`: `open_order` \| `open_claim` \| `open_inquiry` \| `pending_refund` \| `pending_remedy` \| `loyalty_balance` \| `store_credit` \| `active_subscription` \| `linked_login`. 어떤 항목이 막는지(`block`)·알리기만 하는지(`warn`)는 정책이 정합니다.
  - `checked_at`은 탈퇴 요청의 `confirmation.eligibility_checked_at`에 그대로 씁니다.
- 정책: `withdrawal.blockers`, `withdrawal.balances`, `withdrawal.on_withdraw`, `withdrawal.data_handling`, `withdrawal.rejoin`, `withdrawal.identifier_reuse`, `withdrawal.cooling_off_days`, `privacy.masking_default`
- 에러: `not_signed_in` → (로그인 필요) · `already_withdrawn` → (없음) · `customer_not_found` → `search_members`
- 멱등성: 읽기, 부작용 없음.

### `send_password_reset` — Kind: W$
- 논리주소/REST: `customer.person.send_password_reset` · `POST /v1/customer/people/{customer_id}/password-reset`
- 요약: 상담 중 회원이 로그인하지 못할 때, 운영 에이전트가 **회원의 확인된 연락처로** 재설정 링크(또는 코드)를 보냅니다. 회원에게 닿는 쓰기이므로 항상 승인. 임시 비밀번호를 만들지 않고, 새 연락처로 보낼 수 없으며, 비밀번호를 보거나 정하지 않습니다. 회원은 받은 링크로 사람 전용 `POST /password-reset/confirm`을 거칩니다. 로그인 잠금은 재설정이 끝나면 풀립니다.
- consumes → produces: `customer_id` → `recovery_request_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | customer_id | Text | ✓ | `"cus_alex"` |
  | method | Text | | `account_recovery`에서 허용한 방법 id. 기본은 `identity_verification.required_for.password_reset`의 첫 항목 |
  | reason | Text | ✓ | `"Locked out after 5 attempts, asked by phone"` |
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력(프리뷰): `{"preview_id": "pv_r01a", "changes": [{"op": "create", "entity": "recovery_request", "id": "rcv_88", "from": null, "to": "link_sent"}], "checks": [{"rule": "recovery_contact_verified", "passed": true, "detail": "Sends to a***@example.com, verified 2026-04-02", "fix_tool": null}, {"rule": "account_recovery_rate_limit", "passed": true, "detail": "1 of 3 requests this hour", "fix_tool": null}], "needs_approval": true, "approval_reason": "Sends a password reset link to a customer"}`
- 출력(실행): `{"ok": true, "id": "rcv_88", "undo": null}` — 보낸 링크는 되돌릴 수 없고, 새 요청이 오면 이전 링크는 무효.
- 정책: `account_recovery.operator_initiated`, `account_recovery.rate_limits`, `password.lockout.unlock_by`
- 에러: `no_verified_contact` → `update_member` · `rate_limited` → (메시지의 대기 시간 뒤) · `member_not_active` → `get_member` · `customer_not_found` → `search_members`
- 멱등성: `Idempotency-Key = hash(customer_id + "password_reset" + hour)`

---

## membership

> **등급은 데이터입니다.** 스토어는 `MemberProgram`을 하나 이상 두고, 각 프로그램의 `has_tiers`에 등급(`MemberProgramTier`)을 원하는 만큼 둡니다. 등급 이름·개수·요건·혜택은 코드나 enum이 아니라 `define_tier_program`으로 저장한 정의입니다. 다른 도메인은 tier id만 참조합니다(`valid_for_member_tier`). 이 절의 `member`·`gold`·`vip`, 금액·회원 수는 모두 데모 스토어의 **예시**입니다.

**정의 형식** (`get_tier`가 돌려주고 `define_tier_program`이 받는 모양, 값은 예시):
```jsonc
{"program_id": "prg_rewards", "name": "Northwind Rewards", "version": 4,
 "evaluation": {"schedule": {"every": "month", "on_day": 1, "at": "03:00"}, "on_event": ["order_delivered"], "settle_days": 14},
 "movement": {"upgrade": "immediate", "downgrade": "at_schedule", "max_steps_down": 1, "grace_periods": 1},
 "has_tiers": [
   {"tier": "member", "name": "Member", "rank": 0, "has_tier_requirement": [], "has_tier_benefit": []},
   {"tier": "gold", "name": "Gold", "rank": 1,
    "requirement_combine": "any",
    "has_tier_requirement": [{"metric": "spend", "threshold": {"value": 300.00, "currency": "USD"},
                              "window": {"type": "rolling", "length": 6, "unit": "month"},
                              "counts": {"channels": ["web", "app"], "order_status": "delivered", "net_of_returns": true, "include_loyalty_spent": true}}],
    "has_tier_benefit": [{"type": "points_multiplier", "value": 1.5},
                         {"type": "coupon_on_entry", "coupon_id": "cpn_gold_welcome", "quantity": 1}],
    "membership_points_earned": {"value": 1.5, "unit_text": "points per USD"}},
   {"tier": "vip", "name": "VIP", "rank": 2,
    "requirement_combine": "any",
    "has_tier_requirement": [{"metric": "spend", "threshold": {"value": 800.00, "currency": "USD"}, "retain_threshold": {"value": 600.00, "currency": "USD"}},
                             {"metric": "invite"}],
    "has_tier_benefit": [{"type": "member_price"}, {"type": "free_shipping"}, {"type": "points_multiplier", "value": 2.0}],
    "membership_points_earned": {"value": 2.0, "unit_text": "points per USD"}}]}
```

| 요소 | 필드 | 설명 |
|---|---|---|
| 등급 | `tier`(id), `name`, `rank` | id는 스토어 안에서 유일. `rank`가 클수록 위, 중복 금지. 요건 없는 entry tier(최하위) 1개 필수 |
| 요건 `has_tier_requirement[]` | `metric` | `spend` \| `order_count` \| `points_earned` \| `manual`(set_member_tier로만) \| `invite`(초대 수락) \| `custom`(`custom_metric`: analytics 지표 id) |
| | `threshold`, `retain_threshold` | spend는 `{value, currency}`, 나머지는 수. 유지 기준을 진입 기준보다 낮게 두면 경계에서 오르내림이 줄어듦 |
| | `window` | `{type: rolling \| calendar \| lifetime, length, unit}` |
| | `counts` | `channels[]`, `order_status`, `net_of_returns`, `include_loyalty_spent` — 실적에 무엇을 셀지 |
| | `requirement_combine` | 요건 여러 개를 `any` \| `all` |
| 혜택 `has_tier_benefit[]` | `type` | `member_price`(이 tier가 `valid_for_member_tier` 대상) \| `points_multiplier`(`value`) \| `free_shipping`(`min_subtotal?`) \| `free_returns` \| `coupon_on_entry`(`coupon_id`, `quantity`) \| `recurring_coupon`(`coupon_id`, `every`) \| `custom`(`name`, `description`) |
| 적립 | `membership_points_earned` | `{value, unit_text}`. `points_multiplier` 혜택과 같은 값 |
| 프로그램 | `evaluation`, `movement` | 평가 일정(정기·이벤트), 실적 확정 대기, 승급·강등·유예 규칙. 생략하면 [customer.yml](policies/customer.yml) `tier`의 기본값 |

schema.org 대응: `member_price` → `TierBenefitLoyaltyPrice`, `points_multiplier` → `TierBenefitLoyaltyPoints`, `free_shipping` → `TierBenefitLoyaltyShipping`, `free_returns` → `TierBenefitLoyaltyReturns`, 나머지는 `acom:`.

### `list_tiers` — Kind: R
- 논리주소/REST: `customer.membership.list_tiers` · `GET /v1/customer/tiers`
- 요약: 스토어가 정의한 등급과 등급별 회원 수. 운영 에이전트가 "VIP 회원"(예시) 같은 말을 tier id로 바꿔 `set_sale_prices`·`create_coupon`의 `valid_for_member_tier`로 넘길 때 씁니다. 몇 개의 등급이 있든 같은 모양으로 돌려줍니다. MVP 툴([plan.md §3.2](../plan.md)). schema.org `MemberProgramTier`.
- consumes → produces: → `tier`
- 입력: 없음(MVP). 선택 `program_id`, `fields?[]`
- 출력(예시 — 데모 스토어 값):
  ```jsonc
  {"items": [{"tier": "vip", "name": "VIP", "rank": 2, "program_id": "prg_rewards", "member_count": 1284},
             {"tier": "gold", "name": "Gold", "rank": 1, "program_id": "prg_rewards", "member_count": 4210},
             {"tier": "member", "name": "Member", "rank": 0, "program_id": "prg_rewards", "member_count": 18930}],
   "total": 3}
  ```
  - 순서는 `rank` 내림차순. 프로그램이 없으면 빈 목록.
- 정책: `tier`
- 에러: 없음(읽기, 빈 목록 가능).
- 멱등성: 읽기.

### `get_tier` — Kind: R
- 논리주소/REST: `customer.membership.get_tier` · `GET /v1/customer/tiers/{tier}`
- 요약: 등급 하나의 전체 정의(요건·혜택·적립)와 속한 프로그램의 평가·이동 규칙, 회원 수, 이 등급을 가리키는 진행 중 프로모션·쿠폰 수. `define_tier_program`으로 바꾸기 전에 지금 값을 읽을 때 씁니다.
- consumes → produces: `tier` → tier definition
- 입력: `tier`(✓, 예 `"vip"`), `fields?[]`
- 출력(예시): `{"items": [{"tier": "vip", "name": "VIP", "rank": 2, "program": {"program_id": "prg_rewards", "version": 4, "evaluation": {"schedule": {"every": "month", "on_day": 1}}, "movement": {"downgrade": "at_schedule", "grace_periods": 1}}, "has_tier_requirement": [{"metric": "spend", "threshold": {"value": 800.00, "currency": "USD"}, "window": {"type": "rolling", "length": 6, "unit": "month"}}], "has_tier_benefit": [{"type": "member_price"}, {"type": "free_shipping"}], "membership_points_earned": {"value": 2.0, "unit_text": "points per USD"}, "member_count": 1284, "referenced_by": {"active_promotions": 2, "active_coupons": 1}}], "total": 1}`
- 정책: `tier`
- 에러: `tier_not_found` → `list_tiers`
- 멱등성: 읽기.

### `get_member_tier` — Kind: R
- 논리주소/REST: `customer.membership.get_member_tier` · `GET /v1/customer/people/{customer_id}/tier`
- 요약: 회원 한 명의 현재 등급, **왜 그 등급인지**, 다음 등급까지 얼마나 남았는지, 유지가 위태로운지, 다음 평가일. 쇼퍼는 `customer_id: "me"`로 자기 것을 봅니다(예시: "VIP까지 387.50 USD 남았어요").
- consumes → produces: `customer_id` → `tier`
- 입력: `customer_id`(✓, 쇼퍼는 `me`), `program_id?`
- 출력(예시):
  ```jsonc
  {"items": [{
     "customer_id": "cus_jamie", "program_id": "prg_rewards", "tier": "gold", "rank": 1, "since": "2026-06-01",
     "source": "evaluation",                  // evaluation | event | manual_override | entry
     "override": null,                        // 수동 지정이면 {reason, expires_at, after_expiry, by}
     "why": {"rule": "spend >= 300.00 USD over the last 6 months", "value": {"value": 412.50, "currency": "USD"}, "evaluation_id": "tev_2606"},
     "progress": [{"to_tier": "vip", "metric": "spend",
                   "current": {"value": 412.50, "currency": "USD"}, "threshold": {"value": 800.00, "currency": "USD"},
                   "remaining": {"value": 387.50, "currency": "USD"},
                   "pending": {"value": 120.00, "currency": "USD", "detail": "Counts after the return window closes on Nov 17"}}],
     "retention": {"status": "on_track", "retain_threshold": {"value": 300.00, "currency": "USD"}},   // on_track | at_risk | in_grace
     "next_evaluation": "2026-12-01T03:00:00",
     "has_tier_benefit": [{"type": "points_multiplier", "value": 1.5}, {"type": "coupon_on_entry", "coupon_id": "cpn_gold_welcome"}]}],
   "total": 1}
  ```
- 정책: `tier.requirement`, `tier.evaluation`, `tier.movement`
- 에러: `customer_not_found` → `search_members` · `no_tier` → (게스트·탈퇴 회원은 등급 없음)
- 멱등성: 읽기.

### `define_tier_program` — Kind: W$
- 논리주소/REST: `customer.membership.define_tier_program` · `PUT /v1/customer/tier-programs/{program_id}` (없으면 만들고 있으면 새 버전)
- 요약: 등급 프로그램과 등급을 만들거나 바꿉니다(등급 추가·삭제·이름·순서·요건·혜택·평가 일정·이동 규칙). 회원의 등급과 혜택이 바뀌므로 항상 승인. 프리뷰는 **지금 데이터로 평가했을 때 회원이 어느 등급에서 어느 등급으로 몇 명 움직이는지**를 `changes`에 보여 줍니다. 정의는 버전으로 남고 되돌릴 수 있습니다.
- consumes → produces: (`program_id`, `coupon_id`) → `program_id`, `tier[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | program_id | Text | ✓ | `"prg_rewards"` |
  | name | Text | ✓ | `"Northwind Rewards"`(예시) |
  | has_tiers | Object[] | ✓ | 위 정의 형식의 등급 전체 목록. 빠진 등급은 삭제로 봄 |
  | evaluation | Object | | 생략 시 `tier.evaluation` 기본값 |
  | movement | Object | | 생략 시 `tier.movement` 기본값 |
  | tier_migrations | Object[] | 등급 삭제 시 ✓ | `[{from_tier: "silver", to_tier: "gold"}]` — 삭제할 등급의 회원이 갈 곳 |
  | takes_effect | Text(enum) | | `next_evaluation`(기본) \| `immediately`(승인과 함께 평가 실행) |
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력(프리뷰, 예시 — VIP 기준을 800 → 1000 USD로 올리고 Platinum 추가):
  ```jsonc
  {"preview_id": "pv_t01a",
   "changes": [{"op": "update", "entity": "tier_requirement", "id": "vip", "from": 800.00, "to": 1000.00},
               {"op": "create", "entity": "member_program_tier", "id": "platinum", "from": null, "to": "rank 3"},
               {"op": "update", "entity": "membership", "id": "vip→gold", "from": "vip", "to": "gold", "count": 212},
               {"op": "update", "entity": "membership", "id": "vip→platinum", "from": "vip", "to": "platinum", "count": 96}],
   "checks": [{"rule": "tier_entry_present", "passed": true, "detail": "member is the entry tier", "fix_tool": null},
              {"rule": "tier_rank_unique", "passed": true, "detail": "Ranks 0–3, no duplicates", "fix_tool": null},
              {"rule": "tier_requirement_order", "passed": true, "detail": "Higher ranks need more spend", "fix_tool": "define_tier_program"},
              {"rule": "tier_referenced", "passed": true, "detail": "No removed tier is used by promotions or coupons", "fix_tool": "search_promotions"},
              {"rule": "tier_benefit_coupon", "passed": true, "detail": "cpn_platinum_welcome is active", "fix_tool": "create_coupon"},
              {"rule": "tier_member_impact", "passed": true, "detail": "212 move down to gold and 96 up to platinum at the next evaluation (Dec 1); 14 manual overrides stay", "fix_tool": "preview_tier_evaluation"}],
   "needs_approval": true,
   "approval_reason": "Changes the VIP requirement and adds a tier; 308 members would change tier"}
  ```
- 출력(실행): `{"ok": true, "id": "prg_rewards", "undo": {"tool": "undo_change_set", "until": "2026-11-30T23:59:00"}}` — 되돌리면 이전 버전 정의로 돌아갑니다. 이미 실행된 평가는 `run_tier_evaluation`의 되돌리기로.
- 정책: `tier.program_change`, `tier.programs_max`, `tier.tiers_max`, `tier.entry_tier`, `tier.rank`, [approval.yml](policies/approval.yml)
- 에러: `tier_rank_conflict` → `define_tier_program` · `entry_tier_missing` → `define_tier_program` · `tier_in_use` → `search_promotions` · `migration_required` → `define_tier_program`(`tier_migrations`와 함께) · `coupon_not_found` → `create_coupon` · `tier_limit_exceeded` → `list_tiers`
- 멱등성: `Idempotency-Key = hash(program_id + 정의)` — 같은 정의면 새 버전 없음.

### `preview_tier_evaluation` — Kind: R
- 논리주소/REST: `customer.membership.preview_tier_evaluation` · `GET /v1/customer/tier-programs/{program_id}/evaluation-preview`
- 요약: 지금(또는 `as_of` 시점) 평가하면 누가 오르고 내리고 유예에 들어가는지. 아무것도 바꾸지 않습니다. `group_by: "move"`면 이동 경로별 인원만.
- consumes → produces: `program_id` → `customer_id[]`
- 입력: `program_id`(✓), `as_of`(DateTime, 기본 지금), `scope`(`all` \| `segment_id` \| `customer_ids[]`, 기본 `all`), `direction`(`up` \| `down` \| `grace` \| `any`), `group_by`(`member`(기본) \| `move`), `fields?[]`, `limit`(기본 20), `offset`
- 출력(예시): `{"items": [{"customer_id": "cus_alex", "name": "A*** K***", "from": "gold", "to": "vip", "direction": "up", "reason": "spend 812.40 >= 800.00 USD over 6 months", "benefits_on_entry": [{"type": "coupon_on_entry", "coupon_id": "cpn_vip_welcome"}]}, {"customer_id": "cus_7710", "name": "R*** T***", "from": "vip", "to": "vip", "direction": "grace", "reason": "spend 540.00 below 600.00 retain threshold; 1 grace period left"}], "total": 518}` — `group_by: "move"`면 `{"items": [{"from": "gold", "to": "vip", "direction": "up", "members": 310}], "total": 4}`
- 정책: `tier.evaluation.settle_days`, `tier.movement`, `tier.override.evaluation_respects_override`, `privacy.masking_default`
- 에러: `program_not_found` → `list_tiers` · `as_of_in_future` → `preview_tier_evaluation`
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
  | scope | Object | | `{all: true}`(기본) \| `{segment_id}` \| `{customer_ids: [...]}` |
  | notify | Boolean | | 기본 `tier.notify.on`대로 |
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력(프리뷰, 예시): `{"preview_id": "pv_t02b", "changes": [{"op": "update", "entity": "membership", "id": "gold→vip", "from": "gold", "to": "vip", "count": 310}, {"op": "update", "entity": "membership", "id": "vip→gold", "from": "vip", "to": "gold", "count": 212}, {"op": "create", "entity": "coupon_issue", "id": "cpn_vip_welcome", "from": null, "to": "issued", "count": 310}, {"op": "create", "entity": "message", "id": "tpl_tier_change", "from": null, "to": "queued", "count": 522}], "checks": [{"rule": "tier_settle_window", "passed": true, "detail": "Orders delivered after Nov 16 count next cycle", "fix_tool": null}, {"rule": "tier_override_respected", "passed": true, "detail": "14 manual overrides left as they are", "fix_tool": null}, {"rule": "tier_evaluation_exclusive", "passed": true, "detail": "No other evaluation is running", "fix_tool": null}, {"rule": "customer_messages", "passed": true, "detail": "Tier notices are transactional; promotional wording would need marketing consent", "fix_tool": "create_message_template"}], "needs_approval": true, "approval_reason": "Moves 310 members up and 212 down, issues 310 coupons, sends 522 messages"}`
- 출력(실행): `{"ok": true, "id": "tev_2612", "undo": {"tool": "undo_change_set", "until": "2026-12-08T23:59:00"}}` — 되돌리면 등급을 되돌리고 이 실행이 발급한 미사용 쿠폰을 회수합니다. 보낸 메시지는 되돌릴 수 없습니다.
- 정책: `tier.evaluation`, `tier.movement`, `tier.override`, `tier.notify`, `tier.history`, [approval.yml](policies/approval.yml), `consent.marketing_gate`
- 에러: `evaluation_in_progress` → `list_tier_history` · `program_not_found` → `list_tiers` · `as_of_in_future` → `run_tier_evaluation` · `coupon_unavailable` → `create_coupon`
- 멱등성: `Idempotency-Key = hash(program_id + as_of + scope)` — 같은 시점 재실행은 첫 결과 반환.

### `set_member_tier` — Kind: W$
- 논리주소/REST: `customer.membership.set_member_tier` · `POST /v1/customer/people/{customer_id}/tier`
- 요약: 회원 한 명의 등급을 손으로 지정합니다(보상·초대·오류 정정). 사유와 만료가 반드시 있고, 만료 전에는 평가가 바꾸지 않습니다. `metric: manual`인 등급은 이 툴로만 들어갑니다. `clear_override: true`면 수동 지정을 지우고 다음 평가(또는 즉시 재평가)로 돌아갑니다.
- consumes → produces: `customer_id`, `tier` → `customer_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | customer_id | Text | ✓ | `"cus_alex"` |
  | tier | Text | ✓(해제 시 제외) | `"vip"`(예시) |
  | reason | Text | ✓ | `"Compensation for lost parcel, inq_771"` |
  | expires_at | DateTime | | 기본 지금 + `tier.override.max_days`, 그보다 길 수 없음 |
  | after_expiry | Text(enum) | | `re_evaluate`(기본) \| `revert_to_previous` |
  | issue_entry_benefits | Boolean | | 기본 `false` |
  | clear_override | Boolean | | `true`면 수동 지정 해제 |
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력(프리뷰): `{"preview_id": "pv_t03c", "changes": [{"op": "update", "entity": "membership", "id": "cus_alex", "from": "gold", "to": "vip"}], "checks": [{"rule": "tier_override_reason", "passed": true, "detail": "Reason recorded", "fix_tool": null}, {"rule": "tier_override_expiry", "passed": true, "detail": "Expires 2027-03-31, within the 365-day limit; then re-evaluated", "fix_tool": "set_member_tier"}], "needs_approval": true, "approval_reason": "Sets a member's tier by hand until 2027-03-31"}`
- 출력(실행): `{"ok": true, "id": "cus_alex", "undo": {"tool": "undo_change_set", "until": "2026-11-14T23:59:00"}}`
- 정책: `tier.override`, `tier.notify`, `tier.history`, [approval.yml](policies/approval.yml)
- 에러: `tier_not_found` → `list_tiers` · `reason_required` → `set_member_tier` · `override_too_long` → `set_member_tier` · `member_not_active` → `get_member` · `customer_not_found` → `search_members`
- 멱등성: `Idempotency-Key = hash(customer_id + tier + expires_at)`

### `list_tier_history` — Kind: R
- 논리주소/REST: `customer.membership.list_tier_history` · `GET /v1/customer/people/{customer_id}/tier-history`
- 요약: 한 회원의 등급 변경 이력(append만). 각 변경의 원인, 그때의 실적, 누가·왜 바꿨는지. 상담에서 "왜 등급이 내려갔나요?"에 답할 때 씁니다.
- consumes → produces: `customer_id` → history
- 입력: `customer_id`(✓, 쇼퍼는 `me`), `program_id?`, `from`, `through`, `limit`(기본 20), `offset`
- 출력(예시): `{"items": [{"at": "2026-06-01T03:00:00", "from": "member", "to": "gold", "cause": "evaluation", "evaluation_id": "tev_2606", "snapshot": {"metric": "spend", "value": {"value": 412.50, "currency": "USD"}}, "actor": "scheduler", "reason": null}, {"at": "2026-03-10T11:02:00", "from": "gold", "to": "vip", "cause": "manual_override", "actor": "adm_dana", "reason": "Compensation for lost parcel", "expires_at": "2026-05-31T23:59:00"}], "total": 4}` — `cause`: `entry` \| `evaluation` \| `event` \| `manual_override` \| `override_expired` \| `grace_started` \| `grace_ended` \| `program_change` \| `withdrawal`
- 정책: `tier.history`
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

회원 본인(또는 로그인하지 못한 사람)이 UI 화면·버튼으로만 부르는 HTTP 엔드포인트입니다. **툴이 아니며 모델은 보지 못합니다**([plan.md §3.6](../plan.md), [README.md §2](README.md)). 에이전트는 비밀번호를 받거나 보거나 정하지 않습니다.

| 엔드포인트 | 누가 | 무엇 |
|---|---|---|
| `POST /memories/{id}/undo` | 쇼퍼 | 기억한 선호 삭제. `memory_note` 블록의 Undo 버튼. 응답 `{"ok": true, "id": "mem_12"}` |
| `POST /me/withdrawal` | 회원 본인 | 탈퇴. 아래 상세 |
| `POST /password-reset/requests` | 누구나(비로그인) | 비밀번호 재설정 요청. 계정 유무와 관계없이 같은 응답 |
| `POST /password-reset/confirm` | 링크·코드를 받은 사람 | 새 비밀번호 설정, 세션 폐기 |
| `POST /account-recovery/login-id` | 누구나(비로그인) | 로그인 id 찾기 |
| `PUT /me/password` | 회원 본인 | 로그인한 상태에서 비밀번호 변경 |

- 공통: `Idempotency-Key` 필수(재설정 요청은 선택), 에러는 공통 형식(`next_tool` 대신 화면이 다음 단계를 안내하므로 `next_tool: null`). 정책 `withdrawal`, `identity_verification`, `account_recovery`, `password` ([customer.yml](policies/customer.yml)).
- 비밀번호는 이 엔드포인트들의 본문으로만 오가고, 툴 입력·출력·액션 로그·변경셋 어디에도 남지 않습니다. 비밀번호 규칙은 화면이 `get_policy`(`customer.password`)로 읽어 미리 보여 줍니다.

### `POST /me/withdrawal` — 사람 전용
- 누가: 로그인한 회원 본인. 운영자·에이전트 토큰은 `forbidden_initiator`로 거부합니다(`withdrawal.initiator: self_only`).
- 흐름: 탈퇴 화면이 `check_withdrawal_eligibility` 결과(막는 것·없어지는 것·남는 것)를 보여 줌 → 회원이 이유를 고르고 데이터 처리 요약에 동의 → 재인증 → 버튼.
- 본문:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | reasons | Text[] | ✓ | `withdrawal.reasons`의 값. 예 `["not_using", "too_many_messages"]` |
  | reason_text | Text | | 자유 입력(선택, 500자) |
  | confirmation | Object | ✓ | `{acknowledged_data_handling: true, eligibility_checked_at: "2026-11-07T14:30:00"}` — 확인한 자격 결과의 `checked_at`. `withdrawal.confirmation_max_age_minutes`보다 오래되면 거부 |
  | reauthentication | Object | ✓ | `{method: "password", credential: "…"}`(현재 비밀번호, 화면에서만) 또는 `{method: "email_code", verification_token: "…"}`. 허용 방법은 `identity_verification.required_for.withdrawal_reauth` |
- 응답 `200`:
  ```jsonc
  {"ok": true, "id": "cus_jamie",
   "status": "withdrawn",                      // withdrawn | pending_cooling_off
   "effective_at": "2026-11-07T14:36:00",
   "cancel_until": null,                       // cooling_off_days > 0이면 그때까지 로그인하면 취소
   "settled": [{"type": "store_credit", "amount": 25.00, "price_currency": "USD", "action": "pay_out"},
               {"type": "coupons", "count": 3, "action": "void"}],
   "data_retention": [{"data": "order_and_payment_records", "until": "2031-11-06", "why": "tax_and_consumer_protection"},
                      {"data": "claims_and_inquiries", "until": "2029-11-06", "why": "dispute_handling"},
                      {"data": "consent_history", "until": "2031-11-06", "why": "proof_of_consent"},
                      {"data": "rejoin_check_fingerprint", "until": "2027-02-05", "why": "prevent_signup_benefit_abuse"}],
   "rejoin_after": "2027-02-05", "login_id_reusable_after": "2027-02-05"}
  ```
- 에러: `withdrawal_blocked`(`detail`에 막는 항목 목록, 화면은 `check_withdrawal_eligibility` 결과로 안내) · `reauthentication_failed` · `reauthentication_locked`(`password.lockout`) · `confirmation_required` · `eligibility_changed`(확인 뒤 새 주문·상담이 생김, 다시 확인) · `already_withdrawn` · `forbidden_initiator`
- 탈퇴 후(모두 정책 데이터, [customer.yml](policies/customer.yml) `withdrawal`):
  - **상태**: 회원 상태 `withdrawn`, 세션·연결 로그인·기기 토큰 폐기, 마케팅 동의 철회, 등급 제거(`list_tier_history`에 `withdrawal`), 미사용 쿠폰 무효, 임직원 할인 한도 소멸, 구독은 정책대로 기간 끝에 해지. 완료 안내는 거래성 메시지로 한 번.
  - **개인정보 가림**: `data_handling.mask_now`의 필드는 즉시 가리고, `delete_now`의 데이터는 즉시 삭제. 리뷰·글·댓글은 `authored_content`(기본 익명화).
  - **보존**: 주문·결제·클레임·동의 기록은 `data_handling.retain`의 기간과 이유로 운영 데이터와 분리해 암호화 보관하고, 열람은 사유를 남깁니다. 재가입 확인용으로는 연락처·본인 확인 결과의 단방향 해시만 둡니다. 기간이 끝나면 `purge` 작업이 파기합니다. 기간은 스토어 관할 법령에 맞게 바꿉니다.
  - **재가입·id 재사용**: 같은 사람은 `rejoin.block_days` 동안 재가입할 수 없고, 재가입해도 가입 혜택·등급·적립금·쿠폰은 이어지지 않습니다. 로그인 id·이메일은 `identifier_reuse`(기본: 보관 기간 뒤 재사용 가능).
  - 막힌 시도는 사유와 함께 액션 로그에 남습니다(`on_withdraw.log_blocked_attempts`).
- 멱등성: `Idempotency-Key` — 재시도는 첫 결과 반환.

### `POST /password-reset/requests` — 사람 전용
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
- 에러: `invalid_identifier_format`(형식만 검사) · `rate_limited`(`account_recovery.rate_limits`, 계정 유무와 무관하게 같은 응답, `retry_after_seconds` 포함)
- 멱등성: 같은 식별자의 짧은 재시도는 같은 `request_id`.

### `POST /password-reset/confirm` — 사람 전용
- 누가: 링크 또는 코드를 받은 사람.
- 본문: `token`(링크 방식) 또는 `{request_id, code}`(코드 방식) 중 하나(✓), `new_password`(✓, `password` 규칙)
- 응답 `200`: `{"ok": true, "id": "cus_alex", "sessions_revoked": 4, "signed_in": false, "next": "sign_in"}` — `password.on_reset`대로 모든 세션 폐기, 로그인 잠금 해제, 완료 알림을 확인된 연락처로(`account_recovery.notify_on_reset`), 액션 로그에는 "재설정됨"만(비밀번호 없음).
- 에러: `invalid_or_expired`(없는 토큰·만료·사용됨을 구분하지 않음) · `too_many_attempts`(`account_recovery.confirm_max_attempts`를 넘으면 요청 무효, 새 요청 필요) · `password_policy`(`detail`에 어긴 규칙 `["min_length", "contains_login_id"]`) · `password_reused`(`password.history_block`)
- 멱등성: 토큰·코드는 1회용. 성공한 토큰의 재시도는 `invalid_or_expired`.

### `POST /account-recovery/login-id` — 사람 전용
- 누가: 로그인 id를 잊은 누구나.
- 본문: `contact_type`(✓, `email` \| `telephone`), `contact`(✓), `verification_token?`(그 연락처로 받은 본인 확인 토큰, `identity_verification.required_for.login_id_recovery`)
- 응답:
  - 토큰 없음 → `202` 항상 같은 응답 `{"status": "accepted", "message": "If an account uses this contact, we sent its sign-in id there."}`. 등록된 연락처면 그곳으로 id를 보냅니다.
  - 토큰 있음(연락처 소유가 확인됨) + `account_recovery.login_id_recovery.delivery: show_masked_after_verification` → `200` `{"items": [{"login_id": "ja***e", "joined_at": "2025-12", "login_methods": ["password", "linked_login"]}], "total": 1}` (일부 가림)
- 에러: `invalid_contact_format` · `verification_expired` · `rate_limited`
- 멱등성: 읽기와 같음(발송은 `rate_limits`로 제한).

### `PUT /me/password` — 사람 전용
- 누가: 로그인한 회원 본인.
- 본문: `current_password`(✓, 비밀번호가 아직 없는 계정은 대신 `verification_token`), `new_password`(✓)
- 응답 `200`: `{"ok": true, "id": "cus_alex", "sessions_revoked": 2}` — `password.on_change`(기본: 지금 세션 말고 모두 폐기). `password.rotation_reminder_days`가 있으면 그 날짜 계산을 다시 시작합니다(강제 변경은 없음).
- 에러: `current_password_wrong`(로그인 실패와 같이 `password.lockout`에 셈) · `password_policy` · `password_reused`
- 멱등성: `Idempotency-Key`.
