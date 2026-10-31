# A-Commerce API 명세

A-Commerce의 전체 API(에이전트가 호출하는 툴, 사람 전용 엔드포인트, 에이전트 런타임 전용 엔드포인트)를 계층 구조로 정리한 명세입니다. 설계 배경은 [../architecture.md](../architecture.md), 구현(Rust 워크스페이스, 서비스, 변경셋·정책 엔진)은 [../backend/](../backend/README.md)에 있습니다.

| 문서 | 내용 |
|---|---|
| [conventions.md](conventions.md) | 공통 규약: 이름, 식별자 접두, 금액, 비율, 시간, 읽기·쓰기 형식, 툴 스키마, 헤더 |
| [vocabulary.md](vocabulary.md) | schema.org 위의 확장 규칙(`@type` + `additionalType`), 우리 용어. 정의는 `vocab/acom.jsonld` |
| [enums.md](enums.md) | 상태·종류·사유 값 목록의 단일 출처 |
| [errors.md](errors.md) | 에러 모양, 분류(class) → HTTP 상태, 도메인 코드 |
| [chat.md](chat.md) | 사람 ↔ 에이전트: 채팅 SSE 이벤트와 UI 블록 |
| [data-model.md](data-model.md) | 엔티티 ↔ schema.org, DB 명명 |
| [policies/](../../policies/README.md) | 정책 데이터(규칙을 YAML로) |
| [../scenarios.md](../scenarios.md) | 데모 스토어의 사람·상품·정책·장면. 예시의 숫자는 여기서 옵니다 |

도메인 파일(아래 §4의 13개)이 툴마다 입력·출력·체크·에러를 정합니다. 이 README는 그 목록(인덱스)과 모든 툴에 공통인 부분의 요약입니다.

---

## 1. 계층 구조

모든 오퍼레이션은 **group → domain → resource → operation** 4단계로 주소화합니다.

```
a-commerce API
├── Offer      판매하는 것
│   ├── catalog          상품·변형·가격·재고·브랜드·리뷰·문의
│   ├── merchandising    카테고리·진열 슬롯·배너·팝업·기획전
│   └── search           검색·비교·자동완성·사전·색인
├── Transact   장바구니에서 배송까지
│   ├── sales            장바구니·주문 준비·주문·결제·영수증·구독·선물
│   └── fulfillment      클레임(취소·반품·교환)·검수·환불·배송·배송비
├── Engage     고객과 혜택
│   ├── promotion        프로모션·회원가·쿠폰·혜택 계산·적립금·이벤트·세그먼트
│   ├── customer         회원·등급·주소·동의·기억(선호)·위시리스트·상담·보상·FAQ
│   ├── messaging        고객 메시지·템플릿·캠페인·알림
│   └── community        게시판·글·댓글
└── Operate    스토어 운영
    ├── platform         정책·변경셋·할 일·액션 로그·의미 enum·인증·스토어(테넌트)·권한
    ├── analytics        매출·고객·상담 집계(읽기)
    ├── partner          입점 판매자·계약·등급
    └── integration      외부 채널 연계·대량 가져오기·택배/결제 이벤트 수신
```

| 수준 | 의미 | 예 |
|---|---|---|
| group | 비즈니스의 큰 부분 | Engage |
| domain | 자기 규칙·정책을 가진 영역 | `promotion` |
| resource | schema.org 타입 또는 그 확장 | `offer`(회원가 = `UnitPriceSpecification`) |
| operation | 하나의 비즈니스 동사 = 툴 | `set_sale_prices` |

하나의 오퍼레이션 정의에서 세 가지 이름이 생성됩니다.

| 표면 | 형식 | 예 |
|---|---|---|
| 논리 주소 | `domain.resource.operation` | `promotion.offer.set_sale_prices` |
| 툴 이름(모델이 보는 이름) | `verb_noun`, 평이한 영어 | `set_sale_prices` |
| REST 경로 | `METHOD /v1/{domain}/{resources}/...` | `POST /v1/promotion/sale-prices` |

툴 검색은 이 계층을 씁니다. 에이전트마다 쓸 수 있는 도메인(`agents/*.yml`의 `tool_scope`)과 주 도메인(`home_domains`)이 정해져 있고, 그 안에서 요청의 낱말로 툴에 점수를 매긴 뒤(BM25, 주 도메인 가산) 툴 그래프 이웃을 더해 후보 5–8개를 냅니다(`agents/runtime.yml` `tool_search`). 모델은 290개 목록이 아니라 이 후보만 봅니다([../backend/agent.md](../backend/agent.md) §3).

## 2. 오퍼레이션 종류

