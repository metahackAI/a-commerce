# A-Commerce 백엔드 (Rust)

A-Commerce의 서버 측은 **전부 Rust**로 만듭니다. 게이트웨이, 커머스 백엔드(툴 레이어·정책 엔진·변경셋·저장소), 에이전트 런타임(하네스), 평가 러너까지 한 Cargo 워크스페이스에 있습니다. 프런트엔드는 Next.js이고 이 워크스페이스가 생성한 타입을 씁니다.

이 폴더는 **백엔드 구현 설계**입니다. 무엇을 만드는지(오퍼레이션·입출력·정책)는 [../api/](../api/README.md), 왜 이렇게 만드는지는 [../design.md](../design.md)와 [../architecture.md](../architecture.md), 웹은 [../frontend/](../frontend/README.md)에 있습니다.

| 문서 | 내용 |
|---|---|
| (이 파일) | 서비스·크레이트 구성, 의존 규칙, 기술 스택, 로컬 실행 |
| [database.md](database.md) | PostgreSQL: schema.org 이름, 주석, 공통 컬럼, RLS, 이벤트, 검색, API ↔ DB 이름, 테이블 목록 |
| [schema.generated.md](schema.generated.md) | 마이그레이션에서 생성한 컬럼 사전 |
| [commerce.md](commerce.md) | 오퍼레이션 트레이트, 프리뷰·실행·검증·되돌리기, 변경셋 엔진, 정책 엔진, 저장소·이벤트·작업 |
| [agent.md](agent.md) | 에이전트 런타임: 라우터·특화 에이전트·툴 검색·플래너·실행기·체커·UI 블록·기억·모델 클라이언트 |
| [gateway.md](gateway.md) | 인증·주체·스코프·내부 토큰·라우팅·레이트 리밋·SSE 중계 |
| [testing.md](testing.md) | 계약 테스트, 시나리오 재생, 평가(화면형 파사드 대조) |

---

## 1. 서비스

서비스는 세 개이고 모두 같은 워크스페이스의 바이너리입니다. 컨테이너로는 gateway, commerce(`serve`), commerce-worker(같은 이미지의 `work`), agent, 마이그레이션 작업(`migrate`)으로 뜹니다. 서빙 구성은 [../../infra/README.md](../../infra/README.md)에 있습니다.

```
            브라우저 (Next.js: /bo 운영자, /fo 쇼퍼)            외부 에이전트
                         │ HTTPS (쿠키 세션)                     │ MCP (OAuth 토큰)
                         ▼                                       ▼
   ┌────────────────────────────── gateway ──────────────────────────────┐
   │ 세션·토큰 검증 → 주체(Principal)·테넌트 결정 → 내부 토큰 발급          │
   │ 레이트 리밋 · 요청 id · 라우팅 · SSE 중계 · 정적 웹 프록시              │
   └───────┬──────────────────────────────┬──────────────────────┬────────┘
           │ /api/chat/* (SSE)            │ /api/v1/*, 사람 전용   │ /mcp
           ▼                              ▼                       ▼
   ┌──────────────── agent ───────┐   ┌──────────────── commerce ─────────────────┐
   │ 대화·턴 루프                  │   │ 오퍼레이션 레지스트리 (13 도메인)            │
   │ 라우터 → 특화 에이전트        │──▶│ REST /v1 · MCP · 사람 전용 엔드포인트       │
   │ 툴 검색 · 플래너 · 실행기      │툴 │ 프리뷰 저장소 · 변경셋 엔진(승인·실행·검증)  │
   │ 체커 · UI 블록 · 기억          │호출│ 정책 엔진 · 혜택 계산                       │
   │ 모델 클라이언트(Glimmer/Spark) │   │ 이벤트·아웃박스·예약 작업 · 데모 시계        │
   └──────────────┬───────────────┘   └──────────────────┬─────────────────────────┘
                  │                                       │
                  └───────────────┬───────────────────────┘
                                  ▼
                 PostgreSQL 17 + pgvector (스키마: 도메인별 + platform + agent)
```

