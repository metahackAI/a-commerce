# 채팅 스트림과 UI 블록

사람과 에이전트 사이의 계약입니다. 웹(Next.js)은 이 문서만 보고 대화 화면을 만들 수 있어야 하고, 에이전트 런타임([../backend/agent.md](../backend/agent.md))은 이 형식만 내보냅니다.

---

## 1. 턴 요청

```
POST /api/chat/{surface}/turns        surface = bo | fo
Content-Type: application/json
Accept: text/event-stream
```

```jsonc
{
  "conversation_id": "conv_01jc8...",     // 없으면 새 대화
  "message": "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page.",
  "attachments": [{"kind": "image", "upload_id": "upl_..."}],   // 선택. 업로드는 POST /api/uploads
  "ui_event": null                          // 블록 버튼이 보낸 의도일 때 (아래)
}
```

블록의 `say` 버튼은 `message` 대신 `ui_event`를 보냅니다.

```json
{"conversation_id": "conv_01jc8...", "ui_event": {"block_id": "blk_31", "action_id": "add_m", "args": {"product_id": "prd_seersucker_overshirt", "size": "M", "color": "clay"}}}
```

사람이 화면의 요소를 골라 물으면(웹의 "이것에 대해 묻기", [../frontend/README.md](../frontend/README.md) §3.2) `message`와 함께 `context`가 실립니다. 에이전트는 "이것"을 추측하지 않고 `context`를 씁니다.

```json
{"conversation_id": "conv_01jc8...", "message": "Why is this one only 15%?",
 "context": [{"element_ref": "blk_12#change_row:prd_seersucker_overshirt", "kind": "change_row",
              "label": "Seersucker overshirt · 15% · $67.15",
              "ids": {"product_id": "prd_seersucker_overshirt", "change_set_id": "cs_1042"}}]}
```

`GET /api/chat/{surface}/conversations/{id}`는 지난 턴과 블록을 돌려줍니다(새로고침·다른 기기).

## 2. 이벤트 스트림

SSE입니다. 각 이벤트는 `id:`(턴 안 순번), `event:`(이름), `data:`(JSON 한 줄)를 가집니다. 연결이 끊기면 `Last-Event-ID`로 이어 받습니다.

| event | data | 뜻 |
|---|---|---|
| `turn.started` | `{turn_id, conversation_id, agent}` | 턴 시작. `agent`는 라우터가 고른 특화 에이전트 |
| `agent.step` | `{step_id, kind, summary, tool?, status}` | 진행 표시. `kind`: `route` · `search_tools` · `plan` · `call_tool` · `check`. `status`: `running` · `done` · `failed` |
| `message.delta` | `{text}` | 답 텍스트 조각 |
| `message.done` | `{message_id, text}` | 답 텍스트 끝 |
| `ui.block` | 블록(§3) | 블록 추가. 같은 `block_id`가 다시 오면 교체 |
| `memory.saved` | `{memory_id, text}` | 기억이 생김. 웹은 한 줄과 되돌리기 버튼을 보여 줌 |
| `ui.command` | `{command, target, note?}` | 에이전트가 화면을 가리킴. `command`: `highlight` · `scroll_to` · `open` · `focus_input`. `target`은 `element_ref`, 블록 id, 또는 경로. `person` 버튼은 대상이 될 수 없음 |
| `change_set.updated` | `{change_set_id, version, state}` | 변경셋 상태 변화(승인 뒤 실행·검증 포함, 턴 밖에서도 대화 구독으로 옴) |
| `error` | `{code, message}` | 턴을 끝내는 오류. 코드는 [errors.md](errors.md) |
| `turn.done` | `{turn_id, usage: {model_calls, tool_calls, input_tokens, output_tokens, ms, cost_usd}, models: [...]}` | 턴 끝. 화면의 "9 tool calls · Muse Glimmer 30B · 3.8s" |

턴 밖의 변화(승인 뒤 실행, 예약 작업)는 `GET /api/chat/{surface}/conversations/{id}/events`(SSE) 구독으로 받습니다.

## 3. UI 블록