| 표기 | 종류 | 누가 부르나 | 동작 |
|---|---|---|---|
| **R** | read | 에이전트(툴) | 부작용 없음. 목록은 `items`와 `total`을 한 번에, 하나를 가져오는 `get_*`는 엔티티 자체. `fields`로 응답 축소 |
| **W** | write | 에이전트(툴) | 프리뷰 먼저. 승인 필요 여부는 [approval.yml](../../policies/approval.yml)의 임계가 정함(예: `refund_return`은 $200.00 초과일 때만 사람 승인). 승인이 필요 없으면 프리뷰 뒤 바로 실행(장바구니 담기, 쇼퍼 선호 기억, $200 이하 환불) |
| **W$** | write, 항상 승인 | 에이전트(툴) | 고객에게 닿는 가격·혜택·노출·메시지, 권한·계약처럼 정책과 상관없이 사람이 봐야 하는 쓰기. 프리뷰 → 변경셋 → **사람 승인** → 실행기가 `preview_id`로 실행 |
| **W(프리뷰만)** | write, 프리뷰만 | 에이전트(툴) | `prepare_order`, `prepare_subscription`, `prepare_gift_order`. 프리뷰를 만들 뿐 실행 단계가 없고 `needs_approval`은 항상 `false`. 그 `preview_id`를 사람 전용 엔드포인트가 실행(주문하기) |
| **사람 전용** | HTTP 엔드포인트 | 사람이 누른 UI 버튼만 | 아래 표. **툴이 아니며 모델은 보지 못함** |
| **런타임 전용** | HTTP 엔드포인트 | 우리 에이전트의 런타임(변경셋), 백오피스 화면(데모 시계·다시 적재) | 변경셋 만들기·항목·제출, 데모 시계 읽기·옮기기, 데모 스토어 다시 적재. **툴이 아니며 모델은 보지 못함** |

### 2.1 사람 전용 엔드포인트

`human:*` 스코프는 사람이 직접 누른 요청에만 붙고 채팅 요청의 토큰에는 절대 붙지 않으므로, 어떤 프롬프트로도 에이전트가 이 일을 할 수 없습니다([../backend/gateway.md](../backend/gateway.md) §1–§2). 모두 세션 쿠키 + `X-CSRF-Token`이 필요하고, 프리뷰 없이 상태를 바꾸는 것은 `Idempotency-Key`를 받습니다. 에러의 `next_tool`은 항상 `null`입니다([errors.md](errors.md)). 웹은 `person` 버튼으로 이 엔드포인트를 직접 부릅니다([chat.md](chat.md) §3.2). 같은 목록이 [approval.yml](../../policies/approval.yml) `person_only`에 있습니다.