| 서비스 | 책임 | 소유 데이터 | 하지 않는 일 |
|---|---|---|---|
| `gateway` | 사람·외부 에이전트의 진입점. 인증, 테넌트, 스코프, 라우팅 | 세션 | 비즈니스 규칙 |
| `commerce` | 모든 커머스 오퍼레이션과 규칙. **승인과 변경셋 실행의 유일한 주인** | 도메인 스키마 전부, `platform` | 모델 호출 |
| `agent` | 요청을 툴 호출로 바꾸고 결과를 UI 블록으로 돌려줌 | `agent` 스키마(대화·턴·모델 호출 기록) | 커머스 DB 직접 접근, 승인, 사람 전용 동작 |

나누는 기준은 **신뢰 경계**입니다. 에이전트(모델)는 커머스 규칙의 바깥에 있다고 보고, 승인·실행·정책 판정은 전부 `commerce`가 합니다. 에이전트가 고장 나거나 프롬프트가 조작돼도 `commerce`가 받지 않는 요청은 실행되지 않습니다.

MCP는 별도 서비스가 아니라 `commerce`가 REST와 함께 내놓는 또 하나의 표면입니다(같은 레지스트리에서 생성). 모든 서비스는 로컬에서도 운영과 같은 구성(컨테이너, 같은 네트워크 경계, 같은 헬스 체크)으로 뜹니다.

---

## 2. 워크스페이스

```
a-commerce/
├── Cargo.toml                    # [workspace], 공통 의존성 버전, lints
├── rust-toolchain.toml           # stable 고정
├── crates/
│   ├── core/          acom-core          id·Money·시간·schema.org enum·테넌트·시계. I/O 없음
│   ├── contract/      acom-contract      오퍼레이션 메타·봉투·에러·JSON Schema(닫힌 형식)·레지스트리 트레이트
│   ├── policy/        acom-policy        정책 YAML 타입·로드·검증·오버라이드·체크·혜택 계산. I/O 없음
│   ├── store/         acom-store         sqlx 풀·마이그레이션·RLS 트랜잭션·이벤트·아웃박스·작업 큐·멱등
│   ├── auth/          acom-auth          주체·스코프·내부 토큰 발급/검증 (세 서비스 공용)
│   ├── changeset/     acom-changeset     프리뷰 저장소·변경셋 상태기계·실행기·검증·되돌리기
│   ├── domains/
│   │   ├── catalog/   acom-catalog       ┐
│   │   ├── search/    acom-search        │
│   │   ├── merchandising/                │ 도메인 = docs/api/{domain}.md 1:1
│   │   ├── promotion/                    │ 폴더 = API 계층: src/{resource}/{tool}.rs (§2.2)
│   │   ├── customer/                     │
│   │   ├── sales/                        │
│   │   ├── fulfillment/                  │
│   │   ├── messaging/                    │
│   │   ├── platform/                     ┘
│   │   ├── analytics/ acom-analytics     analytics 뷰(읽기 모델)만 읽는 집계·매출 변화 설명
│   │   ├── community/ acom-community     게시판·글·댓글과 모더레이션
│   │   ├── partner/   acom-partner       입점 판매자·계약·등급
│   │   └── integration/ acom-integration 외부 채널·택배사·결제사 어댑터, 대량 가져오기, 이벤트 수신
│   ├── registry/      acom-registry      모든 도메인 오퍼레이션을 하나의 Registry로 조립
│   ├── http/          acom-http          Registry → axum 라우터, 에러→HTTP, 내부 토큰 추출기
│   ├── mcp/           acom-mcp           Registry → MCP 서버(rmcp, streamable HTTP)
│   ├── llm/           acom-llm           모델 클라이언트(OpenAI 호환), 툴콜 파싱·인자 보정, 사용량
│   ├── agent/         acom-agent         에이전트 런타임 전체 (agent.md)
│   ├── facade/        acom-facade        평가용 화면형 API 파사드 (testing.md)
│   ├── server/        acom-server        서비스 공통 런타임: 로그, /healthz·/readyz·/metrics, 요청 id, 종료 신호, 컨테이너 헬스 체크
│   └── testkit/       acom-testkit       시드 로더·데모 시계·시나리오 재생 도우미
├── services/
│   ├── gateway/       바이너리 acom-gateway
│   ├── commerce/      바이너리 acom-commerce
│   └── agent/         바이너리 acom-agent-svc
├── xtask/                        # cargo xtask: codegen · seed · check-docs · eval
├── policies/                     # 정책 기본값 YAML (도메인별 + approval, acom-policy가 로드)
├── agents/                       # 특화 에이전트 정의(YAML) + 지침(md)
├── seed/demo/                    # 데모 스토어 시드 (docs/scenarios.md에서 도출)
├── eval/                         # 과업 세트(design/held_out)와 결과
├── migrations/                   # sqlx 마이그레이션 (도메인 스키마별 파일)
├── infra/                        # 이미지(docker/), Compose(compose/), Kubernetes(k8s/), DB 역할(postgres/) — infra/README.md
├── web/                          # Next.js (App Router)
└── docs/
```

