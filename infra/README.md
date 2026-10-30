# 인프라와 서빙

A-Commerce의 모든 구성 요소는 컨테이너이고, **어디서 띄우든 운영과 같은 구성**으로 뜹니다. 로컬 Docker Compose, 로컬 Kubernetes(kind), 운영 Kubernetes가 같은 이미지·같은 서비스 경계·같은 헬스 체크·같은 설정 이름을 쓰고, 달라지는 것은 내부 설정(DB 위치, 복제 수, 비밀의 출처, TLS)뿐입니다.

| 폴더 | 내용 |
|---|---|
| `infra/docker/` | 이미지: `rust.Dockerfile`(gateway·commerce·agent, 빌드 한 번에 대상별 이미지), `web.Dockerfile` |
| `infra/compose/` | Docker Compose: `compose.yaml`(운영과 같은 구성), `compose.debug.yaml`(포트 공개), `observability/` |
| `infra/k8s/` | Kubernetes(Kustomize): `base/`, `components/`, `overlays/{local,production}`, `kind/`. 자세한 것은 [k8s/README.md](k8s/README.md) |
| `infra/postgres/` | DB 역할을 만드는 `init.sql`(Compose·Kubernetes 공용)과 로컬 클러스터용 Postgres 컴포넌트 |
| `infra/scripts/` | `gen-keys.sh`(게이트웨이 서명 키), `kind-up.sh` / `kind-down.sh` |
| `infra/secrets/` | 로컬 비밀(git 무시). 운영 비밀은 여기 두지 않음 |

명령은 저장소 루트의 `Makefile` 하나로 모읍니다(`make help`).

---

## 1. 원칙

1. **구성 요소마다 컨테이너 하나, 이미지는 어디서나 같음.** 로컬에서 쓴 이미지를 그대로 운영에 올립니다. 환경마다 다른 것은 환경 변수와 비밀입니다(12-factor).
2. **상태는 PostgreSQL 하나에만.** gateway·commerce·agent·web은 상태가 없어 수평 확장되고, 어느 복제본이 요청을 받아도 같습니다. 예약 작업은 `commerce-worker`가 `FOR UPDATE SKIP LOCKED`로 나눠 가지므로 여러 개를 띄워도 안전합니다.
3. **들어오는 길은 게이트웨이 하나.** 밖에 열리는 포트는 게이트웨이뿐이고, 나머지는 내부 네트워크에서만 서로 봅니다. 사람의 결정(`human:*`)과 에이전트의 권한을 가르는 신뢰 경계도 여기서 시작합니다([../docs/backend/gateway.md](../docs/backend/gateway.md)).
4. **마이그레이션은 배포 전에 한 번.** `acom-commerce migrate`가 소유자 역할로 적용하고(동시 실행은 advisory lock으로 안전), 서비스는 마이그레이션이 모두 적용되기 전까지 준비되지 않음(`/readyz` 503)으로 보고합니다.
5. **모든 서비스가 같은 운영 표면.** `/healthz`(살아 있음), `/readyz`(받을 수 있음), `/metrics`(Prometheus), JSON 로그, `x-request-id`, SIGTERM에서 진행 중 요청을 마치고 종료([../crates/server](../crates/server/src/lib.rs)).
6. **최소 권한.** 비루트 사용자, 읽기 전용 루트 파일 시스템, 모든 커널 권한 제거, DB는 서비스 역할(`acom_app`, RLS 적용)과 마이그레이션 역할(`acom_owner`)을 분리.

## 2. 구성 요소

