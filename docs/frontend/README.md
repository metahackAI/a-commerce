# 프런트엔드 (Next.js)

A-Commerce의 웹은 **화면 하나하나가 AI와 함께 움직이는 화면**입니다. 대화 창이 따로 붙은 화면이 아니라, 가격 하나, 체크 한 줄, 장바구니 줄 하나까지 "이것에 대해 묻기", "이것을 이렇게 바꿔 줘"를 받을 수 있고, 에이전트가 화면의 어느 요소를 가리키는지 사람이 볼 수 있습니다. 상태를 바꾸는 사람의 결정(승인, 주문하기, 반품 요청 보내기, 되돌리기)만은 사람이 직접 누릅니다.

- UI 라이브러리와 모양 규칙(Meta Astryx, 테마, 템플릿 우선): [../frontend-ui.md](../frontend-ui.md). 이 문서는 그 위에 얹는 **AI 층**입니다.
- 사람 ↔ 에이전트 계약(SSE 이벤트, UI 블록, 버튼 종류): [../api/chat.md](../api/chat.md)
- 화면 근거: [../../design/](../../design) 시안, 장면: [../scenarios.md](../scenarios.md)
- 백엔드 접점: 게이트웨이 하나(`/api/*`), [../backend/gateway.md](../backend/gateway.md)

---

## 1. 원칙

1. **모든 요소가 AI의 대상이 됩니다.** 화면의 의미 있는 요소(블록, 표의 행, 가격, 체크, 칩, 상품 카드, 입력 칸)는 자기가 무엇인지(엔티티 id, schema.org 용어, 블록 id)를 등록합니다. 사람은 그 요소를 골라 묻거나 고치라고 말할 수 있고, 에이전트는 그 요소를 가리킬 수 있습니다(§3).
2. **화면은 고정 라이브러리로만 그립니다.** 에이전트는 HTML을 쓰지 않고 블록 이름과 데이터를 보냅니다. 웹은 블록마다 컴포넌트 하나를 두고, 그 컴포넌트는 Meta Astryx 부품으로 만듭니다([../frontend-ui.md](../frontend-ui.md) §4).
3. **같은 계약.** API·블록·에러·값 목록의 TypeScript 타입은 `docs/api/`의 명세를 보고 손으로 씁니다(`web/src/ai/types.ts`, 블록 옆의 `web/src/components/blocks/`, `web/src/app/{bo,(storefront)}/_lib/types.ts`). Rust에서 타입을 생성하는 단계는 아직 없습니다.
4. **사람의 결정은 사람의 손으로.** `person` 버튼만 상태를 바꾸고, 게이트웨이를 거쳐 commerce를 직접 부릅니다. 에이전트가 보내는 화면 명령(§3.3)은 강조·스크롤·열기만 할 수 있고 `person` 버튼을 누를 수 없습니다.
5. **설명이 기본.** 블록의 `why`, 체크의 `detail`, "What I used" 같은 근거를 항상 보여 줍니다. 숫자는 툴 결과에서 오고 화면이 계산하지 않습니다(예외: 표시 형식).
6. **외부 에이전트도 읽는 화면.** 스토어프론트 페이지는 schema.org JSON-LD(Product, Offer, `potentialAction`)를 내보내고, 요소에는 schema.org 용어가 담긴 `data-` 속성을 답니다.
7. **대화 없이도 쓸 수 있음.** 상품 페이지, 장바구니, 주문 목록, 변경셋 상세는 일반 페이지로도 열립니다. 페이지는 블록과 같은 Astryx 부품과 `useAiTarget`으로 그려 같은 AI 기능(묻기, 가리키기)을 가집니다.

## 2. 구조

브랜드는 "Ailism Commerce"이고 테마는 `ailism`입니다([../frontend-ui.md](../frontend-ui.md) §4).