### 3.1 공통 형식

```jsonc
{
  "block_id": "blk_31",
  "ui": "compare_table",          // §3.3의 이름 중 하나
  "schema": 1,                    // 블록 데이터 형식 버전
  "title": "Packs smallest · 3 compared",
  "why": "Shown as a table because you compare specs. Say \"show photos\" to change it.",   // 선택
  "data": { },                    // 블록별 형식 (§3.3). 숫자는 모두 툴 출력에서 옴
  "actions": [ ],                 // §3.2
  "suggested_intents": [          // 선택. 요소별 추천 말 (웹이 그 요소 옆에 보여 줌)
    {"element": "change_row:prd_seersucker_overshirt", "say": "Make this 20%"}
  ]
}
```

블록 안의 의미 있는 요소(행, 가격, 체크, 칩, 카드)는 `element_ref` = `<block_id>#<kind>:<id>`로 가리킵니다. 요소 종류(`kind`)는 [../frontend/README.md](../frontend/README.md) §4의 표와 같습니다.

### 3.2 버튼(actions)

| kind | 필드 | 누르면 |
|---|---|---|
| `say` | `action_id`, `label`, `args` | 웹이 `ui_event`로 새 턴을 보냄. 에이전트가 처리 |
| `person` | `action_id`, `label`, `method`, `path`, `body`, `confirm` | 웹이 **게이트웨이를 거쳐 commerce의 사람 전용 엔드포인트를 직접** 부름. 에이전트를 지나지 않음 |
| `open` | `action_id`, `label`, `target` | 다른 블록이나 화면을 엶 (`block:blk_12`, `/bo/change-sets/cs_1042`) |

`person` 버튼만 상태를 바꿉니다. 승인, 거절, 되돌리기, 주문하기, 반품·취소·교환 요청 보내기, 기억 되돌리기가 여기에 해당하고 목록은 [README.md](README.md) §2의 사람 전용 엔드포인트와 같습니다.

### 3.3 블록 라이브러리

이름은 시안([../../design](../../design))을 따릅니다. [../design.md](../design.md) §6의 초기 이름과의 대응은 오른쪽 열에 적었습니다.

**백오피스**

| ui | 쓰는 곳 | data 요지 | 버튼 | design.md §6 |
|---|---|---|---|---|
| `change_set` | 쓰기 요청의 결과 | `change_set_id`, `version`, `state`, `request`(원문들), `understood{who, products, benefit, when, where}`, `checks[]`, `groups[]{op, label, summary, rows[], more}`, `left_out[]`, `preview{segments[], selected, content}`, `after_approval[]`, `undo{until}` | `person` 승인(`{version}`), `person` 거절, `say` 수정 요청, `open` 이전 버전 비교 | `promotion_preview` + `approval_card` |
| `refund_review` | 반품·환불 일괄 판단 | `summary{waiting, refund{count, amount}, ask_photos, decline}`, `requests[]{order_number, item, amount, reason, evidence, day, suggested, because}`, `checks[]`, `messages[]`, `change_set_id` | `person` 승인("Approve 9 refunds and send 3 messages"), `say` 메시지 수정 | `order_detail` |
| `product_drafts` | 상품 초안 작성 진행 | `progress{total, ready, needs_you, working}`, `drafts[]{name, source_code, cost, price, sizes, status, note}`, `rules[]`, `checks[]` | `say` 일시 정지, `person` 준비된 초안 저장 | `product_table` |
| `sales_explainer` | 매출 질문(읽기만) | `summary{revenue, baseline, difference, orders}`, `series[]{date, revenue}`, `causes[]{cause, evidence, estimated}`, `next[]{label, request}` | `say` 다음 행동 각각 | — |
| `task_list` | 운영자 홈(`list_tasks` 결과 그대로) | `groups[]{state: needs_you \| working \| done \| dismissed, items[]{task_id, title, origin, at, summary, tag, change_set_id?, watcher?, updated_at?}}`, `total`, `agents[]{name, stats}`(선택) | `open` 항목, `say` 더 보기. 치우기·되돌리기는 할 일·변경셋 화면의 `person` 버튼 | — |
| `product_table` | 상품 목록 읽기 | `columns[]`, `rows[]`, `total` | `say` 더 보기·거르기 | `product_table` |
| `action_log` | 감사 기록 | `entries[]{at, actor, action_type, tool, object, status, result}` | — | `action_log` |

