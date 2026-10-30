# infra/k8s — Kubernetes 매니페스트 (Kustomize)

A-Commerce의 컨테이너 서비스(gateway, commerce, commerce-worker, agent, web)와 마이그레이션 Job을
Kubernetes에 올리는 Kustomize 매니페스트입니다. `base`는 환경과 무관한 공통 정의, `components`는
켜고 끄는 선택 기능, `overlays`는 환경(local, production)별 차이만 담습니다.

## 파일 구성

```
infra/k8s/
  README.md                  이 문서
  validate.sh                클러스터 없이 렌더링 + 스키마 검사 (아래 "렌더링과 검증")
  secrets.example.yaml       Secret 이름과 키를 설명하는 예시. 어떤 kustomization에도 들어가지 않음
  base/
    kustomization.yaml       base 목록, namespace a-commerce
    namespace.yaml           Namespace (Pod Security "restricted" 강제)
    configmap.yaml           acom-config: 모든 앱 컨테이너가 envFrom으로 읽는 설정
    serviceaccounts.yaml     컴포넌트마다 하나씩, 토큰 자동 마운트 없음
    gateway.yaml             Deployment + Service (7000)
    commerce.yaml            Deployment + Service (7001), `serve`
    commerce-worker.yaml     Deployment + Service (7003, health/metrics 전용), `work`
    agent.yaml               Deployment + Service (7002)
    web.yaml                 Deployment + Service (3000), Next.js standalone
    migrate-job.yaml         마이그레이션 Job, `migrate` (Argo CD PreSync 훅)
    ingress.yaml             Ingress: 모든 경로 / → gateway:7000 (class는 오버레이에서 지정)
    networkpolicies.yaml     기본 ingress 차단 + 허용 경로
  components/
    production/              HPA, PodDisruptionBudget, 존/노드 분산 (topologySpreadConstraints)
    monitoring/              ServiceMonitor 4개 (Prometheus Operator가 있는 클러스터용)
    models-vllm/             GPU에서 vLLM으로 Glimmer 워커 모델 서빙 (Service `models`:8000)
    gateway-api/             Ingress 대신 Gateway API: Gateway(포트 80) + HTTPRoute / → gateway:7000
  overlays/
    local/                   kind용: 로컬 이미지, replica 1, 클러스터 내 PostgreSQL, 데모 모드, Ingress 없음
      patches/configmap.yaml   ACOM_ENV=local, ACOM_DEMO_MODE=true
      secrets/                 git에 올리지 않는 로컬 시크릿 파일 (*.example만 커밋)
    production/              운영: 릴리스 이미지, replica 2, TLS, 큰 리소스, 외부 관리형 DB
      patches/ingress.yaml     TLS(acom-tls) + cert-manager 어노테이션 (class·host는 kustomization의 JSON 패치)
      patches/resources.yaml   운영 리소스 크기
  kind/cluster.yaml          kind 클러스터 설정 (노드 하나, 인그레스 컨트롤러 없음)

infra/postgres/
  kustomization.yaml         로컬용 PostgreSQL Component (kind: Component)
  postgres.yaml              StatefulSet + Service + ServiceAccount + NetworkPolicy
  init.sql                   역할(acom_owner, acom_app, acom_readonly) 생성. ConfigMap으로 생성되어
                             /docker-entrypoint-initdb.d에 마운트됨
```

## base, components, overlays의 관계

- **base**는 모든 워크로드를 정의하지만 Secret 값은 하나도 갖지 않고 이름으로 참조만 합니다.
  이미지는 `ghcr.io/metahackai/a-commerce/<이미지>:latest`입니다.
- **components**(`kind: Component`)는 오버레이가 `components:`에 적을 때만 들어가는 선택 기능입니다.
  리소스를 추가하고(HPA, ServiceMonitor, vLLM) base 객체를 패치(분산 제약, migrate 대기)합니다.
- **overlays**는 base에 컴포넌트를 더하고 환경 차이만 바꿉니다: 이미지 태그(`images:`), replica 수,
  Ingress class와 호스트, 리소스, ConfigMap 값, 로컬 시크릿 생성.

