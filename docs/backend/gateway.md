# gateway 서비스

사람(브라우저)과 외부 에이전트가 들어오는 유일한 입구입니다. 누가 무엇을 할 수 있는지를 여기서 정하고, 그 결과를 **서명된 내부 토큰**으로 뒤쪽 서비스에 넘깁니다. 비즈니스 규칙은 갖지 않습니다.

---

## 1. 주체(Principal)

| 주체 | 누구 | 들어오는 길 | 받는 스코프 |
|---|---|---|---|
| `operator` | 백오피스 운영자 | 쿠키 세션 | 역할에 따른 `tools:*`, `human:approve`·`human:undo` 등 |
| `shopper` | 회원 또는 비회원 쇼퍼 | 쿠키 세션(비회원은 익명 세션) | 쇼퍼 툴, `human:order`·`human:return` 등 |
| `agent_on_behalf` | 우리 에이전트가 위 사람을 대신함 | 게이트웨이가 채팅 요청에 붙여 발급 | 그 사람의 툴 스코프 **에서 `human:*`와 `admin:*`를 뺀 것** |
| `external_agent` | ChatGPT·Gemini 등 | 설계만 있음: `/mcp`와 외부 토큰을 내부 토큰으로 바꾸는 길이 아직 없음(§6) | 정해진 스코프 없음(설계는 읽기, 장바구니, 주문 준비) |
| `system` | 변경셋 실행기(`sys_runner`), 예약 작업(`sys_scheduler`), 감시 작업의 고침(`sys_watcher`), 게이트웨이의 할 일 맡기기 읽기(`sys_task_pickup`) | `commerce` 내부, 게이트웨이 | 모든 `tools:*`, `human:*` 없음 |

핵심 규칙: **`human:*` 스코프는 사람이 직접 누른 요청에만 붙습니다.** 채팅 요청(`/api/chat/*`)으로 들어온 토큰에는 절대 붙지 않으므로, 어떤 프롬프트로도 에이전트가 승인·주문·반품 요청·되돌리기·탈퇴·비밀번호 변경을 할 수 없습니다. `commerce`도 사람 전용 엔드포인트에서 이 스코프를 다시 확인합니다(이중 방어).

## 2. 스코프

| 스코프 | 의미 |
|---|---|
| `tools:read:{domain}` | 그 도메인의 읽기 |
| `tools:write:{domain}` | 그 도메인의 쓰기 프리뷰, 승인 불필요 쓰기 실행 |
| `changeset:draft` | 변경셋 생성·항목·제출 (에이전트 위임 주체) |
| `human:approve`, `human:reject`, `human:undo` | 변경셋 결정 |
| `human:task` | 할 일 치우기 |
| `human:order`, `human:cancel`, `human:return`, `human:exchange`, `human:claim` | 쇼퍼의 주문·클레임 확정 |
| `human:account` | 탈퇴, 비밀번호, 로그인 아이디 찾기 |
| `admin:demo` | 데모 시계 조작(`PUT /v1/platform/demo-clock`, 운영자 본인의 요청만), 시드 리셋 (데모 모드에서만) |

운영자의 스코프는 역할 데이터(`platform.admin_user_role` → `platform.role_scope`)에서 오고, 게이트웨이는 세션을 찾을 때 읽어 30초 동안 캐시합니다. 쇼퍼와 게스트는 `acom-auth::scopes`의 고정 집합을 받습니다(게스트는 주문 스코프 없음). 외부 에이전트용 `external:*` 스코프는 아직 정의돼 있지 않습니다.

## 3. 내부 토큰

- 게이트웨이만 서명합니다(Ed25519, `ACOM_GATEWAY_SIGNING_KEY_FILE`). `commerce`와 `agent`는 공개 키(`ACOM_GATEWAY_PUBLIC_KEY_FILE`)로만 검증하고(각자의 `aud`), 게이트웨이 토큰이 없는 요청은 받지 않습니다.
- 요청마다 발급합니다. 수명은 API 요청 5분, 채팅 턴 15분(턴의 모델 호출과 툴 호출이 모두 이 토큰을 씀).
- 클레임:

