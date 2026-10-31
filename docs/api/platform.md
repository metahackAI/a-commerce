# API 명세 — `platform` (정책·변경셋·할 일·액션 로그·의미 enum·인증·스토어·권한)

> 계층: `platform.{resource}.{operation}`. 공통 규약(응답 형식·프리뷰/실행·멱등·헤더)은 [conventions.md](conventions.md)와 [README.md](README.md) §3, 에러는 [errors.md](errors.md), 값 목록은 [enums.md](enums.md), 데이터 타입은 [data-model.md](data-model.md), 정책 데이터는 [policies/](../../policies/README.md). 변경셋 엔진과 정책 엔진의 구현은 [../backend/commerce.md](../backend/commerce.md) §3–§4, 주체와 스코프는 [../backend/gateway.md](../backend/gateway.md), 채팅 이벤트와 블록은 [chat.md](chat.md). 데모 값은 [../scenarios.md](../scenarios.md).
>
> **설계 축**: ① 사람의 로그인·세션은 게이트웨이가 하고, 서비스는 게이트웨이가 서명한 내부 토큰만 받습니다. 스토어(테넌트)는 토큰의 `tenant` 클레임 하나로 정해집니다. ② enum은 숫자·문자 코드 대신 의미 있는 단어로 주고, 각 값의 뜻과 허용 전이를 함께 노출(목록의 출처는 enums.md). ③ 정책은 데이터로, 값마다 사람이 읽는 표기와 함께. ④ 쓰기 프리뷰는 변경셋으로 묶이고, 승인·거절·되돌리기는 사람만 누르는 엔드포인트. ⑤ 운영자 홈의 할 일은 감시 작업(watcher)과 대화에서 생기고, 무시는 사람만. ⑥ 모든 툴 호출·프리뷰·승인·실행은 schema.org `Action`으로 액션 로그에 남음.

