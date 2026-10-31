# 테스트와 평가

세 층으로 "작동한다"를 확인합니다. 아래 층일수록 빠르고 많고, 위 층일수록 느리고 적습니다.

| 층 | 무엇을 | 어디서 | 언제 |
|---|---|---|---|
| 1. 단위·계약 | 상태기계, 정책, 혜택 계산, 오퍼레이션 하나의 입력→출력·에러·프리뷰, 레지스트리와 명세 대조 | 소스 안의 `#[cfg(test)]`(오퍼레이션 파일, 도메인의 `tests.rs`), `acom-registry` | 모든 PR (`cargo test --workspace`, DB 없이) |
| 2. 장면 재생 | 데모 장면을 **모델 없이** 정해진 툴 호출 순서로 재생하고 결과를 다시 읽어 확인 | 도메인의 장면 테스트(`b1.rs`, `tests.rs`, `integration.rs`), `crates/agent/src/runtime/scenes.rs` | DB 없는 것은 모든 PR, DB가 필요한 것(`#[ignore]`)은 요청 시 |
| 3. 에이전트 평가 | 떠 있는 스택에서 실제 모델로 과업 세트를 돌려 지표 측정, 화면형 파사드와 대조 | `cargo xtask eval`, `eval/` | 수동 |

---

## 1. 계약 테스트

- 오퍼레이션마다(`crates/domains/{domain}/src/{resource}/{tool}.rs` 안): 정상 입력의 출력 형태, 명세에 적힌 에러 코드, 쓰기라면 프리뷰가 상태를 바꾸지 않는다는 것과 같은 `preview_id`로 두 번 실행해도 결과가 같다는 것. DB가 필요한 것은 `#[ignore = "integration: needs a PostgreSQL database"]`로 표시하고 `DATABASE_URL`의 데모 스토어(마이그레이션과 시드 적용, 데모 시계 Fri Nov 6 11:00)에 대고 돕니다.
- 레지스트리 대조(`acom-registry`의 테스트, `cargo test`가 돌림): 등록된 오퍼레이션마다 툴 이름·주소·종류·REST 경로가 [../api/](../api/README.md)의 그 절과 같은지(`every_operation_matches_its_spec`), 사람 전용 엔드포인트가 `approval.yml` `person_only`에 있는지, 두 오퍼레이션이 같은 경로를 쓰지 않는지. 레지스트리는 만들 때 닫히지 않은 입력 스키마(추가 필드 허용, `$ref`·`anyOf`·`oneOf`)를 거부합니다.
- 명세와 코드 검사(xtask, CI):
  - `cargo xtask check-docs`: [../api/README.md](../api/README.md) §4의 인덱스와 도메인 파일의 절이 같은 툴·같은 종류인지.
  - `cargo xtask check-coverage`: 명세의 툴 291개와 사람 전용 엔드포인트 18개가 모두 만들어졌는지("every specified operation is built").
  - `cargo xtask check-layout`: 오퍼레이션마다 `{domain}/src/{resource}/{tool}.rs`에 있는지("layout follows domain/resource/tool").
- 정책 파일 검사(`acom-policy`의 테스트): 기본 파일이 모두 읽히는지, 덮어쓰기가 깊게 병합되고 버전이 바뀌는지, 감시 작업 설정과 승인 기준을 읽는지.

## 2. 장면 재생

[../scenarios.md](../scenarios.md)의 장면을 사람이 썼을 툴 호출 순서로 재생합니다(모델 없이). 두 가지입니다.

- **도메인 장면 테스트**는 오퍼레이션과 변경셋 엔진을 직접 부릅니다. DB 없이 시드와 정책만으로 도는 것(`acom-promotion`·`acom-merchandising`의 `b1.rs`, fulfillment의 B2 분류)과, 데모 스토어 DB에 대고 돌며 결과를 다시 읽어 확인하는 것(`#[ignore]`: B1 끝까지, B2 환불·메시지, B3 매출 설명, B4 공급사 시트, F2·F4·F5)이 있습니다.
- **에이전트 장면 재생**(`crates/agent/src/runtime/scenes.rs`)은 B2·F3·F4·F6을 기록한 모델 출력(라우팅·계획·인자·답)과 장면의 툴 출력을 돌려주는 가짜 commerce로 돌립니다. 네트워크·DB·실제 모델이 없습니다. 블록이 툴의 숫자를 그대로 싣는지, 승인과 주문은 사람의 버튼으로만 되는지, 선물 받는 사람에 대해 아무것도 기억하지 않는지를 봅니다.

예: VIP 주말 세일 변경셋(`acom-merchandising`의 `b1_vip_weekend_sale_end_to_end`, DB 필요)

