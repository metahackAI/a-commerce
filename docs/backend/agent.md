# agent 서비스 (에이전트 런타임)

사람의 말 한 마디를 툴 호출로 바꾸고, 결과를 UI 블록으로 돌려주는 런타임입니다. [../design.md](../design.md) §5(하네스)와 §6(UI 프로토콜)을 Rust로 구현합니다. 커머스 규칙은 하나도 갖지 않습니다. 규칙은 `commerce`가 강제하고, 에이전트는 그 결과(체크·에러·승인 필요)를 사람에게 보여 줍니다.

```
요청 ─► 라우터 ─► 툴 검색 ─► 계획 ─► 실행(읽기·프리뷰) ─► 변경셋 제출 ─► [사람 승인] ─► commerce 실행·검증
         │          │          │         │                                         │
     특화 에이전트  계층+그래프  Glimmer   id는 슬롯으로 전달                        의도 대조(체커)
                  +임베딩     (Spark)                                              실패 시 Spark 인계
                                                         └──────────────▶ UI 블록 + 짧은 답 (SSE)
```

관련 명세: 채팅 스트림·UI 블록 [../api/chat.md](../api/chat.md) · 툴 매니페스트·변경셋 API [commerce.md](commerce.md) §6

---

## 1. 턴 루프

`POST /chat/{surface}/turns` (`surface` = `bo` | `fo`) 하나가 턴 하나입니다. 응답은 SSE 스트림입니다.

1. **문맥 구성:** 대화 기록(최근 턴 + 요약), 이 대화에 등장한 엔티티 id, 열린 변경셋, 쇼퍼라면 보이는 기억(선호), 데모 시계의 현재 시각.
2. **라우팅:** 특화 에이전트 선택(§2). 같은 대화의 다음 턴은 이전 에이전트를 기본값으로 둡니다.
3. **툴 검색:** 후보 툴 5~8개(§3).
4. **계획:** 단계 목록(§4).
5. **실행:** 단계별 호출, 쓰기는 프리뷰와 변경셋으로(§5).
6. **확인:** 프리뷰 체크와 요청 의도를 대조(§6). 실패하면 계획 수정 또는 상위 모델 인계.
7. **답:** UI 블록 + 짧은 텍스트(§7). 턴 종료 이벤트에 사용량(모델 호출 수, 토큰, 시간, 비용).

턴마다 상한이 있습니다: 모델 호출 12회, 툴 호출 20회, 계획 수정 2회, 60초. 넘으면 지금까지의 결과와 함께 멈추고 이유를 말합니다.

UI 블록의 버튼이 보내는 의도(`ui_event`, 예: "M 사이즈 담기", "사진으로 보기")도 같은 턴 루프로 들어옵니다. 단, 사람 전용 동작(승인·주문·반품 요청·되돌리기)은 버튼이 게이트웨이를 거쳐 `commerce`를 직접 부르고 에이전트를 지나지 않습니다.

---

## 2. 라우터와 특화 에이전트

특화 에이전트는 코드가 아니라 **데이터**입니다(`agents/*.yml` + 지침 `agents/*.md`). 모두 같은 런타임·같은 툴 레이어·같은 정책 위에서 돌고, 다른 것은 툴 범위·지침·쓸 수 있는 블록·모델 선택입니다.

```yaml
# agents/promotion.yml
id: promotion
surface: bo
title: Promotion agent
description: Sales, member prices, coupons, and the banners that announce them.   # 라우터가 읽음
home_domains: [promotion, merchandising]       # 툴 검색 가중치
tool_scope:                                    # 이 범위 밖의 툴은 검색되지 않음
  domains: [promotion, merchandising, catalog, search, customer, platform]
  kinds: [read, write, write_approval]
blocks: [change_set, product_table, promotion_preview, action_log]
models: { worker: glimmer, planner: spark }
instructions: promotion.md
examples: promotion.examples.yml               # 툴 검색·계획용 몇 개의 예시 (설계 과업에서만 뽑음)
```

| 에이전트 | 면 | 홈 도메인 | MVP |
|---|---|---|---|
| `router` | 둘 다 | — (에이전트 선택만) | ✓ |
| `promotion` | bo | promotion, merchandising | ✓ |
| `orders` | bo | fulfillment, sales, messaging | ✓ |
| `catalog` | bo | catalog, merchandising | ✓ (B4: 공급사 시트 → 초안, `product_drafts` 블록) |
| `care` | bo | customer, messaging, community | |
| `insights` | bo | analytics (읽기만) | ✓ |
| `shopper` | fo | search, catalog(읽기), sales(장바구니·주문 준비), fulfillment(반품 옵션), customer(내 정보) | ✓ |

