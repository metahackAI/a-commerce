# API 명세 — `platform` (정책·변경셋·액션 로그·의미 enum·인증·스토어·권한)

> 계층: `platform.{resource}.{operation}`. 공통 규약(응답 형식·에러·프리뷰/실행·멱등·헤더)은 [README.md](README.md) §3, 데이터 타입은 [data-model.md](data-model.md), 정책 데이터는 [policies/](policies/README.md). MVP 툴(`get_policy`, `verify_change_set`)의 시그니처와 변경셋 생명주기·사람 전용 엔드포인트는 [../plan.md](../plan.md) §3.2·§3.5·§3.6과 같고, 설계 배경은 [../architecture.md](../architecture.md) §3.3·§5.5를 따릅니다.
>
> **설계 축**: ① JWT(액세스·리프레시)에 역할·스코프. ② `X-Tenant-Id`는 토큰의 스토어 클레임과 같아야 함(다르면 거부). ③ enum은 숫자·문자 코드 대신 의미 있는 단어로 주고, 각 값의 뜻과 허용 전이를 함께 노출. ④ 정책은 데이터로, 값마다 한 줄 뜻과 함께. ⑤ 쓰기 프리뷰는 변경셋으로 묶이고, 승인·되돌리기는 사람만 누르는 엔드포인트. ⑥ 모든 툴 호출·프리뷰·승인·실행은 schema.org `Action`으로 액션 로그에 남음.
>
> **응답 형식**: 읽기 `{items, total}`, 프리뷰 `{preview_id, changes[{op, entity, id, from, to}], checks[{rule, passed, detail, fix_tool}], needs_approval, approval_reason}`, 실행 `{ok, id, undo{tool, until}}`, 에러 `{error: {code, message, next_tool}}`. 목록은 `limit`·`offset`.