1. 변경셋을 만들고 `set_sale_prices` v1을 프리뷰 → `price_floor`·`promo_min_stock` 두 체크가 실패, "Changes 42 prices".
2. 대화 수정에 해당하는 재프리뷰(오버셔츠 15%, 재고 미달 제외) → 체크 모두 통과, "Changes 40 prices".
3. `create_coupon`, `schedule_banner`를 `$item[1].promotion_id` 플레이스홀더로 프리뷰 → `submit`.
4. `approve {version}` → 실행기 → 검증 → `live`, 되돌리기 기한 Nov 9 23:59.
5. 프로모션을 다시 읽음: 상품 40개, 쿠폰 `cpn_vipweekend20`.
6. `undo` → 회원가·쿠폰이 사라지고 프로모션은 `ended`.

변경셋을 쓰고 되돌리므로 데모 스토어의 복사본에서 돌립니다.

## 3. 에이전트 평가

[../design.md](../design.md) §7의 실험입니다.

- **과업 세트:** `eval/tasks/{design,held_out}/{직무}.yml`(marketer, merchandiser, operations, owner). 과업 = 직무 + 페르소나 + 면(bo/fo) + 데모 시계 + 사람의 말(여러 턴 가능) + 기대 결과(사실, 변경셋) + 허용 툴 + 부르면 안 되는 툴. 설계 세트는 개발 중에 보고, 보류 세트는 마지막에 한 번만 돌려 그 숫자를 씁니다. 형식·출처·커버리지는 [../../eval/README.md](../../eval/README.md), 검사는 `cargo xtask check-eval`.
- **두 툴 레이어:** 같은 백엔드 위에
  - A-Commerce 툴(레지스트리 매니페스트)
  - 화면형 파사드(`acom-facade`): [../design.md](../design.md) §2의 패턴(화면 단위 엔드포인트, 축약 이름, 코드값, POST 읽기와 별도 카운트, 행 상태 플래그가 있는 일괄 저장, 버전 중복)을 **우리가 직접 쓴 일반적인 예**로 재현한 툴 목록. 같은 도메인 서비스를 부르므로 결과 데이터는 같습니다. 화면 목록과 서빙·문(데모 모드, `?set=facade`)은 [../api/README.md](../api/README.md) §7, 턴의 전환은 [../api/chat.md](../api/chat.md) §1(`tool_set`)입니다.
- **같은 런타임:** 에이전트 런타임은 매니페스트만 바꿔 끼웁니다. 툴 검색·계획·실행 코드는 같습니다.
- **모델:** 지금까지의 결과는 모두 Glimmer입니다. 라이브 데모에 Spark가 설정돼 있지 않아 모든 모델 호출이 Glimmer로 갑니다([agent.md](agent.md) §8.1).
- **돌리기와 판정:** `cargo xtask eval --base <url> --set design --tool-set acom|facade|both --repeat N`이 게이트웨이로 과업마다 로그인해 말하고, 대화의 턴 로그(`GET /api/chat/{surface}/conversations/{id}?log=true`)에서 툴 호출과 모델 호출을 읽습니다. 완료는 필수 툴이 에러 없이 불리고 부르면 안 되는 툴이 불리지 않았으며, 기대한 변경셋이 마지막 `change_set` 블록에 있고, 숫자가 든 사실의 숫자가 마지막 답이나 블록에 모두 보일 때입니다. DB 상태는 읽지 않습니다.
- **지표:** 과업 완료율, pass^k(같은 과업 k회가 모두 완료), 툴 정밀도·재현율, 과업당 툴 호출·모델 호출·토큰·시간. 모델 가격이 비어 있어 비용은 세지 않습니다.
- **결과:** `eval/results/{date}-{model}-{set}-{toolset}.md`(`--out`으로 쓴 보고서). 2026-10-31 설계 세트(과업 12개 × 3회, 모두 Glimmer): A-Commerce 툴 58% 완료(21/36), pass^3 50%, 화면형 파사드 0%(0/36). [../design.md](../design.md) §7에 그 줄이 있습니다. 보류 세트는 아직 돌리지 않았습니다.

## 4. CI

`.github/workflows/ci.yml`의 `rust` 작업(툴체인 1.99)이 게이트입니다.

```
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace            # DB 없이. #[ignore] 테스트는 돌리지 않음
cargo xtask check-docs            # 명세 인덱스 = 도메인 파일
cargo xtask check-schema          # 마이그레이션 규칙, 컬럼 사전·어휘가 최신인지
cargo xtask check-seed            # 시드가 스키마에 맞는지
cargo xtask check-layout          # 오퍼레이션마다 제 파일에 있는지
cargo xtask check-coverage        # 명세의 모든 툴·사람 전용 엔드포인트가 만들어졌는지
```

같은 워크플로의 `web`(타입 검사, 빌드), `infra`(Compose 설정, Kubernetes 매니페스트 검증), `images`(네 이미지 빌드)도 PR마다 돕니다. 로컬에서는 `make check`(Rust만은 `make check-rust`).

DB를 띄우는 테스트(`#[ignore]`)는 통합 테스트이며 요청이 있을 때 `cargo test --workspace -- --ignored`(또는 크레이트 하나, 예 `cargo test -p acom-merchandising -- --ignored`)로 돌립니다.