- **라우터**는 Glimmer에 에이전트 설명 목록과 요청을 주고 가이드 디코딩으로 `{agent, confidence}` 하나를 고르게 합니다. 확신이 낮으면 면(bo/fo)의 기본 에이전트로 갑니다. 그 전에 아래 라우팅 힌트를 보고, 다음으로 요청에 한 에이전트의 `route_when_said` 문구(소문자, 예: insights의 `why did sales`)만 들어 있으면 모델 없이 그 에이전트로 보냅니다. 문장을 말하지 않는 버튼(`ui_event`)은 대화의 에이전트가 이어 받습니다.
- **라우팅 힌트**(`agents/runtime.yml` `router.hints`): 요청에 든 말(대소문자 무시, 예 "supplier sheet")이나 첨부 종류(`sheet`)가 그 면의 에이전트 하나만 가리키면 모델을 부르지 않고 그 에이전트로 갑니다. 버튼 의도(`ui_event`)는 힌트를 쓰지 않습니다.
- **첨부**: 턴 요청의 `attachments[]{kind, upload_id}`의 `upload_id`는 가리킨 id처럼 툴의 `upload_id` 슬롯을 채웁니다(`batch_import`의 `acom:file`은 `x-slot: upload_id`).
- **한 요청이 여러 도메인에 걸치는 것은 정상입니다**(VIP 세일 = 회원가 + 쿠폰 + 배너). 특화 에이전트는 도메인 하나에 갇히지 않고 자기 `tool_scope` 안에서 홈 도메인을 우선할 뿐입니다.
- **에이전트끼리 넘길 때는 id를 넘깁니다**(요약 텍스트가 아니라). 넘김은 같은 턴의 계획에 다른 에이전트의 단계를 붙이는 것으로 구현합니다.

---

## 3. 툴 검색

모델은 전체 툴 목록을 보지 않습니다. 매니페스트(`GET /v1/tools`, 스코프로 이미 걸러짐)에서 후보를 고릅니다.

1. **범위:** 에이전트의 `tool_scope`와 호출자 스코프의 교집합. `tool_scope.always`의 툴은 말과 상관없이 늘 후보입니다(에이전트마다 대부분의 요청이 시작하는 읽기: 쇼퍼의 `search_products`·`get_my_profile`·`get_cart`·`get_my_orders`, 운영 에이전트의 `list_tasks` 등). 사람의 말("light jacket")은 툴 설명에 없는 말이라 BM25만으로는 상품 검색을 놓치기 때문입니다.
2. **점수:** 이름·설명·필드 설명의 BM25 + 설명 임베딩 유사도(pgvector, 매니페스트가 바뀔 때만 재계산) + 홈 도메인 가중치. 사람이 쓰는 말과 툴이 쓰는 말이 다른 경우는 `runtime.yml`의 `tool_search.related_words`(데이터)가 검색어를 넓힙니다("fit" → measurements, reviews, orders; 이어서 따라감). 맞는 말이 하나도 없으면 홈 도메인의 읽기부터 채웁니다.
3. **그래프 확장:** 상위 후보의 `consumes`를 생산하는 툴(앞 단계)과 `produces`를 소비하는 툴(뒤 단계)을 1홉 추가. 예: `set_sale_prices` → 앞에 `search_products`, `list_tiers`, 뒤에 `create_coupon`, `schedule_banner`.
4. **자르기:** 5~8개. 결과는 결정적(같은 입력 → 같은 후보)이고 턴 로그에 남깁니다.

툴 검색 자체는 모델 호출이 아닙니다. 요청이 모호할 때만 Glimmer가 검색어를 다시 쓰게 합니다.

---

## 4. 계획

플래너는 후보 툴과 문맥을 받아 **짧은 단계 목록**을 JSON으로 냅니다. 출력은 가이드 디코딩(vLLM `guided_json` / `response_format: json_schema`)으로 아래 스키마를 강제합니다.

```jsonc
{
  "interpretation": ["VIP 회원 대상", "여름 아우터 전체", "이번 주말(토·일)", "메인 배너 노출"],  // 변경셋의 '이렇게 이해했습니다'
  "steps": [
    {"id": "s1", "tool": "search_products", "args": {"category": "summer_outerwear"}},
    {"id": "s2", "tool": "list_tiers", "args": {}},
    {"id": "s3", "tool": "set_sale_prices", "args": {"percent_off": 20, "valid_from": "2026-11-07T00:00", "valid_through": "2026-11-08T23:59"},
     "uses": ["s1", "s2"]},
    {"id": "s4", "tool": "create_coupon", "args": {"name": "VIP weekend"}, "uses": ["s3"]},
    {"id": "s5", "tool": "schedule_banner", "args": {"slot": "home_hero"}, "uses": ["s3"]}
  ],
  "reply_plan": "change_set"     // 끝에 보여 줄 블록
}
```