```jsonc
{
  "iss": "acom-gateway", "aud": ["commerce", "agent"],
  "sub": "cus_jamie",                 // 사람 id (외부 에이전트면 클라이언트 id)
  "kind": "agent_on_behalf",          // §1의 주체 종류
  "tenant": "store_demo",
  "surface": "fo",                    // bo | fo | mcp | system
  "scopes": ["tools:read:search", "tools:write:sales", "..."],
  "request_id": "req_01JC8...",
  "name": "Jamie",                    // 선택: 화면과 로그에 보일 이름
  "session_id": "ses_...",            // 선택
  "iat": 1793979100,
  "exp": 1793980000
}
```

- 에이전트가 `commerce`를 부를 때는 받은 토큰을 그대로 씁니다. 에이전트는 토큰을 새로 만들 수 없으므로 자기 권한을 넓힐 수 없습니다.

## 4. 라우팅

게이트웨이가 웹과 API의 단일 출처입니다(쿠키·CORS 단순화).

| 경로 | 대상 | 인증 |
|---|---|---|
| `/api/auth/*` | 게이트웨이 자신 | 로그인·로그아웃·세션·데모 로그인 |
| `/api/chat/{bo,fo}/*` | agent | 세션 → `agent_on_behalf` 토큰. SSE를 버퍼링 없이 중계 |
| `/api/v1/*` | commerce | 세션 → 사람 토큰(웹이 툴 REST를 직접 부를 때: 목록 화면, 블록 새로고침) |
| `/api/v1/**/approve`, `/undo`, `/orders`, … (사람 전용) | commerce | 세션 + CSRF 토큰 + `human:*` |
| `/mcp`, `/mcp/*` | commerce | 토큰 없이 그대로 넘김. `commerce`에 `/mcp` 경로가 아직 없어 답하지 않음 |
| `/*` | web (Next.js) | 그대로 |

## 5. 기타

- **세션:** 서버 세션(`platform.session`, httpOnly·SameSite=Lax 쿠키). 비밀번호는 argon2. 데모 모드에서는 `/api/auth/demo-login {persona}`로 시나리오의 운영자·쇼퍼로 바로 들어갑니다. 엔드포인트는 아래 §6.
- **테넌트 결정:** 게이트웨이 하나가 스토어 하나를 섬깁니다(`ACOM_STORE_ID`, 기본 `store_northwind`). 토큰의 `tenant`가 `commerce`의 유일한 근거이고 클라이언트가 보낸 테넌트 헤더는 무시합니다.
- **레이트 리밋:** 게이트웨이에는 아직 없습니다. 한도가 업무 규칙인 곳(커뮤니티 글쓰기, 계정 찾기, 전시 캐시 비우기)은 그 오퍼레이션이 정책 값으로 `rate_limited`를 냅니다.
- **관측성:** 요청 id를 만들어(`X-Request-Id`가 오면 이어받음, `acom-server`) 내부 토큰과 로그에 싣고 응답 헤더로 돌려줍니다. 이 id가 에이전트 턴과 커머스 액션 로그를 잇습니다.
- **할 일 맡기기:** 스토어가 `platform.yml` `task_pickup.enabled`를 켜면, 게이트웨이가 커머스 이벤트(`GET /v1/platform/events`)를 구독하다가 감시 작업이 **새로 만든** `agent_noticed` 할 일(`task.updated`의 `created: true`)의 요청 문장을 `POST /chat/bo/turns {message, task_id}`로 보냅니다. 토큰은 `task_pickup.acts_as` 운영자의 위임 토큰(§3, `human:*` 없음)이라 초안은 승인 대기에서 멈춥니다. 데모 스토어는 켜 두었고 `acts_as`는 `adm_sj`입니다(`seed/demo/01_platform.yml`의 정책 덮어쓰기). 갱신된 할 일과 시드의 할 일은 맡지 않고, 턴은 하나씩 순서대로 돌립니다. 정책과 할 일을 읽는 요청은 `sys_task_pickup` 시스템 주체로 서명합니다. 코드: `services/gateway/src/pickup.rs`.
- **타임아웃:** 게이트웨이는 따로 두지 않습니다. 채팅 턴은 에이전트의 턴 상한(60초)에서 멈춥니다. 업스트림이 답하지 않으면 502, 세션 저장소가 답하지 않으면 503이고, 둘 다 `{"error": {"code": "upstream_unavailable", ...}}`입니다.