| 오버레이 | 포함하는 컴포넌트 | 렌더링 결과 |
|---|---|---|
| `overlays/local` | `infra/postgres` | 34개 객체: Deployment 5, StatefulSet 1, Job 1, Service 6, ServiceAccount 7, ConfigMap 2, Secret 4, NetworkPolicy 7, Namespace 1 (Ingress는 `$patch: delete`로 뺌) |
| `overlays/production` | `components/production`, `components/monitoring` | 39개 객체: Deployment 5, Job 1, Service 5, ServiceAccount 6, ConfigMap 1, NetworkPolicy 6, HPA 4, PDB 5, ServiceMonitor 4, Ingress 1, Namespace 1 |

`components/models-vllm`과 `components/gateway-api`는 기본으로 켜져 있지 않습니다(아래 "선택 컴포넌트").
production의 kustomization에는 gateway-api 줄이 주석으로 들어 있습니다.

이름(스크립트가 쓰는 것): Deployment `gateway`, `commerce`, `commerce-worker`, `agent`, `web`,
Job `migrate`, local의 StatefulSet `postgres`, 모두 namespace `a-commerce`.

## 서비스 요약

| 컴포넌트 | 이미지 | 인자 | 포트 | 프로브 | 사용자 |
|---|---|---|---|---|---|
| gateway | acom-gateway | (entrypoint) | 7000 | liveness `/healthz`, readiness `/readyz` | 65532 |
| commerce | acom-commerce | `serve` | 7001 | `/healthz`, `/readyz` (DB 연결 + 마이그레이션 완료 시에만 ready) | 65532 |
| commerce-worker | acom-commerce | `work` | 7003 (health/metrics만) | `/healthz`, `/readyz` | 65532 |
| agent | acom-agent | (entrypoint) | 7002 | `/healthz`, `/readyz` | 65532 |
| web | acom-web | (`node server.js`) | 3000 | startup + liveness + readiness `/healthz` | 1000 |
| migrate (Job) | acom-commerce | `migrate` | — | — | 65532 |

- Rust 서비스는 같은 포트의 `/metrics`로 Prometheus 지표를 내고, 파드에
  `prometheus.io/scrape|port|path` 어노테이션이 붙어 있습니다.
- 모든 파드: `automountServiceAccountToken: false`, 자기 ServiceAccount, `runAsNonRoot`,
  `seccompProfile: RuntimeDefault`, 컨테이너는 `allowPrivilegeEscalation: false`,
  `readOnlyRootFilesystem: true`, `capabilities.drop: [ALL]`. 쓰기가 필요한 곳은 emptyDir입니다
  (web: `/app/.next/cache`, `/tmp`; Rust 서비스: `/tmp`). Namespace는 Pod Security `restricted`를 강제합니다.
- `enableServiceLinks: false`: Kubernetes가 Service마다 넣는 환경 변수(`WEB_PORT=...` 등)를 막아
  설정은 acom-config와 Secret에서만 옵니다.

## 렌더링과 검증

```bash
# 렌더링 (kubectl 1.27+에 들어 있는 kustomize v5, 또는 kustomize 단독 실행 파일)
kubectl kustomize infra/k8s/overlays/local
kustomize build infra/k8s/overlays/production

# 렌더링 + kubeconform 스키마 검사 (클러스터 불필요). make check-infra도 이것을 부름
infra/k8s/validate.sh
KUSTOMIZE=~/bin/kustomize KUBECONFORM=~/bin/kubeconform infra/k8s/validate.sh
```

- local 오버레이는 `secrets/` 아래 파일로 Secret을 만들기 때문에, 그 파일이 없으면 렌더링이 실패합니다
  (아래 "로컬 시크릿 준비").
- `validate.sh`는 `infra/k8s`와 `infra/postgres`를 임시 폴더에 복사하고, 거기에 `*.example`로 로컬 시크릿
  파일을 채운 뒤(PEM은 자리표시자) base, local, production, local + models-vllm,
  production + gateway-api(Ingress 삭제) 다섯 가지를 렌더링해 `kubeconform -strict`(Kubernetes 1.31.0
  스키마)로 검사하고, `secrets.example.yaml`도 검사합니다. 실제 로컬 시크릿은 읽지 않으며 작업 트리를
  바꾸지 않습니다. ServiceMonitor, Gateway, HTTPRoute 같은 CRD는 공개 CRD 스키마 카탈로그로 검사하며,
  오프라인이면 `CRD_SCHEMAS=""`로 CRD를 건너뜁니다. `OUT_DIR=<폴더>`를 주면 렌더링 결과를 남깁니다.