- **id는 모델이 옮기지 않습니다.** `uses`로 앞 단계를 가리키면 실행기가 툴 메타데이터의 `produces`/`consumes` 슬롯 이름(`product_id[]`, `tier_id`, `promotion_id`)으로 값을 채웁니다. 42개 상품 id를 모델이 베껴 쓰다 틀리는 일이 구조적으로 없습니다. `uses`를 적은 단계는 그 단계들(과 사람이 가리킨 것)에서만 id를 받고, 없으면 추측하지 않고 인자를 되묻습니다(F3: 주문을 읽은 뒤에 가진 windbreaker의 id를 채움).
- **행마다 실행(`each`, `when`):** `"each": "s1"`은 그 단계 결과의 행마다 한 번씩 실행하고, 실행마다 그 행의 같은 이름 필드(`claim_id`, `customer_id`, `order_id`)로 슬롯을 채웁니다. `"when": {"suggested": "refund"}`는 값이 같은 행만 고릅니다. B2: 요청 12건 평가는 `evaluate_return_request` 한 단계, 환불 9건·메시지 3건은 세 단계.
- **앞서 보인 블록을 단계처럼:** 대화 기록은 블록을 id와 함께 보여 줍니다(`[showed refund_review blk_31]`). `uses`·`each`에 `blk_…`를 쓰면 그 블록의 행(`requests`, `items`, `picks`)이 결과처럼 쓰여 다시 읽지 않습니다(B2의 변경셋, F4의 선물 고르기).
- **블록 문구(`notes`)와 출처(`source`):** 모델은 블록 요소에 붙일 짧은 말(`{"element": "size:L", "text": "Over a hoodie"}`, 첫째가 추천)과 이해한 줄의 출처(`said`·`profile`·`his`·`hers`, 칩)를 냅니다. 둘 다 툴 결과나 사람의 말에 없는 숫자가 들면 쓰지 않습니다.
- **모델 선택:** 단계 ≤ 3이고 한 도메인이면 Glimmer, 아니면 Spark. 에이전트 정의의 `models`로 바꿀 수 있습니다. 모든 결정은 턴 로그에 남아 평가에서 비교됩니다.
- **계획 수정:** 읽기 결과가 다음 인자를 바꿔야 할 때(예: 하한을 넘는 상품 2개는 15%로) 플래너를 한 번 더 부릅니다. 턴당 2회까지.
- **빈 계획:** 단계가 없는 계획은 아무것도 읽지 않고 답하게 됩니다. 버튼(`ui_event`)이 아닌 요청에서 단계가 없으면 그 이유를 적어 플래너를 한 번 더 부르고, 다시 계획한 쪽에 단계가 있으면 그것을 씁니다(인사·감사는 단계 없이 끝날 수 있음).

---

## 5. 실행

실행기는 계획의 단계를 순서대로(독립 단계는 동시에) 돌립니다.

| 단계 종류 | 실행기가 하는 일 |
|---|---|
| 읽기 | 인자 보정(§8.3) → `GET`. 결과를 단계 출력과 슬롯 값으로 저장 |
| 쓰기, 승인 불필요(장바구니 담기, 선호 기억) | 프리뷰 → 체크 모두 통과면 `preview_id`로 바로 실행 → 결과 |
| 쓰기, 승인 필요 | 열린 변경셋이 없으면 생성(요청 원문 + `interpretation`) → `change_set_id`를 붙여 프리뷰 → 체크 실패면 §6 → 모든 항목이 준비되면 `submit` |
| 주문 준비(`prepare_order`) | 프리뷰만. 결과로 `checkout_card` 블록을 만들고, 주문하기는 쇼퍼의 버튼이 함 |

- 에이전트는 **승인이 필요한 쓰기를 실행할 수 없습니다.** 승인 후 실행은 `commerce` 실행기의 일이고, 에이전트는 `GET /v1/platform/events`로 `running → verifying → live`를 받아 대화에 흘립니다.
- 대화로 고치면("오버셔츠는 15%로") 해당 항목만 다시 프리뷰해 교체하고 변경셋 버전이 오릅니다(v1 → v2).
- 툴 에러는 그대로 모델에 돌려주지 않고, `next_tool`이 후보에 있으면 그 툴로 계획을 고치게 하고, 없으면 사람에게 이유를 보여 줍니다.