| 구성 요소 | 이미지 | 명령 | 포트 | 하는 일 | 확장 |
|---|---|---|---|---|---|
| **gateway** | `acom-gateway` | (기본) | 7000 | 유일한 입구. `/api/v1/*`·`/mcp` → commerce, `/api/chat/*` → agent(SSE 스트리밍), 나머지 → web. 세션·스코프·내부 토큰은 M3 | 수평 |
| **web** | `acom-web` | `node server.js` | 3000 | Next.js(Meta Astryx). `/`(스토어프론트), `/bo`(백오피스) | 수평 |
| **commerce** | `acom-commerce` | `serve` | 7001 | 툴 REST·MCP·사람 전용 엔드포인트, 정책 엔진, 변경셋 | 수평 |
| **commerce-worker** | `acom-commerce` | `work` | 7003(헬스·지표) | 예약 작업, 아웃박스 | 수평(작업을 잠금으로 나눔) |
| **agent** | `acom-agent` | (기본) | 7002 | 대화 턴, 툴 검색·계획·실행, 모델 호출 | 수평 |
| **migrate** | `acom-commerce` | `migrate` | — | 마이그레이션 적용 후 종료(Compose 서비스 / Kubernetes Job) | 한 번 |
| **postgres** | `pgvector/pgvector:pg17` | — | 5432 | 유일한 상태 저장소. 운영은 관리형 Postgres | — |
| models (선택) | `vllm/vllm-openai` | `--model …` | 8000 | 작업 모델(Glimmer)을 OpenAI 호환 API로. GPU 필요 | GPU 수 |
| prometheus·grafana (선택) | 공식 이미지 | — | 9090·3000 | 지표 수집과 화면 | — |

## 3. 요청 흐름과 네트워크

```
                    브라우저 · 외부 에이전트
                              │ :8080 (Compose) / Ingress·Gateway API (Kubernetes)
  ┌─ edge ────────────────────▼──────────────────────────────────────────────────┐
  │                        gateway :7000                                          │
  └──────┬───────────────────┬────────────────────────┬─────────────────────────┘
  ┌─ app (내부) ─────────────┼────────────────────────┼─────────────────────────┐
  │      ▼ /*                ▼ /api/v1/*, /mcp        ▼ /api/chat/* (SSE)       │
  │   web :3000         commerce :7001  ◀──────── agent :7002 ──┐               │
  │                     commerce-worker :7003                    │               │
  └─────────────────────────┬────────────────────────────────────┼──────────────┘
  ┌─ data (내부) ────────────▼────────────────────────────────────▼──────────────┐
  │                 postgres :5432   ◀── migrate (배포 전 한 번)                   │
  └──────────────────────────────────────────────────────────────────────────────┘
                                                    agent ──▶ egress: 모델 엔드포인트
```

- Compose는 이 영역을 네트워크로(`edge`, `app`·`data`는 인터넷 없는 내부 망, agent만 `egress`), Kubernetes는 NetworkPolicy로 같은 규칙을 만듭니다.
- 웹의 서버 컴포넌트는 `ACOM_GATEWAY_INTERNAL_URL`(게이트웨이)로 API를 부르고, 브라우저는 같은 출처의 `/api/...`를 부릅니다. 쿠키·CSRF가 CORS 없이 동작합니다.

## 4. 이미지

| 이미지 | Dockerfile | 기반 | 사용자 | 메모 |
|---|---|---|---|---|
| `acom-gateway`, `acom-commerce`, `acom-agent` | `docker/rust.Dockerfile`(`--target gateway\|commerce\|agent`) | `gcr.io/distroless/cc-debian12:nonroot` | 65532 | 빌드 단계 하나에서 세 바이너리를 같이 컴파일(BuildKit 캐시). 셸이 없으므로 컨테이너 헬스 체크는 `<binary> healthcheck` |
| `acom-web` | `docker/web.Dockerfile` | `node:24-bookworm-slim` | `node`(1000) | Next.js `standalone` 출력, `node server.js` |

- 태그: 로컬 `acom/acom-<이름>:local`, 운영 `ghcr.io/metahackai/a-commerce/acom-<이름>:<버전 또는 git sha>`.
- `commerce` 이미지에는 정책 파일(`/app/policies`)이 들어 있고, 마이그레이션은 바이너리에 내장됩니다.
- 빌드 컨텍스트는 저장소 루트이고 `.dockerignore`가 문서·시안·비밀을 뺍니다.