## 시크릿

base에는 값이 든 Secret이 없습니다. 이름과 키는 `secrets.example.yaml`에 설명되어 있습니다.

| Secret | 키 | 쓰는 곳 |
|---|---|---|
| `acom-database` | `DATABASE_URL` (역할 acom_app), `MIGRATION_DATABASE_URL` (역할 acom_owner) | commerce, commerce-worker, agent는 `DATABASE_URL`; migrate Job은 `MIGRATION_DATABASE_URL` |
| `acom-gateway-keys` | 파일 `gateway_signing_key.pem`, `gateway_public_key.pem` | `/run/secrets/acom`에 읽기 전용 마운트. gateway는 둘 다, commerce/worker/agent는 공개 키만 |
| `acom-models` | `META_API_KEY`, `META_API_BASE_URL`, `SPARK_MODEL`, `GLIMMER_BASE_URL`, `GLIMMER_MODEL` | agent만. 모든 키가 `optional: true`라 없어도 agent가 뜸 |
| `acom-postgres` | `POSTGRES_PASSWORD` | 로컬 PostgreSQL 컴포넌트만 |
| `acom-tls` | TLS 인증서 | production Ingress. cert-manager가 만듦 |

### 로컬 시크릿 준비

`overlays/local/secrets/`의 파일은 `.gitignore`로 커밋되지 않습니다(`*.example`만 커밋).
local 오버레이의 `secretGenerator`가 이 파일로 Secret을 만들며, 이름에 해시를 붙이지 않아
base가 참조하는 이름과 같습니다. 필요한 파일은 다섯 개입니다:

| 파일 (`infra/k8s/overlays/local/secrets/`) | 만들어지는 Secret | 예시 |
|---|---|---|
| `database.env` | `acom-database` | `database.env.example` |
| `models.env` | `acom-models` | `models.env.example` |
| `postgres.env` | `acom-postgres` | `postgres.env.example` |
| `gateway_signing_key.pem` | `acom-gateway-keys` | `infra/scripts/gen-keys.sh`가 `infra/secrets/`에 만듦 |
| `gateway_public_key.pem` | `acom-gateway-keys` | 위와 같음 |

```bash
cd infra/k8s/overlays/local/secrets
cp database.env.example database.env   # init.sql의 로컬 역할/비밀번호와 맞춰져 있음
cp postgres.env.example postgres.env
cp models.env.example models.env       # 가진 모델 키만 주석을 풀어 채움 (없어도 됨)
cd -
infra/scripts/gen-keys.sh              # (make keys) infra/secrets/에 Ed25519 키 쌍 생성
cp infra/secrets/gateway_signing_key.pem infra/secrets/gateway_public_key.pem \
   infra/k8s/overlays/local/secrets/
```

kustomize는 kustomization 폴더 밖의 파일(심볼릭 링크 포함)을 읽지 않으므로 키 파일은 복사합니다.

### 운영 시크릿

운영에서는 저장소에 시크릿 파일을 두지 않습니다. 위 표의 Secret을 같은 이름과 키로 클러스터에 만들어 두면
됩니다. 권장 방식:

- **External Secrets Operator**: 클라우드 시크릿 저장소의 값을 `ExternalSecret`으로 동기화합니다.
  `target.name`을 `acom-database` 등으로 지정합니다.
- **sealed-secrets**: 암호화된 `SealedSecret`은 커밋해도 되므로 GitOps 저장소에 둘 수 있습니다.

```yaml
apiVersion: external-secrets.io/v1
kind: ExternalSecret
metadata:
  name: acom-database
  namespace: a-commerce
spec:
  secretStoreRef: {kind: ClusterSecretStore, name: <store>}
  target: {name: acom-database}
  data:
    - secretKey: DATABASE_URL
      remoteRef: {key: <path>/database-url}
    - secretKey: MIGRATION_DATABASE_URL
      remoteRef: {key: <path>/migration-database-url}
```

## 마이그레이션 순서