## 6. 로그인과 세션 API (`/api/auth/*`)

| 엔드포인트 | 본문 | 응답 |
|---|---|---|
| `POST /api/auth/login` | `{login_id, password}` | 운영자 로그인(`platform.admin_user.login_id` + argon2id). `{ok, user, scopes, csrf_token, expires_at}`. 틀리면 `invalid_credentials`(401, 아이디가 없는 경우와 같은 응답), 정지·잠금이면 `account_locked`(423) |
| `POST /api/auth/demo-login` | `{persona}` | 데모 모드만. `adm_…`(운영자), `cus_…`(쇼퍼), `guest` |
| `GET /api/auth/personas` | — | 데모 모드만. `ACOM_DEMO_PERSONAS`의 사람들(`{persona, kind, name, description}`) |
| `POST /api/auth/guest` | — | 게스트 세션(로그인 없이 장바구니). 살아 있는 세션이 있으면 그대로 |
| `GET /api/auth/session` | — | `{signed_in, user, scopes, csrf_token, expires_at, demo_mode}`. `user.kind`는 `operator` \| `shopper` \| `guest` |
| `POST /api/auth/logout` | — | 세션 폐기, 쿠키 삭제 |

- **쿠키:** `acom_session`(httpOnly, 세션 토큰)과 `acom_csrf`(페이지가 읽음). DB에는 둘 다 SHA-256 해시만 저장합니다. `ACOM_COOKIE_SECURE=true`면 `Secure`.
- **CSRF:** 세션이 있는 요청이 상태를 바꾸면(GET·HEAD·OPTIONS 외) `X-CSRF-Token: <acom_csrf 값>`이 있어야 하고, 없거나 다르면 `csrf_mismatch`(403)입니다.
- **세션 없는 요청:** 읽기는 익명(읽기 스코프만)으로 통과합니다. 상태를 바꾸는 첫 요청(게스트의 장바구니 담기)은 그 자리에서 게스트 세션을 만들고 쿠키를 붙여 돌려줍니다.
- **내부 토큰:** `/api/v1/*`는 사람의 스코프 그대로(수명 5분), `/api/chat/*`는 `on_behalf()`(`human:*`·`admin:*` 제거, 운영자는 `changeset:draft` 추가, 한 턴 15분). 브라우저가 보낸 `Authorization`은 버리고, commerce·agent로 쿠키를 넘기지 않습니다.
- **운영자 스코프:** 역할(`platform.admin_user_role` → `platform.role_scope`)의 합 + `changeset:draft`. 데모 모드의 플랫폼 관리자는 `admin:demo`도 받습니다. 데모 계정은 `admin` / `admin123`(모든 역할과 승인 역할).
- **API 클라이언트 세션:** 외부 에이전트·연동 클라이언트는 API 키로 `login` 툴([../api/platform.md](../api/platform.md#auth))을 불러 토큰을 받습니다. 그 세션도 `platform.session`의 행이지만 `actor_kind: external_agent`이고 주체가 키(`key_…`)라 쿠키 조회는 이 행을 사람 세션으로 받지 않습니다. 이 토큰을 `/mcp`에서 받아 `external_agent` 주체로 바꾸는 일은 게이트웨이의 후속 작업입니다.