| 엔드포인트 | 본문 | 누가 | 스코프 | 무엇 | 명세 |
|---|---|---|---|---|---|
| `POST /v1/platform/change-sets/{id}/approve` | `{version}` | 운영자 | `human:approve` | 실행 시작. 최신 버전이 아니면 거부, 다시 계산이 다르면 `estimate_changed` | [platform.md](platform.md#사람-전용-엔드포인트) |
| `POST /v1/platform/change-sets/{id}/reject` | `{reason}` | 운영자 | `human:reject` | 변경셋 거절 | 〃 |
| `POST /v1/platform/change-sets/{id}/undo` | — | 운영자 | `human:undo` | 변경셋의 모든 변경을 역순으로 되돌림(기한 안) | 〃 |
| `POST /v1/platform/tasks/{task_id}/dismiss` | `{reason?}` | 운영자 | `human:task` | 할 일 치우기 | [platform.md](platform.md#task) |
| `POST /v1/sales/orders` | `{preview_id}` | 쇼퍼 | `human:order` | 주문하기(`prepare_order`·`prepare_gift_order`의 프리뷰) | [sales.md](sales.md) |
| `POST /v1/sales/subscriptions` | `{preview_id}` | 쇼퍼 | `human:order` | 정기 주문 시작(`prepare_subscription`의 프리뷰) | 〃 |
| `POST /v1/sales/gift-orders/{id}/accept` | `{delivery_address, size?, color?}` | 선물 받는 사람 | `human:order` | 선물 수락(가격은 못 바꿈) | 〃 |
| `POST /v1/fulfillment/orders/{order_id}/cancel` | `get_cancel_options` 결과 기반 | 쇼퍼 | `human:cancel` | 주문 전체·일부 취소 | [fulfillment.md](fulfillment.md#사람-전용-엔드포인트) |
| `POST /v1/fulfillment/returns` | `get_return_options` 결과 기반 | 쇼퍼 | `human:return` | 반품 요청 보내기 | 〃 |
| `POST /v1/fulfillment/exchanges` | `get_exchange_options` 결과 기반 | 쇼퍼 | `human:exchange` | 교환 요청 | 〃 |
| `POST /v1/fulfillment/claims/{claim_id}/pay-charge` | `{amount, payment_method_ref}` | 쇼퍼 | `human:claim` | 클레임 진행 전 추가 비용 결제 | 〃 |
| `POST /v1/fulfillment/claims/{claim_id}/withdraw` | `{reason_text?}` | 쇼퍼 | `human:claim` | 반품·교환 요청 철회 | 〃 |
| `POST /v1/customer/me/memories/{memory_id}/undo` | — | 쇼퍼 | `human:undo` | 기억한 선호 삭제 | [customer.md](customer.md#사람-전용-엔드포인트) |
| `POST /v1/customer/me/withdrawal` | 재인증 + `check_withdrawal_eligibility` 결과 | 회원 본인 | `human:account` | 탈퇴 | 〃 |
| `POST /v1/customer/password-reset/requests` | `{identifier}` | 누구나(익명 세션) | `human:account` | 비밀번호 재설정 요청(계정 유무와 무관하게 같은 응답) | 〃 |
| `POST /v1/customer/password-reset/confirm` | `{token, code?, new_password}` | 회원 본인 | `human:account` | 비밀번호 재설정 확정 | 〃 |
| `POST /v1/customer/account-recovery/login-id` | `{contact}` | 누구나(익명 세션) | `human:account` | 로그인 아이디 찾기 | 〃 |
| `PUT /v1/customer/me/password` | `{current_password, new_password}` | 회원 본인 | `human:account` | 로그인 상태에서 비밀번호 변경 | 〃 |

18개 모두 commerce에 있습니다(`cargo xtask check-coverage`). 웹에 버튼이 있는 것은 승인·거절·되돌리기, 할 일 치우기, 주문하기, 선물 수락, 주문 취소, 반품 요청, 기억 되돌리기의 아홉입니다. 정기 주문 시작, 교환 요청, 추가 비용 결제, 클레임 철회, 탈퇴, 비밀번호 재설정 요청·확정, 로그인 아이디 찾기, 비밀번호 변경의 아홉은 아직 웹 화면이 없습니다.

에이전트는 `get_*_options`·`prepare_*`로 준비만 하고, 보내는 것은 사람입니다. 서버는 보내는 순간 다시 계산해 사람이 본 금액과 다르면 `estimate_changed`로 거부합니다. 비밀번호는 어떤 툴도 다루지 않습니다.

### 2.2 런타임 전용 엔드포인트

우리 에이전트의 런타임(위임 주체 토큰, 스코프 `changeset:draft`)만 부릅니다. 모델은 쓰기 툴을 "무엇을 바꿀지"로만 부르고, 런타임이 프리뷰를 만들어 변경셋에 붙입니다([../backend/agent.md](../backend/agent.md) §5). 외부 에이전트(MCP)에는 열지 않습니다. 명세는 [platform.md](platform.md#런타임-엔드포인트).

| 엔드포인트 | 본문 | 결과 |
|---|---|---|
| `POST /v1/platform/change-sets` | `{title, request_text, interpretation[], conversation_id, task_id?}` | `{id, number, version: 1, state: "drafting"}` |
| `PATCH /v1/platform/change-sets/{id}` | `{title?, append_request_text?, interpretation?[]}` | 요청 문장 덧붙이기·해석 교체(대화로 고침) |
| `POST /v1/platform/change-sets/{id}/items` | `{preview_id, depends_on?: [preview_id]}` | 항목 붙이기 |
| `PUT /v1/platform/change-sets/{id}/items` | `{items: [{preview_id, depends_on?}]}` | 항목 교체(대화로 고침). `version+1`, `drafting` |
| `DELETE /v1/platform/change-sets/{id}/items/{preview_id}` | — | 항목 빼기 |
| `POST /v1/platform/change-sets/{id}/submit` | `{version}` | `waiting_for_approval` |

**데모 시계와 다시 적재.** 백오피스 화면(운영자 세션)이 부르는 런타임 전용 엔드포인트 셋입니다. 이것도 툴이 아니고, 채팅 턴의 토큰에는 `admin:*` 스코프가 없어 에이전트는 시간을 옮길 수 없습니다. 명세는 [platform.md](platform.md#데모-시계).

| 엔드포인트 | 본문 | 결과 |
|---|---|---|
| `GET /v1/platform/demo-clock` | — (`tools:read:platform`) | `{now, is_running, demo_mode}` |
| `PUT /v1/platform/demo-clock` | `{now, is_running?}` (`admin:demo`, 데모 모드에서만) | 그 사이 기한이 된 예약 작업을 순서대로 실행한 뒤 `{now, is_running, demo_mode, previous, ran[]}` |
| `POST /v1/platform/demo/reset` | — | 스토어를 비우고 데모 시드를 다시 적재(`admin:demo`, 데모 스택에서만). 모든 세션이 끝남 |

쓰기 프리뷰는 `?change_set_id=cs_…`로 바로 붙일 수도 있습니다. 내부 읽기 `GET /v1/platform/store`(스토어의 통화·시간대·현재 시각), `GET /v1/platform/previews/{preview_id}`와 이벤트 구독 `GET /v1/platform/events`(SSE: `change_set.updated`, `task.updated`, `job.updated`)는 [platform.md](platform.md#내부-엔드포인트).

## 3. 공통 규약 (요약)

전문은 [conventions.md](conventions.md), 에러는 [errors.md](errors.md), 값 목록은 [enums.md](enums.md)입니다.

### 3.1 이름과 값
- 타입은 schema.org 이름(PascalCase), 확장은 `acom:`. 필드는 schema.org 프로퍼티의 snake_case(`validFrom` → `valid_from`). enum 값은 snake_case 단어, 코드값 금지([conventions.md](conventions.md) §1).
- 식별자는 `{접두}_{접미}`(`prd_windbreaker`, `var_windbreaker_m_slate`, `cs_1042`, `pv_8f2c`). 엔티티 자신의 id는 `id`, 다른 엔티티를 가리키면 `<entity>_id`. 사람에게 보이는 번호는 따로(`order_number: "20402"`, 변경셋 `number: 1042`). 접두 표는 [conventions.md](conventions.md) §9.
- 금액은 `{"value": 71.20, "currency": "USD"}`(가격은 `price` + `price_currency`로 펼쳐도 됨). 툴 입력의 `percent_off`는 0–100, 정책 파일의 비율은 0–1. 시각은 RFC 3339 + 스토어 오프셋(`2026-11-07T00:00:00-05:00`), 기간은 `valid_from`~`valid_through` 둘 다 포함.
- 스토어(테넌트)는 내부 토큰의 `tenant` 클레임 하나로 정해집니다. 헤더는 [conventions.md](conventions.md) §10.

### 3.2 읽기

```jsonc
{"items": [...], "total": 42, "next_cursor": "c_…"}   // 목록. total은 필터를 적용한 전체 수, next_cursor는 다음 쪽이 있을 때만
```
`limit`(기본 20, 최대 100), `cursor`, `fields`, `sort`. 하나를 가져오는 `get_*`는 목록 봉투 없이 엔티티를 돌려주고, 없으면 `not_found`.

`fields`의 규칙(없는 이름은 `invalid_input`)은 [conventions.md](conventions.md) §6.

### 3.3 쓰기 (전선 위의 형식)

모든 쓰기는 두 단계입니다. 도메인 파일의 입력 표에는 이 단계를 위한 필드(`preview`, `preview_id`)가 없습니다. 모델이 보는 툴 스키마에도 없고, 우리 에이전트의 런타임이 처리합니다([conventions.md](conventions.md) §7–§8).

| 단계 | 요청 | 응답 |
|---|---|---|
| 프리뷰 | `POST {path}?preview=true` + 입력 (+ `&change_set_id=cs_…`, 선택) | 아래 프리뷰 |
| 실행 | `POST {path}` + `{"preview_id": "pv_…"}` (다른 필드 금지) | 아래 실행 결과 |

```jsonc
// 프리뷰 — 상태를 바꾸지 않음
{"preview_id": "pv_8f2c",
 "expires_at": "2026-11-06T11:35:00-05:00",          // 기본 30분(approval.yml preview_ttl_minutes). 변경셋에 붙으면 승인 때 다시 계산
 "change_set_id": "cs_1042",                          // 변경셋에 붙었을 때만
 "changes": [{"op": "update", "entity": "unit_price_specification", "id": null,
              "from": {"product_id": "prd_windbreaker", "price": 89.00, "price_currency": "USD", "price_type": "list_price"},
              "to":   {"product_id": "prd_windbreaker", "price": 71.20, "price_currency": "USD", "price_type": "sale_price", "percent_off": 20}}],
 "checks": [{"rule": "price_floor", "passed": true, "detail": "Overshirt moved to 15% to stay at cost + 15%", "fix_tool": null}],
 "needs_approval": true,
 "approval_reason": "Changes 40 prices"}

// 실행 — 같은 preview_id로 다시 오면 첫 결과
{"ok": true, "id": "promo_1042", "undo": {"by": "person", "target": "cs_1042", "until": "2026-11-09T23:59:00-05:00"}}

// 에러
{"error": {"code": "below_price_floor", "class": "policy_violation",
           "message": "At 20% the Seersucker overshirt would sell below cost + 15%.",
           "next_tool": "set_sale_prices", "detail": {"rule": "price_floor", "items": ["prd_seersucker_overshirt"]}}}
```

- `changes[].op`는 `create` \| `update` \| `delete` \| `schedule` \| `send` \| `grant` \| `revoke` \| `exclude`(대상에서 빠진 것, 실행하지 않고 기록만). `entity`는 데이터 모델의 엔티티 이름(가격은 `unit_price_specification` 또는 `offer`), `id`는 바뀌는 엔티티의 id이고 실행 전에 없으면 `null`([enums.md](enums.md) §4).
- `checks[].rule`은 정책 키(파일 이름을 뺀 키 경로, §6), `fix_tool`은 실패를 고칠 수 있는 툴 이름 또는 `null`.
- `needs_approval: true`인 프리뷰는 승인된 변경셋 안에서 실행기만 실행합니다. 단독 실행은 `approval_required`.
- `undo`는 항상 `{by: "person", target, until}`입니다. `target`은 변경셋 id(운영 쓰기) 또는 기억 id(`mem_…`, 쇼퍼 선호)이고, 되돌리기는 그 대상의 사람 전용 엔드포인트로만 합니다. 기한이 없으면 `until: null`. 되돌릴 수 없는 쓰기(환불, 고객 메시지, 주문)와 변경셋 밖의 쇼퍼 쓰기(장바구니)는 `"undo": null`. 툴로 되돌리는 경로는 없습니다.
- 공통 에러(오퍼레이션마다 적지 않음): `unauthenticated`, `forbidden`, `invalid_input`, `rate_limited`, `upstream_unavailable`, `internal`, 쓰기의 `preview_expired`, `approval_required`, `estimate_changed`([errors.md](errors.md) §3).

### 3.4 오퍼레이션 명세 형식

도메인 파일의 각 오퍼레이션은 `### \`tool\` — Kind: X` 제목 하나에 툴 하나이고, 다음 항목을 씁니다: 논리 주소/REST · 요약 · consumes → produces · 입력 · 출력 · 프리뷰(쓰기) · 정책 · 에러(`code` → 다음 툴 또는 `null`) · 멱등성. `cargo xtask check-docs`가 아래 인덱스와 도메인 파일의 제목·종류를 대조합니다.

## 4. 전체 오퍼레이션 인덱스

툴 **290개**(R 131 · W 119 · W$ 40), 도메인 13개. 그 밖에 사람 전용 엔드포인트 18개(§2.1)와 런타임 전용 엔드포인트 9개(§2.2: 변경셋 6개, 데모 시계 2개, 데모 스토어 다시 적재 1개). **굵게** 표시한 30개는 데모 시나리오가 쓰는 툴이고, 위치는 [architecture.md §3.4](../architecture.md)와 같습니다.

### Offer

**[catalog](catalog.md)**
| resource | operations |
|---|---|
| product_group | `search_product_groups` R · `get_product_group` R · `create_product_group` W · `update_product_group` W |
| product | **`get_product`** R · **`get_product_measurements`** R · `create_product` W · `update_product` W · `set_variants` W · `discontinue_product` W · `get_return_policy` R · `get_delivery_policy` R · `batch_import` W$ · `generate_image` W$ · `generate_copy` W |
| offer | `get_price_breakdown` R · `set_price` W · `schedule_price_change` W |
| inventory | `get_inventory` R · `set_variant_stock` W · `set_safety_stock` W |
| reorder | `create_reorder` W · `receive_reorder` W |
| approval | `submit_for_review` W · `approve_product` W$ · `reject_product` W · `get_change_history` R |
| brand | `list_brands` R · `get_brand` R · `create_brand` W · `update_brand` W |
| review | **`get_review_summary`** R · `list_reviews` R · `moderate_review` W · `create_review` W · `update_review` W · `delete_review` W · `vote_review_helpful` W · `report_review` W |
| question | `list_questions` R · `answer_question` W · `moderate_question` W · `ask_question` W · `update_question` W · `delete_question` W |
| bundle | `create_bundle` W |
| restock_alert | `subscribe_restock` W · `list_restock_subscriptions` R |

**[merchandising](merchandising.md)**
| resource | operations |
|---|---|
| category | `list_categories` R · `get_category` R · `create_category` W · `update_category` W · `map_category` W · `unmap_category` W |
| display_slot | **`list_display_slots`** R · `get_display_slot` R · `arrange_display_slot` W · `schedule_display` W$ |
| banner | **`schedule_banner`** W$ · `list_banners` R · `set_banner` W |
| popup | `list_popups` R · `set_popup` W |
| collection_page | `list_curations` R · `get_curation` R · `create_curation` W · `update_curation` W · `publish_curation` W$ |
| cache | `invalidate_display_cache` W |

**[search](search.md)**
| resource | operations |
|---|---|
| product | **`search_products`** R · **`compare_products`** R · `autocomplete` R · `get_facets` R |
| lexicon | `get_lexicon` R · `update_lexicon` W |
| index | `reindex` W · `get_index_status` R |

### Transact

**[sales](sales.md)**
| resource | operations |
|---|---|
| cart | **`get_cart`** R · **`add_to_cart`** W(승인 없음) · `update_cart_item` W · `remove_cart_item` W · `clear_cart` W |
| checkout | `get_payment_methods` R |
| order | **`prepare_order`** W(프리뷰만) · **`get_my_orders`** R · **`get_order`** R · `search_orders` R · `verify_guest` W · `change_shipping_address` W · 사람 전용 `POST /v1/sales/orders` |
| payment | `get_payment` R · `confirm_virtual_account_deposit` W$ |
| receipt | `issue_receipt` W · `get_receipt` R |
| subscription | `prepare_subscription` W(프리뷰만) · `pause_subscription` W · `resume_subscription` W · `cancel_subscription` W · `update_schedule` W · `list_subscriptions` R · 사람 전용 `POST /v1/sales/subscriptions` |
| installment | `set_installment` W |
| gift_order | `prepare_gift_order` W(프리뷰만) · `get_gift_order` R · 사람 전용 `POST /v1/sales/gift-orders/{id}/accept`(받는 사람) |

**[fulfillment](fulfillment.md)**
| resource | operations |
|---|---|
| claim | **`search_return_requests`** R · **`evaluate_return_request`** R · **`get_return_options`** R · `get_cancel_options` R · `get_exchange_options` R · `search_claims` R · `get_claim` R · `cancel_order_lines` W$(스토어가 시작하는 취소) · `approve_claim` W$ · `reject_claim` W · `convert_exchange_to_return` W · `upload_evidence` W · 사람 전용 `POST /v1/fulfillment/orders/{order_id}/cancel`, `POST /v1/fulfillment/returns`, `POST /v1/fulfillment/exchanges`, `POST /v1/fulfillment/claims/{claim_id}/pay-charge`, `POST /v1/fulfillment/claims/{claim_id}/withdraw` |
| inspection | `assess_return_condition` W |
| refund | **`refund_return`** W(승인은 `thresholds.refund`: $200.00 초과) · `get_refund` R · `retry_refund` W$ |
| parcel_delivery | **`get_delivery_dates`** R · `get_delivery` R · `track_shipment` R · `schedule_shipment` W · `confirm_shipment` W · `register_shortage` W |
| shipping_rate | `list_shipping_zones` R · `manage_shipping_zone` W · `set_shipping_rate` W |

### Engage

**[promotion](promotion.md)**
| resource | operations |
|---|---|
| promotion | **`list_active_promotions`** R · `search_promotions` R · `get_promotion` R · `create_promotion` W$ · `update_promotion` W · `pause_promotion` W · `end_promotion` W |
| offer | **`set_sale_prices`** W$ (회원·기간 한정 판매가) |
| coupon | **`create_coupon`** W$ · `issue_coupon` W$ · `redeem_coupon` W · `restore_coupon` W · `revoke_coupon` W · `search_coupons` R · `get_coupon` R · `list_member_coupons` R |
| benefit | `calculate_benefits` R (장바구니 혜택 계산의 단일 기준) |
| conditional_gift | `create_conditional_gift` W · `list_conditional_gifts` R |
| loyalty | `get_loyalty_balance` R · `list_loyalty_transactions` R · `grant_loyalty` W$ · `spend_loyalty` W · `expire_loyalty` W |
| event | `list_event_types` R · `list_reward_types` R · `search_events` R · `get_event` R · `create_event` W · `update_event` W · `enter_event` W · `list_my_entries` R · `run_draw` W$ · `get_draw_record` R · `dispatch_reward` W$ — 참여 방식·당첨 방식·보상 종류는 확장 가능한 레지스트리 |
| segment | `define_segment` W · `preview_segment` R · `list_segments` R |

**[customer](customer.md)**
| resource | operations |
|---|---|
| person | **`get_my_profile`** R · **`get_return_history`** R · `search_members` R · `get_member` R · `register_member` W · `update_member` W · `block_member` W$ · `unblock_member` W$ · `verify_identity` W · `check_availability` R · `verify_business` W$ · `check_withdrawal_eligibility` R · `send_password_reset` W$ · 사람 전용 `POST /v1/customer/me/withdrawal`, `POST /v1/customer/password-reset/requests`, `POST /v1/customer/password-reset/confirm`, `POST /v1/customer/account-recovery/login-id`, `PUT /v1/customer/me/password` |
| membership | **`list_tiers`** R · `get_tier` R · `get_member_tier` R · `define_tier_program` W$ · `preview_tier_evaluation` R · `run_tier_evaluation` W$ · `set_member_tier` W$ · `list_tier_history` R — 등급 수·조건·혜택은 스토어가 데이터로 정의 |
| address | `list_addresses` R · `add_address` W · `update_address` W · `set_primary_address` W |
| consent | `get_consent` R · `set_consent` W |
| preference | **`update_my_preferences`** W(승인 없음) · `get_preferences` R · 사람 전용 `POST /v1/customer/me/memories/{memory_id}/undo` |
| wishlist | `list_wishlist` R · `add_to_wishlist` W · `remove_from_wishlist` W |
| employee | `register_employee` W · `set_discount_limit` W |
| inquiry | `search_inquiries` R · `get_inquiry` R · `create_inquiry` W · `assign_inquiry` W · `respond_inquiry` W$ · `close_inquiry` W |
| remedy | `request_remedy` W · `approve_remedy` W$ · `pay_remedy` W$ |
| faq_notice | `list_faqs` R · `get_faq` R · `list_notices` R · `get_notice` R · `publish_notice` W |

**[messaging](messaging.md)**
| resource | operations |
|---|---|
| message | **`send_customer_message`** W$ · `send_bulk` W$ |
| template | `create_message_template` W · `list_message_templates` R |
| campaign | `create_campaign` W$ · `search_campaigns` R · `get_campaign` R |
| notification | `list_notifications` R |

**[community](community.md)**
| resource | operations |
|---|---|
| post | `list_posts` R · `get_post` R · `create_post` W · `update_post` W · `delete_post` W · `report_post` W |
| reply | `list_replies` R · `create_reply` W · `update_reply` W · `delete_reply` W |
| board | `list_boards` R · `configure_board` W |

### Operate

**[platform](platform.md)**
| resource | operations |
|---|---|
| policy | **`get_policy`** R · `list_policies` R |
| change_set | **`get_change_set`** R · `search_change_sets` R · **`verify_change_set`** R · 사람 전용 `POST /v1/platform/change-sets/{id}/approve`, `…/reject`, `…/undo` · 런타임 전용 만들기·항목·제출(§2.2) |
| task | `list_tasks` R · `get_task` R · 사람 전용 `POST /v1/platform/tasks/{task_id}/dismiss` |
| demo_clock | 런타임 전용 `GET`·`PUT /v1/platform/demo-clock`(§2.2) — 예약 작업([platform.md](platform.md#예약-작업))을 시각에 맞춰 실행 |
| action_log | `search_action_log` R · `get_action_log` R |
| schema | `describe_data` R (DB 데이터 사전) |
| enum | `list_enums` R |
| auth | `login` W · `refresh_token` W · `logout` W |
| tenant | `get_tenant` R · `list_tenants` R |
| role | `list_roles` R · `get_role` R · `create_role` W · `assign_role` W$ · `list_permissions` R |
| menu | `list_menus` R · `set_menu` W · `set_favorite` W |
| admin_user | `search_admin_users` R · `create_admin_user` W$ · `set_admin_status` W$ |
| org | `list_orgs` R · `set_org` W |
| api_key | `list_api_keys` R · `issue_api_key` W$ · `revoke_api_key` W$ |

**[analytics](analytics.md)** (읽기 전용)
| resource | operations |
|---|---|
| sales | **`explain_sales_change`** R · `get_sales_overview` R · **`get_sales_trend`** R |
| customer | `get_customer_overview` R |
| service | `get_cs_overview` R |

**[partner](partner.md)**
| resource | operations |
|---|---|
| seller | `search_sellers` R · `get_seller` R · `onboard_seller` W$ · `update_seller` W · `set_seller_status` W$ |
| contract | `get_contract` R · `set_contract` W$ |
| grade | `get_seller_grade` R · `set_seller_grade` W |
| dashboard | `get_seller_overview` R |

**[integration](integration.md)**
| resource | operations |
|---|---|
| product_import | `import_products` W$ · `get_import_status` R |
| order | `lookup_external_order` R · `sync_order_status` W |
| claim | `lookup_external_claim` R · `sync_claim_status` W |
| channel | `list_channels` R · `register_channel` W$ |
| event | `receive_carrier_event` W · `receive_payment_event` W (시스템 주체 전용, 모델의 툴 매니페스트에서 빠짐) |

## 5. 툴 그래프

각 오퍼레이션의 `consumes → produces`에서 생성됩니다. 데모 장면([../scenarios.md](../scenarios.md) §6)의 핵심 간선:

```
B1 VIP 주말 세일 (변경셋 #1042)
get_policy ──policy_key──▶ (계획)
search_products ──product_id[]/category_id──▶ set_sale_prices ──promotion_id──▶ create_coupon
list_tiers ──────tier_id──────▶ set_sale_prices, create_coupon, schedule_banner
list_active_promotions ──promotion_id[]──▶ (promotion_overlap 근거)
list_display_slots ──slot_id──▶ schedule_banner ◀──promotion_id── set_sale_prices
set_sale_prices, create_coupon, schedule_banner ──(런타임이 cs_1042에 붙임)──▶ get_change_set ──▶ (사람) approve ──▶ verify_change_set

B2 반품 검토 (변경셋 #1043)
list_tasks ──task_id──▶ search_return_requests ──claim_id[]──▶ evaluate_return_request ──▶ refund_return
                                     │                      └──customer_id──▶ get_return_history
                                     └──order_id──▶ get_order
evaluate_return_request ──claim_id──▶ send_customer_message(ask_for_photos | decline)

B3 매출 질문 (읽기만)
explain_sales_change ──(원인: 품절 상품 prd_ · 미발송 캠페인 cmp_)──▶ sales_explainer 블록의 다음 행동(각각 변경셋 요청)
get_sales_trend ──(일별 매출 14일)──▶ sales_explainer 블록의 차트

F1–F6 쇼퍼
get_my_profile ──▶ search_products ──product_id[]──▶ compare_products ──▶ update_my_preferences ──memory_id──▶ (사람) memories/{id}/undo
search_products ──product_id──▶ get_product_measurements, get_review_summary, get_delivery_dates
search_products ──product_id──▶ add_to_cart ──cart_id──▶ get_cart ──▶ prepare_order ──preview_id──▶ (사람) POST /v1/sales/orders
get_my_orders ──order_id──▶ get_return_options ──▶ (사람) POST /v1/fulfillment/returns ──claim_id──▶ (B2의 search_return_requests)
```

## 6. 정책과의 연결

쓰기 프리뷰의 `checks`는 정책 파일에서 옵니다. **check `rule`은 정책 키 그 자체**(파일 이름을 뺀 키 경로)이고, 실패한 체크가 실행 때 다시 실패하면 에러의 `detail.rule`에 같은 키가 들어갑니다([errors.md](errors.md) 규칙 3). 데모 스토어가 덮어쓰는 값은 [../scenarios.md](../scenarios.md) §5.

| check `rule` | 정책 파일 | 적용 오퍼레이션 | 실패 시 `fix_tool` |
|---|---|---|---|
| `max_discount` | [promotion.yml](../../policies/promotion.yml) | `set_sale_prices`, `create_promotion` | `set_sale_prices` |
| `price_floor` | [catalog.yml](../../policies/catalog.yml) | `set_sale_prices`, `create_promotion`, `set_price`, `schedule_price_change`, `import_products` | `set_sale_prices` / `set_price` |
| `promo_min_stock` | [promotion.yml](../../policies/promotion.yml) | `set_sale_prices` | `set_sale_prices`(`exclude_low_stock: true`) |
| `promotion_overlap` | [promotion.yml](../../policies/promotion.yml) | `set_sale_prices`, `create_promotion` | `list_active_promotions` |
| `member_benefits_per_order` (+ `coupon.stacking`) | [promotion.yml](../../policies/promotion.yml) | `create_coupon` | `create_coupon` |
| `slot_free` | [merchandising.yml](../../policies/merchandising.yml) | `schedule_banner`, `schedule_display` | `list_display_slots` |
| `refund.to`, `refund.coupon_deducted` | [fulfillment.yml](../../policies/fulfillment.yml) | `refund_return`, `approve_claim`, `retry_refund` | `refund_return` |
| `defect_refund`, `changed_mind_return_shipping`, `damage_claim_needs_photo`, `return_window_days`, `return_suggestions.flag_return_history` | [fulfillment.yml](../../policies/fulfillment.yml) | `evaluate_return_request`(제안의 `rule`), `reject_claim`, `send_customer_message` | — |
| `change_set.irreversible`, `customer_messages` | [approval.yml](../../policies/approval.yml) | `refund_return`, `send_customer_message`, `send_bulk` | — |
| `checkout_validation.*`(6개) | [order.yml](../../policies/order.yml) | `prepare_order`, `prepare_subscription`, `prepare_gift_order` | `get_payment_methods`, `get_product`, `get_cart` |
| `cart.max_lines`, `cart.no_mix` | [order.yml](../../policies/order.yml) | `add_to_cart` | `remove_cart_item` |
| `preference.about_shopper_only`, `preference.no_sensitive_data` | [customer.yml](../../policies/customer.yml) | `update_my_preferences` | — |
| `consent.marketing_gate` ([customer.yml](../../policies/customer.yml)), `messaging.quiet_hours`, `messaging.daily_send_cap` | [messaging.yml](../../policies/messaging.yml) | `send_bulk`, `create_campaign` | `get_consent`, `create_campaign` |
| 승인 판정 `thresholds.*`, `customer_messages`, `no_approval`, `preview_only` | [approval.yml](../../policies/approval.yml) | 모든 W·W$ (체크가 아니라 `needs_approval`·`approval_reason`) | (사람 승인) |
| 할 일 감시 `watchers.*` | [platform.yml](../../policies/platform.yml) | `list_tasks`, `get_task`(감시 작업이 만듦) | — |

## 7. 평가용 화면형 파사드

[../design.md](../design.md) §7의 대조 실험 **전용**입니다. 제품의 일부가 아니고, 툴 인덱스(§4)·MCP·에이전트의 보통 턴에는 나오지 않습니다. 같은 백엔드 위에 [../design.md](../design.md) §2의 패턴을 우리가 직접 쓴 일반적인 예로 재현합니다(`acom-facade`). 화면마다 실제 오퍼레이션을 레지스트리로 불러서, 두 툴 세트가 같은 데이터를 읽고 바꿉니다.

- **서빙:** 같은 엔드포인트에 `set=facade`를 붙입니다. 매니페스트 `GET /v1/tools?set=facade`, 호출 `POST /v1/tools/{screen}?set=facade` + `{"input": {...}}`. `set` 없이는 화면 이름을 찾지 못하고(`not_found`), `set=facade`로는 A-Commerce 툴을 찾지 못합니다.
- **문:** 데모 모드(`ACOM_DEMO_MODE`)에서만 답하고, 아니면 `demo_mode_off`. 백오피스·스토어프론트의 사람(과 그 사람을 대신하는 우리 에이전트)만 쓰고 외부 에이전트·시스템은 `forbidden`. 매니페스트와 호출은 보통 툴처럼 대상(audience)·도메인 스코프로 거르고, 화면이 부르는 실제 오퍼레이션마다 같은 검사를 다시 합니다. 행 수준 보안도 그대로입니다.
- **쓰기:** 레거시 백엔드처럼 바로 실행합니다. 프리뷰를 돌려주지 않고 변경셋·사람 승인이 없으며, 실제 오퍼레이션의 정책 체크도 적용하지 않습니다(액션 로그에는 `via`와 적용하지 않은 체크 수를 남김). 주문하기만은 예외로, 주문서(`getOrdShtInitData`)는 `prepare_order` 프리뷰를 만들 뿐이고 주문은 쇼퍼의 버튼(사람 전용)만 합니다. 같은 스토어를 쓰는 안전은 평가가 실행 사이마다 데모 스토어를 다시 적재(`POST /v1/platform/demo/reset`)해서 지킵니다.
- **턴:** 턴 요청의 `tool_set: "facade"`([chat.md](chat.md) §1)가 런타임을 이 매니페스트로 바꿉니다. 턴 기록의 `usage.tool_set`이 어느 세트를 썼는지 남깁니다.

| 화면 | 종류 | 실제 오퍼레이션 | 보여 주는 패턴 |
|---|---|---|---|
| `getPrdGridList` · `getPrdGridListCnt` | R | `search_products` | POST 읽기와 따로 부르는 개수(목록에는 합계 없음), 코드(`sellStatCd`) |
| `getPrdDtlInfo` · `getPrdDtlInfoV2` | R | `get_product` | 버전 중복(V2는 옵션·속성을 더함) |
| `getMbrGrdList` | R | `list_tiers` | 코드(`mbrGrdCd` `G01`…, `G00` = 모두) |
| `getPrmtTgtMbrList` · `getPrmtTgtMbrListCnt` | R | `search_members` | 축약 이름, 따로 부르는 개수 |
| `getPrmtGridList` · `getPrmtGridListCnt` | R | `search_promotions` | 코드(`prmtTpCd` `T01`–`T05`, `prmtStatCd`), 따로 부르는 개수 |
| `savePrmtList` | W | `set_sale_prices` · `pause_promotion` · `end_promotion` | 행 상태(`rowStat` I/U/D)가 있는 일괄 저장, 머릿속에만 있는 규칙(체크 없음) |
| `saveCpnList` | W | `create_coupon` | 일괄 저장, 코드(`cpnAplyTpCd`, `dupAplyCd`) |
| `getDispCornerList` · `saveDispBnrList` | R · W | `list_display_slots` · `schedule_banner` | 화면 단위, 일괄 저장 |
| `getClmGridList` · `getClmGridListCnt` | R | `search_return_requests` | 그리드 + 개수, 코드(`clmStatCd`, `clmRsnCd`) |
| `getRfndPopInitData` · `saveRfndProc` | R · W | `get_claim` + `get_order` · `refund_return` | 환불 하나가 그리드·팝업 초기화·저장으로 나뉨 |
| `saveCustMsgSend` | W | `send_customer_message` | 축약 이름, 코드(`msgTpCd`, `sndChnlCd`) |
| `getCartList` · `saveCartList` | R · W | `get_cart` · `add_to_cart` · `update_cart_item` · `remove_cart_item` | 일괄 저장 하나에 담기·바꾸기·빼기 |
| `getOrdShtInitData` | R(부작용 있음) | `get_cart` + `prepare_order` 프리뷰 | 안전한지 알 수 없는 POST 읽기 |
| `getSaleAnlsGridList` | R | `explain_sales_change` | 코드(`cmprBaseCd`), 그리드 한 줄 |

코드표(`crates/facade/src/codes.rs`)는 스키마와 설명 어디에도 없습니다. 설명은 화면 말투로 짧고("Promotion list save.") 정책 힌트가 없으며, `consumes`·`produces`·`checks`도 선언하지 않습니다. `cargo xtask check-docs`는 §4만 읽으므로 이 절은 대조 대상이 아닙니다.