1. **migrate Job**(`acom-commerce migrate`)이 소유자 역할 acom_owner(`MIGRATION_DATABASE_URL`)로
   마이그레이션을 적용하고 끝납니다. `backoffLimit: 3`, 끝난 Job은 600초 뒤 자동 삭제됩니다.
2. commerce, commerce-worker, agent의 `/readyz`는 DB에 연결되고 모든 마이그레이션이 적용됐을 때만
   성공합니다. 그 전까지 파드는 떠 있되 **unready**이므로 Service 트래픽을 받지 않고, 롤링 업데이트도
   `maxUnavailable: 0`이라 기존 파드를 내리지 않습니다. liveness(`/healthz`)는 DB와 무관해서 재시작
   루프에 빠지지 않습니다.

**Argo CD**: Job에 `argocd.argoproj.io/hook: PreSync`가 붙어 있어 동기화 때 Deployment보다 먼저 실행되고,
성공해야 나머지가 적용됩니다. `hook-delete-policy: BeforeHookCreation`으로 다음 동기화 전에 이전 Job을
지우고 새로 만듭니다. PreSync는 일반 리소스보다 먼저 돌기 때문에 첫 동기화를 위해

- migrate ServiceAccount도 PreSync 훅(sync-wave -1)으로 표시해 Job보다 먼저 만들어지게 했고,
- Job의 `acom-config` 참조는 `optional: true`라 ConfigMap이 아직 없어도 시작합니다.
- `acom-database` Secret은 동기화 전에 있어야 합니다(별도 Application의 ExternalSecret 등).
  Application에는 `syncOptions: [CreateNamespace=true]`를 줍니다.

새 버전의 마이그레이션이 도는 동안 이전 버전 파드가 계속 서비스하므로, 마이그레이션은 이전 코드와도 호환되게
(컬럼 추가 → 코드 전환 → 정리 순서로) 작성합니다.

**kubectl apply -k**: 모든 객체가 한 번에 만들어집니다. local에서는 PostgreSQL 컴포넌트가 migrate Job에
`wait-for-postgres` init 컨테이너를 붙여, DB가 init.sql까지 끝내고 접속을 받을 때까지(최대 5분) 기다린 뒤
마이그레이션을 시작합니다. Job의 pod template은 바꿀 수 없으므로 같은 Job을 다시 돌리려면 먼저 지웁니다:

```bash
kubectl -n a-commerce delete job migrate --ignore-not-found
kubectl apply -k infra/k8s/overlays/local
```

## 로컬(kind) 흐름

`make kind-up`(`infra/scripts/kind-up.sh`)이 아래 과정을 한 번에 하고, `make kind-down`
(`infra/scripts/kind-down.sh`)이 클러스터를 지웁니다. 로컬 클러스터에는 인그레스 컨트롤러를 두지 않고
gateway Service로 port-forward해서 접속합니다(local 오버레이는 Ingress를 `$patch: delete`로 뺍니다).
손으로 하면:

```bash
# 1. 클러스터 (노드 하나)
kind create cluster --config infra/k8s/kind/cluster.yaml

# 2. 이미지 빌드 (저장소 루트에서, infra/docker의 Dockerfile)
docker build -f infra/docker/rust.Dockerfile --target gateway  -t acom/acom-gateway:local  .
docker build -f infra/docker/rust.Dockerfile --target commerce -t acom/acom-commerce:local .
docker build -f infra/docker/rust.Dockerfile --target agent    -t acom/acom-agent:local    .
docker build -f infra/docker/web.Dockerfile                    -t acom/acom-web:local      .

# 3. 이미지를 kind 노드에 넣기 (local 오버레이는 imagePullPolicy: IfNotPresent)
kind load docker-image --name a-commerce \
  acom/acom-gateway:local acom/acom-commerce:local acom/acom-agent:local acom/acom-web:local

# 4. 로컬 시크릿 준비 (위 "로컬 시크릿 준비")

# 5. 배포하고 기다리기
kubectl apply -k infra/k8s/overlays/local
kubectl -n a-commerce wait --for=condition=complete job/migrate --timeout=10m
kubectl -n a-commerce rollout status deploy/commerce deploy/commerce-worker deploy/agent deploy/web deploy/gateway

# 6. 접속: http://localhost:8080
kubectl -n a-commerce port-forward svc/gateway 8080:7000
```