## 5. 설정과 비밀

설정은 환경 변수, 비밀은 파일(또는 Kubernetes Secret)입니다. 이름은 모든 환경에서 같습니다.

| 이름 | 서비스 | 로컬 Compose | 운영 |
|---|---|---|---|
| `ACOM_*_ADDR` | 각 서비스 | `0.0.0.0:<포트>` | 같음 |
| `ACOM_COMMERCE_URL`, `ACOM_AGENT_URL`, `ACOM_WEB_URL` | gateway, agent | 서비스 이름(`http://commerce:7001`) | 같음 |
| `DATABASE_URL` | commerce, worker, agent | `acom_app@postgres` | 관리형 Postgres(비밀) |
| `MIGRATION_DATABASE_URL` | migrate | `acom_owner@postgres` | 관리형 Postgres(비밀) |
| `ACOM_POLICY_DIR` | commerce | 이미지 안 `/app/policies` | 같음 |
| `ACOM_GATEWAY_SIGNING_KEY_FILE` / `_PUBLIC_KEY_FILE` | gateway / 나머지 | `infra/secrets/*.pem`(`make keys`) | 비밀 저장소 |
| `META_API_KEY`, `META_API_BASE_URL`, `SPARK_MODEL`, `GLIMMER_BASE_URL`, `GLIMMER_MODEL` | agent | `.env` | 비밀 저장소 |
| `ACOM_ENV`, `ACOM_DEMO_MODE`, `RUST_LOG`, `ACOM_LOG_FORMAT` | 전부 | `local`, `true`, `info`, `json` | `production`, `false`, `info`, `json` |

## 6. 운영 표면

| 경로 | 뜻 | 쓰는 곳 |
|---|---|---|
| `/healthz` | 프로세스가 살아 있음 | Compose 헬스 체크, Kubernetes liveness |
| `/readyz` | 요청을 받을 수 있음. commerce·worker·agent는 DB 연결과 마이그레이션 적용까지 확인 | Kubernetes readiness |
| `/metrics` | `http_requests_total`, `http_request_duration_seconds` 등 | Prometheus(파드 주석, 선택 컴포넌트의 ServiceMonitor) |

로그는 한 줄 JSON이고 요청마다 `request_id`가 있습니다. 게이트웨이가 만든(또는 받은) `x-request-id`가 commerce·agent 로그와 액션 로그까지 이어집니다.

## 7. Docker Compose

```bash
make up            # .env와 키가 없으면 만들고, 이미지를 빌드하고, 마이그레이션 후 전부 시작
                   # → http://localhost:8080  (스토어프론트 /, 백오피스 /bo)
make ps            # 상태와 헬스
make logs SERVICE=commerce
make up-debug      # Postgres :5432와 각 서비스 포트(7001·7002·7003·3000)도 열고, 읽기 쉬운 로그
make up-models     # GPU로 작업 모델(vLLM)도 실행 (.env의 VLLM_MODEL)
make up-observability   # Prometheus :9090, Grafana :3001
make down          # 멈춤(DB 볼륨 유지) / make clean: 볼륨까지 삭제
```

- 시작 순서: `postgres`(healthy) → `migrate`(성공 종료) → `commerce`·`commerce-worker`·`agent` → `web`·`gateway`.
- 컨테이너는 읽기 전용 루트 파일 시스템, 권한 제거, `no-new-privileges`로 뜹니다(운영과 같음).
- 시드 데이터 적재(`seed` 작업)는 M2에서 같은 방식(한 번 실행되는 서비스)으로 추가합니다.

## 8. Kubernetes

Kustomize입니다(`kubectl`에 내장, Helm 불필요). 구조와 세부는 [k8s/README.md](k8s/README.md).