### 2.1 의존 규칙

```
core ◀── contract ◀── policy
  ▲         ▲            ▲
  │         │            │
  └──── store ◀── changeset ◀── domains/* ◀── registry ◀── http, mcp ◀── services/commerce
                                                 │
auth ◀── (http, services/*)                      └──(tools.json 매니페스트만)──▶ agent
llm ◀── agent ◀── services/agent
```

1. **`core`·`contract`·`policy`는 I/O가 없습니다.** DB·네트워크·파일 시스템을 쓰지 않습니다(정책 YAML은 문자열로 받아 파싱). 단위 테스트만으로 검증됩니다.
2. **도메인 크레이트는 서로를 DAG로만 참조합니다.** 다른 도메인의 테이블을 직접 읽지 않고, 그 도메인이 공개한 서비스 트레이트(`catalog::Prices`, `customer::Tiers` 등)를 씁니다.

   | 도메인 | 의존 |
   |---|---|
   | catalog, platform | (없음) |
   | customer | platform(비밀번호를 바꾸거나 탈퇴할 때 그 사람의 세션 끝내기) |
   | search | catalog |
   | promotion | catalog, customer |
   | merchandising | catalog, promotion |
   | sales | catalog, promotion, customer |
   | fulfillment | sales, catalog, customer |
   | messaging | customer |
   | community | catalog, customer, sales |
   | analytics | sales(회원 수 세기). 나머지는 자기 읽기 모델(`analytics` 뷰)만 |
   | partner, integration | catalog, sales, fulfillment |
3. **`agent`는 도메인 크레이트에 의존하지 않습니다.** 툴 정보는 `commerce`가 내놓는 매니페스트(`GET /v1/tools`, 빌드 때 생성되는 `tools.json`)로만 압니다. 에이전트와 커머스는 HTTP로만 만납니다(테스트에서는 같은 프로세스 안의 `ToolClient` 구현으로 대체).
4. **비즈니스 상수는 코드에 두지 않습니다.** 한도·하한·기간·승인 기준은 `acom-policy` 타입을 통해 `policies/*.yml`에서 옵니다. 리뷰에서 숫자 리터럴을 찾습니다.
5. **`unsafe` 금지, `unwrap`/`expect`는 테스트와 시작 단계에서만.** 워크스페이스 lints로 강제합니다.

### 2.2 도메인 크레이트의 폴더 = API 계층

API는 `domain.resource.operation` 계층입니다(`catalog.offer.set_price`). 코드도 같은 계층을 따릅니다: **도메인 = 크레이트, 리소스 = 폴더, 오퍼레이션 = 파일(툴 이름)**.