코드를 바꾼 뒤에는 2–3단계를 다시 하고 `kubectl -n a-commerce rollout restart deploy`로 파드를 새 이미지로
바꿉니다. 스키마가 바뀌었으면 위의 migrate Job 재실행도 합니다.

port-forward는 kubelet을 거쳐 파드 안으로 바로 들어가므로 NetworkPolicy와 무관하게 동작합니다.
NetworkPolicy는 CNI가 지원할 때만 실제로 적용됩니다. 정책 자체를 시험하려면 NetworkPolicy를 지원하는 CNI로
클러스터를 만듭니다.

## local과 production의 차이

| 항목 | local | production |
|---|---|---|
| 이미지 | `acom/<이미지>:local`, `imagePullPolicy: IfNotPresent` (kind load) | `ghcr.io/metahackai/a-commerce/<이미지>:0.1.0` (자리표시자; `digest:`로 고정 권장) |
| replica | 1 | 2 + HPA (gateway, commerce, agent, web: CPU 70%, 2–6) |
| 가용성 | — | PDB `minAvailable: 1` (5개 Deployment), 존/노드 분산 |
| 데이터베이스 | 클러스터 내 PostgreSQL (`infra/postgres` 컴포넌트, PVC 5Gi) | 외부 관리형 PostgreSQL. 컴포넌트 없음 |
| 시크릿 | `secretGenerator`로 로컬 파일에서 생성 | 미리 존재해야 함 (External Secrets / sealed-secrets). 생성하지 않음 |
| 입구 | Ingress 없음. `port-forward svc/gateway 8080:7000` | Ingress class `traefik`(예시), `shop.example.com`, TLS `acom-tls`, `cert-manager.io/cluster-issuer: letsencrypt`. 또는 gateway-api 컴포넌트 |
| 설정 | `ACOM_ENV=local`, `ACOM_DEMO_MODE=true` | `ACOM_ENV=production`, `ACOM_DEMO_MODE=false` (base 그대로) |
| 리소스 | base의 작은 값 | `patches/resources.yaml`의 큰 값 |
| 모니터링 | 파드 어노테이션만 | + ServiceMonitor (monitoring 컴포넌트) |

운영 시 확인할 점:

- HPA에는 metrics-server가 필요합니다. Argo CD로 배포하면 HPA가 바꾼 replica를 되돌리지 않도록
  Deployment의 `/spec/replicas`를 `ignoreDifferences`에 넣습니다.
- commerce-worker도 replica 2입니다. 워커 작업은 여러 인스턴스가 동시에 돌아도 안전해야 합니다
  (DB 잠금으로 한 작업을 한 워커만 가져가기). 그렇지 않으면 워커 replica를 1로 줄입니다.
- 관리형 DB에는 init.sql과 같은 역할 구성(acom_owner, acom_app, acom_readonly와 권한, `vector` 확장)을
  DB 관리자가 실제 비밀번호로 만들어 둡니다. 접속 URL에는 `sslmode=require`를 붙입니다.
- egress는 막지 않습니다(agent가 외부 모델 API를 호출). egress를 제한하려면 DNS, DB, 모델 엔드포인트를
  허용해야 합니다.

## NetworkPolicy

namespace 전체에 기본 ingress 차단(`default-deny-ingress`)을 두고 필요한 경로만 엽니다.

| 출발 | 도착 | 정책 |
|---|---|---|
| 라벨 `acom.dev/ingress: "true"`가 붙은 namespace (인그레스 컨트롤러 / Gateway API 프록시) | gateway:7000 | `gateway-ingress` |
| web | gateway:7000 (서버 측 렌더링이 `ACOM_GATEWAY_INTERNAL_URL` 호출) | `gateway-ingress` |
| gateway | web:3000, commerce:7001, agent:7002 | `web-ingress`, `commerce-ingress`, `agent-ingress` |
| agent | commerce:7001 | `commerce-ingress` |
| namespace `monitoring` | gateway, commerce, commerce-worker, agent의 `http` 포트(/metrics) | `metrics-from-monitoring` |
| commerce, commerce-worker, agent, migrate | postgres:5432 (local) | `postgres-ingress` (infra/postgres) |
| agent | models:8000 (models-vllm) | `models-ingress` (components/models-vllm) |

## 입구: Ingress 또는 Gateway API