```
web/
├── AGENTS.md                         Astryx가 설치한 코딩 에이전트 안내(`astryx init`)
├── next.config.ts                    standalone 출력(컨테이너: infra/docker/web.Dockerfile)
├── public/brand/                     Ailism 워드마크, 어시스턴트 얼굴
└── src/
    ├── app/                          # Next.js App Router
    │   ├── layout.tsx                제목 "Ailism Commerce", Astryx CSS(reset → components), Pretendard, Providers
    │   ├── providers.tsx             Astryx Theme(ailism) + LinkProvider(next/link)
    │   ├── healthz/route.ts          컨테이너·Kubernetes 헬스 체크
    │   ├── media/garment/[file]/     상품 자리표시 그림(옷 모양 + 색, SVG)
    │   ├── (storefront)/             # /  쇼퍼. layout: StorefrontShell + 어시스턴트 서랍, AiProvider(surface="fo")
    │   │   ├── page.tsx              홈(대상별 hero 배너, 진열)
    │   │   ├── ask/                  대화 + 캔버스 전체 화면(Workspace, F1·F2·F6)
    │   │   ├── search/  c/[category_id]/  sale/[slug]/  compare/  p/[product_id]/ (상품 페이지는 JSON-LD 포함)
    │   │   ├── cart/  checkout/  checkout/gift/  gifts/[gift_id]/  orders/  orders/[order_id]/  account/  login/
    │   │   └── _lib/                 서버 호출, 응답 타입, JSON-LD
    │   └── bo/                       # 운영자
    │       ├── login/
    │       ├── (console)/layout.tsx  세션 확인, AppShell + BoNav, AiProvider(surface="bo"), AgentDock
    │       ├── (console)/page.tsx    홈: 할 일(task_list) + Workspace (B5)
    │       ├── (console)/…           change-sets/  activity/  tasks/[task_id]/  products/(new, drafts, [product_id])  categories/
    │       │                         brands/  promotions/  merchandising/  orders/  returns/  claims/[claim_id]/  customers/
    │       │                         insights/  policies/  data/(데이터 사전)
    │       └── _lib/                 서버 호출, 쓰기, 응답 타입
    ├── ai/                           # AI 층 (§3)
    │   ├── AiProvider.tsx            대화 상태, 턴 보내기, 고른 요소, 요소 등록부, ui.command
    │   ├── useAiTarget.ts            요소 등록(ref, data-acom-*, ask)
    │   ├── stream.ts                 SSE 클라이언트(조각난 이벤트 처리)
    │   └── types.ts                  채팅 계약 타입
    ├── components/
    │   ├── blocks/{bo,fo}/           ui 이름 하나 = 컴포넌트 하나, registry.ts(모르는 블록은 제목과 why만 보이는 카드)
    │   ├── workspace/Workspace.tsx   대화 + 캔버스 (Astryx `ai-chat` 템플릿에서 시작)
    │   ├── bo/                       백오피스 페이지 부품(AgentDock, BoNav, StoreClock, ChangeSetView, TaskView, SalesChart …)
    │   ├── fo/                       스토어프론트 부품(StorefrontShell, AssistantDrawer, ProductCard, CheckoutFlow, ReturnPanel …)
    │   ├── brand/Ailism.tsx          워드마크, 어시스턴트 얼굴, 브랜드 문구, 푸터
    │   └── PersonButton.tsx          사람의 결정 (Astryx Button + AlertDialog)
    ├── lib/                          api.ts(같은 출처 호출, CSRF), server-api.ts(서버 컴포넌트), session.ts(로그인)
    ├── theme/                        ailism.ts(우리 테마) → 빌드 결과 ailism.js·ailism.css, theme.template.ts
    └── astryx-themes.ts              생성물(`astryx theme add neutral`). 지금은 어디서도 가져오지 않음
```

에이전트는 스토어프론트에서는 `/ask`(전체 화면)와 모든 페이지의 서랍(`AssistantDrawer`), 백오피스에서는 홈의 Workspace와 모든 페이지의 독(`AgentDock`)에 있습니다. 한 면 안에서는 `AiProvider` 하나가 대화를 들고 있어 페이지를 옮겨도 같은 대화입니다. 10월 31일에 더한 툴 대부분(커뮤니티, 입점 판매자, 외부 연계 등)은 전용 블록이나 백오피스 페이지가 아직 없습니다.