---

## 6. 확인 (체커)

두 층입니다.

1. **규칙 확인 = commerce.** 프리뷰 `checks`, 실행 후 `verify_change_set`. 에이전트는 결과를 해석만 합니다.
2. **의도 확인 = agent.** 계획의 `interpretation`과 프리뷰 `changes`를 대조합니다. 대상 수, 할인율, 기간, 대상 등급, 노출 위치처럼 요청에서 뽑은 값이 프리뷰에 그대로 있는지를 **결정적으로** 비교하고(모델 판단 아님), 다르면 계획을 고칩니다. 두 번 실패하면 Spark가 이어받고, 그래도 안 되면 사람에게 차이를 보여 줍니다.

체크가 실패한 상태로 변경셋을 제출할 수 있는 경우는 하나뿐입니다: 사람이 판단해야 하는 실패(예: "재고 10개 미만 3개 상품 제외됨")가 `detail`로 명확히 보일 때. 이것은 정책의 `on_violation: warn`으로 정합니다.

---

## 7. 답: UI 블록

에이전트는 HTML을 쓰지 않습니다. 고정된 블록 라이브러리([../api/chat.md](../api/chat.md) §3)에서 골라 **툴 출력으로** 채웁니다.

- 블록 데이터는 Rust 타입(`acom-agent::blocks`)이고, 툴 출력에서 변환하는 함수가 블록마다 있습니다. 모델은 어떤 블록을 쓸지와 짧은 문구(`title`, `why`)만 정합니다. **숫자는 모델이 쓰지 않습니다.**
- 블록마다 `actions[]`가 있습니다. `say`(대화로 돌아오는 의도), `person`(사람 전용 엔드포인트, 웹이 직접 호출), `open`(다른 블록·화면 열기).
- 쇼퍼 블록의 형태는 기억된 선호를 따릅니다(스펙을 비교하는 쇼퍼에게 `compare_table`, 사진으로 보는 쇼퍼에게 `look_gallery`). 형태를 바꾼 이유는 `why`에 한 줄로 씁니다.
- 텍스트 답은 짧게(두세 문장). 툴 결과에 없는 사실을 말하지 않도록, 텍스트에 나온 숫자·상품명을 이번 턴의 툴 출력과 대조하는 후처리 검사를 둡니다. 결과에 없는 숫자가 있으면 그 숫자를 알려 주고 한 번 다시 쓰게 하고, 그래도 있으면 결정적인 짧은 답("Here is what I found.")을 씁니다.
- **읽은 것은 늘 보입니다:** 계획의 블록을 결과로 만들 수 없거나 계획이 `text`를 골랐고 다른 블록도 없으면, 마지막으로 읽은 목록을 그대로 보여 줍니다(상품이면 `product_list`, 그 밖은 `product_table`, 에이전트가 쓸 수 있는 블록일 때). 무엇을 읽었는데 "찾지 못했다"고 답하는 일은 없습니다.
- **요소 문맥:** 턴 요청의 `context[]`(사람이 고른 화면 요소의 `element_ref`·`ids`)는 계획의 첫 입력입니다. "이것", "이 상품"은 `context`의 id로 풀고, 슬롯 값으로 바로 씁니다.
- **추천 말:** 블록을 만들 때 요소마다 다음에 할 만한 말(`suggested_intents`)을 붙입니다. 실패한 체크에는 `fix_tool`에서 나온 수정 문장, 변경 행에는 "빼 줘"·"20%로" 같은 문장입니다.
- **화면 가리키기:** 설명이 특정 요소에 관한 것이면 `ui.command`(`highlight`, `scroll_to`)를 함께 보냅니다. `person` 버튼은 가리킬 수 없습니다.

---

## 8. 모델 클라이언트 (`acom-llm`)

### 8.1 제공자

| 모델 | 역할 | 연결 |
|---|---|---|
| Muse Glimmer 30B | 라우팅·툴 선택·인자 채우기·단순 계획·짧은 답 | self-hosted vLLM, OpenAI 호환 (`GLIMMER_BASE_URL`) |
| Muse Spark | 여러 단계 계획, 체크 실패 인계 | Meta Model API (`META_API_KEY`) |
| Muse Image, SAM 3.1 | 상품·배너 이미지, 누끼 | Meta Model API (catalog의 `generate_image` 오퍼레이션이 호출, 에이전트가 직접 부르지 않음) |
| 임베딩 | 툴 검색·상품 검색 | 설정된 임베딩 엔드포인트 |