```
crates/domains/catalog/src/
├── lib.rs                  register(): 리소스 순서대로 모든 오퍼레이션, 도메인 에러 코드
├── offer/                  docs/api/catalog.md 의 `## offer`
│   ├── mod.rs              리소스 안에서 함께 쓰는 것(타입, 상태기계, SQL 도우미)
│   ├── set_price.rs        catalog.offer.set_price — 입력 타입, OpMeta, ReadOp/WriteOp, 계약 테스트
│   └── get_price_breakdown.rs
├── inventory/
│   ├── get_inventory.rs
│   └── set_variant_stock.rs
├── service/                for_{소비 도메인}.rs — 다른 도메인이 읽거나 부를 수 있는 것
└── repo.rs · model.rs · rules.rs   도메인 전체가 쓰는 SQL·모델·정책 읽기(선택)
```

- 오퍼레이션의 `OpMeta`는 `crates/domains/{domain}/src/{resource}/{tool}.rs`에 하나만 있습니다. 사람 전용 엔드포인트도 같습니다(`sales.gift_order.accept` → `sales/src/gift_order/accept_gift_order.rs`).
- `cargo xtask check-layout [domain…]`이 등록된 오퍼레이션마다 어디 있고 어디에 있어야 하는지 대조하고, `cargo xtask check-coverage [domain…]`이 명세의 툴과 사람 전용 엔드포인트가 모두 만들어졌는지 봅니다.
- 다른 도메인의 기능이 필요하면 그 도메인의 `service/for_{내 도메인}.rs`에 함수를 더합니다(테이블을 직접 읽지 않음).

---

## 3. 기술 스택

| 관심사 | 선택 | 메모 |
|---|---|---|
| 런타임·HTTP | tokio, axum 0.8, tower, tower-http | 미들웨어(추적·타임아웃·압축·CORS)는 tower 레이어 |
| 게이트웨이 프록시 | hyper-util 클라이언트 + axum | 스트리밍 바디 그대로 중계(SSE 포함) |
| DB | PostgreSQL 17, pgvector, sqlx 0.9 | 마이그레이션은 바이너리에 포함(`sqlx::migrate!`). 쿼리는 런타임 바인딩 + `FromRow`로 시작하고, DB를 띄우는 통합 단계에서 컴파일 시 검증(`query!`, 오프라인 캐시 `.sqlx/`)으로 옮김 |
| 직렬화 | serde, serde_json(`arbitrary_precision`) | 금액을 정확한 십진수로 읽고 씀 |
| 금액·시간 | rust_decimal, chrono, chrono-tz | float 금지 |
| 스키마 | schemars 1.x → 닫힌 JSON Schema 후처리 | [../api/conventions.md](../api/conventions.md) §툴 스키마 |
| OpenAPI | 레지스트리에서 직접 생성(xtask) | utoipa는 레지스트리에 안 맞으면 쓰지 않음 |
| MCP | rmcp (공식 Rust SDK), streamable HTTP | |
| 모델 호출 | reqwest + eventsource-stream | OpenAI 호환 API(vLLM, Meta Model API) |
| 인증 | jsonwebtoken(EdDSA), argon2(비밀번호) | 내부 토큰은 게이트웨이만 서명 |
| 레이트 리밋 | governor | 주체·테넌트 키 |
| 정책 YAML | serde_norway (serde_yaml 후속 포크) | serde_yaml은 유지보수 중단 |
| 관측성 | tracing, tracing-subscriber(JSON), OpenTelemetry(선택) | 요청 id = 액션 로그 상관 id |
| 테스트 | `#[sqlx::test]`(테스트마다 새 DB), insta(스냅샷) | testing.md |

MVP 인프라는 **PostgreSQL 하나**입니다. 멱등 키·락·작업 큐·검색(전문 검색 + pgvector)을 모두 Postgres로 처리합니다. Redis·OpenSearch·메시지 버스는 트레이트 뒤에 두고, 부하가 근거를 줄 때 붙입니다.

---

## 4. 설정과 로컬 실행