- **Meta 라이브러리를 먼저.** 화면은 Astryx 템플릿에서 시작하고(`npm run astryx -- build "<화면 설명>"`가 템플릿과 블록을 골라 줌), 부품은 Astryx 컴포넌트를 씁니다(173종, `npm run astryx -- search "<무엇>"`). 대화 화면은 Astryx `Chat` 계열(`ChatLayout`, `ChatMessageList`, `ChatComposer`, 툴 호출 표시)을 그대로 씁니다. 우리가 만드는 것은 데이터와 AI 층뿐입니다.
- **고치기(커스텀)는 단계대로.** 컴포넌트 props → 테마 토큰 → `className` → `swizzle`(소스를 가져와 고침, 이유를 커밋에 적음). `:root` 덮어쓰기와 하드코딩한 색·크기는 쓰지 않습니다.
- **참고.** Astryx의 템플릿·블록 예제(`npm run astryx -- template <이름>`)와 문서(`npm run astryx -- docs <주제>`)를 먼저 봅니다.
- **점검.** 넘기기 전에 `npm run astryx -- doctor --built src`. 10월 31일 기준 실패는 없고 경고는 넷입니다.
  - 테마 둘: 앱이 생성물 `src/astryx-themes.ts` 대신 빌드한 `ailism` 테마를 가져오는 것, 글꼴을 외부에서 불러오는 것(Pretendard는 jsDelivr에서).
  - 테마 패키지 점검(위 둘을 묶은 경고).
  - 소스 20곳: 날 `<div>`·`<svg>`와 인라인 스타일(`Workspace`의 컨테이너 쿼리는 Astryx `ai-chat` 템플릿에서 가져온 것, `SalesChart`는 Astryx에 차트가 없어 SVG로 그림).
- 서버 컴포넌트는 페이지의 첫 데이터를 게이트웨이 REST(`/api/v1/...`, 읽기 툴)에서 가져옵니다. 대화와 블록 갱신은 클라이언트 컴포넌트가 SSE로 받습니다.
- **층 나누기:** Astryx 부품은 모양과 접근성, 우리 컴포넌트는 데이터와 AI 층(`useAiTarget`, 추천 말, `PersonButton`)을 맡습니다. Astryx 부품을 바꾸거나(swizzle) 다른 라이브러리로 갈아타도(Mantine 대안, [../frontend-ui.md](../frontend-ui.md) §5) AI 층과 블록 계약은 그대로입니다.

## 3. AI 층

### 3.1 요소 등록

```tsx
const t = useAiTarget({
  kind: "change_row",                          // 요소 종류 (§4 표)
  label: "Seersucker overshirt · 15% · $67.15",// 사람과 모델이 읽는 한 줄
  ids: { product_id: "prd_seersucker_overshirt", change_set_id: "cs_1042" },
  term: "schema:UnitPriceSpecification",
  blockId: "blk_12",
});
// t.props = data-acom-kind / -id / -term / -ref, t.ask() = 이 요소를 "이것"으로 고르기
return <TableRow ref={t.ref} {...t.props}>…</TableRow>;
```

- 등록된 요소는 `element_ref`(`blk_12#change_row:prd_seersucker_overshirt`)를 가집니다.
- 요소마다 DOM에 `data-acom-kind`, `data-acom-id`, `data-acom-term`, `data-acom-ref`가 붙습니다. 외부 브라우저 에이전트와 테스트가 같은 표시를 씁니다.
- 추천 말은 등록 인자가 아니라 블록의 `suggested_intents`에서 요소별로 찾아 그 요소 옆에 보입니다.

### 3.2 묻기와 고치기

- 요소를 고르면(묻기 버튼 "Ask about this") 입력 칸에 그 요소가 칩으로 붙고, 턴 요청의 `context`에 `element_ref`, `kind`, `ids`, `label`이 실립니다. 에이전트는 "이것"이 무엇인지 추측하지 않습니다.
- 추천 말은 블록 데이터의 `suggested_intents`에서 오고, 누르면 그 문장이 그대로 대화로 갑니다.
- 계획: 입력 칸(`NaturalInput`)이 자연어를 받아(기간 칸에 "this weekend") 에이전트가 스토어 시간대로 풀고 `looking_for` 칩으로 확인받는 것. 아직 없습니다. 지금은 대화 입력 칸 하나입니다.

### 3.3 에이전트가 화면을 가리킴

에이전트는 SSE `ui.command` 이벤트로 화면을 움직일 수 있습니다([../api/chat.md](../api/chat.md) §2).