`ChatModel` 트레이트 하나 뒤에 제공자 구현을 둡니다. 모델 이름·엔드포인트·가격표는 `agents/models.yml`에 있고, 에이전트 정의는 `glimmer`/`spark` 같은 별칭만 씁니다.

### 8.2 구조화 출력

라우팅·계획·인자는 모두 JSON Schema 기반 가이드 디코딩으로 받습니다. 제공자가 가이드 디코딩을 못 하면 네이티브 툴콜, 그것도 안 되면 텍스트에서 JSON을 추출합니다(코드 블록, `<tool_call>` 태그, `name(args)` 형태). 이 파서들은 `acom-llm::parse`에 있고 픽스처 테스트를 가집니다.

### 8.3 인자 보정

모델이 낸 인자를 툴 스키마에 맞추는 결정적 단계입니다. 스키마가 닫혀 있고(추가 필드 금지, `$ref`·`anyOf` 없음) 필드마다 설명·예시·별칭이 있어서 가능합니다.

| 입력 | 보정 |
|---|---|
| `"20%"`, `"20"` → `percent_off: number` | `20` |
| `"VIP members"`, `"vips"` → enum `vip` | 계약의 `x-aliases`로 대응, 없으면 거부 |
| `"Saturday"`, `"2026-11-07"` → `valid_from` | 스토어 시간대·데모 시계 기준 날짜시각 |
| `"$89"` → Money | `{"value": 89, "currency": "USD"}` (스토어 통화) |
| 모르는 필드 | 버리고 단계 로그에 기록 |
| 필수 필드 누락 | `uses`로 채울 수 있으면 채움, 아니면 모델에 한 번 되물음 |

보정 후에도 검증에 실패하면 에러 문장을 붙여 같은 모델에 한 번 더, 그다음은 Spark로 넘깁니다.

### 8.4 기록

모든 모델 호출을 `agent.model_call`(모델, 목적, 입력·출력 토큰, 지연, 비용, 성공 여부, 실패 이유: 쓸 수 없는 계획, 결과에 없는 숫자)에 남깁니다. 대화의 주인은 `GET /api/chat/{surface}/conversations/{id}?log=true`로 턴마다 단계(입력·출력·에러)와 모델 호출을 봅니다. 턴(`agent.turn`)과 단계(`agent.step`)는 `request_id`로 `commerce`의 액션 로그와 이어져, 한 요청의 모든 모델 호출과 툴 호출을 한 줄로 볼 수 있습니다. 평가 지표(완료율, 툴 선택 정확도, 과업당 호출·비용, 반복 일관성)는 이 기록에서 계산합니다.

---

## 9. 기억

- 쇼퍼의 기억은 `customer` 도메인의 선호(`update_my_preferences`)이고, 에이전트는 따로 숨긴 기억을 두지 않습니다. 기억이 생기면 `memory_note` 블록으로 한 줄과 되돌리기 버튼(사람 전용)을 보여 줍니다.
- 다른 사람을 위한 요청(선물)은 선호로 저장하지 않습니다. 계획 스키마에 `about: self | other`가 있고, `other`면 선호 저장 단계를 만들 수 없습니다.
- 대화 기록은 `agent` 스키마에 있고, 보존 기간은 정책(`customer.yml`)을 따릅니다.

---

## 10. 먼저 일하는 에이전트

운영자 홈의 할 일 목록(반품 대기, 품절 임박, 원가 이하 판매 등)은 에이전트가 만드는 것이 아닙니다. `commerce`의 감시 작업이 정책 기준으로 **할 일**(`platform.task`)을 만들고, 에이전트는 운영자가 그 항목을 열 때 항목의 요청 문장으로 새 턴을 시작합니다. 감지는 결정적이고, 처리는 대화입니다.

---

## 11. 크레이트 구성

```
acom-agent/src/
  runtime.rs     턴 루프, 상한, 취소
  router.rs      특화 에이전트 선택
  profile.rs     agents/*.yml 로드·검증
  toolsearch.rs  BM25 + 임베딩 + 그래프 확장
  planner.rs     계획 스키마, 플래너 호출, 계획 수정
  executor.rs    단계 실행, 슬롯 바인딩, 프리뷰·변경셋 처리
  checker.rs     의도 대조
  blocks/        블록 타입과 툴 출력 → 블록 변환
  memory.rs      대화 문맥, 기억 블록
  client.rs      ToolClient 트레이트 (HTTP 구현 / 테스트용 인프로세스 구현)
  stream.rs      SSE 이벤트
  store.rs       agent 스키마 (대화·턴·단계·모델 호출)
```
