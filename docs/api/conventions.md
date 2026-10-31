# 공통 규약

모든 오퍼레이션과 엔드포인트가 따르는 형식입니다. [README.md](README.md) §3은 이 문서의 요약입니다. 구현은 `acom-core`(값 타입)와 `acom-contract`(봉투·에러·스키마)가 강제합니다.

---

## 1. 이름

- **타입**은 schema.org 이름(PascalCase), 우리 확장은 `acom:` 네임스페이스([data-model.md](data-model.md) §확장).
- **필드**는 schema.org 프로퍼티를 snake_case로(`validFrom` → `valid_from`). JSON-LD 컨텍스트가 원래 용어로 되돌립니다.
- **enum 값**은 schema.org 열거 멤버를 snake_case 단어로(`InStock` → `in_stock`). 코드값 금지. 값 목록의 단일 출처는 [enums.md](enums.md).
- **툴 이름**은 `verb_noun` 평이한 영어. **논리 주소**는 `domain.resource.operation`. **REST**는 `/v1/{domain}/{resources}...`.

## 2. 식별자

- 형식: `{접두}_{접미}`. 접두는 엔티티 종류, 접미는 소문자·숫자·`_`.
- 시드 데이터는 읽을 수 있는 접미(`prd_windbreaker`, `cs_1042`), 실행 중 생성은 소문자 ULID(`pv_01jc8x...`).
- 사람에게 보이는 번호(주문 번호 #20402, 변경셋 #1042)는 별도 필드(`order_number`, `change_set_number`)이고 id 접미와 같게 시드합니다. 실행 중에 생긴 레코드는 id 접미가 ULID라 번호와 다릅니다. 번호에 접두를 붙여 id를 만들지 않습니다.
- **id 자리에 사람이 쓰는 값도 받습니다**([../design.md](../design.md) 원칙 1). 아래 엔티티를 가리키는 id 필드는 그 값을 그대로 받고, 서버가 툴을 실행하기 전에 레코드를 찾아 id로 바꿉니다. 툴(오퍼레이션)은 항상 id만 봅니다. 표의 원본은 `acom_core::handle::HANDLES`입니다.

  | 엔티티 | 받는 값 | 예 | 못 찾으면 |
  |---|---|---|---|
  | 주문 `ord` | 주문 번호(`#` 생략 가능) | `27184`, `#27184` | `not_found`, 다음 툴 `search_orders` |
  | 변경셋 `cs` | 변경셋 번호 | `1042` | `not_found`, 다음 툴 `search_change_sets` |
  | 고객 `cus` | 이메일(대소문자 무시) | `jamie.rivera@example.com` | `not_found`, 다음 툴 `search_members` |

  레코드 하나만 가리키는 값만 받습니다. 상품 이름처럼 여러 레코드에 맞을 수 있는 값은 검색 툴(`search_products`)로 찾습니다. 같은 값이 둘 이상에 맞으면 `invalid_input`(`reason: "ambiguous"`)입니다.
- 접두 표는 §9.

## 3. 금액

schema.org `MonetaryAmount` 형태입니다.

```json
{"value": 71.20, "currency": "USD"}
```

- `value`는 JSON **숫자**, 통화의 주 단위(달러). 서버는 정확한 십진수로 읽고 씁니다(부동소수점 미사용).
- 계산 결과는 통화의 보조 단위로 반올림(USD 센트, 0.5는 올림). 반올림은 줄마다 하고 합계는 반올림된 줄의 합입니다.
- 가격 필드는 schema.org를 따라 `price` + `price_currency`로 펼쳐 쓸 수도 있습니다(`Offer`, `UnitPriceSpecification`). 그 밖의 금액(환불액, 소계, 한도)은 위 객체 형태입니다.
- 서로 다른 통화의 덧셈은 `currency_mismatch` 에러입니다.

## 4. 비율

| 이름 꼴 | 뜻 | 범위 | 예 |
|---|---|---|---|
| `percent_off`, `*_percent` | 퍼센트 | 0–100 | `"percent_off": 20` |
| `*_rate`, 정책의 상한 값 | 비율 | 0–1 | `max_discount.everyone: 0.20` |

모델이 쓰는 툴 입력은 퍼센트(사람이 말하는 방식), 정책 파일은 비율입니다. 이름이 섞이지 않게 접미어로 구분합니다.

## 5. 시간

- **출력:** 시각은 항상 RFC 3339 + 스토어 오프셋(`2026-11-07T00:00:00-05:00`). 날짜만이면 `YYYY-MM-DD`.
- **입력:** RFC 3339, 또는 오프셋 없는 로컬 시각(`2026-11-07T00:00`), 또는 날짜(그날 00:00)를 받습니다. 오프셋이 없으면 **스토어 시간대**로 읽습니다.
- **기간:** `valid_from`(포함) ~ `valid_through`(포함, 분 단위). "Sat 00:00 – Sun 23:59"는 `valid_from: 2026-11-07T00:00`, `valid_through: 2026-11-08T23:59`.
- **현재 시각**은 서버의 시계(데모 모드에서는 스토어별 데모 시계)입니다. 상대 표현("이번 주말")은 에이전트가 이 시계로 풉니다.
- DB에는 UTC로 저장합니다.

## 6. 읽기

- 응답: `{"items": [...], "total": 42, "next_cursor": "..."}`. `total`은 필터를 적용한 전체 개수, `next_cursor`는 다음 쪽이 있을 때만.
- 페이지: `limit`(기본 20, 최대 100), `cursor`(이전 응답의 `next_cursor`). 쪽 번호·offset은 쓰지 않습니다.
- `fields`: 반환할 필드 목록(`fields=product_id,name,price`). 모델에게 필요한 만큼만 돌려줘 문맥을 아낍니다.
- 정렬: `sort`(`price`, `-price`). 허용 값은 오퍼레이션마다 enum으로 선언.
- 하나를 가져오는 읽기(`get_*`)는 목록 봉투 없이 그 엔티티를 돌려줍니다. 없으면 `not_found`.

## 7. 쓰기

흐름은 [../backend/commerce.md](../backend/commerce.md) §2가 정의합니다. 요약:

| 단계 | 요청 | 응답 |
|---|---|---|
| 프리뷰 | `POST {path}?preview=true` + 입력 (+ `change_set_id` 쿼리, 선택) | `{preview_id, expires_at, changes[], checks[], needs_approval, approval_reason, change_set_id?}` |
| 실행 | `POST {path}` + `{"preview_id": "pv_..."}` (다른 필드 금지) | `{ok, id, undo?}` |

- 프리뷰는 상태를 바꾸지 않습니다. 기본 유효 시간 30분(`approval.yml` `preview_ttl_minutes`).
- 실행은 같은 `preview_id`로 다시 와도 첫 결과를 돌려줍니다.
- `needs_approval: true`인 프리뷰는 승인된 변경셋 안에서만 실행됩니다. 단독 실행 시도는 `approval_required`.
- `undo`는 항상 `{"by": "person", "target": "cs_1042", "until": "..."}`. `target`은 변경셋 id(운영 쓰기) 또는 기억 id(`mem_…`, 쇼퍼 선호)이고, 되돌리기는 그 대상의 사람 전용 엔드포인트로만 합니다. 기한이 없으면 `until: null`. 툴로 되돌리는 경로는 없습니다.
- `checks[].rule`은 정책 키, `fix_tool`은 실패를 고칠 수 있는 툴 이름.

## 8. 툴 스키마 (모델이 보는 입력)

`GET /v1/tools` 매니페스트의 입력 스키마는 다음을 지킵니다. `cargo xtask check-docs`와 레지스트리 빌드가 검사합니다.

1. **닫힌 객체:** 모든 객체에 `additionalProperties: false`(서버도 모르는 필드를 거부).
2. **평평함:** `$ref`, `$defs`, `anyOf`, `oneOf`, `allOf`, `not`을 쓰지 않습니다. 선택 입력은 `required`에서 빼는 것으로만 표현하고 `null`을 허용하지 않습니다.
3. **선택형 입력은 판별 필드로:** "A 또는 B"가 필요하면 `mode` enum 필드 + 각 모드의 선택 필드로 쓰고, 어떤 모드에 어떤 필드가 필요한지 설명에 적습니다(서버가 검증, 위반은 `invalid_input`).
4. **enum은 `enum` 목록:** 값마다 뜻은 필드 설명에 "값: 뜻; 값: 뜻"으로. 사람이 쓰는 다른 표현은 `x-aliases`로 선언해 에이전트의 인자 보정이 씁니다(`"VIP members"` → `vip`).
5. **모든 필드에 `description`과 `examples`.**
6. **id는 패턴으로:** `"pattern": "^prd_[a-z0-9_]+$"`. 사람이 쓰는 값을 받는 id 필드(§2)는 매니페스트에서 패턴을 넓히고 `x-handle`로 표시합니다: `"pattern": "^(?:ord_[a-z0-9_]+|#?[0-9]+)$", "x-handle": {"field": "order_number", "shape": "number"}`, 설명 끝에 "Also takes the order number people see (27184)."를 붙이고 예시를 하나 더합니다. 오퍼레이션의 스키마(`op.input_schema()`)는 id 패턴 그대로입니다.
7. **모델 보기에는 `preview`/`preview_id`가 없습니다.** 우리 에이전트의 런타임이 프리뷰·실행을 처리하고, 외부 에이전트(MCP)와 REST 호출자만 전선 위 형태를 봅니다.

## 9. 식별자 접두

형식 `^[a-z]{2,5}_[a-z0-9][a-z0-9_]*$`. 엔티티 자신의 id 필드는 `id`, 다른 엔티티를 가리키는 필드는 `<entity>_id`(`product_id`, `order_id`)입니다. 다른 엔티티의 번호를 id에 넣지 않습니다(시드의 읽기 쉬운 접미는 예외). 외부 시스템의 id는 `external_id`에 둡니다.

| 엔티티 | 접두 | 엔티티 | 접두 |
|---|---|---|---|
| 스토어(테넌트) | `store` | 고객(Person) | `cus` |
| 운영자 계정 | `adm` | 시스템 주체(스케줄러·실행기) | `sys` |
| 상품(사람이 말하는 상품 = schema.org `ProductGroup`) | `prd` | 변형(사이즈·색 = schema.org `Product`, SKU 단위) | `var` |
| 가격 제안 `Offer` | `off` | 회원가·기간가 `UnitPriceSpecification` | `ups` |
| 브랜드 | `brn` | 카테고리(표준·전시, `term_set`으로 구분) | `cat` |
| 카테고리 매핑 | `map` | 진열 슬롯 | `slot` |
| 진열 예약(배너·슬롯 일정) | `dsp` | 배너 이미지 | `bnr` |
| 팝업 | `pop` | 기획전 `CollectionPage` | `col` |
| 캐시 무효화 | `cin` | 리뷰 / 상품 문의 / 답변 | `rev` / `qst` / `ans` |
| 재입고 알림 | `rsa` | 생성 이미지 | `img` |
| 재주문(공급처에 하는 주문) | `ro` | 생성 문구 초안 | `cpd` |
| 가져오기 작업 | `imp` | 색인 재구성 작업 | `rij` |
| 장바구니 / 장바구니 줄 | `cart` / `cln` | 주문 / 주문 품목 | `ord` / `oln` |
| 프리뷰(주문 준비·구독 준비 포함) | `pv` | 선물 주문 | `gift` |
| 구독 | `sub` | 결제 / 영수증 / 청구서 | `pay` / `rct` / `inv` |
| 클레임(취소·반품·교환, `claim_type`으로 구분) | `clm` | 증빙 / 검수 | `evd` / `ins` |
| 환불 | `rfd` | 배송 | `dlv` |
| 배송 구역 | `zone` | 프로모션 | `promo` |
| 쿠폰 정의 / 회원에게 발급된 쿠폰 | `cpn` / `mcp` | 조건부 사은품 | `cgf` |
| 적립금 거래 | `lyt` | 이벤트 / 응모 / 추첨 / 보상 / 보상 일괄 지급 | `evt` / `ent` / `drw` / `rwd` / `rwb` |
| 세그먼트 | `seg` | 회원 프로그램 | `prg` |
| 등급 평가 실행 | `tev` | 주소 / 동의 / 기억(선호) | `adr` / `cns` / `mem` |
| 본인 확인 / 계정 복구 요청 | `ivf` / `rcv` | 임직원 | `emp` |
| 고객 문의 / 문의 답글 | `inq` / `cmt` | 보상(remedy) | `rmd` |
| FAQ / 공지 | `faq` / `ntc` | 메시지 / 템플릿 / 캠페인 / 알림 | `msg` / `tpl` / `cmp` / `ntf` |
| 게시판 / 글 / 댓글 / 신고 | `brd` / `post` / `rpl` / `rpt` | 변경셋 | `cs` |
| 액션 로그 항목 | `act` | 할 일 | `task` |
| 역할 / API 키 / 세션 | `rol` / `key` / `ses` | 메뉴 / 조직 | `mnu` / `org` |
| 판매자 / 계약 | `slr` / `ctr` | 연동 채널 / 받은 웹훅 | `chn` / `whk` |
| 반품 정책 | `mrp` | 예약 작업 / 에이전트 단계 | `job` / `step` |
| 대화 / 턴 / UI 블록 | `conv` / `turn` / `blk` | 업로드 / 요청 추적 | `upl` / `req` |

- **등급 id**는 스토어가 정한 단어(`vip`, `gold`)이고 접두가 없습니다. `"everyone"`은 예약어입니다([enums.md](enums.md) §5).
- **사람에게 보이는 번호**는 따로 둡니다: 주문 `order_number: "20402"`, 변경셋 `number: 1042`(`#`은 화면 표기일 뿐 값에 넣지 않음).
- **모델(에이전트)** 은 id가 아니라 액션 로그의 `agent: {"@type": "SoftwareApplication", "name": "muse-glimmer-30b"}`입니다.
- **상품과 변형:** 사람과 모델은 상품(`prd_`)으로 이야기하고, 장바구니·재고·주문 품목은 변형(`var_`)을 씁니다. 상품 + 옵션(`size`, `color`)을 받는 툴은 변형을 찾아 주고, 결과에 `variant_id`를 돌려줍니다.

## 10. 헤더와 인증

| 헤더 | 쓰는 곳 | 설명 |
|---|---|---|
| `Cookie: acom_session` | 브라우저 → 게이트웨이 | 사람 세션 |
| `Authorization: Bearer` | 외부 에이전트 → 게이트웨이(OAuth) / 게이트웨이 → 서비스(내부 토큰) | 서비스는 게이트웨이가 서명한 내부 토큰만 받음 |
| `X-CSRF-Token` | 사람 전용 엔드포인트 | 세션과 짝 |
| `Idempotency-Key` | 프리뷰 없이 상태를 바꾸는 사람 전용 엔드포인트 | 재시도 시 첫 결과 |
| `X-Request-Id` | 전부 | 없으면 게이트웨이가 만듦. 액션 로그·에이전트 턴과 이어짐 |

스토어(테넌트)는 내부 토큰의 `tenant` 클레임 하나로만 정해집니다. 주체와 스코프는 [../backend/gateway.md](../backend/gateway.md).

## 11. 버전

경로 버전은 `/v1`. 파괴적 변경만 `/v2`를 만들고, 같은 기능의 버전 복제본은 두지 않습니다. 블록 데이터는 블록마다 `schema` 번호를 가집니다.