| command | 하는 일 |
|---|---|
| `highlight` | 요소로 스크롤하고 4초 동안 테두리(설명 문구는 대화에 씀) |
| `scroll_to` | 요소로 스크롤 |
| `open` | 이 사이트의 페이지 열기(`/bo/change-sets/cs_1042`) |
| `focus_input` | 요소 안의 입력 칸에 커서(보내는 것은 사람) |

화면 명령은 `useAiTarget`으로 등록된 요소에만 닿습니다(`AiProvider`의 `runCommand`). `PersonButton`, 결제 정보, 비밀번호 칸은 등록하지 않으므로 대상이 될 수 없습니다.

### 3.4 진행과 근거 보이기

- `agent.step` 이벤트와 턴의 사용량(모델 호출, 툴 호출, 모델 이름)은 `AiProvider`가 받아 두지만 아직 화면에 그리지 않습니다(계획: 블록 위의 `StepTrail`, "9 tool calls · Muse Glimmer 30B · 3.8s"). 지난 일은 `/bo/activity`(액션 로그)에서 봅니다.
- 블록의 `why`, 체크의 `detail`·`fix_tool`, `fit_check`의 "What I used"는 접지 않고 보입니다.
- 기억이 생기면 `memory_note` 블록으로 한 줄과 되돌리기 버튼(`person`)이 나타나고, 쇼퍼의 `/account`에서도 되돌릴 수 있습니다.

### 3.5 브라우저 에이전트용 페이지 툴 (계획, 아직 없음)

브라우저가 페이지 툴 등록을 지원하면(`navigator.modelContext` 계열 제안), 스토어프론트는 읽기·장바구니 의도(`search_products`, `compare_products`, `add_to_cart`, `prepare_order`)만 페이지 툴로 등록합니다. 주문하기는 등록하지 않습니다. 지원하지 않으면 아무것도 하지 않습니다. 계획한 MCP(`/mcp`, 아직 없음)와 같은 범위입니다.

## 4. 요소별 AI 기능

| 요소 | 어디 | 묻기 예 | 고치기·의도 예 | 에이전트가 하는 일 |
|---|---|---|---|---|
| `Price` | 카드, 비교표, 장바구니 | "왜 이 가격이야?" | — | 혜택 내역(`calculate_benefits`)으로 설명 |
| `CheckRow` | change_set, refund_review | "이 체크는 왜 실패했어?" | "고쳐 줘" → `fix_tool`로 수정안 | 실패 상품을 `highlight` |
| `ChangeRow` | change_set 변경 표 | "이 상품은 왜 15%야?" | "이건 20%로", "빼 줘" | 해당 항목만 다시 프리뷰, 버전 +1 |
| `UnderstoodField` | change_set "Your request, as I understood it" | "1,284명은 어디서 나온 숫자야?" | "VIP만이 아니라 Gold도" | 해석 수정 → 다시 계획 |
| `TaskItem` | task_list | "이건 뭐야?" | "지금 처리해" | 그 할 일로 새 턴 |
| `RequestRow` | refund_review | "이 고객 반품 이력 보여 줘" | "이건 사진 요청으로" | 분류만 바꾸고 다시 프리뷰 |
| `DraftRow` | product_drafts | "이 이름 괜찮아?" | "설명을 더 짧게" | 초안 다시 쓰기 |
| `Chip` | looking_for | — | 칩 제거, "M 말고 L" | 조건을 바꿔 다시 검색 |
| `ProductCard` | look_gallery, product_list | "이거 비 오면 괜찮아?" | "이거랑 비교해 줘" | `compare_products`, 형태 전환 |
| `CompareColumn`·`CompareRow` | compare_table | "무게가 왜 중요해?" | "사진으로 보여 줘", "가격 순으로" | 정렬·형태 변경 |
| `CartLine` | cart | "금요일까지 와?" | "하나 더", "L로 바꿔" | `get_delivery_dates`, 장바구니 쓰기 |
| `OrderRow` | order_detail, 주문 목록 | "어디쯤 왔어?" | "반품하고 싶어" | `return_options` 준비(보내기는 사람) |
| `MeasurementTable` | fit_check | "어깨는 괜찮을까?" | — | 실측 비교 설명 |
| `FitOption` | fit_check 사이즈 선택지 | "왜 L이야?" | "스웨터만 입으면?" | 같은 읽기로 다시 판단, 선택지 순서 바꿈 |
| `GiftOptions`·`GiftField` | gift_picker, 선물 결제(`/checkout/gift`) | "포장비는 얼마야?" | "카드 문구 바꿔 줘" | `prepare_gift_order` 다시 프리뷰(주문하기는 사람) |
| `MemoryNote`·`ProfileFact` | 대화, profile_card | "이건 왜 기억했어?" | "이건 잊어" → 되돌리기 버튼 안내 | 기억 출처 설명 |
| `SearchBox`·`NaturalInput` | 헤더, 필터, 기간 | — | 자연어 그대로 | 조건으로 풀어 칩으로 확인 |
| `ProductImage` | 상품, 초안(운영자) | "이 사진 뭐가 문제야?" | (운영자) "배경 지워 줘" | catalog 이미지 오퍼레이션(`generate_image`) 프리뷰. 이미지 생성은 자리표시 그림을 만드는 스텁 |
| `Figure`·`CauseRow`·`SeriesPoint` (`figure`, `cause`, `series_point`) | sales_explainer, `/bo/insights` | "이 날은 왜 낮아?", "−$660은 뭐야?" | 다음 행동 버튼("Send the weekly email today") | `explain_sales_change`·`get_sales_trend`로 설명. 다음 행동은 그 문장으로 새 턴(변경셋은 그 일을 맡은 에이전트가) |