- `base/`: 네임스페이스, 설정, 서비스 계정, 각 구성 요소의 Deployment·Service, 마이그레이션 Job, Ingress, NetworkPolicy.
- `components/`: 선택 기능을 켜는 조각. `production`(HPA·PDB·영역 분산), `monitoring`(ServiceMonitor), `models-vllm`(GPU 모델 서버), `gateway-api`(Ingress 대신 Gateway API).
- `overlays/local`: kind용. 복제 1, 로컬 이미지, 클러스터 안 Postgres(`infra/postgres`), 로컬 비밀 파일에서 Secret 생성.
- `overlays/production`: 복제 2 이상, 레지스트리 이미지, 관리형 Postgres, TLS, 더 큰 자원, 외부 비밀 관리자(External Secrets 등)가 만든 Secret 사용.
- **입구:**
  - 특정 컨트롤러에 묶이지 않습니다(upstream ingress-nginx는 은퇴). Ingress의 `ingressClassName`은 오버레이에서 정합니다. production 예시는 `traefik`이고, 대안으로 `components/gateway-api`(Gateway API)를 씁니다.
  - 컨트롤러가 있는 네임스페이스에 `acom.dev/ingress: "true"` 라벨을 달아야 NetworkPolicy가 gateway:7000으로 들어오는 것을 허용합니다.
  - 로컬 kind는 컨트롤러 없이 `port-forward`로 들어갑니다.
- **보안 기준:** 네임스페이스는 Pod Security `restricted`이고, 모든 파드가 비루트·읽기 전용 루트·권한 제거로 뜹니다.
- **마이그레이션 Job `migrate`:** Argo CD를 쓰면 `PreSync` 훅으로 배포 전에 실행됩니다. `kubectl apply`만 쓸 때는 서비스가 마이그레이션이 끝날 때까지 `/readyz`에서 준비 안 됨으로 남습니다.

```bash
make kind-up       # kind 클러스터 + 이미지 빌드·적재 + 로컬 오버레이 적용
                   # → kubectl -n a-commerce port-forward svc/gateway 8080:7000 후 http://localhost:8080
make k8s-render    # 로컬 오버레이 출력 (OVERLAY=production)
make check-infra   # Compose 검사 + 두 오버레이 렌더링 + 스키마 검사(클러스터 없이)
make kind-down
```

## 9. 환경별 차이

| | Compose (로컬) | kind (로컬 Kubernetes) | 운영 Kubernetes |
|---|---|---|---|
| 이미지 | `acom/*:local` 로컬 빌드 | 같은 이미지를 kind에 적재 | 레지스트리의 버전 태그 |
| 복제 | 1 | 1 | 2 이상 + HPA, PDB |
| Postgres | 컨테이너 | 클러스터 안 StatefulSet | 관리형 Postgres |
| 입구 | `localhost:8080` → gateway | port-forward → gateway | Ingress 또는 Gateway API + TLS |
| 비밀 | `infra/secrets/`, `.env` | 로컬 파일에서 생성한 Secret | 외부 비밀 관리자 |
| 모델 | 외부 엔드포인트 또는 `models` 프로필 | 외부 또는 `models-vllm` 컴포넌트 | 외부 또는 GPU 노드의 `models-vllm` |
| 지표 | `observability` 프로필 | 파드 주석 | `monitoring` 컴포넌트 |
| 데모 모드 | 켬 | 켬 | 끔 |

## 10. 검사

DB를 띄우거나 서비스를 함께 돌리지 않는 검사는 항상 돌립니다.
- `make check-infra`: Compose 파일 검사, 두 오버레이 렌더링, Kubernetes 스키마 검사(kubeconform).
- `make images`: 이미지가 빌드되는지.

스택을 실제로 띄워 보는 확인(`make up` 후 요청, kind)은 통합 테스트로 분류하고, 요청이 있을 때 돌립니다.