**스토어프론트**

| ui | 쓰는 곳 | data 요지 | 버튼 | design.md §6 |
|---|---|---|---|---|
| `looking_for` | 대화에서 이해한 조건 칩 | `chips[]{label, source: said \| profile \| his \| hers, removable}` | `say` 칩 제거 | — |
| `compare_table` | 스펙 비교 | `products[]{product_id, name, color, material, badge?}`, `rows[]{label, values[]}`, `prices[]{price, list_price, label}`, `sorted_by` | `say` 담기(사이즈), `say` "show photos" | `product_compare` |
| `look_gallery` | 사진 위주 | `items[]{product_id, name, price, list_price?, image, caption}` | `say` 자세히·비교 | `product_gallery` |
| `product_list` | 글 위주 목록 | `items[]{product_id, name, price, facts[]}` | `say` 좁히기 | `product_table` |
| `fit_check` | 사이즈 판단 | `product{...}`, `options[]{label, size, reason, price}`, `measurements{columns[], rows[]}`, `reviews{sample, fit[], mentions[]}`, `used[]`, `not_used[]` | `say` 사이즈 담기 | — |
| `gift_picker` | 선물 | `picks[]{product_id, name, price, list_price?, why, chosen_size?}`, `gift_options{wrap, card_message, receipt, exchange_until}`, `delivery{to, arrives, address_saved: false}`, `total` | `say` 고르기, `open` 주문 확인 | — |
| `cart` | 장바구니 | `lines[]{product_id, name, color, size, quantity, note}`, `subtotal`, `discounts[]{label, amount}`, `shipping`, `total`, `delivery{address_label, arrives}`, `payment{label}` | `say` 수량·삭제, `open` 주문 확인 | `cart` |
| `checkout_card` | 주문 확인(`prepare_order` 결과) | `preview_id`, `expires_at`, `lines[]`, `subtotal`, `discounts[]`, `shipping`, `total`, `delivery`, `payment`, `note: "Only you can place it."` | **`person` 주문하기** (`POST /v1/sales/orders {preview_id}`) | `checkout_card` |
| `return_options` | 반품·교환 선택 | `order{order_number, item, delivered_at, photos}`, `options[]{kind: replacement \| refund, facts[], send_back{pickup, drop_off}}`, `selected`, `steps[]` (`get_return_options` 출력과 같은 모양) | **`person` 요청 보내기** (`POST /v1/fulfillment/returns`) | `order_detail` |
| `order_detail` | 주문 조회 | `order_number`, `status`, `lines[]`, `delivery`, `payment`, `totals` | `say` 반품·교환 문의 | `order_detail` |
| `memory_note` | 기억 한 줄 | `memory_id`, `text` | **`person` 되돌리기** | — |
| `profile_card` | "What Northwind knows about you" | `facts[]{key, value, source}` | `say` 수정, `person` 삭제 | — |

블록마다 Rust 타입(`acom-agent::blocks`)과 JSON Schema(`ui-blocks.schema.json`, `cargo xtask codegen`이 생성)가 있고, 웹 컴포넌트는 그 스키마에서 만든 TS 타입을 씁니다.

### 3.4 형태 고르기

쇼퍼 블록의 형태는 기억된 선호와 대화에서 정합니다. 규칙은 데이터(`agents/shopper.yml`의 `layout_rules`)에 둡니다.

| 신호 | 블록 |
|---|---|
| 프로필 `compares_specs`, 또는 "compare", "which is lighter" | `compare_table` |
| 기록 없음(첫 방문), 또는 "photos", "less text" | `look_gallery` |
| 그 밖 | `product_list` |

형태를 바꾼 이유는 `why`에 한 줄로 쓰고, 사람이 말로 언제든 바꿀 수 있습니다.