모든 요소는 이 표의 "묻기"를 기본으로 지원하고, "고치기"는 블록 데이터의 `suggested_intents`가 있을 때 보입니다.

## 5. 사람의 결정 (`PersonButton`)

- 블록의 `person` 버튼은 `PersonButton` 하나로만 그립니다(Astryx 버튼 위에 확인 단계를 얹은 우리 컴포넌트): 확인 문구(`confirm`), CSRF 토큰, `Idempotency-Key`, 진행·결과 표시, 에러(`estimate_changed`면 새 금액을 보여 주고 다시 확인).
- 대상: 변경셋 승인·거절·되돌리기, 주문하기·선물 주문하기, 선물 받기, 반품·교환·취소 요청 보내기, 기억 되돌리기, 할 일 치우기, 데모 시계 옮기기(`admin:demo`, 백오피스의 스토어 시각 `StoreClock`).
- 에이전트 이벤트로 눌리지 않고, 페이지 툴로 등록되지 않습니다.

## 6. 상태

- 대화 상태(대화 id, 메시지, 블록, 고른 요소)는 면마다 `AiProvider` 하나의 React 상태입니다. 블록은 `block_id`로 저장하고 같은 id가 오면 교체합니다.
- 같은 면 안에서 페이지를 옮겨도 대화가 남지만, 새로고침하면 처음부터입니다. 끊긴 SSE를 `Last-Event-ID`로 이어 받는 것과 `GET /api/chat/{surface}/conversations/{id}`로 복원하는 것은 아직 웹에 없습니다(API는 있음).
- 서버 데이터는 페이지 첫 로드에 서버 컴포넌트가 가져오고, 사람의 버튼으로 쓴 뒤에는 `router.refresh()`로 다시 읽습니다.
- 스토어 진행 이벤트(`GET /api/v1/platform/events`)는 화면이 데이터를 다시 읽는 신호로만 씁니다: 받은 편지함은 `task.updated`가 오면 할 일 목록을 다시 읽습니다(감시 작업이 갱신한 할 일 포함). 숫자는 이벤트가 아니라 읽기에서 옵니다.

## 7. 테스트

- 지금 웹의 검사는 CI의 `npm run typecheck`와 `npm run build`, 그리고 손으로 돌리는 `npm run astryx -- doctor --built src`(§2)입니다. 블록의 데이터 모양은 Rust 쪽 블록 테스트(`crates/agent/src/blocks/`)와 장면 재생 테스트가 확인합니다.
- 계획(아직 없음): 블록마다 컴포넌트 테스트(Vitest + Testing Library)로 [../scenarios.md](../scenarios.md)의 숫자로 만든 픽스처를 그리고, 요소 등록(`data-acom-*`)과 `person` 버튼이 화면 명령에 반응하지 않는 것을 확인.
- 브라우저 끝단 테스트(Playwright)는 통합 테스트로 분류하고 요청이 있을 때만 돌립니다(아직 없음).