특정 컨트롤러에 묶이지 않도록 컨트롤러별 설정은 오버레이에 둡니다.

- **Ingress**: `base/ingress.yaml`의 `ingressClassName: nginx`는 자리표시자입니다. 오버레이에서 쓰는
  컨트롤러의 class(Traefik, HAProxy, F5 NGINX 등)로 바꿉니다. production은 예시로 `traefik`을 씁니다.
- **Gateway API**: `components/gateway-api`를 켜고 Ingress를 지웁니다(아래 "선택 컴포넌트").
- 어느 쪽이든 **컨트롤러(또는 Gateway 프록시)가 도는 namespace에 라벨을 붙여야** gateway:7000에 닿습니다:

  ```bash
  kubectl label namespace <컨트롤러 namespace> acom.dev/ingress=true
  ```

- 채팅 턴은 SSE로 스트리밍됩니다. 컨트롤러가 응답을 버퍼링하거나 유휴 업스트림 연결을 일찍 끊는다면,
  그 컨트롤러의 어노테이션이나 정책으로 버퍼링을 끄고 읽기 타임아웃을 늘립니다(오버레이에서 패치).

## 선택 컴포넌트

### components/models-vllm

GPU 노드(NVIDIA device plugin)에서 vLLM이 Glimmer 워커 모델을 OpenAI 호환 API로 서빙합니다.
Deployment/Service `models`(8000), 모델 캐시 PVC `models-cache`(100Gi), ConfigMap `models-config`.
`nvidia.com/gpu: 1`을 요청하고, 첫 시작 때 모델을 내려받으므로 startup 프로브가 최대 30분을 기다립니다.

1. 오버레이의 `components:`에 `- ../../components/models-vllm`을 추가합니다.
2. ConfigMap `models-config`의 `VLLM_MODEL`(기본값 `set-me`)을 실제 Hugging Face 모델 id로 패치합니다.
3. Component는 Secret 안의 값을 바꿀 수 없으므로, **`acom-models` Secret의 `GLIMMER_BASE_URL`을
   `http://models:8000/v1`로**, `GLIMMER_MODEL`을 `VLLM_MODEL`과 같은 id로 직접 설정합니다
   (local이면 `models.env`). 접근 제한 모델이면 같은 Secret에 `HF_TOKEN`을 넣습니다(models 파드만 읽음).

```yaml
# 오버레이 kustomization.yaml에 추가하는 예
components:
  - ../../components/models-vllm
patches:
  - target: {kind: ConfigMap, name: models-config}
    patch: |-
      - op: replace
        path: /data/VLLM_MODEL
        value: <org>/<model>
```

### components/gateway-api

Ingress 대신 Gateway API로 입구를 만듭니다. Gateway `a-commerce`(HTTP 리스너, 포트 80)와 HTTPRoute
`a-commerce`(경로 `/` 전부 → Service gateway:7000). Gateway API CRD와 이를 구현하는 컨트롤러가 필요합니다.

- `gatewayClassName: eg`는 자리표시자(Envoy Gateway의 흔한 이름)입니다. 쓰는 컨트롤러의 GatewayClass로
  패치합니다.
- 컨트롤러가 프록시를 띄우는 namespace(예: Envoy Gateway는 기본이 `envoy-gateway-system`)에
  `acom.dev/ingress=true` 라벨을 붙입니다.
- 공개 호스트에는 HTTPRoute에 `hostnames`를, Gateway에 `certificateRefs`가 있는 HTTPS 리스너를 더합니다.
- production에서 쓰려면 `overlays/production/kustomization.yaml`의 주석 처리된 컴포넌트 줄과
  Ingress 삭제 패치(`$patch: delete`)의 주석을 함께 풉니다.

### components/monitoring

Prometheus Operator가 있는 클러스터에서 gateway, commerce, commerce-worker, agent의 Service `http` 포트
`/metrics`를 30초마다 수집하는 ServiceMonitor입니다. production 오버레이에 들어 있습니다. Prometheus가
ServiceMonitor를 고르는 라벨(예: `release: <설치 이름>`)이 필요하면 오버레이에서 패치로 붙이고,
Prometheus는 `monitoring` namespace에서 돌아야 NetworkPolicy가 수집을 허용합니다.
