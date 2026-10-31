# commerce 서비스

커머스의 모든 규칙이 사는 곳입니다. 오퍼레이션(툴)을 실행하고, 정책을 판정하고, 변경셋을 승인·실행·검증·되돌립니다. 에이전트와 웹, 외부 에이전트가 모두 이 서비스의 같은 오퍼레이션을 부릅니다.

관련 명세: 응답 형식·규약 [../api/README.md](../api/README.md) §3, [../api/conventions.md](../api/conventions.md) · 변경셋 API [../api/platform.md](../api/platform.md#change_set) · 에러 [../api/errors.md](../api/errors.md) · 상태 값 [../api/enums.md](../api/enums.md)

---

## 1. 오퍼레이션 모델

### 1.1 메타데이터

모든 오퍼레이션은 정적 메타데이터를 가집니다. 이것이 MCP 툴 스펙, REST 라우트, OpenAPI, 툴 그래프, 문서 대조의 단일 원천입니다.

```rust
// acom-contract
pub struct OpMeta {
    pub address: &'static str,        // "promotion.offer.set_sale_prices"
    pub tool: &'static str,           // "set_sale_prices"
    pub group: Group,                 // Offer | Transact | Engage | Operate
    pub kind: Kind,                   // Read | Write | WriteApproval  (= R | W | W$)
    pub method: Method,               // POST
    pub path: &'static str,           // "/v1/promotion/sale-prices"
    pub summary: &'static str,        // 한 줄 설명 (모델이 툴을 고르는 근거)
    pub consumes: &'static [Slot],    // [Slot::many("product_id"), Slot::one("tier_id")]
    pub produces: &'static [Slot],    // [Slot::one("promotion_id")]
    pub checks: &'static [&'static str],   // 정책 체크 키 ("max_discount", "price_floor", ...)
    pub errors: &'static [ErrorCode],      // 이 오퍼레이션이 낼 수 있는 에러 코드
    pub audience: Audience,           // Operator | Shopper | Both | Internal
    pub action_type: ActionType,      // schema.org Action (SearchAction, UpdateAction, ...)
}
```

필요한 스코프는 메타에서 나옵니다: 읽기는 `tools:read:{domain}`, 쓰기는 `tools:write:{domain}`([gateway.md](gateway.md) §2). `audience`가 `Internal`이면 모델 매니페스트와 MCP에 나오지 않습니다.

### 1.2 트레이트

```rust
pub trait Operation: Send + Sync + 'static {
    const META: OpMeta;
    type Input: DeserializeOwned + JsonSchema;   // 닫힌 스키마로 내보낼 수 있어야 함
    type Output: Serialize + JsonSchema;
}

pub trait ReadOp<C: Ctx>: Operation {
    async fn read(&self, cx: &C, input: Self::Input) -> Result<Self::Output, ToolError>;
}

pub trait WriteOp<C: Ctx>: Operation {
    /// 프리뷰에서 계산해 저장하고, 실행 때 그대로 쓰는 계획. 입력이 아니라 "무엇을 바꿀지"의 확정본.
    type Plan: Serialize + DeserializeOwned + Send + Sync;

    /// 상태를 바꾸지 않는다. 바꿀 내용·정책 체크·승인 필요 여부·근거 버전을 계산한다.
    async fn preview(&self, cx: &C, input: Self::Input) -> Result<Planned<Self::Plan>, ToolError>;

    /// 트랜잭션 안에서 계획을 적용한다. 같은 preview_id로 두 번 불려도 결과가 같아야 한다.
    async fn commit(&self, tx: &mut C::Tx, plan: &Self::Plan) -> Result<Committed, ToolError>;

    /// 적용 결과를 다시 읽어 계획과 비교한다(변경셋 verifying 단계).
    async fn verify(&self, cx: &C, plan: &Self::Plan, done: &Committed) -> Result<Verification, ToolError>;

    /// 되돌리기. 되돌릴 수 없는 오퍼레이션(메시지 발송 등)은 Undo::NotSupported를 선언한다.
    async fn undo(&self, tx: &mut C::Tx, plan: &Self::Plan, done: &Committed) -> Result<(), ToolError>;
    const UNDO: UndoSupport;   // Supported | NotSupported (메시지 발송, 송금된 환불)
}

pub struct Planned<P> {
    pub plan: P,
    pub changes: Vec<Change>,          // 응답의 changes[]
    pub checks: Vec<Check>,            // 응답의 checks[] (정책 엔진 결과)
    pub basis: Vec<Basis>,             // 근거로 읽은 엔티티와 버전 (실행 때 낙관적 동시성 검사)
    pub approval: ApprovalNeed,        // NotNeeded | Needed { reason }  (approval.yml 판정)
}
```

레지스트리는 이 타입들을 JSON 입출력의 `DynOperation`으로 지워서 보관합니다. 도메인 크레이트는 `pub fn register(r: &mut RegistryBuilder)`만 공개합니다.

### 1.3 실행 문맥

`acom-contract`의 트레이트는 문맥 타입을 매개변수로 받습니다(`ReadOp<C: Ctx>`). 실제 문맥 `Cx`와 트랜잭션 문맥 `TxCx`(`Ctx::Tx`)는 `acom-store`에 있습니다. 그래서 `contract`는 I/O 없이 유지되고 테스트는 가짜 문맥으로 돕니다.

```rust
pub struct Cx {
    pub tenant: TenantId,
    pub principal: Principal,      // 누가(사람·에이전트 위임·외부 에이전트·시스템), 스코프 포함 (gateway.md)
    pub request_id: RequestId,
    pub clock: Arc<dyn Clock>,     // 시스템 시계 또는 데모 시계
    pub policy: Arc<PolicySet>,    // 이 요청 시점의 정책 스냅샷 (버전 해시 포함)
    pub db: PgPool,
}
```

`TxCx`는 `Cx` + 열린 트랜잭션입니다. 트랜잭션을 열 때 `set_config('app.tenant_id', $1, true)`를 실행해 RLS가 다른 스토어의 행을 막습니다.

---

## 2. 쓰기의 생애: 프리뷰 → 실행 → 검증 → 되돌리기

### 2.1 프리뷰

`POST {rest}?preview=true` + 입력 → `WriteOp::preview` → 프리뷰 저장 → 응답:

```jsonc
{"preview_id": "pv_01JC8...", "expires_at": "2026-11-06T10:42:00-05:00",
 "changes": [...], "checks": [...], "needs_approval": true, "approval_reason": "...",
 "change_set_id": "cs_1042"}          // 변경셋에 붙었을 때만
```

`platform.preview` 테이블에 저장하는 것:

| 열 | 내용 |
|---|---|
| `preview_id` | `pv_` + ULID |
| `operation` | 논리 주소 |
| `input`, `input_hash` | 정규화한 입력과 해시 (같은 입력 재프리뷰 감지) |
| `plan` | `WriteOp::Plan` JSON |
| `changes`, `checks`, `needs_approval`, `approval_reason` | 응답 그대로 |
| `basis` | 근거 엔티티 id와 버전 |
| `policy_version` | 정책 스냅샷 해시 |
| `principal`, `change_set_id`, `expires_at`, `state` | `open` → `committed` / `expired` / `superseded` |

프리뷰는 기본 30분 뒤 만료됩니다(`approval.yml`의 `preview_ttl_minutes`). 변경셋에 붙은 프리뷰는 만료 대신 승인 시점에 재검증합니다(§3.3).

### 2.2 실행

`POST {rest}` + `{"preview_id": "..."}` (다른 필드는 금지: `preview_input_conflict`).

`commerce`는 다음을 순서대로 확인한 뒤에만 실행합니다.

1. 프리뷰가 있고 `open`이며 만료 전인가 (`preview_not_found`, `preview_expired`).
2. 같은 테넌트이고 호출자가 그 오퍼레이션의 스코프를 가졌는가.
3. **승인:** `needs_approval=false`이거나, 프리뷰가 승인된 변경셋에 속하고 호출자가 변경셋 실행기(시스템 주체)인가. 아니면 `approval_required`.
4. `basis`의 버전이 지금도 같은가. 다르면 다시 프리뷰해서 `changes`가 같으면 진행, 다르면 `estimate_changed`(새 프리뷰를 `detail`에 담음).
5. 정책 버전이 바뀌었으면 체크를 다시 돌려 하나라도 실패하면 `policy_changed`.

실행은 한 트랜잭션입니다: `commit` → 프리뷰 `committed` → 도메인 이벤트 append → 아웃박스 → 액션 로그(`CompletedActionStatus`). 같은 `preview_id`로 다시 오면 저장된 결과를 그대로 돌려줍니다(멱등). `Idempotency-Key` 헤더는 프리뷰 없이 실행되는 사람 전용 엔드포인트의 재시도 보호에 씁니다.

### 2.3 모델이 보는 툴과 전선 위의 툴

| | 모델에게 보이는 툴(에이전트 매니페스트) | 전선 위(REST·MCP) |
|---|---|---|
| 입력 | 오퍼레이션 입력만 | 입력 + `preview` 또는 `preview_id` |
| 프리뷰·실행 | 에이전트 런타임이 대신 처리 | 호출자가 직접 |
| 변경셋 | 런타임이 열고 붙임 | `change_set_id` 쿼리 파라미터 |

소형 모델이 `preview`와 `preview_id` 두 모드를 헷갈리지 않도록, 우리 에이전트의 모델은 **"무엇을 바꿀지"만 말하고** 프리뷰·실행·승인 흐름은 런타임이 결정적으로 처리합니다([agent.md](agent.md) §5). 외부 에이전트(MCP)는 전선 위 형태를 그대로 봅니다.

---

## 3. 변경셋 엔진 (`acom-changeset`)

### 3.1 상태기계

```
drafting ──submit──▶ waiting_for_approval ──approve(version)──▶ running ──▶ verifying ──▶ live
   ▲                     │        │                                │            │          │
   └──── edit(version+1)─┘        └─reject──▶ rejected             └─▶ failed ◀─┘          └─undo──▶ undone
```

| 전이 | 누가 | 조건 |
|---|---|---|
| create → `drafting` | 에이전트 런타임(위임 주체) | 요청 원문, 해석(`interpretation[]`), 대화 id |
| 항목 추가·교체(`drafting`, `waiting_for_approval`에서 → `drafting`, version+1) | 에이전트 런타임 | 같은 변경셋의 프리뷰만 |
| `submit` → `waiting_for_approval` | 에이전트 런타임 | 항목 ≥ 1, 모든 체크 통과 또는 사람이 볼 실패만 남음 |
| `approve {version}` → `running` | **사람 전용** (`human:approve`) | 최신 버전만(`version_out_of_date`). `AuthorizeAction` 기록 |
| `reject {reason}` → `rejected` | 사람 전용 | |
| `running` → `verifying` → `live` / `failed` | 시스템(실행기) | §3.2 |
| `undo` → `undone` | 사람 전용 (`human:undo`) | `live` 상태, 기한 전(`undo_window_closed`), 모든 항목이 되돌릴 수 있음 |

### 3.2 실행기

승인되면 `commerce` 안의 실행기(시스템 주체)가 항목을 **선언 순서대로** 실행합니다. 에이전트는 실행하지 않고 이벤트로 진행을 받습니다.

1. 각 항목: §2.2의 실행 경로(재검증 포함). 항목 사이 의존(`create_coupon`이 `set_sale_prices`의 `promotion_id`를 소비)은 프리뷰 때 플레이스홀더(`$item[1].promotion_id`, `n`은 항목의 `position`이며 1부터)로 저장했다가 실행 때 바인딩합니다.
2. 한 항목이 실패하면 멈추고, 이미 실행한 항목은 역순으로 `undo` → 변경셋 `failed`(어느 항목이 왜 실패했는지 기록).
3. 모두 성공하면 `verifying`: 항목마다 `verify` → 결과를 `verify_change_set`이 돌려주는 형태로 저장 → 전부 일치하면 `live`, 아니면 `failed`.
4. 상태가 바뀔 때마다 `change_set_event`를 append하고 아웃박스로 알립니다. 에이전트는 이를 구독해 대화에 `change_set.updated`를 흘립니다([../api/chat.md](../api/chat.md)).

예약 실행(배너를 금요일 0시에 켜기 등)은 실행이 아니라 **예약 레코드를 만드는 것**이 커밋입니다. 실제 켜짐은 작업 큐(§5.3)가 시각에 맞춰 처리하고, `verify`는 예약 레코드를 검증합니다.

### 3.3 승인 시 재검증

승인 시점에 모든 항목을 다시 프리뷰합니다. `changes`가 하나라도 달라지면 실행하지 않고 변경셋을 `drafting`으로 되돌려 버전을 올리고 `estimate_changed`를 돌려줍니다. 사람은 항상 **실행될 내용 그대로**를 승인합니다.

### 3.4 되돌리기

되돌리기는 변경셋 단위입니다. 기한은 정책(`approval.yml`의 `undo_window`)으로 정하고 변경셋에 기록합니다. 실행 응답의 `undo`는 다음 형태입니다.

```json
{"by": "person", "target": "cs_1042", "until": "2026-11-09T23:59:00-05:00"}
```

모델은 이 값을 보고 "기한까지 되돌릴 수 있다"고 안내할 뿐, 되돌리기를 호출할 수 없습니다. 되돌릴 수 없는 항목(고객 메시지 발송, 환불 송금)이 있는 변경셋은 프리뷰 단계에서 `checks`에 `irreversible`로 표시합니다.

---

## 4. 정책 엔진 (`acom-policy`)

- **파일:** `policies/*.yml`(도메인별 파일 + `approval.yml`)이 기본값입니다. 파일마다 Rust 구조체가 있고(`#[serde(deny_unknown_fields)]`), 시작할 때 스키마 검증에 실패하면 서비스가 뜨지 않습니다.
- **스토어별 덮어쓰기:** `platform.policy_override`(스토어별 부분 YAML) → 기본값에 깊은 병합 → `PolicySet`(+ 버전 해시). 덮어쓰기 변경도 `platform.policy`의 쓰기 오퍼레이션이라 프리뷰·승인을 거칩니다.
- **식 언어 없음:** YAML 안에 `"subtotal >= min_subtotal"` 같은 문자열 식을 두지 않습니다. 조건은 구조화된 형태(`{field: subtotal, op: gte, value: {value: 50, currency: USD}}`)로 쓰고, 작은 평가기(`policy::cond`)가 해석합니다. 계산식은 코드에 있고 YAML에는 그 파라미터만 둡니다.
- **체크:** 체크 하나 = 순수 함수 `fn(&PolicySet, &CheckInput) -> Check`. 체크 키(`max_discount`, `price_floor`, ...)는 정책 키와 같고, 실패 시 `fix_tool`을 함께 냅니다. 오퍼레이션 메타의 `checks`가 어떤 체크를 돌릴지 정합니다.
- **승인 판정:** `approval.yml`이 오퍼레이션 종류·금액·대상 수로 `needs_approval`과 사유를 정합니다. `W$`는 정책과 상관없이 항상 승인입니다.
- **혜택 계산:** `calculate_benefits`(회원가·쿠폰·적립금 적용 순서와 한도)는 `acom-policy::benefit`의 순수 함수 하나이고, 장바구니·주문 준비·프로모션 프리뷰가 모두 이것을 부릅니다. 운영자가 정한 가격과 쇼퍼 에이전트가 말하는 가격이 같은 이유입니다.

---

## 5. 저장소 (`acom-store`)

### 5.1 스키마와 테이블

- PostgreSQL 스키마는 도메인별(`catalog`, `merchandising`, `search`, `promotion`, `customer`, `sales`, `fulfillment`, `messaging`, `platform`) + 에이전트용 `agent`.
- 테이블·컬럼 이름은 [../api/data-model.md](../api/data-model.md) §DB 명명을 따릅니다. 모든 스토어 범위 테이블에 `tenant_id`와 RLS 정책.
- 앱 접속 역할(`acom_app`)은 RLS를 우회하지 못합니다. 마이그레이션 역할만 소유자입니다.

### 5.2 상태와 이벤트

- 상태가 있는 집계(주문, 주문 품목, 클레임, 환불, 쿠폰, 적립금, 변경셋)는 **현재 상태 열 + 이벤트 테이블**(`*_event`, append-only)을 같은 트랜잭션에서 씁니다. 상태 전이는 도메인 크레이트의 상태기계 함수(`fn transition(from, event) -> Result<State, DomainError>`)만 할 수 있습니다.
- 적립금은 원장(`loyalty_ledger_entry`)으로, 잔액은 합계입니다.
- 도메인 간 알림은 **아웃박스**(`platform.outbox`)로 합니다. 예: `order_delivered` → 적립금 확정, 등급 즉시 평가. 디스패처가 `LISTEN/NOTIFY` + 주기 폴링으로 꺼내 해당 도메인 핸들러를 부릅니다(최소 한 번 전달, 핸들러는 멱등).

### 5.3 작업 큐와 시계

- 예약 작업(배너 켜고 끄기, 프로모션 시작·종료, 쿠폰 만료, 등급 정기 평가, 적립 확정)은 `platform.job`(`run_at`, `FOR UPDATE SKIP LOCKED`)으로 돌립니다.
- **처리기는 도메인이 등록합니다.** 오퍼레이션 옆에 `RegistryBuilder::job(…)`으로 `Job` 구현(`acom-contract::job`)을 하나씩 올립니다. 지금 있는 종류: `promotion.start`·`promotion.end`·`loyalty.expire`(promotion, 적립금 만료는 매일 스토어 자정에 돌고 다음 날 실행을 스스로 예약), `watcher.returns_over_refund_threshold`(fulfillment), `watcher.stock_out_forecast`(sales, 재고는 catalog의 `product_stock`), `watcher.price_below_cost`(catalog). 처리기는 작업의 트랜잭션 안에서 돌고, 결과로 `Done {result, events}`(도메인 이벤트는 엔진이 outbox에 씀) 또는 감시 작업의 `Watched(Vec<TaskDraft>)`를 돌려줍니다. 도메인은 `platform.task`를 직접 쓰지 않습니다.
- **실행기**는 `acom-changeset::jobs::run_due(registry, cx, until)`입니다. 기한이 된 작업을 `run_at` 순서로 하나씩 잡아 작업마다 트랜잭션 하나에서 실행하고(시계는 그 작업의 `run_at`), 성공이면 `done_at`·`attempts`, 실패면 처리기의 쓰기를 세이브포인트로 되돌린 뒤 `attempts`·`last_error`와 1·2·4…60분 뒤의 `run_at`을 기록합니다. 감시 작업은 `platform.yml` `every_minutes`로 다음 실행을 만들고, 놓친 실행은 가장 늦은 한 번으로 합칩니다. 감시 작업의 초안으로 같은 감시 작업·원인의 열린 할 일을 갱신하거나 새로 만들고, 고침이 있으면 `sys_watcher`로 변경셋을 만들어 제출합니다(`run_fix: standing_approval`일 때만 승인 없이 실행). `job.updated`·`task.updated`는 변경셋 이벤트처럼 `pg_notify('acom_events', …)`로 나갑니다. 규칙 전체는 [../api/platform.md](../api/platform.md#예약-작업).
- **워커**(`acom-commerce work`)는 5초마다 `ACOM_STORE_ID`의 스토어(쉼표로 여러 개)마다 `run_due`를 부르고, 종료 신호를 받으면 잡은 작업을 끝낸 뒤 멈춥니다. `acom_app`은 RLS 때문에 스토어 목록을 읽을 수 없으므로 스토어를 설정으로 받습니다.
- 시간은 `Clock` 트레이트로만 얻습니다. 데모 모드에서는 스토어별 데모 시계(`platform.demo_clock`)가 기준이고, 시계를 앞으로 돌리면(`PUT /v1/platform/demo-clock`, `admin:demo`) 그 사이 예약 작업이 순서대로 실행된 뒤 시계가 옮겨집니다. 같은 시각으로 다시 옮기면 아무것도 실행하지 않습니다. 시나리오(금요일 아침 운영, 토요일 쇼핑)를 재현하는 장치입니다.

### 5.4 검색

`search` 도메인은 `SearchBackend` 트레이트 뒤에 Postgres 구현(전문 검색 `tsvector` + pgvector 임베딩, 하이브리드 점수)을 둡니다. 등급·재고·판매 상태 필터는 **질의 안에서** 적용합니다(결과를 받은 뒤 거르지 않음). 임베딩은 상품 저장 시 아웃박스 → 색인 작업으로 갱신합니다.

---

## 6. 표면 (`acom-http`, `acom-mcp`)

| 표면 | 경로 | 내용 |
|---|---|---|
| 툴 REST | `/v1/{domain}/...` | 레지스트리의 모든 오퍼레이션. 읽기는 GET(필터는 쿼리), 쓰기는 POST/PUT/PATCH/DELETE + `?preview=true` |
| 툴 매니페스트 | `GET /v1/tools` | 스코프로 거른 툴 목록: 이름·설명·주소·종류·consumes/produces·닫힌 입력 스키마·체크·에러 |
| 변경셋(런타임용) | `/v1/platform/change-sets/...` | 생성·항목·제출·조회·검증. [../api/platform.md](../api/platform.md#change_set) |
| 사람 전용 | `/v1/platform/change-sets/{id}/approve` 등 | `human:*` 스코프 필수. [../api/README.md](../api/README.md) §2 |
| 이벤트 구독 | `GET /v1/platform/events` (SSE) | 변경셋·작업 진행을 에이전트와 웹에 전달 |
| 데모 시계(런타임용) | `GET`·`PUT /v1/platform/demo-clock` | 스토어 시각 읽기, 데모 모드에서 옮기기(`admin:demo`). [../api/platform.md](../api/platform.md#데모-시계) |
| MCP | `/mcp` | 외부 에이전트 스코프의 툴만. 전선 위 형태(preview/preview_id 포함) |
| 운영 | `/healthz`, `/readyz`, `/metrics` | |

에러는 한 곳에서 HTTP로 바꿉니다: `ToolError{code, class, message, next_tool, detail}` → 상태 코드는 class에서([../api/errors.md](../api/errors.md) §1), 바디는 `{"error": {...}}`.