리소스: [policy](#policy) · [change_set](#change_set) · [task](#task) · [action_log](#action_log) · [schema](#schema) · [enum](#enum) · [auth](#auth) · [tenant](#tenant) · [role](#role) · [menu](#menu) · [admin_user](#admin_user) · [org](#org) · [api_key](#api_key) · [예약 작업](#예약-작업) · [데모 시계](#데모-시계) · [내부 엔드포인트](#내부-엔드포인트)

---

## policy

### `get_policy` — Kind: R  (MVP)
- 논리 주소 · REST: `platform.policy.get` · `GET /v1/platform/policies?key=max_discount`
- 요약: 이 스토어에 적용되는 정책 값을 돌려줍니다(기본값 위에 스토어 덮어쓰기를 병합한 값). 키를 정확히 주면 그 값 하나, 앞부분을 주면 그 아래 값 전부(`max_discount` → `max_discount.*`). 에이전트는 프리뷰 전에 이 값으로 계획을 세우고, 프리뷰의 `checks[].rule`은 같은 키를 씁니다. 장면 B1의 첫 호출입니다.
- consumes → produces: `policy_key` → `policy_key[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |---|---|:--:|---|---|
  | `key` | Text, `^[a-z][a-z0-9_]*(\.[a-z0-9_]+)*$` | ✓ | 정책 키 또는 그 앞부분(점으로 구분, 파일 이름은 빼고). 리프 값만 돌려줌 | `"max_discount"` |
  | `file` | enum | | `approval` \| `catalog` \| `community` \| `customer` \| `fulfillment` \| `integration` \| `merchandising` \| `messaging` \| `order` \| `partner` \| `platform` \| `promotion` \| `search`. 같은 키가 여러 파일에 있을 때 좁힘 | `"promotion"` |
- 출력(데모 스토어, [../scenarios.md](../scenarios.md) §5):
  ```jsonc
  {"items": [
     {"key": "max_discount.everyone", "value": 0.20, "display": "20%", "file": "promotion", "overridden_by_store": false},
     {"key": "max_discount.by_tier.vip", "value": 0.30, "display": "30% for vip", "file": "promotion", "overridden_by_store": true},
     {"key": "max_discount.on_unknown_tier", "value": "use_everyone", "display": "Tiers without their own limit use the everyone limit", "file": "promotion", "overridden_by_store": false},
     {"key": "max_discount.on_violation", "value": "block", "display": "Items over the limit fail the preview", "file": "promotion", "overridden_by_store": false}],
   "total": 4}
  ```
  | 필드 | 타입 | 설명 |
  |---|---|---|
  | `key` | Text | 파일 안의 전체 키 경로 |
  | `value` | 저장된 값 그대로 | 비율은 0–1, 금액은 `{value, currency}`, 조건은 `{field, op, value}`([conventions.md](conventions.md) §3–§4) |
  | `display` | Text | 사람이 읽는 표기(`"30%"`, `"$200.00"`). 에이전트와 화면은 이 문구를 인용 |
  | `file` | enum | 값이 있는 정책 파일 |
  | `overridden_by_store` | Boolean | 스토어 덮어쓰기(`platform.policy_override`) 값이면 `true` |
- 프리뷰: 해당 없음(읽기)
- 정책: 값은 [policies/](../../policies/README.md) 기본값 + 스토어 덮어쓰기. 비밀은 정책에 넣지 않음
- 에러: `not_found`(`detail.entity: policy`, 그런 키가 없음) → `list_policies`
- 멱등성: 읽기(부작용 없음). 응답의 값은 요청 시점의 정책 스냅샷(버전은 `list_policies`)

### `list_policies` — Kind: R
- 논리 주소 · REST: `platform.policy.list` · `GET /v1/platform/policies/index`
- 요약: 로드된 정책 파일 목록(이름·버전·도메인·한 줄 요약·덮어쓴 키 수)과 정책 스냅샷 버전. 값은 `get_policy`.
- consumes → produces: → `policy_key[]`
- 입력: `domain?`(enum, README §1의 도메인 이름), `limit`(기본 20), `cursor?`
- 출력: `{"items": [{"file": "promotion", "version": 3, "domain": "promotion", "summary": "Discount caps, minimum stock, overlap and coupon rules", "overridden_keys": 1}], "total": 12, "policy_version": "a41c09e2b7f3"}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_filter` → `list_policies`
- 멱등성: 읽기(부작용 없음)

---

## change_set

변경셋(`acom:ChangeSet`, `ItemList`의 하위 타입)은 한 요청에서 나온 쓰기 프리뷰들을 묶은 것이고, 사람이 승인하는 단위입니다. 항목 하나가 프리뷰 하나(`pv_…`)이며 액션 로그에는 `action_status: potential`인 Action으로 남습니다. 묶음 전체에 `version`, 사람에게 보이는 번호 `number`, 되돌리기 기한이 있습니다. 엔진 동작(실행 순서, 의존 바인딩, 실패 시 역순 되돌림)은 [../backend/commerce.md](../backend/commerce.md) §3.

**누가 무엇을 부르나**

| 부르는 쪽 | 무엇 | 경로 |
|---|---|---|
| 우리 에이전트의 런타임(스코프 `changeset:draft`) | 만들기·항목 붙이기·교체·빼기·제출 | 아래 [런타임 엔드포인트](#런타임-엔드포인트). **툴이 아니며 모델은 보지 못함** |
| 모델(툴) | 읽기만 | `get_change_set`, `search_change_sets`, `verify_change_set` |
| 사람(운영자, `human:*`) | 승인·거절·되돌리기 | 아래 [사람 전용 엔드포인트](#사람-전용-엔드포인트) |
| 실행기(시스템 주체) | 승인된 항목 실행·검증 | 내부. 결과는 `verify_change_set`과 이벤트로 |

모델은 "무엇을 바꿀지"만 말하고(쓰기 툴 호출), 런타임이 그 호출을 `POST {path}?preview=true&change_set_id=cs_…`로 바꿔 프리뷰를 이 셋에 붙입니다([conventions.md](conventions.md) §7·§8).

**상태기계** ([enums.md](enums.md) §1.8 `acom:change_set_state`)

```
drafting ──submit{version}──▶ waiting_for_approval ──approve{version}──▶ running ──▶ verifying ──▶ live ──undo──▶ undone
   ▲                              │        │                              │             │           │
   └── 항목 변경(version+1) ──────┘        └──reject──▶ rejected          └────▶ failed ◀┘           └─(되돌림 실패)─▶ failed
   ▲
   └── 승인 때 다시 계산한 changes가 다름(estimate_changed, version+1)
```

| 전이 | 누가 | 조건·효과 |
|---|---|---|
| 만들기 → `drafting` (`version: 1`) | 런타임 | 요청 원문, 해석, 대화 id |
| 항목 붙이기·빼기 | 런타임 | `drafting`, `waiting_for_approval`에서. 한 번 제출된 셋이면 `version+1`, 상태 `drafting`. 첫 제출 전에는 버전 그대로 |
| 항목 교체(대화로 고침) | 런타임 | 항상 `version+1`, 상태 `drafting`. 교체된 프리뷰는 `superseded`([enums.md](enums.md) §1.9) |
| `submit {version}` → `waiting_for_approval` | 런타임 | 항목 ≥ 1, 최신 버전, 만료된 프리뷰 없음. 실패한 체크가 있어도 제출할 수 있고 카드에 그대로 보임 |
| `approve {version}` → `running` | **사람 전용** | 최신 버전만(`version_out_of_date`). 승인 권한(`approval.yml` 임계의 `approver_role`). 모든 항목을 다시 프리뷰해 `changes`가 하나라도 다르면 실행하지 않고 `drafting`, `version+1`, `estimate_changed`. 액션 로그에 `AuthorizeAction` |
| `reject {reason}` → `rejected` | **사람 전용** | `waiting_for_approval`에서. 끝 상태 |
| `running` → `verifying` → `live` / `failed` | 실행기 | 항목을 선언 순서대로 실행, 하나라도 실패하면 이미 실행한 항목을 역순으로 되돌리고 `failed`. 모두 성공하면 다시 읽어 검증, 전부 일치하면 `live` |
| `undo` → `undone` | **사람 전용** | `live`이고 `undo_until` 전이고 되돌릴 수 없는 항목이 없을 때. 항목을 역순으로 되돌림. 되돌림이 중간에 실패하면 `failed`(어느 항목까지 되돌렸는지 기록) |

- **되돌리기 기한(`undo_until`)**: 기간이 있는 변경(회원가·쿠폰·배너)은 가장 늦은 기간 끝 + `change_set.undo_window_after_end_days`일 23:59, 그 밖은 실행일 + `change_set.undo_window_after_run_days`일 23:59(스토어 시간대, [approval.yml](../../policies/approval.yml)). #1042는 Nov 9 23:59.
- **되돌릴 수 없는 항목**: `change_set.irreversible`에 있는 오퍼레이션(환불, 고객 메시지)이 들어간 셋은 프리뷰 단계에서 `checks`에 `change_set.irreversible`가 표시되고 `undo_until`이 `null`입니다.
- **승인이 필요 없는 항목**(`needs_approval: false`)도 셋에 붙으면 셋과 함께 승인·실행됩니다. 셋 밖에서 단독 실행하는 것은 승인이 필요 없는 쓰기만 가능합니다([conventions.md](conventions.md) §7).

### `get_change_set` — Kind: R
- 논리 주소 · REST: `platform.change_set.get` · `GET /v1/platform/change-sets/{change_set_id}`
- 요약: 변경셋 하나를 펼칩니다: 상태·버전·번호, 요청 원문과 해석, 항목마다 프리뷰의 `changes`·`checks`·승인 필요 여부, 빠진 것(`left_out`), 되돌리기 기한. 운영자에게 "지금 무엇이 승인 대기인지" 설명하거나, 대화로 고치기 전에 현재 버전을 읽을 때 씁니다. `change_set` 블록([chat.md](chat.md) §3.3)의 데이터입니다.
- consumes → produces: `change_set_id` → `preview_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |---|---|:--:|---|---|
  | `change_set_id` | Text, `^cs_[a-z0-9_]+$` | ✓ | 변경셋 id | `"cs_1042"` |
  | `version` | Integer, ≥ 1 | | 이전 버전을 볼 때. 없으면 최신 | `1` |
  | `fields` | Text[] | | 돌려받을 최상위 필드. 예: 항목을 빼고 상태만 | `["state", "version", "undo_until"]` |
- 출력(장면 B1, v2 제출 직후 Fri Nov 6 11:05):
  ```jsonc
  {"id": "cs_1042", "number": 1042, "version": 2, "state": "waiting_for_approval",
   "title": "VIP weekend sale on summer outerwear",
   "request": {"text": ["Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page.",
                        "Drop the low-stock ones. Give the overshirt 15% instead."],
               "conversation_id": "conv_01jc8x2k7m", "requested_by": "adm_sj"},
   "interpretation": [
     {"slot": "who", "text": "VIP tier · 1,284 members", "value": {"valid_for_member_tier": "vip"}},
     {"slot": "products", "text": "Summer outerwear · 40 of 42", "value": {"category_id": "cat_summer_outerwear", "product_count": 40}},
     {"slot": "benefit", "text": "20% off, overshirt 15%", "value": {"percent_off": 20, "overrides": [{"product_id": "prd_seersucker_overshirt", "percent_off": 15}]}},
     {"slot": "when", "text": "Sat Nov 7 00:00 – Sun Nov 8 23:59", "value": {"valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00"}},
     {"slot": "where", "text": "Main page hero, VIP only", "value": {"slot_id": "slot_home_hero", "audience": "vip"}}],
   "items": [
     {"position": 1, "preview_id": "pv_8f2c", "tool": "set_sale_prices", "address": "promotion.offer.set_sale_prices",
      "depends_on": [], "action_status": "potential", "needs_approval": true, "approval_reason": "Changes 40 prices",
      "changes": [
        {"op": "update", "entity": "unit_price_specification", "id": null,
         "from": {"product_id": "prd_windbreaker", "price": 89.00, "price_currency": "USD", "price_type": "list_price"},
         "to": {"product_id": "prd_windbreaker", "price": 71.20, "price_currency": "USD", "price_type": "sale_price", "percent_off": 20}}
        // … 39개 update, 2개 exclude 더 (set_sale_prices 프리뷰와 같음)
      ],
      "changes_total": 43,
      "checks": [{"rule": "max_discount", "passed": true, "detail": "Highest is 20%; policy allows 30% for vip", "fix_tool": null},
                 {"rule": "price_floor", "passed": true, "detail": "Overshirt moved to 15% ($67.15) to stay at cost + 15% ($66.70)", "fix_tool": null},
                 {"rule": "promo_min_stock", "passed": true, "detail": "2 products under 10 units excluded: chore coat (7), rain parka (4)", "fix_tool": null},
                 {"rule": "promotion_overlap", "passed": true, "detail": "Checked 4 running promotions", "fix_tool": null}]},
     {"position": 2, "preview_id": "pv_8f31", "tool": "create_coupon", "address": "promotion.coupon.create",
      "depends_on": ["pv_8f2c"], "action_status": "potential", "needs_approval": true,
      "approval_reason": "Applies automatically at checkout for 1,284 VIP members",
      "changes": [{"op": "create", "entity": "coupon", "id": null, "from": null,
                   "to": {"code": "VIPWEEKEND20", "promotion_id": "$item[1].promotion_id", "valid_for_member_tier": "vip", "application": "automatic_at_checkout"}}],
      "changes_total": 1,
      "checks": [{"rule": "member_benefits_per_order", "passed": true, "detail": "Doesn't stack with tier coupons; one member benefit per order", "fix_tool": null}]},
     {"position": 3, "preview_id": "pv_b71e", "tool": "schedule_banner", "address": "merchandising.banner.schedule",
      "depends_on": ["pv_8f2c"], "action_status": "potential", "needs_approval": true,
      "approval_reason": "Shows on the main page to 1,284 VIP members",
      "changes": [{"op": "schedule", "entity": "display_booking", "id": null, "from": null,
                   "to": {"slot_id": "slot_home_hero", "audience": "vip", "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00"}}],
      "changes_total": 1,
      "checks": [{"rule": "slot_free", "passed": true, "detail": "Hero slot open Nov 7–8 for VIP", "fix_tool": null}]}],
   "left_out": [
     {"entity": "product", "id": "prd_chore_coat", "name": "Cotton chore coat", "rule": "promo_min_stock", "reason": "7 units, under 10"},
     {"entity": "product", "id": "prd_rain_parka", "name": "Light rain parka", "rule": "promo_min_stock", "reason": "4 units, under 10"}],
   "checks_summary": {"passed": 6, "failed": 0, "total": 6},
   "needs_approval": true,
   "undo_until": "2026-11-09T23:59:00-05:00",
   "created_by": "adm_sj", "agent": {"@type": "SoftwareApplication", "name": "muse-glimmer-30b"},
   "created_at": "2026-11-06T11:02:10-05:00", "submitted_at": "2026-11-06T11:05:40-05:00",
   "approved_by": null, "approved_at": null, "verification": null}
  ```
  | 필드 | 설명 |
  |---|---|
  | `number` | 사람에게 보이는 번호(`#1042`의 1042). `#`은 화면 표기 |
  | `interpretation[]` | 런타임이 요청을 푼 결과. `slot`은 `who` \| `products` \| `benefit` \| `when` \| `where` \| `other`, 블록의 `understood{}`가 됨 |
  | `items[].depends_on` | 앞 항목의 결과를 쓰는 항목. 실행 때 `$item[n].promotion_id` 같은 자리 표시를 실제 id로 바꿈 |
  | `items[].changes` | 그 프리뷰의 `changes`. 많으면 앞부분과 `changes_total` |
  | `left_out[]` | 프리뷰의 `exclude` 줄을 모은 것(실행하지 않음, 기록만) |
  | `undo_until` | 되돌리기 기한. 되돌릴 수 없는 항목이 있으면 `null` |
  | `verification` | `verifying` 뒤에는 `verify_change_set`과 같은 요약 `{state, matched, total}` |
- 프리뷰: 해당 없음(읽기)
- 정책: [approval.yml](../../policies/approval.yml) `change_set`
- 에러: `change_set_not_found`(class `not_found`) → `search_change_sets`
- 멱등성: 읽기(부작용 없음)

### `search_change_sets` — Kind: R
- 논리 주소 · REST: `platform.change_set.search` · `GET /v1/platform/change-sets?state=waiting_for_approval&limit=20`
- 요약: 변경셋을 상태·만든 사람·기간으로 찾습니다. "승인 대기 중인 것", "어제 실행된 것", "아직 되돌릴 수 있는 것"에 답할 때 씁니다.
- consumes → produces: → `change_set_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |---|---|:--:|---|---|
  | `state` | enum[] | | `drafting` \| `waiting_for_approval` \| `running` \| `verifying` \| `live` \| `failed` \| `rejected` \| `undone`. 없으면 전부 | `["waiting_for_approval"]` |
  | `created_by` | Text, `^(adm\|sys)_[a-z0-9_]+$` | | 요청한 운영자 또는 시스템 주체(자동화) | `"adm_sj"` |
  | `created_from` | DateTime | | 만든 시각 하한(포함) | `"2026-11-06T00:00"` |
  | `created_through` | DateTime | | 만든 시각 상한(포함) | `"2026-11-06T23:59"` |
  | `undoable_only` | Boolean | | `true`면 지금 되돌릴 수 있는 것(`live`이고 기한 전)만. 기본 `false` | `true` |
  | `sort` | enum | | `-created_at`(기본) \| `created_at` \| `-number` | `"-created_at"` |
  | `limit` | Integer, 1–100 | | 기본 20 | `20` |
  | `cursor` | Text | | 이전 응답의 `next_cursor` | |
- 출력:
  ```jsonc
  {"items": [
     {"id": "cs_1042", "number": 1042, "title": "VIP weekend sale on summer outerwear", "state": "waiting_for_approval", "version": 2,
      "item_count": 3, "needs_approval": true, "created_by": "adm_sj", "created_at": "2026-11-06T11:02:10-05:00",
      "undo_until": "2026-11-09T23:59:00-05:00"},
     {"id": "cs_1043", "number": 1043, "title": "Review 12 returns over $200", "state": "waiting_for_approval", "version": 1,
      "item_count": 12, "needs_approval": true, "created_by": "adm_sj", "created_at": "2026-11-06T10:40:00-05:00",
      "undo_until": null}],
   "total": 2}
  ```
- 프리뷰: 해당 없음(읽기)
- 정책: 없음(읽기)
- 에러: `invalid_filter` → `search_change_sets`
- 멱등성: 읽기(부작용 없음)

### `verify_change_set` — Kind: R  (MVP)
- 논리 주소 · REST: `platform.change_set.verify` · `GET /v1/platform/change-sets/{change_set_id}/verification`
- 요약: 실행기가 기록한 검증 결과를 돌려줍니다. 실행기는 `running` 뒤 항목마다 저장소를 다시 읽어 프리뷰의 `changes[].to`와 비교하고, 그 결과로 셋을 `live` 또는 `failed`로 옮깁니다. 이 툴은 그 기록을 읽을 뿐 **상태를 바꾸지 않으므로** 언제든 다시 불러도 됩니다. 장면 B1의 4단계에서 에이전트가 "모두 일치, live"라고 말하는 근거입니다. Action 종류는 `AssessAction`.
- consumes → produces: `change_set_id` → verification
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |---|---|:--:|---|---|
  | `change_set_id` | Text, `^cs_[a-z0-9_]+$` | ✓ | 검증 결과를 볼 변경셋 | `"cs_1042"` |
- 출력(승인 뒤 Fri Nov 6 11:09):
  ```jsonc
  {"items": [
     {"preview_id": "pv_8f2c", "tool": "set_sale_prices", "result_id": "promo_1042", "matched": true, "checked": 40, "mismatches": []},
     {"preview_id": "pv_8f31", "tool": "create_coupon", "result_id": "cpn_vipweekend20", "matched": true, "checked": 1, "mismatches": []},
     {"preview_id": "pv_b71e", "tool": "schedule_banner", "result_id": "dsp_1042", "matched": true, "checked": 1, "mismatches": []}],
   "total": 3,
   "change_set_id": "cs_1042", "version": 2, "state": "live", "verified_at": "2026-11-06T11:09:02-05:00"}
  ```
  - `total`은 항목 수입니다. `checked`는 그 항목에서 다시 읽은 변경 수(회원가 40개).
  - 다를 때: `{"matched": false, "mismatches": [{"entity": "unit_price_specification", "id": "ups_windbreaker_m_slate_1042", "field": "price", "expected": 71.20, "actual": 89.00}]}`이고 셋의 `state`는 `failed`.
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `change_set_not_found` → `search_change_sets` · `change_set_not_run`(class `invalid_state`, 아직 실행 전이라 검증 기록이 없음) → `get_change_set`
- 멱등성: 읽기(부작용 없음)

### 런타임 엔드포인트

우리 에이전트의 런타임만 부르는 HTTP 엔드포인트입니다(스코프 `changeset:draft`, 위임 주체 토큰). **툴이 아니며 모델은 보지 못합니다.** 외부 에이전트(MCP)에는 열지 않습니다. 오류는 공통 형식이고 `next_tool`은 `null`입니다(런타임이 처리).

| 엔드포인트 | 본문 | 응답 | 에러 |
|---|---|---|---|
| `POST /v1/platform/change-sets` | `{title, request_text, interpretation[], conversation_id, task_id?}` — `task_id`: 이 변경셋이 답하는 할 일. 열려 있고 변경셋이 없을 때만 묶임 | `{id, number, version: 1, state: "drafting", task_id}` (`task_id`: 묶인 할 일, 없으면 `null`) | `invalid_input` |
| `PATCH /v1/platform/change-sets/{id}` | `{title?, append_request_text?, interpretation?[]}` — 대화로 고친 요청: 문장을 `request_text`에 덧붙이고 해석을 바꿈. 항목은 아래 교체로 | `{id, version, state, item_count}` | `change_set_not_found`, `invalid_state`(끝 상태·실행 중) |
| `POST /v1/platform/change-sets/{id}/items` | `{preview_id, depends_on?: [preview_id]}` | `{id, version, state, item_count}` | `change_set_not_found`, `preview_not_found`, `preview_expired`, `invalid_state`(끝 상태·실행 중), `already_exists`(같은 프리뷰) |
| `PUT /v1/platform/change-sets/{id}/items` | `{items: [{preview_id, depends_on?}]}` — 대화로 고친 항목 전체로 교체 | `{id, version, state: "drafting", superseded: [preview_id]}` | `change_set_not_found`, `preview_not_found`, `preview_expired`, `invalid_state` |
| `DELETE /v1/platform/change-sets/{id}/items/{preview_id}` | — | `{id, version, state, item_count}` | `change_set_not_found`, `preview_not_found`, `invalid_state` |
| `POST /v1/platform/change-sets/{id}/submit` | `{version}` | `{id, version, state: "waiting_for_approval"}` | `change_set_not_found`, `version_out_of_date`, `invalid_state`(항목 없음·이미 제출), `preview_expired` |

- 쓰기 프리뷰는 `POST {path}?preview=true&change_set_id=cs_…`로 바로 붙일 수 있습니다. 이 경우 `POST …/items`를 따로 부르지 않으며, 프리뷰 응답에 `change_set_id`가 들어옵니다.
- 같은 오퍼레이션을 다시 프리뷰해 교체하면 이전 프리뷰는 `superseded`가 되고, 그 `preview_id`로는 실행할 수 없습니다(`preview_expired`, `detail.superseded_by`).
- 셋에 붙은 프리뷰는 `preview_ttl_minutes`로 만료되지 않고 승인 때 다시 계산합니다([approval.yml](../../policies/approval.yml)).
- 멱등성: 만들기와 제출은 `Idempotency-Key`. 항목 교체는 같은 본문이면 버전을 올리지 않습니다.

### 사람 전용 엔드포인트

운영자가 `change_set` 블록의 버튼으로만 부르는 엔드포인트입니다([README.md](README.md) §2). **툴이 아니며 모델은 보지 못합니다.** 세션 쿠키 + `X-CSRF-Token` + `human:*` 스코프가 필요하고, 에러의 `next_tool`은 항상 `null`입니다.

#### `POST /v1/platform/change-sets/{id}/approve`
- 스코프: `human:approve`. 승인자는 항목들에 걸린 [approval.yml](../../policies/approval.yml) 임계의 `approver_role`을 모두 가져야 합니다.
- 본문: `{"version": 2}` — 화면에 보인 버전.
- 동작: 최신 버전인지 확인 → 모든 항목을 다시 프리뷰(`change_set.revalidate_on_approve`) → `changes`가 모두 같으면 `running`으로 옮기고 실행기가 시작. 하나라도 다르면 실행하지 않고 셋을 `drafting`, `version+1`로 되돌림.
- 응답: `{"ok": true, "id": "cs_1042", "state": "running", "version": 2, "approved_by": "adm_sj", "approved_at": "2026-11-06T11:08:30-05:00"}` — 이후 진행은 `change_set.updated` 이벤트([chat.md](chat.md) §2)
- 에러: `change_set_not_found` → `null` · `version_out_of_date`(`detail.latest_version`, 화면을 새로 고침) → `null` · `invalid_state`(`waiting_for_approval`이 아님) → `null` · `estimate_changed`(`detail.new_version`, 새 버전을 다시 검토) → `null` · `forbidden`(`detail.reason: role`, 필요한 `approver_role` 없음) → `null`
- 멱등성: `Idempotency-Key`. 같은 버전을 두 번 승인해도 실행은 한 번.

#### `POST /v1/platform/change-sets/{id}/reject`
- 스코프: `human:reject`
- 본문: `{"reason": "Wait until the denim bundle ends."}` (필수, 500자까지)
- 응답: `{"ok": true, "id": "cs_1042", "state": "rejected"}` — 프리뷰는 모두 `expired`. 에이전트는 대화로 새 셋을 만들 수 있음
- 에러: `change_set_not_found` → `null` · `invalid_state` → `null` · `invalid_input`(`reason` 없음) → `null`
- 멱등성: `Idempotency-Key`. 이미 `rejected`면 첫 결과.

#### `POST /v1/platform/change-sets/{id}/undo`
- 스코프: `human:undo`
- 본문: 없음
- 동작: `live`이고 `undo_until` 전이면 항목을 역순으로 되돌림(각 오퍼레이션의 `undo`). 예약 레코드(배너)는 취소, 회원가는 삭제, 쿠폰은 회수.
- 응답: `{"ok": true, "id": "cs_1042", "state": "undone", "undone_items": 3, "undone_at": "2026-11-07T09:12:00-05:00"}`
- 에러: `change_set_not_found` → `null` · `invalid_state`(`live`가 아님) → `null` · `undo_window_closed`(`detail.until`) → `null` · `irreversible_change`(환불·메시지가 들어 있음) → `null`
- 멱등성: `Idempotency-Key`. 이미 `undone`이면 첫 결과.

---

## task

할 일(`acom:Task`)은 운영자 홈(`task_list` 블록, [chat.md](chat.md) §3.3)에 보이는 일 단위입니다. 상태는 `acom:task_state`(`needs_you` \| `working` \| `done` \| `dismissed`, [enums.md](enums.md) §1.14), 출처 `origin`은 셋 중 하나입니다. 에이전트가 할 일을 맡아 변경셋을 만들면(턴의 `task_id`, [chat.md](chat.md) §1) 할 일은 그 변경셋과 대화에 묶여 `working`("Drafting change set #N")이 되고, 변경셋이 제출되면 `needs_you`("Change set #N is waiting for your approval")가 됩니다. 묶인 변경셋이 `live`나 `rejected`가 되면 할 일은 `done`이 되고 `summary`는 그 끝을 말합니다(`"#1041 ran: Reorder linen shirt: 3 sizes, 240 units"`, `"#1041 was rejected: …"`).

| `origin` | 어떻게 생기나 | 예(장면 B2·B5) |
|---|---|---|
| `you_asked` | 운영자가 대화로 요청하면 런타임이 대화와 묶어 만듦 | "메인 배너를 Fall layers로 교체"(09:30 → 10:00) |
| `agent_noticed` | 감시 작업이 기준을 넘는 것을 찾고 운영 에이전트가 맡음 | "$200 넘는 반품 요청 12건 대기"(10:40), "Linen shirt 약 4일 뒤 품절"(08:40) |
| `automation` | 감시 작업이 찾은 것을 미리 승인된 자동 조치로 처리 | "원가 이하 상품 4개를 하한으로"(07:55 → 08:31, Nov 7까지 되돌리기) |

**감시 작업(watcher).** `commerce`의 예약 작업(`platform.job`)이 정해진 주기마다 정책 기준을 읽어 데이터를 확인하고, 기준을 넘으면 할 일을 만들거나 갱신합니다. 기준과 주기는 코드가 아니라 [platform.yml](../../policies/platform.yml) `watchers`에 있습니다: 환불 임계(`approval.yml` `thresholds.refund`)를 넘는 반품 요청이 기다리는지(`returns_over_refund_threshold`), 최근 판매 속도로 며칠 안에 품절될 변형이 있는지(`stock_out_forecast`), 정가가 원가보다 낮은 상품이 있는지(`price_below_cost`). 같은 원인으로 열린 할 일이 있으면 새로 만들지 않고 그 할 일을 갱신합니다. 감지는 결정적이고 처리는 대화입니다: 할 일마다 요청 문장(`request`)이 있고, 운영자가 항목을 열면(`agent_noticed`는 감시 작업이 바로) 에이전트가 그 문장으로 새 턴을 시작합니다([../backend/agent.md](../backend/agent.md) §10). 할 일이 만든 쓰기는 모두 변경셋으로 가고(`change_set_id`), 할 일은 변경셋이 `live`·`rejected`가 되면 `done`이 됩니다(엔진이 옮기고 `task.updated`를 보냄). 운영자는 필요 없는 할 일을 사람 전용 `dismiss`로 치웁니다.

**감시 작업이 할 일을 쓰는 방식.** 감지기는 데이터를 가진 도메인에 있습니다: 반품 요청은 `fulfillment`, 품절 예측은 `sales`(판매량, 재고는 `catalog`가 내놓는 `product_stock`), 원가 아래 정가는 `catalog`. 감지기는 할 일 초안만 돌려주고 `platform.task`는 작업 실행기(`acom-changeset::jobs`, [예약 작업](#예약-작업))가 씁니다. 원인은 `stock_out_forecast`가 상품 하나, 나머지 둘은 감시 작업 전체입니다. 같은 감시 작업·같은 원인의 열린 할 일(`needs_you`·`working`)이 있으면 그 할 일의 `title`·관찰값(`watcher.observed`)·관련 엔티티만 새로 쓰고(상태·요약·요청 문장·변경셋은 그대로, 버전 +1), 없을 때만 새로 만듭니다(`needs_you`로 시작). 기준을 넘는 것이 없으면 아무것도 바꾸지 않습니다. 데모 시계 Fri Nov 6 11:00에 세 감시 작업을 돌리면 시드의 할 일 둘이 같은 관찰값으로 갱신됩니다: 반품 요청 12건(가장 오래된 것 2026-11-04T09:12-05:00), Linen shirt 재고 38·최근 7일 61개 판매·4.4일. 원가 아래 상품은 없습니다(#1040이 08:31에 고침).

**고침과 승인(`run_fix`).** 정책이 고침을 정한 감시 작업(`price_below_cost`의 `fix: raise_to_price_floor`)은 찾은 것과 함께 변경셋 초안을 냅니다: 원가 아래 변형마다 `set_price`, 가격은 [catalog.yml](../../policies/catalog.yml) `price_floor`(원가 × 1.15, 센트 반올림). 엔진은 시스템 주체 `sys_watcher`로 변경셋을 만들어 할 일에 붙이고 **제출까지만** 합니다. 시스템은 변경셋을 만들 수 있어도 승인할 수는 없습니다.

| `run_fix` | 변경셋 | 할 일 |
|---|---|---|
| `ask`(기본값) | `waiting_for_approval`. 사람이 `approve`해야 실행 | `needs_you`, 요약 "Change set #N is waiting for your approval" |
| `standing_approval` | 스토어가 정책 덮어쓰기(`platform.policy_override`, 그 자체가 승인을 거친 쓰기)로 이 고침을 미리 승인한 것으로 보고 제출 직후 실행. `approved_by: null`, 승인 이벤트와 `AuthorizeAction`의 결과에 `standing_approval: "watchers.price_below_cost.run_fix"`. 되돌리기 기한은 다른 변경셋과 같음 | `working` → 변경셋이 `live`면 `done`, 요약 "Raised to the price floor at 08:31; undo until Nov 7". 실패하면 `needs_you` |

열린 할 일에 이미 변경셋이 있으면 다시 만들지 않습니다. 데모 스토어는 `standing_approval`입니다(장면 B5의 `task_below_cost`, #1040).

### `list_tasks` — Kind: R
- 논리 주소 · REST: `platform.task.list` · `GET /v1/platform/tasks?state=needs_you&state=working`
- 요약: 운영자 홈의 할 일 목록. "지금 내가 볼 것", "에이전트가 하고 있는 것", "자동으로 끝난 것"에 답할 때 씁니다.
- consumes → produces: → `task_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |---|---|:--:|---|---|
  | `state` | enum[] | | `needs_you` \| `working` \| `done` \| `dismissed`. 기본 `[needs_you, working, done]` | `["needs_you"]` |
  | `origin` | enum[] | | `you_asked` \| `agent_noticed` \| `automation`. 없으면 전부 | `["agent_noticed", "automation"]` |
  | `since` | DateTime | | 이 시각 뒤에 만들어지거나 바뀐 것만 | `"2026-11-06T00:00"` |
  | `limit` | Integer, 1–100 | | 기본 20 | `20` |
  | `cursor` | Text | | 이전 응답의 `next_cursor` | |
- 출력(Fri Nov 6 11:00, 장면 B2·B5):
  ```jsonc
  {"items": [
     {"id": "task_returns_1106", "title": "12 return requests over $200 are waiting", "state": "needs_you", "origin": "agent_noticed",
      "at": "2026-11-06T10:40:00-05:00", "summary": "9 refunds ($2,463.00), 2 photo requests, 1 decline", "tag": "returns",
      "change_set_id": "cs_1043", "watcher": "returns_over_refund_threshold"},
     {"id": "task_linen_reorder", "title": "Linen shirt sells out in about 4 days", "state": "needs_you", "origin": "agent_noticed",
      "at": "2026-11-06T08:40:00-05:00", "summary": "Reorder draft: 3 sizes, 240 units", "tag": "inventory",
      "change_set_id": "cs_1041", "watcher": "stock_out_forecast"},
     {"id": "task_size_charts", "title": "Check missing size charts", "state": "working", "origin": "you_asked",
      "at": "2026-11-05T16:20:00-05:00", "summary": "15 of 18 checked, no changes yet", "tag": "catalog",
      "change_set_id": null, "watcher": null},
     {"id": "task_below_cost", "title": "4 products were priced below cost", "state": "done", "origin": "automation",
      "at": "2026-11-06T07:55:00-05:00", "summary": "Raised to the price floor at 08:31; undo until Nov 7", "tag": "pricing",
      "change_set_id": "cs_1040", "watcher": "price_below_cost"},
     {"id": "task_fall_layers", "title": "Swap the main banner to Fall layers", "state": "done", "origin": "you_asked",
      "at": "2026-11-06T09:30:00-05:00", "summary": "Live since 10:00, verified", "tag": "merchandising",
      "change_set_id": "cs_1039", "watcher": null}],
   "total": 5}
  ```
  항목마다 `updated_at`(마지막으로 바뀐 시각, 감시 작업이 관찰값을 새로 쓴 때 포함)과 `conversation_id`도 있습니다(위 예에서는 생략).
- 프리뷰: 해당 없음(읽기)
- 정책: [platform.yml](../../policies/platform.yml) `watchers`
- 에러: `invalid_filter` → `list_tasks`
- 멱등성: 읽기(부작용 없음)

### `get_task` — Kind: R
- 논리 주소 · REST: `platform.task.get` · `GET /v1/platform/tasks/{task_id}`
- 요약: 할 일 하나의 전체: 왜 생겼는지(감시 기준과 관찰값), 상태 이력, 연결된 대화·변경셋·대상.
- consumes → produces: `task_id` → `change_set_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |---|---|:--:|---|---|
  | `task_id` | Text, `^task_[a-z0-9_]+$` | ✓ | 할 일 id | `"task_returns_1106"` |
- 출력:
  ```jsonc
  {"id": "task_returns_1106", "title": "12 return requests over $200 are waiting", "state": "needs_you", "origin": "agent_noticed",
   "tag": "returns", "summary": "9 refunds ($2,463.00), 2 photo requests, 1 decline",
   "request": "Review the 12 return requests over $200 and suggest what to do with each.",
   "watcher": {"key": "returns_over_refund_threshold",
               "threshold": {"policy_key": "thresholds.refund", "value": {"value": 200.00, "currency": "USD"}},
               "observed": {"count": 12, "oldest_requested_at": "2026-11-04T09:12:00-05:00"}},
   "related": {"change_set_id": "cs_1043", "conversation_id": "conv_01jc8w9r2p",
               "entities": [{"entity": "claim", "id": "clm_20402"}, {"entity": "claim", "id": "clm_20431"}]},
   "history": [{"at": "2026-11-06T10:40:00-05:00", "state": "working", "note": "Operations agent started triage"},
               {"at": "2026-11-06T10:46:00-05:00", "state": "needs_you", "note": "Change set #1043 ready: approve 9 refunds and send 3 messages"}],
   "created_at": "2026-11-06T10:40:00-05:00", "updated_at": "2026-11-06T10:46:00-05:00"}
  ```
- 프리뷰: 해당 없음(읽기)
- 정책: [platform.yml](../../policies/platform.yml) `watchers`
- 에러: `task_not_found`(class `not_found`) → `list_tasks`
- 멱등성: 읽기(부작용 없음)

#### 사람 전용 `POST /v1/platform/tasks/{task_id}/dismiss`
- 스코프: `human:task`. 툴이 아니며 운영자가 `task_list` 블록에서만 누릅니다.
- 본문: `{"reason": "Already handled by phone."}`(선택)
- 응답: `{"ok": true, "id": "task_linen_reorder", "state": "dismissed"}` — 연결된 변경셋이 `drafting`·`waiting_for_approval`이면 함께 `rejected`(사유 같음). 같은 원인이 다시 기준을 넘으면 감시 작업이 새 할 일을 만듭니다.
- 에러: `task_not_found` → `null` · `invalid_state`(이미 `done`·`dismissed`) → `null`
- 멱등성: `Idempotency-Key`. 이미 `dismissed`면 첫 결과.

---

## action_log

> 액션 로그의 항목은 schema.org `Action`입니다(`UpdateAction`, `CreateAction`, `ScheduleAction`, `AssessAction`, `AuthorizeAction` …).
> - `action_status`: `potential`(프리뷰, 승인 대기) · `active`(실행 중) · `completed`(실행·검증 완료, `result` 포함) · `failed`(`error` 포함)
> - `agent`: 누가 — 모델(`SoftwareApplication`), 사람(`Person`, `adm_…`/`cus_…`), 시스템 주체(`sys_…`)
> - `object`: 무엇에 — 대상 엔티티
> - `result`: 무엇이 됐나 — 예 새 `UnitPriceSpecification`
> - `instrument`: 어떤 툴로 — 툴 이름(예 `"set_sale_prices"`)
> - 그 밖에 `start_time`, `end_time`, `acom:change_set_id`, `acom:preview_id`, `acom:request_id`(게이트웨이의 `X-Request-Id`)
>
> 로그는 추가만 되고 고치지 않습니다.

### `search_action_log` — Kind: R
- 논리 주소 · REST: `platform.action_log.search` · `GET /v1/platform/action-logs`
- 요약: 액션 로그를 조건으로 찾습니다. 감사, 분쟁 확인, 평가 지표(호출 수·토큰·지연)에 씁니다.
- consumes → produces: → `action_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |------|------|:--:|--------|
  | agent | Text | | 모델 이름 또는 사람·시스템 id. 예 `"adm_sj"` |
  | instrument | Text | | 툴 이름, 예 `"set_sale_prices"` |
  | object_id | Text | | 대상 id, 예 `"prd_windbreaker"` |
  | change_set_id | Text | | 예 `"cs_1042"` |
  | action_status | enum | | `potential` \| `active` \| `completed` \| `failed` |
  | start_time_from / start_time_through | DateTime | | 기간 |
  | limit / cursor | Integer / Text | | 기본 20, 최대 100 / 이전 응답의 `next_cursor` |
- 출력:
```jsonc
{"items": [{
  "@type": "UpdateAction", "id": "act_1042_03",
  "action_status": "completed",
  "agent": {"@type": "SoftwareApplication", "name": "muse-glimmer-30b"},
  "instrument": "set_sale_prices",
  "object": {"@type": "Offer", "id": "off_windbreaker_m_slate"},
  "result": {"@type": "UnitPriceSpecification", "id": "ups_windbreaker_m_slate_1042", "price": 71.20, "price_currency": "USD",
             "price_type": "sale_price", "valid_for_member_tier": "vip",
             "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00"},
  "start_time": "2026-11-06T11:08:41-05:00", "end_time": "2026-11-06T11:08:42-05:00",
  "acom:change_set_id": "cs_1042", "acom:preview_id": "pv_8f2c", "acom:request_id": "req_01jc8x9a4b",
  "acom:tokens": {"input": 1840, "output": 220}, "acom:latency_ms": 742}],
 "total": 47,
 "next_cursor": "c_act_1042_04"}
```
- 프리뷰: 해당 없음(읽기)
- 정책: 개인정보는 기본 가림. 다른 스토어는 볼 수 없음(토큰의 `tenant`)
- 에러: `forbidden` → `list_roles` · `invalid_filter` → `search_action_log`
- 멱등성: 읽기(부작용 없음)

### `get_action_log` — Kind: R
- 논리 주소 · REST: `platform.action_log.get` · `GET /v1/platform/action-logs/{action_id}`
- 요약: 액션 하나를 펼쳐 보여줍니다(프리뷰의 `changes`·`checks` 전체, 승인한 사람, 정책 버전).
- consumes → produces: `action_id` → action
- 입력: `action_id`(✓, `^act_[a-z0-9_]+$`), `fields?[]`
- 출력: `Action`(위 형식) + `acom:policy_version`, `acom:approved_by`
- 프리뷰: 해당 없음(읽기)
- 정책: 토큰의 스토어 범위 안에서만
- 에러: `action_not_found` → `search_action_log` · `forbidden` → `list_roles`
- 멱등성: 읽기(부작용 없음)

---


## schema

### `describe_data` — Kind: R
- 논리 주소 · REST: `platform.schema.describe` · `GET /v1/platform/schema?term=orderStatus`
- 요약: DB의 데이터 사전을 돌려줍니다. 테이블·컬럼마다 schema.org 용어, 한 문장 설명, 타입, 값 목록, 참조 대상, API 이름이 있습니다. 인사이트 에이전트가 "order_status가 무슨 뜻인가", "일별 매출은 어디에 있나"를 확인할 때 씁니다. 원천은 마이그레이션의 주석이고 `cargo xtask check-schema --write`가 만든 `schema-map.json`과 같습니다([../backend/database.md](../backend/database.md) §7).
- consumes → produces: — → `table_name[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |---|---|:--:|---|---|
  | `term` | Text | | schema.org 또는 acom 용어(`orderStatus`, `schema:Order`, `acom:percentOff`). 이 용어를 쓰는 테이블·컬럼만 | `"orderStatus"` |
  | `schema` | enum | | `platform` \| `catalog` \| `merchandising` \| `search` \| `promotion` \| `customer` \| `sales` \| `fulfillment` \| `messaging` \| `analytics`. 그 스키마만 | `"sales"` |
  | `table` | Text | | `<스키마>.<테이블>` 하나 | `"sales.sales_order"` |
  | `api_name` | Text | | API에서 쓰는 이름으로 찾기(`variant_id`, `product_id`) | `"variant_id"` |
- 출력:
  ```jsonc
  {"items": [
     {"table": "sales.sales_order", "term": "schema:Order", "id_prefix": "ord", "api_name": "order",
      "description": "An order a shopper placed. order_number is what people see.",
      "columns": [
        {"name": "order_status", "type": "text", "nullable": false, "term": "schema:orderStatus",
         "description": "Where the order is as a whole.", "values": ["payment_due", "processing", "in_transit", "delivered", "returned", "cancelled", "problem"]}]}],
   "total": 1}
  ```
- 프리뷰: 해당 없음(읽기)
- 정책: 운영자 스코프만(`tools:read:platform`). 원가처럼 운영자 전용 컬럼도 설명은 보이지만 값은 이 툴로 읽지 않습니다
- 에러: `not_found`(`detail.entity: term` 또는 `table`) → `describe_data`(조건 없이 다시)
- 멱등성: 읽기(부작용 없음). 배포된 마이그레이션 버전에 따라 바뀜

## enum

### `list_enums` — Kind: R
- 논리 주소 · REST: `platform.enum.list` · `GET /v1/platform/enums`
- 요약: enum마다 값, 쇼퍼·운영자용 라벨, 뜻, 순서, 허용 전이를 돌려줍니다. 값 목록의 단일 출처는 [enums.md](enums.md)이고 이 툴은 그것을 그대로 내보냅니다. 숫자·문자 코드 대신 의미 있는 단어만 쓰므로 에이전트는 값을 올바르게 고르고 검증할 수 있습니다. 쓰기 입력 검증도 이 집합을 씁니다.
- consumes → produces: → enum definitions
- 입력: `names?[]`(예 `["order_status", "availability"]`), `applies_to?`(예 `"sales.order"`), `in_language?`(기본 스토어 언어)
- 출력:
```jsonc
{"items": [
  {"name": "order_status", "applies_to": ["sales.order", "sales.order_item"], "schema_org": "OrderStatus",
   "values": [
     {"value": "payment_due", "label": "Payment due", "order": 1},
     {"value": "processing",  "label": "Processing",  "order": 2},
     {"value": "in_transit",  "label": "In transit",  "order": 3},
     {"value": "delivered",   "label": "Delivered",   "order": 4},
     {"value": "returned",    "label": "Returned",    "order": 5, "terminal": true},
     {"value": "cancelled",   "label": "Cancelled",   "order": 6, "terminal": true},
     {"value": "problem",     "label": "Needs attention", "order": 7}],
   "transitions": {"payment_due": ["processing", "cancelled", "problem"], "processing": ["in_transit", "cancelled", "problem"],
                   "in_transit": ["delivered", "problem"], "delivered": ["returned"], "problem": ["processing", "in_transit", "cancelled"]}},
  {"name": "availability", "applies_to": ["catalog.product", "catalog.inventory"], "schema_org": "ItemAvailability",
   "values": [
     {"value": "in_stock", "label": "In stock"}, {"value": "out_of_stock", "label": "Out of stock"},
     {"value": "pre_order", "label": "Pre-order"}, {"value": "back_order", "label": "Back order"},
     {"value": "discontinued", "label": "Discontinued", "terminal": true}]},
  {"name": "change_set_state", "applies_to": ["platform.change_set"],
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

> 사람(운영자·쇼퍼)의 로그인·로그아웃·세션은 게이트웨이의 `/api/auth/*`가 합니다([../backend/gateway.md](../backend/gateway.md) §4–§5). 아래 툴은 **외부 에이전트와 연동 클라이언트**의 토큰을 위한 것이며 비밀번호를 받지 않습니다(비밀번호는 사람 전용 엔드포인트만, [README.md](README.md) §2).
>
> - **클라이언트 = API 키.** `client_id`는 [api_key](#api_key)의 id(`key_…`)이고 비밀 값은 `issue_api_key`가 한 번 보여 준 값입니다. 클라이언트 세션은 `platform.session`의 한 행(`actor_kind: external_agent`, 주체 = 키)이라 게이트웨이의 쿠키 세션 조회는 이 행을 사람으로 받지 않고, 이 툴들은 사람의 브라우저 세션을 건드리지 않습니다. 대상 `audience`는 `shopper`(외부 에이전트가 들어오는 MCP 면)입니다.
> - **비밀은 저장하지 않음.** `client_secret_ref`·`code`·`refresh_token` 입력은 툴 스키마에서 `writeOnly`이고 프리뷰 저장소에는 `"(not stored)"`로 남습니다. 토큰은 SHA-256 해시로만 저장하고, 실행 응답에 한 번만 나옵니다(같은 `preview_id` 재실행은 토큰 없이 첫 결과). 상한·수명·잠금은 [platform.yml](../../policies/platform.yml) `client_sessions`.
> - **프리뷰 → 실행.** 다른 쓰기와 같이 프리뷰가 자격을 확인하고 만들 세션(`changes`: `create session`, 상한을 넘으면 가장 오래된 세션의 `revoke`)을 보여 주며, 실행이 토큰을 냅니다. 승인은 필요 없습니다. 예외 둘은 방어용 기록이라 프리뷰에서도 남습니다: 틀린 비밀 값의 횟수(잠금), 재사용된 리프레시 토큰(그 클라이언트의 모든 세션 종료).
> - 게이트웨이가 아직 세션 없는 외부 클라이언트를 이 경로로 보내지 않으므로, 지금은 `tools:write:platform`을 가진 호출자만 부를 수 있습니다(게이트웨이 연결은 후속 작업).

### `login` — Kind: W
- 논리 주소 · REST: `platform.auth.login` · `POST /v1/platform/auth/login`
- 요약: 외부 에이전트의 OAuth 코드 또는 연동 클라이언트의 자격으로 액세스·리프레시 토큰을 받습니다. 토큰의 스코프는 요청과 허용의 교집합이고, 외부 에이전트는 `external:*`(읽기·장바구니·주문 준비)만 받습니다.
- consumes → produces: → access token
- 입력: `grant_type`(✓, `oauth_code` \| `client_credentials`), `client_id`(✓), `code`(`oauth_code`일 때 ✓), `client_secret_ref`(`client_credentials`일 때 ✓, 발급된 API 키), `requested_scopes?[]`
- 출력: `{"ok": true, "id": "ses_7q2", "access_token": "…", "refresh_token": "…", "token_type": "Bearer", "expires_in": 900, "claims": {"sub": "key_31", "tenant": "store_northwind", "kind": "external_agent", "scopes": ["external:read", "external:cart"]}, "undo": null}` — `access_token`·`refresh_token`은 이 응답에 한 번만
- 프리뷰: `changes` = `create` `session`(`to`: 클라이언트·스코프·`expires_in`), 클라이언트 세션 수가 `client_sessions.max_sessions_per_client`를 넘으면 가장 오래된 세션의 `revoke`. `needs_approval: false`. 이 스토어는 아직 OAuth 승인 화면이 없어 `oauth_code`는 언제나 `invalid_credentials`(`detail.hint`)
- 정책: [customer.yml](../../policies/customer.yml) `password.lockout`의 실패 잠금 규칙을 클라이언트 자격에도 적용(`client_sessions.lockout_from`). 틀린 비밀 값은 키의 `failed_attempt_count`에 세고, 기준에 닿으면 `locked_until`까지 `account_locked`
- 에러: `invalid_credentials`(class `unauthenticated`) → `login` · `account_locked` → `null` · `scope_not_granted`(class `forbidden`) → `list_permissions`
- 멱등성: 발급은 상태가 없어 필요 없음. 리프레시 토큰은 한 번만 쓰고 교체

### `refresh_token` — Kind: W
- 논리 주소 · REST: `platform.auth.refresh` · `POST /v1/platform/auth/refresh`
- 요약: 리프레시 토큰으로 새 액세스 토큰(과 교체된 리프레시 토큰)을 받습니다. 이전 리프레시 토큰은 바로 무효이고, 재사용이 감지되면 그 클라이언트의 모든 세션을 끊습니다.
- consumes → produces: refresh token → access token
- 입력: `refresh_token`(✓). 스토어·스코프는 원래 토큰에서 이어받고 넓힐 수 없음
- 출력: `login`과 같은 형식
- 프리뷰: `changes` = `create` `session`(같은 클라이언트·스코프) + `revoke` 이전 세션. `needs_approval: false`
- 정책: 리프레시 토큰 교체(이전 세션은 `replaced_by_id`로 새 세션을 가리킴), 세션 수 상한, 수명은 [platform.yml](../../policies/platform.yml) `client_sessions`
- 에러: `refresh_expired` → `login` · `refresh_reused` → `login` · `revoked` → `login`
- 멱등성: 한 번만 쓸 수 있음

### `logout` — Kind: W
- 논리 주소 · REST: `platform.auth.logout` · `POST /v1/platform/auth/logout`
- 요약: 현재 세션(또는 그 클라이언트의 모든 세션)의 리프레시 토큰을 폐기합니다. 액세스 토큰은 짧은 만료까지 유효합니다.
- consumes → produces: →
- 입력: `scope?`(`current` \| `all_sessions`, 기본 `current`). 현재 세션은 요청 토큰의 세션이고, API 클라이언트(`external_agent`)의 요청만 받습니다(사람은 `forbidden`, 게이트웨이의 `/api/auth/logout`)
- 출력: `{"ok": true, "id": "ses_7q2", "sessions_ended": 1, "undo": null}`
- 프리뷰: `changes` = 끝낼 세션마다 `revoke` `session`. `needs_approval: false`
- 정책: 없음
- 에러: `already_revoked`(class `invalid_state`) → `login`
- 멱등성: 다시 불러도 같은 결과

---

## tenant

### `get_tenant` — Kind: R
- 논리 주소 · REST: `platform.tenant.get` · `GET /v1/platform/tenants/{tenant_id}`
- 요약: 스토어 정보(이름·언어·통화·시간대·기능 플래그·정책 덮어쓰기 요약).
- consumes → produces: `tenant_id` → tenant
- 입력: `tenant_id`(✓, 토큰의 `tenant`와 같아야 함, `^store_[a-z0-9_]+$`), `fields?[]`
- 출력: `{"id": "store_northwind", "name": "Northwind Apparel", "url": "northwind.example", "in_language": "en", "currency": "USD", "acom:time_zone": "America/New_York", "acom:feature_flags": {}, "acom:policy_overrides": 9, "acom:status": "active"}`
- 프리뷰: 해당 없음(읽기)
- 정책: 다른 스토어 조회는 플랫폼 관리자만. 서비스 접속은 행 수준 보안으로 요청의 스토어만 보므로, 지금은 토큰의 `tenant`가 아닌 스토어는 누구에게나 `forbidden`
- 에러: `forbidden`(`detail.reason: tenant`) → `null` · `tenant_not_found` → `list_tenants`
- 멱등성: 읽기(부작용 없음)

### `list_tenants` — Kind: R
- 논리 주소 · REST: `platform.tenant.list` · `GET /v1/platform/tenants`
- 요약: 전체 스토어 목록. 플랫폼 관리자 전용입니다. 행 수준 보안 아래에서는 요청의 스토어 하나가 보입니다.
- consumes → produces: → `tenant_id[]`
- 입력: `status?`(`active` \| `suspended`), `limit`(기본 20), `cursor?`
- 출력: `{"items": [{"id": "store_northwind", "name": "Northwind Apparel", "acom:status": "active"}], "total": 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: 플랫폼 관리자 역할만
- 에러: `forbidden` → `list_roles`
- 멱등성: 읽기(부작용 없음)

---

## role

> 스코프 목록은 [../backend/gateway.md](../backend/gateway.md) §2(`tools:read:{domain}`, `tools:write:{domain}`, `changeset:draft`, `human:*`, `external:*`, `admin:demo`). 역할은 스코프 묶음이고, 게이트웨이가 역할 → 스코프를 읽어 토큰을 만듭니다.
>
> 이 절과 `menu`·`admin_user`·`org`·`api_key`·`list_tenants`의 "관리자 역할만"은 `tools:write:platform` 스코프(플랫폼 관리자 역할만 가짐)를 가진 호출자 또는 그 사람을 대신하는 에이전트입니다. 권한이 바뀌는 W$(`assign_role`, `create_admin_user`, `set_admin_status`, `issue_api_key`, `revoke_api_key`)는 [approval.yml](../../policies/approval.yml) `thresholds.access_change`로 `approver_role: platform_admin`이 승인합니다.

### `list_roles` — Kind: R
- 논리 주소 · REST: `platform.role.list` · `GET /v1/platform/roles`
- 요약: 역할 목록(이름·스코프·시스템 역할 여부).
- consumes → produces: → `role_id[]`
- 입력: `limit`(기본 20), `cursor?`
- 출력: `{"items": [{"id": "rol_merchandiser", "name": "Merchandiser", "scopes": ["tools:read:promotion", "tools:write:promotion", "tools:write:merchandising", "changeset:draft", "human:approve", "human:reject", "human:undo"], "acom:is_system": true}], "total": 6}`
- 프리뷰: 해당 없음(읽기)
- 정책: 관리자 역할만
- 에러: `forbidden` → `null`
- 멱등성: 읽기(부작용 없음)

### `get_role` — Kind: R
- 논리 주소 · REST: `platform.role.get` · `GET /v1/platform/roles/{role_id}`
- 요약: 역할 하나(스코프·승인 역할·배정 인원).
- consumes → produces: `role_id` → role
- 입력: `role_id`(✓)
- 출력: `{"id": "rol_merchandiser", "name": "Merchandiser", "scopes": […], "approver_roles": ["merchandiser", "operator"], "acom:assigned_count": 1}` — `approver_roles`는 [approval.yml](../../policies/approval.yml) 임계의 `approver_role` 값
- 프리뷰: 해당 없음(읽기)
- 정책: 관리자 역할만
- 에러: `role_not_found` → `list_roles`
- 멱등성: 읽기(부작용 없음)

### `create_role` — Kind: W
- 논리 주소 · REST: `platform.role.create` · `POST /v1/platform/roles`
- 요약: 사용자 정의 역할(스코프 묶음)을 만듭니다. 시스템 역할은 고칠 수 없습니다. `human:*` 스코프를 가진 역할도 만들 수 있지만 그 스코프는 사람이 누른 요청에만 붙습니다.
- consumes → produces: → `role_id`
- 입력: `name`(✓), `scopes[]`(✓), `approver_roles?[]`, `description?`
- 출력: 프리뷰 → 실행 `{"ok": true, "id": "rol_returns_desk", "undo": {"by": "person", "target": "cs_1047", "until": "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: `changes` = `create` `role`. checks `roles.known_scopes`(없는 스코프 거부). `needs_approval: false`(배정은 `assign_role`이 승인).
- 정책: [platform.yml](../../policies/platform.yml) `roles.known_scopes`
- 에러: `duplicate_role` → `list_roles` · `invalid_scope` → `list_permissions`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 같은 이름이면 `duplicate_role`.

### `assign_role` — Kind: W$
- 논리 주소 · REST: `platform.role.assign` · `POST /v1/platform/roles/{role_id}/assignments`
- 요약: 관리자 계정에 역할을 주거나 뺍니다. 권한이 바뀌므로 항상 변경셋 승인을 거칩니다.
- consumes → produces: `role_id`, `admin_user_id` → `role_id`
- 입력: `role_id`(✓), `admin_user_id`(✓), `action`(✓, `grant` \| `revoke`)
- 출력: 프리뷰 → 실행 `{"ok": true, "id": "rol_returns_desk", "undo": {"by": "person", "target": "cs_1047", "until": "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: `changes` = `grant`/`revoke` `role`(`id`: 역할, `to`: 관리자 id). checks `roles.least_privilege`. 항상 `needs_approval: true`, 예 `approval_reason: "Gives refund rights to 1 person"`.
- 정책: [platform.yml](../../policies/platform.yml) `roles.least_privilege`, [approval.yml](../../policies/approval.yml)
- 에러: `already_assigned` → `get_role` · `not_assigned`(class `invalid_state`, 가지지 않은 역할을 빼려 함) → `get_role` · `role_not_found` → `list_roles` · `admin_user_not_found` → `search_admin_users`
- 멱등성: `(role_id, admin_user_id)`로 한 건

### `list_permissions` — Kind: R
- 논리 주소 · REST: `platform.role.list_permissions` · `GET /v1/platform/permissions`
- 요약: 역할을 설계할 때 고를 수 있는 스코프 목록과 뜻.
- consumes → produces: → permissions
- 입력: `domain?`, `limit`(기본 20), `cursor?`
- 출력: `{"items": [{"scope": "tools:write:promotion", "domain": "promotion", "description": "Preview promotion writes and run the ones that need no approval"}], "total": 31}`
- 프리뷰: 해당 없음(읽기)
- 정책: 관리자 역할만
- 에러: `forbidden` → `null`
- 멱등성: 읽기(부작용 없음)

---

## menu

### `list_menus` — Kind: R
- 논리 주소 · REST: `platform.menu.list` · `GET /v1/platform/menus`
- 요약: 현재 역할로 볼 수 있는 운영 화면 메뉴 트리와 즐겨찾기.
- consumes → produces: → `menu_id[]`
- 입력: 없음
- 출력: `{"items": [{"id": "mnu_promotions", "name": "Promotions", "path": "/bo/promotions", "required_scope": "tools:read:promotion", "children": [], "favorited": true}], "total": 9}`
- 프리뷰: 해당 없음(읽기)
- 정책: 역할 스코프로 걸러짐
- 에러: `unauthenticated` → `null`
- 멱등성: 읽기(부작용 없음)

### `set_menu` — Kind: W
- 논리 주소 · REST: `platform.menu.set` · `PUT /v1/platform/menus/{menu_id}`
- 요약: 메뉴 정의(경로·이름·필요 스코프·순서·표시 여부)를 정합니다. 관리자 전용.
- consumes → produces: → `menu_id`
- 입력: `menu_id`(✓), `name?`, `path?`, `required_scope?`, `position?`, `visible?`
- 출력: 프리뷰 → 실행 `{"ok": true, "id": "mnu_promotions", "undo": {"by": "person", "target": "cs_1047", "until": "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: checks `roles.known_scopes`. `needs_approval: false`.
- 정책: 관리자 역할만
- 에러: `invalid_scope` → `list_permissions` · `menu_not_found` → `list_menus`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과.

### `set_favorite` — Kind: W
- 논리 주소 · REST: `platform.menu.set_favorite` · `PUT /v1/platform/menus/{menu_id}/favorite`
- 요약: 내 즐겨찾기 메뉴를 켜거나 끕니다. 자기 화면 설정이라 변경셋에 넣지 않고 바로 실행합니다.
- consumes → produces: `menu_id` → `menu_id`
- 입력: `menu_id`(✓), `favorited`(✓, Boolean)
- 출력: 프리뷰 → 실행 `{"ok": true, "id": "mnu_promotions", "undo": null}` — 다시 부르면 되돌아감
- 프리뷰: `needs_approval: false`
- 정책: 없음
- 에러: `menu_not_found` → `list_menus`
- 멱등성: 상태 하나로 덮어씀

---

## admin_user

### `search_admin_users` — Kind: R
- 논리 주소 · REST: `platform.admin_user.search` · `GET /v1/platform/admin-users`
- 요약: 관리자 계정 목록. 개인정보는 기본 가림.
- consumes → produces: → `admin_user_id[]`
- 입력: `role_id?`, `status?`(`active` \| `suspended` \| `locked`), `org_id?`, `query?`, `limit`(기본 20), `cursor?`
- 출력: `{"items": [{"id": "adm_sj", "name": "Son SeongJun", "job_title": "Merchandiser", "roles": ["rol_merchandiser"], "status": "active"}], "total": 4}`
- 프리뷰: 해당 없음(읽기)
- 정책: 관리자 역할만
- 에러: `forbidden` → `list_roles`
- 멱등성: 읽기(부작용 없음)

### `create_admin_user` — Kind: W$
- 논리 주소 · REST: `platform.admin_user.create` · `POST /v1/platform/admin-users`
- 요약: 관리자 계정을 만들고 역할·조직을 줍니다. 권한을 가진 사람이 새로 생기므로 항상 변경셋 승인. 첫 로그인 때 비밀번호를 정하게 하는 링크를 보냅니다(비밀번호는 툴이 다루지 않음).
- consumes → produces: `role_id[]`, `org_id?` → `admin_user_id`
- 입력: `name`(✓), `email`(✓), `role_ids[]`(✓), `org_id?`
- 출력: 프리뷰 → 실행 `{"ok": true, "id": "adm_kim", "invitation": {"channel": "email", "to": "…", "sent": false, "note": "…"}, "undo": {"by": "person", "target": "cs_1047", "until": "2026-11-07T23:59:00-05:00"}}` — 메일 발송은 이 빌드에서 스텁이라 `sent: false`와 그 사실을 적습니다. 되돌리기는 한 번도 로그인하지 않은 계정만 지웁니다
- 프리뷰: `changes` = `create` `admin_user`(`id: null`). checks `roles.known_roles`(모든 `role_ids`가 스토어의 역할), `admin_users.email_unique`. 항상 `needs_approval: true`.
- 정책: [platform.yml](../../policies/platform.yml) `roles.known_roles`, `admin_users.email_unique`, [approval.yml](../../policies/approval.yml)
- 에러: `duplicate_email` → `search_admin_users` · `invalid_role` → `list_roles`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 같은 이메일이면 `duplicate_email`.

### `set_admin_status` — Kind: W$
- 논리 주소 · REST: `platform.admin_user.set_status` · `PUT /v1/platform/admin-users/{admin_user_id}/status`
- 요약: 관리자 계정을 정지·잠금 해제·복원합니다. 항상 변경셋 승인.
- consumes → produces: `admin_user_id` → `admin_user_id`
- 입력: `admin_user_id`(✓), `status`(✓, `active` \| `suspended` \| `locked`), `reason`(✓)
- 출력: 프리뷰 → 실행 `{"ok": true, "id": "adm_kim", "undo": {"by": "person", "target": "cs_1047", "until": "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: `changes` = `update` `admin_user`(`from`/`to` 상태). 항상 `needs_approval: true`.
- 정책: [approval.yml](../../policies/approval.yml)
- 에러: `already_in_state` → `search_admin_users` · `admin_user_not_found` → `search_admin_users`
- 멱등성: 같은 상태로의 재요청은 성공으로 처리.

---

## org

### `list_orgs` — Kind: R
- 논리 주소 · REST: `platform.org.list` · `GET /v1/platform/orgs`
- 요약: 조직(`Organization`) 트리(이름·종류·상위 조직·인원).
- consumes → produces: → `org_id[]`
- 입력: `parent_org_id?`, `limit`(기본 20), `cursor?`
- 출력: `{"items": [{"id": "org_merch", "name": "Merchandising", "acom:org_type": "department", "parent_org_id": null, "acom:member_count": 3}], "total": 4}` — `acom:org_type`: `department` \| `vendor` \| `branch`
- 프리뷰: 해당 없음(읽기)
- 정책: 관리자 역할만
- 에러: `forbidden` → `list_roles`
- 멱등성: 읽기(부작용 없음)

### `set_org` — Kind: W
- 논리 주소 · REST: `platform.org.set` · `PUT /v1/platform/orgs/{org_id}`
- 요약: 조직을 만들거나 고칩니다(이름·종류·상위 조직).
- consumes → produces: `org_id?` → `org_id`
- 입력: `org_id?`(없으면 새로), `name`(✓), `org_type`(✓, `department` \| `vendor` \| `branch`), `parent_org_id?`
- 출력: 프리뷰 → 실행 `{"ok": true, "id": "org_merch", "undo": {"by": "person", "target": "cs_1047", "until": "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: checks `orgs.no_cycle`(자기 자신·하위를 상위로 둘 수 없음). `needs_approval: false`.
- 정책: [platform.yml](../../policies/platform.yml) `orgs.no_cycle`
- 에러: `cycle_detected` → `list_orgs` · `org_not_found`(`parent_org_id`가 없음) → `list_orgs` · `invalid_input`(`detail.reason: taken`, 다른 조직의 이름) → `list_orgs`. `org_id` 없이 같은 이름의 조직이 있으면 그 조직을 고칩니다
- 멱등성: 같은 `preview_id` 재실행은 첫 결과.

---

## api_key

### `list_api_keys` — Kind: R
- 논리 주소 · REST: `platform.api_key.list` · `GET /v1/platform/api-keys`
- 요약: API 키 목록. 비밀 값은 돌려주지 않고 앞자리만 보여줍니다.
- consumes → produces: → `api_key_id[]`
- 입력: `status?`(`active` \| `revoked`), `limit`(기본 20), `cursor?`
- 출력: `{"items": [{"id": "key_31", "name": "Outside shopping agent", "scopes": ["external:read", "external:cart", "external:prepare_order"], "status": "active", "key_prefix": "ak_live_3f", "last_used_at": "2026-11-06T09:14:00-05:00"}], "total": 2}`
- 프리뷰: 해당 없음(읽기)
- 정책: 관리자 역할만
- 에러: `forbidden` → `list_roles`
- 멱등성: 읽기(부작용 없음)

### `issue_api_key` — Kind: W$
- 논리 주소 · REST: `platform.api_key.issue` · `POST /v1/platform/api-keys`
- 요약: 외부 에이전트·연동용 API 키를 발급합니다. 스코프는 발급자 역할과의 교집합으로 줄어듭니다. 비밀 값은 실행 응답에 한 번만 나옵니다. 항상 변경셋 승인.
- consumes → produces: → `api_key_id`
- 입력: `name`(✓), `scopes[]`(✓), `expires?`(DateTime)
- 출력: 프리뷰 → 실행 `{"ok": true, "id": "key_31", "secret": "(한 번만 표시)", "undo": {"by": "person", "target": "cs_1047", "until": "2026-11-07T23:59:00-05:00"}}` — 되돌리면 키가 폐기됨. W$라 실행은 승인된 변경셋 안에서 일어나므로 비밀 값은 승인한 사람의 `approve` 응답 `shown_once[]`(`{preview_id, tool, result_id, secret}`)에 한 번만 나오고, 프리뷰·변경셋·액션 로그·멱등 키 기록 어디에도 남지 않습니다
- 프리뷰: `changes` = `create` `api_key`(`id: null`). checks `api_keys.scope_within_issuer`(발급자 = 요청한 운영자, 또는 에이전트가 대신하는 운영자. 그 사람의 역할 스코프를 넘으면 `scope_not_granted`), `api_keys.external_agent_scopes`(`external:*`를 가진 키는 목록의 스코프만, 실패 시 `fix_tool: issue_api_key`). `human:*`·`admin:*`·`changeset:*`는 사람과 우리 에이전트의 것이라 키에 주지 않음(`scope_not_granted`). 항상 `needs_approval: true`.
- 정책: [platform.yml](../../policies/platform.yml) `api_keys.scope_within_issuer`, `api_keys.external_agent_scopes`, [approval.yml](../../policies/approval.yml). 비밀 값은 해시로만 저장
- 에러: `scope_not_granted` → `list_permissions` · `duplicate_key_name` → `list_api_keys`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과(비밀 값은 다시 보여 주지 않음).

### `revoke_api_key` — Kind: W$
- 논리 주소 · REST: `platform.api_key.revoke` · `POST /v1/platform/api-keys/{api_key_id}/revoke`
- 요약: API 키를 폐기합니다(`active` → `revoked`, 바로 무효). 항상 변경셋 승인.
- consumes → produces: `api_key_id` → `api_key_id`
- 입력: `api_key_id`(✓), `reason`(✓)
- 출력: 프리뷰 → 실행 `{"ok": true, "id": "key_31", "undo": null}` — 폐기는 되돌릴 수 없음(새로 발급)
- 프리뷰: `changes` = `update` `api_key` `active` → `revoked`(`note`: 사유). 마지막 사용 시각과 끝날 세션 수는 `approval_reason`에(승인 때 다시 계산해도 `changes`가 바뀌지 않게). 항상 `needs_approval: true`.
- 정책: [approval.yml](../../policies/approval.yml)
- 에러: `already_revoked` → `list_api_keys` · `api_key_not_found` → `list_api_keys`
- 멱등성: 이미 폐기된 키는 같은 결과

---

## 예약 작업

`platform.job`의 행 하나가 예약 작업 하나입니다(`job_` id, `kind`, `run_at`, `payload`). 종류마다 처리기가 하나 있고, 처리기는 그 데이터를 가진 도메인이 오퍼레이션 옆에 등록합니다(`RegistryBuilder::job`). **툴이 아니며 모델은 보지 못합니다.**

| `kind` | 도메인 | 하는 일 |
|---|---|---|
| `promotion.start`, `promotion.end` | promotion | `payload {promotion_id}`. 상태는 읽을 때마다 기간과 스토어 시계로 계산되므로 동작은 바뀌지 않습니다. 그 시각에 읽기가 보여 주는 상태(`running`, `ended`)를 `promotion_status`에 기록하고, 바뀌었으면 outbox에 `promotion.started`·`promotion.ended`(`{promotion_id, status, at}`)를 남깁니다. 멈춘·초안·끝난 프로모션은 그대로 둡니다 |
| `watcher.returns_over_refund_threshold` | fulfillment | [task](#task)의 감시 작업. `claim_status`가 `requested`이고 금액이 `approval.thresholds.refund`를 넘는 반품 요청 |
| `watcher.stock_out_forecast` | sales(재고는 catalog) | 오늘을 뺀 최근 `lookback_days`일(스토어 날짜)의 판매량(취소 줄 제외)으로 `재고 × 일수 ≤ days_until_stockout_lte × 판매량`인 상품 |
| `watcher.price_below_cost` | catalog | 정가 < 원가인 변형. 고침은 `price_floor`까지 `set_price` |

**실행기**(`acom-changeset::jobs`, `commerce`의 워커와 데모 시계가 같이 씀):

- 스토어마다 `done_at IS NULL AND run_at <= 스토어 시계`인 작업을 `run_at` 순서로 하나씩 `FOR UPDATE SKIP LOCKED`로 잡고, 작업마다 스토어 범위 트랜잭션 하나에서 실행합니다. 작업의 시계는 그 작업이 기한이 된 시각(`run_at`)입니다. 두 실행기가 같은 작업을 돌리지 않습니다.
- 성공하면 `done_at`·`attempts`를 기록합니다. 실패하면 처리기의 쓰기를 되돌리고 `attempts`·`last_error`를 남긴 뒤 1, 2, 4 … 분(최대 60분) 뒤로 `run_at`을 옮겨 다시 시도합니다. 한 번 부를 때 같은 작업은 한 번만 시도합니다.
- 감시 작업은 되풀이됩니다: 끝나면 `every_minutes` 뒤에 다음 작업을 만듭니다(종류마다 기다리는 작업 하나). 시계가 건너뛰어(데모 시계, 워커 중단) 놓친 실행은 가장 늦은 한 번으로 합칩니다. 감시 작업이 하나도 없는 스토어에는 첫 작업을 만듭니다. `enabled: false`면 감지는 하지 않고 일정만 이어 갑니다.
- 이벤트: 작업마다 `job.updated {job_id, kind, state, run_at}`(`state`: `scheduled` \| `retrying` \| `done`, [enums.md](enums.md) §1.14), 할 일마다 `task.updated`. 이벤트는 트랜잭션이 커밋될 때 나갑니다.
- 워커(`acom-commerce work`)는 몇 초마다 `ACOM_STORE_ID`의 스토어마다 기한이 된 작업을 돌립니다. `commerce`는 RLS 아래의 `acom_app`으로 접속해 스토어 목록을 읽을 수 없으므로 돌릴 스토어를 설정으로 받습니다(게이트웨이와 같은 설정, 쉼표로 여러 개). 종료 신호를 받으면 잡은 작업을 끝내고 멈춥니다.

---

## 데모 시계

데모 스토어의 시각(`platform.demo_clock`)을 읽고 옮기는 런타임 전용 엔드포인트입니다([README.md](README.md) §2.2). **툴이 아니며 모델은 보지 못합니다.** 채팅 턴의 토큰에는 `admin:*` 스코프가 없으므로 에이전트는 시간을 옮길 수 없습니다. 게이트웨이는 `/api/v1/platform/demo-clock`을 운영자 세션의 스코프 그대로 전달하고, 데모 모드의 플랫폼 관리자만 `admin:demo`를 받습니다([../backend/gateway.md](../backend/gateway.md) §2). 백오피스 머리글의 스토어 시각과 장면 시각 이동이 이것을 씁니다.

| 엔드포인트 | 스코프 | 본문 | 응답 |
|---|---|---|---|
| `GET /v1/platform/demo-clock` | `tools:read:platform` | — | `{now, is_running, demo_mode}` |
| `PUT /v1/platform/demo-clock` | `admin:demo`(운영자 본인의 요청만), 데모 모드에서만 | `{now, is_running?}` | `{now, is_running, demo_mode: true, previous, ran[]}` |
| `POST /v1/platform/demo/reset` | `admin:demo`(운영자 본인의 요청만), 데모 모드에서만, `commerce`에 소유자 연결(`MIGRATION_DATABASE_URL`)이 주어진 데모 스택에서만 | — | `{ok: true, store_id, seconds}`. 스토어 테이블을 비우고 데모 시드를 다시 적재(약 15초). 데모에서 한 일과 모든 세션이 지워지므로 다시 로그인합니다. 그 밖의 곳에서는 `demo_reset_unavailable`(invalid_state) |

- `now`: RFC 3339 또는 스토어 시간대의 로컬 시각(`"2026-11-07T10:00"`). `is_running`: `true`면 그때부터 실제 시간과 함께 흐르고(워커가 기한이 된 작업을 돌림), 기본 `false`면 멈춰 있습니다. 데모 모드가 아니면 `GET`은 시스템 시각과 `is_running: true, demo_mode: false`를 돌려줍니다.
- `PUT`은 시계를 옮기기 **전에** `run_at <= now`인 예약 작업을 `run_at` 순서로 실행하고, 실행한 것을 `ran[]`에 적어 답합니다: `{job_id, kind, run_at, state: done | retrying, attempts, error?, result?, tasks?: [{task_id, state, created, change_set_id?}], next_run_at?}`. 같은 시각으로 다시 부르면 아무것도 실행하지 않습니다(`ran: []`). 뒤로 옮겨도 이미 실행된 것은 그대로입니다(장면을 처음부터 하려면 `acom-commerce seed --reset`).
- 장면 시각: Fri Nov 6 11:00(B1·B2·B5), Sat Nov 7 10:00(쇼퍼 장면), Mon Nov 9 09:00(주말 뒤).
- 에러: `forbidden`(`detail.reason: scope`, `detail.required_scope: "admin:demo"`) → `null` · `demo_mode_off`(class `invalid_state`, 실제 시간으로 도는 스토어) → `null` · `invalid_input`(`detail.field: now`) → `null`
- 예: Fri Nov 6 11:00에서 `PUT {"now": "2026-11-07T10:00"}`
  ```jsonc
  {"now": "2026-11-07T10:00:00-05:00", "is_running": false, "demo_mode": true,
   "previous": "2026-11-06T11:00:00-05:00",
   "ran": [{"job_id": "job_watch_returns", "kind": "watcher.returns_over_refund_threshold", "run_at": "2026-11-07T09:40:00-05:00",
            "state": "done", "attempts": 1, "tasks": [{"task_id": "task_returns_1106", "state": "working", "created": false}],
            "next_run_at": "2026-11-07T10:10:00-05:00"},
           {"job_id": "job_watch_stock", "kind": "watcher.stock_out_forecast", "run_at": "2026-11-07T09:40:00-05:00",
            "state": "done", "attempts": 1, "next_run_at": "2026-11-07T10:40:00-05:00"},
           {"job_id": "job_watch_price", "kind": "watcher.price_below_cost", "run_at": "2026-11-07T09:55:00-05:00",
            "state": "done", "attempts": 1, "next_run_at": "2026-11-07T10:55:00-05:00"}]}
  ```
- 멱등성: 같은 `now`로 다시 불러도 결과 상태가 같습니다.

---

## 내부 엔드포인트

툴이 아니고 사람 전용도 아닌, 서비스끼리 쓰는 읽기입니다. 게이트웨이를 거친 웹(운영자 세션)과 agent 서비스만 부릅니다.

| 엔드포인트 | 스코프 | 내용 |
|---|---|---|
| `GET /v1/platform/store` | 토큰만 | 요청 문맥의 스토어: `{store_id, currency, time_zone, now}`. `now`는 데모 모드면 데모 시계. agent 서비스가 턴마다 스토어 시각·통화를 여기서 받습니다(commerce 테이블을 직접 읽지 않음) |
| `GET /v1/platform/previews/{preview_id}` | `changeset:draft` 또는 `tools:read:platform` | 저장된 프리뷰 하나: `{preview_id, operation, input, changes[], checks[], needs_approval, approval_reason, expires_at, state, change_set_id, policy_version, created_at}`. `state`는 [enums.md](enums.md) §1.9. 런타임이 실행 전 확인하거나 화면이 프리뷰를 다시 그릴 때 |
| `GET /v1/platform/events` (SSE) | `tools:read:platform` | 스토어의 진행 이벤트: `change_set.updated {change_set_id, version, state}`, `task.updated {task_id, state, change_set_id?}`, `job.updated {job_id, kind, state, run_at}`([예약 작업](#예약-작업)). 각 이벤트는 `id:`(스토어 안 순번)를 가지며 `Last-Event-ID`로 이어 받음. agent 서비스가 구독해 대화의 `change_set.updated`([chat.md](chat.md) §2)로 흘림 |