설정은 환경 변수(`ACOM_*`)와 `config/{local,demo}.toml`을 겹쳐 읽습니다. 비밀(`META_API_KEY`, 서명 키)은 환경 변수로만 받고 레포에 두지 않습니다.

| 변수 | 쓰는 곳 | 예 |
|---|---|---|
| `DATABASE_URL` | commerce, agent | `postgres://acom_app:acom_app@localhost:5432/acom` (RLS가 적용되는 서비스 역할) |
| `MIGRATION_DATABASE_URL` | `xtask db` | `postgres://acom_owner:acom_owner@localhost:5432/acom` (마이그레이션 역할, [database.md](database.md) §8) |
| `ACOM_GATEWAY_SIGNING_KEY_FILE` / `ACOM_GATEWAY_PUBLIC_KEY_FILE` | gateway / commerce·agent | Ed25519 PEM 파일 경로(`make keys`) |
| `ACOM_COMMERCE_URL`, `ACOM_AGENT_URL`, `ACOM_WEB_URL` | gateway, agent | `http://localhost:7001` |
| `ACOM_DEMO_MODE` | 전부 | `true`면 데모 로그인·데모 시계 활성 |
| `GLIMMER_BASE_URL`, `GLIMMER_MODEL` | agent | self-hosted vLLM의 OpenAI 호환 엔드포인트 |
| `META_API_KEY`, `META_API_BASE_URL`, `SPARK_MODEL` | agent | Meta Model API |

전체를 컨테이너로 띄우는 방법은 [../../infra/README.md](../../infra/README.md) §7(`make up`)입니다. 서비스 하나를 컨테이너 밖에서 고치며 돌릴 때:

```bash
make up-debug                     # 전체 스택 + Postgres :5432, 각 서비스 포트 공개
cargo run -p acom-commerce -- migrate   # MIGRATION_DATABASE_URL (스택이 이미 적용함)
cargo run -p acom-commerce -- serve     # :7001 (해당 컨테이너는 먼저 멈춤)
cargo run -p acom-commerce -- work      # :7003
cargo run -p acom-agent-svc             # :7002 (바이너리 acom-agent)
cargo run -p acom-gateway               # :7000 (웹·API 단일 출처)
cargo xtask codegen               # tools.json · openapi.json · ui-blocks.schema.json · web/src/contract/*.ts
cargo test --workspace            # 단위·계약 테스트 (DB 없이)
cargo test --workspace -- --ignored   # 통합 테스트 (DB 필요, 요청 시)
cargo xtask check-schema --write  # 마이그레이션 규칙 검사 + 컬럼 사전·어휘(vocab/acom.jsonld) 생성
cargo xtask check-seed            # 데모 시드가 스키마에 맞는지 (DB 없이)
```

---

## 5. 작업 규칙 (코드 리뷰 기준)

- 새 오퍼레이션은 [../api/](../api/README.md)에 먼저 있어야 합니다. 이름·주소·종류·REST 경로는 명세와 같아야 하고, `cargo xtask check-docs`가 레지스트리와 명세 인덱스를 대조합니다.
- 오퍼레이션 하나 = `Operation` 구현 하나 + 계약 테스트 하나 이상([commerce.md](commerce.md) §1, [testing.md](testing.md) §1).
- 툴 체크리스트는 [../../AGENTS.md](../../AGENTS.md)를 따릅니다(한 동사, 단어 enum, 필드 설명·예시, 프리뷰, consumes/produces, 실행 가능한 에러, 정책은 데이터).
- 마이그레이션은 되돌릴 수 있게 쓰고(확장 → 이전 → 축소), 테이블·컬럼 이름은 [../api/data-model.md](../api/data-model.md) §DB 명명을 따릅니다.
- `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`, `cargo xtask check-docs`, `cargo xtask check-schema`, `cargo xtask check-seed`, `cargo xtask check-layout`, `cargo xtask check-coverage`가 CI 게이트입니다.