리소스: [policy](#policy) · [change_set](#change_set) · [action_log](#action_log) · [enum](#enum) · [auth](#auth) · [tenant](#tenant) · [role](#role) · [menu](#menu) · [admin_user](#admin_user) · [org](#org) · [api_key](#api_key)

---

## policy

### `get_policy` — Kind: R  (MVP)
- 논리주소/REST: `platform.policy.get` · `GET /v1/platform/policies`
- 요약: 스토어 정책 값을 값마다 한 줄 뜻과 함께 돌려줍니다. 이름을 주면 그 정책만, 없으면 전부. 에이전트는 프리뷰 전에 이 값으로 계획을 세우고, 프리뷰의 `checks[].rule`은 같은 키를 씁니다.
- consumes → produces: → policy
- 입력: `name?`(예 `"max_discount"`)
- 출력:
```jsonc
{"items": [
  {"name": "max_discount.vip", "value": 30, "unit_text": "percent",
   "meaning": "A VIP sale price can be at most 30% below the list price."},
  {"name": "max_discount.everyone", "value": 20, "unit_text": "percent",
   "meaning": "A sale for everyone can be at most 20% off."}],
 "total": 2}
```
- 프리뷰: 해당 없음(읽기)
- 정책: 값은 [policies/](policies/README.md)에서 읽음. 운영 비밀은 정책에 넣지 않음
- 에러: `policy_not_found` → `list_policies`
- 멱등성: 읽기(부작용 없음)

### `list_policies` — Kind: R
- 논리주소/REST: `platform.policy.list` · `GET /v1/platform/policies/index`
- 요약: 로드된 정책 파일 목록(이름·버전·도메인·한 줄 요약). 값은 `get_policy`.
- consumes → produces: → policy names
- 입력: `domain?`, `limit=50`, `offset=0`
- 출력: `{items: [{name: "promotion", version: 3, domain: "promotion", summary: "Discount caps, minimum stock, overlap and stacking"}], total: 9}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_filter` → `list_policies`
- 멱등성: 읽기(부작용 없음)

---

## change_set

> 변경셋(`acom:ChangeSet`, `ItemList`의 하위 타입)은 한 요청에서 나온 프리뷰들을 묶은 것입니다. 각 항목은 `action_status: "potential"`인 Action이고, 묶음 전체에 `version`과 되돌리기 기한이 있습니다.
>
> **생명주기**
> ```
> drafting → waiting_for_approval → running → verifying → live
>                     ↑  │                         └→ failed (맞지 않은 변경을 표시)
>      대화로 수정 ┘  └→ rejected                    live → undone (되돌리기 기한까지)
> ```
> - `drafting`: 하네스가 프리뷰를 모으는 중.
> - `waiting_for_approval`: 운영자에게 변경셋 카드가 보임. 대화로 고치면 프리뷰를 다시 돌리고 버전이 오름(v1 → v2).
> - `running`: 승인 뒤 실행기가 각 툴을 `preview_id`로 실행(#1042는 `set_sale_prices`, `create_coupon`, `schedule_banner` 뒤 `verify_change_set`).
> - `verifying` → `live`: `verify_change_set`이 모든 변경을 다시 읽어 맞으면 `live`, 하나라도 다르면 `failed`.
> - `rejected`: 운영자가 거절. `undone`: 운영자가 되돌림. 되돌리기 기한은 프로모션 종료 + 1일(#1042는 Nov 9, 23:59, [approval.yml](policies/approval.yml) `undo_window_after_end_days`).
>
> **사람 전용 엔드포인트**(툴이 아니며 모델은 보지 못함)
> | 엔드포인트 | 누가 | 무엇 |
> |---|---|---|
> | `POST /change-sets/{id}/approve` `{version}` | 운영자 | 실행 시작. 최신 버전이 아니면 거부(`version_out_of_date`). 액션 로그에 `AuthorizeAction` |
> | `POST /change-sets/{id}/undo` | 운영자 | 변경셋의 모든 변경을 함께 되돌림. 기한이 지나면 거부(`undo_window_closed`) |

### `verify_change_set` — Kind: R  (MVP)
- 논리주소/REST: `platform.change_set.verify` · `GET /v1/platform/change-sets/{change_set_id}/verification`
- 요약: 실행된 변경셋의 모든 변경을 저장소에서 다시 읽어 프리뷰의 `to`와 비교하고, 변경마다 `matches: true/false`를 돌려줍니다. 실행기가 마지막에 부르며, 결과에 따라 변경셋이 `live` 또는 `failed`가 됩니다. 부작용이 없으므로 언제든 다시 불러도 됩니다.
- consumes → produces: `change_set_id` → verification
- 입력: `change_set_id`(예 `"cs_1042"`)
- 출력:
```jsonc
{"items": [
  {"op": "update", "entity": "price", "id": "prd_windbreaker", "expected": 71.20, "actual": 71.20, "matches": true},
  {"op": "update", "entity": "price", "id": "prd_overshirt", "expected": 67.15, "actual": 67.15, "matches": true},
  {"op": "create", "entity": "coupon", "id": "cpn_vipweekend20", "expected": "VIPWEEKEND20", "actual": "VIPWEEKEND20", "matches": true},
  {"op": "schedule", "entity": "banner", "id": "dsp_1042", "expected": "slot_main_hero vip Nov 7–8", "actual": "slot_main_hero vip Nov 7–8", "matches": true}],
 "total": 42,
 "all_match": true}
```
- 프리뷰: 해당 없음(읽기). Action 종류는 `AssessAction`
- 정책: 없음
- 에러: `change_set_not_found` → `search_action_log` · `change_set_not_run` → `verify_change_set`(실행 전이면 아직 읽을 것이 없음)
- 멱등성: 읽기(부작용 없음)

---

## action_log

> 액션 로그의 항목은 schema.org `Action`입니다(`UpdateAction`, `CreateAction`, `ScheduleAction`, `AssessAction`, `AuthorizeAction` …).
> - `action_status`: `potential`(프리뷰, 승인 대기) · `active`(실행 중) · `completed`(실행·검증 완료, `result` 포함) · `failed`(`error` 포함)
> - `agent`: 누가 — 모델(`SoftwareApplication`) 또는 사람(`Person`)
> - `object`: 무엇에 — 대상 엔티티
> - `result`: 무엇이 됐나 — 예 새 `UnitPriceSpecification`
> - `instrument`: 어떤 툴로 — 툴 이름(예 `"set_sale_prices"`)
> - 그 밖에 `start_time`, `end_time`, `acom:change_set_id`, `acom:preview_id`
>
> 로그는 추가만 되고 고치지 않습니다.

### `search_action_log` — Kind: R
- 논리주소/REST: `platform.action_log.search` · `GET /v1/platform/action-logs`
- 요약: 액션 로그를 조건으로 찾습니다. 감사, 분쟁 확인, 평가 지표(호출 수·토큰·지연)에 씁니다.
- consumes → produces: → `action_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | agent | Text | | 모델 이름 또는 사람 id |
  | instrument | Text | | 툴 이름, 예 `"set_sale_prices"` |
  | object_id | Text | | 대상 id, 예 `"prd_windbreaker"` |
  | change_set_id | Text | | 예 `"cs_1042"` |
  | action_status | enum | | `potential` \| `active` \| `completed` \| `failed` |
  | start_time_from / start_time_to | DateTime | | 기간 |
  | limit / offset | Integer | | `20` / `0` |
- 출력:
```jsonc
{"items": [{
  "@type": "UpdateAction", "identifier": "act_1042_03",
  "action_status": "completed",
  "agent": {"@type": "SoftwareApplication", "name": "muse-glimmer-30b"},
  "instrument": "set_sale_prices",
  "object": {"@type": "Offer", "identifier": "off_windbreaker_m_slate"},
  "result": {"@type": "UnitPriceSpecification", "price": 71.20, "price_currency": "USD",
             "price_type": "sale_price", "valid_for_member_tier": "tier_vip",
             "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00"},
  "start_time": "2026-11-06T11:07:12-05:00", "end_time": "2026-11-06T11:07:13-05:00",
  "acom:change_set_id": "cs_1042", "acom:preview_id": "pv_8f2c",
  "acom:tokens": {"input": 1840, "output": 220}, "acom:latency_ms": 742}],
 "total": 47}
```
- 프리뷰: 해당 없음(읽기)
- 정책: 개인정보는 기본 가림. 다른 스토어 조회는 플랫폼 관리자 스코프만
- 에러: `forbidden` → `list_roles` · `invalid_filter` → `search_action_log`
- 멱등성: 읽기(부작용 없음)

### `get_action_log` — Kind: R
- 논리주소/REST: `platform.action_log.get` · `GET /v1/platform/action-logs/{action_id}`
- 요약: 액션 하나를 펼쳐 보여줍니다(프리뷰의 `changes`·`checks` 전체, 승인한 사람, 정책 버전).
- consumes → produces: `action_id` → action
- 입력: `action_id`, `fields?[]`
- 출력: `{items: [Action], total: 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: `X-Tenant-Id` 범위 안에서만
- 에러: `action_not_found` → `search_action_log` · `forbidden` → `list_roles`
- 멱등성: 읽기(부작용 없음)

---

## enum

### `list_enums` — Kind: R
- 논리주소/REST: `platform.enum.list` · `GET /v1/platform/enums`
- 요약: enum마다 값, 쇼퍼·운영자용 라벨, 뜻, 순서, 허용 전이를 돌려줍니다. 숫자·문자 코드 대신 의미 있는 단어만 쓰므로, 에이전트는 값을 올바르게 고르고 검증할 수 있습니다. 쓰기 입력 검증도 이 집합을 씁니다.
- consumes → produces: → enum definitions
- 입력: `names?[]`(예 `["order_status", "item_availability"]`), `applies_to?`(예 `"sales.order"`), `in_language?`(기본 스토어 언어)
- 출력:
```jsonc
{"items": [
  {"name": "order_status", "applies_to": ["sales.order"], "schema_org": "OrderStatus",
   "values": [
     {"value": "payment_due", "label": "Payment due", "order": 1},
     {"value": "processing",  "label": "Processing",  "order": 2},
     {"value": "in_transit",  "label": "In transit",  "order": 3},
     {"value": "delivered",   "label": "Delivered",   "order": 4},
     {"value": "cancelled",   "label": "Cancelled",   "order": 5, "terminal": true}],
   "transitions": {"payment_due": ["processing", "cancelled"], "processing": ["in_transit", "cancelled"], "in_transit": ["delivered"]}},
  {"name": "item_availability", "applies_to": ["catalog.product", "catalog.inventory"], "schema_org": "ItemAvailability",
   "values": [
     {"value": "in_stock", "label": "In stock"}, {"value": "out_of_stock", "label": "Out of stock"},
     {"value": "pre_order", "label": "Pre-order"}, {"value": "discontinued", "label": "Discontinued", "terminal": true}]},
  {"name": "change_set_status", "applies_to": ["platform.change_set"],
   "values": [{"value": "drafting"}, {"value": "waiting_for_approval"}, {"value": "running"}, {"value": "verifying"},
              {"value": "live"}, {"value": "failed", "terminal": true}, {"value": "rejected", "terminal": true}, {"value": "undone", "terminal": true}]}],
 "total": 3}
```
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `enum_not_found` → `list_enums` · `unsupported_language` → `list_enums`
- 멱등성: 읽기(부작용 없음)

---

## auth

### `login` — Kind: W
- 논리주소/REST: `platform.auth.login` · `POST /v1/platform/auth/login`
- 요약: 자격 증명(또는 외부 에이전트의 OAuth 코드)으로 액세스·리프레시 JWT를 받습니다. 액세스 토큰에는 역할·스코프·스토어가 담깁니다.
- consumes → produces: → access token
- 입력: `grant_type`(`password` | `oauth_code` | `client_credentials`), `principal?`(이메일), `credential`, `tenant_id`(`X-Tenant-Id`와 같아야 함), `requested_scopes?[]`(실제 발급은 역할과의 교집합. 외부 에이전트는 읽기·장바구니만)
- 출력: `{ok: true, id: "ses_7Q2", access_token: "...", refresh_token: "...", token_type: "Bearer", expires_in: 900, claims: {sub, tenant_id, roles, scopes}}`
- 프리뷰: 없음. 토큰 발급은 바로 실행
- 정책: [customer.yml](policies/customer.yml)(본인 확인·휴면·차단)
- 에러: `invalid_credentials` → `login` · `tenant_mismatch` → `login` · `account_locked` → `login` · `scope_not_granted` → `list_roles`
- 멱등성: 발급은 상태가 없어 필요 없음. 리프레시 토큰은 한 번만 쓰고 교체

### `refresh_token` — Kind: W
- 논리주소/REST: `platform.auth.refresh` · `POST /v1/platform/auth/refresh`
- 요약: 리프레시 토큰으로 새 액세스 토큰(과 교체된 리프레시 토큰)을 받습니다. 이전 리프레시 토큰은 바로 무효이고, 재사용이 감지되면 모든 세션을 끊습니다.
- consumes → produces: refresh token → access token
- 입력: `refresh_token`. 스토어·스코프는 원래 토큰에서 이어받고 넓힐 수 없음
- 출력: `login`과 같은 형식
- 프리뷰: 없음
- 정책: 리프레시 토큰 교체, 세션 수 상한
- 에러: `refresh_expired` → `login` · `refresh_reused` → `login` · `revoked` → `login`
- 멱등성: 한 번만 쓸 수 있음

### `logout` — Kind: W
- 논리주소/REST: `platform.auth.logout` · `POST /v1/platform/auth/logout`
- 요약: 현재 세션(또는 모든 세션)의 리프레시 토큰을 폐기합니다. 액세스 토큰은 짧은 만료까지 유효합니다.
- consumes → produces: →
- 입력: `scope?`(`current` | `all_sessions`, 기본 `current`)
- 출력: `{ok: true, id: "ses_7Q2", undo: null}`
- 프리뷰: 없음
- 정책: 없음
- 에러: `already_revoked` → `login`
- 멱등성: 다시 불러도 같은 결과

---

## tenant

### `get_tenant` — Kind: R
- 논리주소/REST: `platform.tenant.get` · `GET /v1/platform/tenants/{tenant_id}`
- 요약: 스토어 정보(이름·언어·통화·시간대·기능 플래그·정책 재정의 요약).
- consumes → produces: `tenant_id` → tenant
- 입력: `tenant_id`(`X-Tenant-Id`와 같아야 함), `fields?[]`
- 출력: `{items: [{id: "t_northwind", name: "Northwind Apparel", url: "northwind.example", in_language: "en", price_currency: "USD", acom:time_zone: "America/New_York", acom:feature_flags: {}, acom:status: "active"}], total: 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: 다른 스토어 조회는 플랫폼 관리자만
- 에러: `tenant_mismatch` → `get_tenant` · `forbidden` → `list_roles` · `tenant_not_found` → `list_tenants`
- 멱등성: 읽기(부작용 없음)

### `list_tenants` — Kind: R
- 논리주소/REST: `platform.tenant.list` · `GET /v1/platform/tenants`
- 요약: 전체 스토어 목록. 플랫폼 관리자 전용입니다.
- consumes → produces: → `tenant_id[]`
- 입력: `status?`, `limit=20`, `offset=0`
- 출력: `{items: Tenant[], total}`
- 프리뷰: 해당 없음(읽기)
- 정책: `platform:admin` 스코프
- 에러: `forbidden` → `list_roles`
- 멱등성: 읽기(부작용 없음)

---

## role

### `list_roles` — Kind: R
- 논리주소/REST: `platform.role.list` · `GET /v1/platform/roles`
- 요약: 역할 목록(이름·스코프·시스템 역할 여부).
- consumes → produces: → `role_id[]`
- 입력: `limit=20`, `offset=0`
- 출력: `{items: [{id: "rol_merchandiser", name: "Merchandiser", scopes: ["catalog:read", "promotion:write"], acom:is_system: true}], total}`
- 프리뷰: 해당 없음(읽기)
- 정책: 관리자 스코프
- 에러: `forbidden` → `list_roles`
- 멱등성: 읽기(부작용 없음)

### `get_role` — Kind: R
- 논리주소/REST: `platform.role.get` · `GET /v1/platform/roles/{role_id}`
- 요약: 역할 하나(스코프·권한·배정 인원).
- consumes → produces: `role_id` → role
- 입력: `role_id`
- 출력: `{items: [{id, name, scopes, permissions, acom:assigned_count}], total: 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: 관리자 스코프
- 에러: `role_not_found` → `list_roles`
- 멱등성: 읽기(부작용 없음)

### `create_role` — Kind: W
- 논리주소/REST: `platform.role.create` · `POST /v1/platform/roles`
- 요약: 사용자 정의 역할(스코프·권한 묶음)을 만듭니다. 시스템 역할은 고칠 수 없습니다.
- consumes → produces: → `role_id`
- 입력: `name`, `scopes[]`, `permissions?[]`, `description?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id: "rol_returns_desk", undo}`
- 프리뷰: checks `known_scopes`(없는 스코프 거부). `needs_approval: false`(배정은 `assign_role`이 승인).
- 정책: 관리자 전용
- 에러: `duplicate_role` → `list_roles` · `invalid_scope` → `list_permissions`
- 멱등성: `Idempotency-Key = hash(tenant + name)`

### `assign_role` — Kind: W$
- 논리주소/REST: `platform.role.assign` · `POST /v1/platform/roles/{role_id}/assignments`
- 요약: 관리자 계정에 역할을 주거나 뺍니다. 권한이 바뀌므로 항상 변경셋 승인을 거칩니다.
- consumes → produces: `role_id`, `admin_user_id` → `role_id`
- 입력: `role_id`, `admin_user_id`, `action`(`grant` | `revoke`), `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`grant`/`revoke`. checks `least_privilege`. 항상 `needs_approval: true`, 예 `approval_reason: "Gives refund rights to 1 person"`.
- 정책: [approval.yml](policies/approval.yml)
- 에러: `already_assigned` → `get_role` · `role_not_found` → `list_roles` · `admin_user_not_found` → `search_admin_users`
- 멱등성: `(role_id, admin_user_id)`로 한 건

### `list_permissions` — Kind: R
- 논리주소/REST: `platform.role.list_permissions` · `GET /v1/platform/permissions`
- 요약: 역할을 설계할 때 고를 수 있는 권한 목록.
- consumes → produces: → permissions
- 입력: `domain?`, `limit=50`, `offset=0`
- 출력: `{items: [{permission: "promotion:write", resource: "promotion.offer", action: "write", description: "Create and change member sale prices"}], total}`
- 프리뷰: 해당 없음(읽기)
- 정책: 관리자 스코프
- 에러: `forbidden` → `list_roles`
- 멱등성: 읽기(부작용 없음)

---

## menu

### `list_menus` — Kind: R
- 논리주소/REST: `platform.menu.list` · `GET /v1/platform/menus`
- 요약: 현재 역할로 볼 수 있는 운영 화면 메뉴 트리와 즐겨찾기.
- consumes → produces: → `menu_id[]`
- 입력: 없음
- 출력: `{items: [{id, name, path, required_scope, children: []}], total, favorites: []}`
- 프리뷰: 해당 없음(읽기)
- 정책: 역할 스코프로 걸러짐
- 에러: `forbidden` → `login`
- 멱등성: 읽기(부작용 없음)

### `set_menu` — Kind: W
- 논리주소/REST: `platform.menu.set` · `PUT /v1/platform/menus/{menu_id}`
- 요약: 메뉴 정의(경로·이름·필요 스코프·순서·표시 여부)를 정합니다. 관리자 전용.
- consumes → produces: → `menu_id`
- 입력: `menu_id`, `name?`, `path?`, `required_scope?`, `position?`, `visible?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: checks `known_scopes`. `needs_approval: false`.
- 정책: 관리자 스코프
- 에러: `invalid_scope` → `list_permissions` · `menu_not_found` → `list_menus`
- 멱등성: `Idempotency-Key`

### `set_favorite` — Kind: W
- 논리주소/REST: `platform.menu.set_favorite` · `PUT /v1/platform/menus/{menu_id}/favorite`
- 요약: 내 즐겨찾기 메뉴를 켜거나 끕니다.
- consumes → produces: `menu_id` → `menu_id`
- 입력: `menu_id`, `favorited`(Boolean), `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `needs_approval: false`
- 정책: 없음
- 에러: `menu_not_found` → `list_menus`
- 멱등성: 상태 하나로 덮어씀

---

## admin_user

### `search_admin_users` — Kind: R
- 논리주소/REST: `platform.admin_user.search` · `GET /v1/platform/admin-users`
- 요약: 관리자 계정 목록. 개인정보는 기본 가림.
- consumes → produces: → `admin_user_id[]`
- 입력: `role_id?`, `status?`(`active` | `suspended` | `locked`), `org_id?`, `query?`, `limit=20`, `offset=0`
- 출력: `{items: [{id: "adm_son", name: "Son SeongJun", job_title: "Merchandiser", roles: ["rol_merchandiser"], status: "active"}], total}`
- 프리뷰: 해당 없음(읽기)
- 정책: 관리자 스코프
- 에러: `forbidden` → `list_roles`
- 멱등성: 읽기(부작용 없음)

### `create_admin_user` — Kind: W$
- 논리주소/REST: `platform.admin_user.create` · `POST /v1/platform/admin-users`
- 요약: 관리자 계정을 만들고 역할·조직을 줍니다. 권한을 가진 사람이 새로 생기므로 항상 변경셋 승인. 첫 로그인 때 비밀번호를 바꾸게 합니다.
- consumes → produces: `role_id[]`, `org_id?` → `admin_user_id`
- 입력: `name`, `email`, `role_ids[]`, `org_id?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id: "adm_kim", undo}`
- 프리뷰: `changes`=`create` `admin_user`. checks `known_roles`, `email_unique`. 항상 `needs_approval: true`.
- 정책: [approval.yml](policies/approval.yml)
- 에러: `duplicate_email` → `search_admin_users` · `invalid_role` → `list_roles`
- 멱등성: `Idempotency-Key = hash(tenant + email)`

### `set_admin_status` — Kind: W$
- 논리주소/REST: `platform.admin_user.set_status` · `PUT /v1/platform/admin-users/{admin_user_id}/status`
- 요약: 관리자 계정을 정지·잠금 해제·복원합니다. 항상 변경셋 승인.
- consumes → produces: `admin_user_id` → `admin_user_id`
- 입력: `admin_user_id`, `status`(`active` | `suspended` | `locked`), `reason`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` `status` `from`/`to`. 항상 `needs_approval: true`.
- 정책: [approval.yml](policies/approval.yml)
- 에러: `already_in_state` → `search_admin_users` · `admin_user_not_found` → `search_admin_users`
- 멱등성: `Idempotency-Key`

---

## org

### `list_orgs` — Kind: R
- 논리주소/REST: `platform.org.list` · `GET /v1/platform/orgs`
- 요약: 조직(`Organization`) 트리(이름·종류·상위 조직·인원).
- consumes → produces: → `org_id[]`
- 입력: `parent_organization?`, `limit=50`, `offset=0`
- 출력: `{items: [{id, name, acom:org_type: "department" | "vendor" | "branch", parent_organization, acom:member_count}], total}`
- 프리뷰: 해당 없음(읽기)
- 정책: 관리자 스코프
- 에러: `forbidden` → `list_roles`
- 멱등성: 읽기(부작용 없음)

### `set_org` — Kind: W
- 논리주소/REST: `platform.org.set` · `PUT /v1/platform/orgs/{org_id}`
- 요약: 조직을 만들거나 고칩니다(이름·종류·상위 조직).
- consumes → produces: `org_id?` → `org_id`
- 입력: `org_id?`(없으면 새로), `name`, `org_type`(`department` | `vendor` | `branch`), `parent_organization?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: checks `no_cycle`(자기 자신·하위를 상위로 둘 수 없음). `needs_approval: false`.
- 정책: 관리자 스코프
- 에러: `circular_hierarchy` → `list_orgs` · `org_not_found` → `list_orgs`
- 멱등성: `Idempotency-Key`

---

## api_key

### `list_api_keys` — Kind: R
- 논리주소/REST: `platform.api_key.list` · `GET /v1/platform/api-keys`
- 요약: API 키 목록. 비밀 값은 돌려주지 않고 앞자리만 보여줍니다.
- consumes → produces: → `api_key_id[]`
- 입력: `status?`(`active` | `revoked`), `limit=20`, `offset=0`
- 출력: `{items: [{id, name, scopes, status, key_prefix: "ak_live_3f", last_used_at}], total}`
- 프리뷰: 해당 없음(읽기)
- 정책: 관리자 스코프
- 에러: `forbidden` → `list_roles`
- 멱등성: 읽기(부작용 없음)

### `issue_api_key` — Kind: W$
- 논리주소/REST: `platform.api_key.issue` · `POST /v1/platform/api-keys`
- 요약: 외부 에이전트·연동용 API 키를 발급합니다. 스코프는 발급자 역할과의 교집합으로 줄어듭니다. 비밀 값은 실행 응답에 한 번만 나옵니다. 항상 변경셋 승인.
- consumes → produces: → `api_key_id`
- 입력: `name`, `scopes[]`, `expires?`(DateTime), `preview`
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "key_31", secret: "(한 번만 표시)", undo}`
- 프리뷰: `changes`=`create` `api_key`. checks `scope_within_issuer`, `external_agent_scopes`. 항상 `needs_approval: true`.
- 정책: [approval.yml](policies/approval.yml). 비밀 값은 해시로만 저장
- 에러: `scope_not_granted` → `list_permissions` · `duplicate_key_name` → `list_api_keys`
- 멱등성: `Idempotency-Key = hash(tenant + name)`

### `revoke_api_key` — Kind: W$
- 논리주소/REST: `platform.api_key.revoke` · `POST /v1/platform/api-keys/{api_key_id}/revoke`
- 요약: API 키를 폐기합니다(`active` → `revoked`, 바로 무효). 항상 변경셋 승인.
- consumes → produces: `api_key_id` → `api_key_id`
- 입력: `api_key_id`, `reason`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo: null}`
- 프리뷰: `changes`=`update` `status` `active` → `revoked`. `detail`에 마지막 사용 시각. 항상 `needs_approval: true`.
- 정책: [approval.yml](policies/approval.yml)
- 에러: `already_revoked` → `list_api_keys` · `api_key_not_found` → `list_api_keys`
- 멱등성: 이미 폐기된 키는 같은 결과
