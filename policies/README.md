# 정책 데이터 (Policy as Data)

> **규칙은 코드가 아니라 데이터입니다.** 할인 상한, 가격 하한, 반품 기간, 승인 임계 같은 비즈니스 규칙을 YAML 파일에 두고, 백엔드(`commerce` 서비스의 `acom-policy`)가 읽어 강제합니다. 프롬프트나 코드 상수에 숨기지 않습니다. 설계 배경은 [../docs/architecture.md](../docs/architecture.md) §6.4, 엔진 동작은 [../docs/backend/commerce.md](../docs/backend/commerce.md) §4, API에서 쓰는 방식은 [../docs/api/README.md](../docs/api/README.md) §6입니다.

---

## 1. 기본값과 데모 스토어 값

이 폴더의 값은 **새 스토어의 기본값**입니다. 스토어는 키 단위로 덮어쓸 수 있고(`platform.policy_override`, 기본값에 깊은 병합), 데모 스토어 Northwind Apparel이 덮어쓰는 값은 [../docs/scenarios.md](../docs/scenarios.md) §5에 있습니다(예: `max_discount.by_tier.vip: 0.30`은 기본값이 아니라 데모 덮어쓰기). 둘이 다르면 데모 장면의 기대값은 scenarios.md를 따릅니다.

자주 인용되는 키:

| 키 | 기본값 | 파일 |
|---|---|---|
| `max_discount.everyone` / `max_discount.by_tier` | 0.20 (20%) / 비어 있음(등급별 상한 없음) | [promotion.yml](promotion.yml) |
| `price_floor` | 원가 × 1.15 이상(같아도 통과) | [catalog.yml](catalog.yml) |
| `promo_min_stock` | 10 (미만은 세일에서 제외) | [promotion.yml](promotion.yml) |
| `member_benefits_per_order` | 1 (쿠폰은 등급 쿠폰과 중복 안 됨, `coupon.stacking`) | [promotion.yml](promotion.yml) |
| `thresholds.refund` | `{value: 200.00, currency: USD}` 초과 환불은 사람 승인, 이하는 사람 없이 실행 | [approval.yml](approval.yml) |
| `return_window_days` | 30 | [fulfillment.yml](fulfillment.yml) |
| `defect_refund` | 전액 환불, 반품 배송비 무료 | [fulfillment.yml](fulfillment.yml) |
| `changed_mind_return_shipping` | 반품 배송비 고객 부담 | [fulfillment.yml](fulfillment.yml) |
| `damage_claim_needs_photo` | true | [fulfillment.yml](fulfillment.yml) |
| `return_suggestions.flag_return_history` | 90일 안 4회 이상 반품이면 검토 표시 | [fulfillment.yml](fulfillment.yml) |
| `customer_messages` | `always_ask` | [approval.yml](approval.yml) |
| `preview_ttl_minutes` | 30 | [approval.yml](approval.yml) |
| `change_set.undo_window_after_end_days` / `undo_window_after_run_days` | 1 / 1 | [approval.yml](approval.yml) |
| `free_shipping_min_subtotal` | `{value: 80.00, currency: USD}`, 회원 혜택 적용 전 소계 기준 | [order.yml](order.yml) |
| `thresholds.loyalty_grant` | `{value: 50.00, currency: USD}` 초과 | [approval.yml](approval.yml) |
| `pricing_rule` | 원가 × 2.5, 가장 가까운 x9, 동률이면 위로 | [catalog.yml](catalog.yml) |
| `watchers.*` | 반품 대기 30분마다, 품절 예측 5일, 원가 이하 판매 | [platform.yml](platform.yml) |
| `task_pickup` | 꺼짐(데모 스토어는 켜고 `adm_sj`로). 감시 작업이 새로 연 할 일을 에이전트가 바로 맡음 | [platform.yml](platform.yml) |

## 2. 왜 데이터인가

- **에이전트가 규칙을 모른 채 어기지 않게.** 위반은 읽을 수 있는 이유와 고칠 툴(`fix_tool`)로 돌아옵니다.
- **하나의 판정기.** Rust 로더(`acom-policy`)가 이 YAML을 파일별 타입으로 읽고 검증하며(모르는 키 거부), 프리뷰·실행·검증과 혜택 계산이 모두 같은 판정기를 씁니다. CI의 `cargo test -p acom-policy`가 기본 파일을 읽어 확인합니다.
- **배포 없이 규칙 변경.** 정책 파일의 `version`을 올려 바꾸고, 스토어 덮어쓰기 변경도 프리뷰·승인을 거치는 쓰기라 감사 기록에 남습니다.
- **프리뷰·실행·검증이 같은 규칙.** 프리뷰가 정책 스냅샷의 버전 해시를 기록하고, 실행 때 정책이 바뀌었으면 체크를 다시 돌립니다.

## 3. 파일 구성

| 파일 | 도메인 | 핵심 내용 |
|---|---|---|
| [catalog.yml](catalog.yml) | catalog | **가격 하한(원가 + 15%)**, 가격 규칙, 콘텐츠 자동·구조 변경 승인, 변형 단위 재고, 번들·상품권 제약, 리뷰·문의 |
| [promotion.yml](promotion.yml) | promotion | **할인 상한**(전체 20%, 등급별 덮어쓰기), **세일 최소 재고 10**, **주문당 회원 혜택 1개**, 할인 층, 프로모션 겹침, 쿠폰 적용·상태·복원, 적립금, 사은품, 이벤트 |
| [order.yml](order.yml) | sales | **무료 배송 $80.00**(혜택 전 소계), 장바구니, 체크아웃 검증 키, 결제, 재고 예약, 주문 상태 |
| [fulfillment.yml](fulfillment.yml) | fulfillment | **반품 30일**, 불량·변심·파손 규칙, 반품 제안 규칙, 원 결제 수단 환불·쿠폰 차감, 취소·교환, 귀책별 배송비, 배송 상태 |
| [approval.yml](approval.yml) | platform | **승인 임계**(환불 $200.00 초과, 적립금 $50.00 초과 등), 고객 메시지 항상 승인, 프리뷰 유효 시간, 변경셋 되돌리기 기한, 승인 없는 쓰기, 프리뷰만 쓰기, 사람 전용 엔드포인트 |
| [customer.yml](customer.yml) | customer | 회원 상태, 등급 프로그램 스키마, 동의 게이트, 개인정보 마스킹, 쇼퍼 기억, 탈퇴, 상담·보상, 메시지 발송 시간·상한 |
| [merchandising.yml](merchandising.yml) | merchandising | **슬롯 중복 예약 금지**, 카테고리 묶음·매핑, 진열 기간 계산·대상, 기획전 공개 |
| [search.yml](search.yml) | search | 검색 모드와 확장, 스토어 격리, 등급 필터를 쿼리 안에서, 랭킹 신호, 색인 교체, 비교 속성 |
| [community.yml](community.yml) | community | 게시판 쓰기 권한, 금칙어·스팸·첨부, 신고·블라인드 |
| [partner.yml](partner.yml) | partner | 입점 서류·금지 업종, 판매자 상태 전이, 수수료 범위, 계약 기간, 등급 |
| [integration.yml](integration.yml) | integration | 가져오기 배치 한도·필수 필드, 외부 상태 매핑, 자격 증명 보관, 이벤트 서명·재생 창, 대상이 없는 이벤트의 재시도 간격·포기 시간 |
| [platform.yml](platform.yml) | platform | **할 일 감시 작업(`watchers`)**, 역할·관리자·조직·API 키 규칙 |

`acom-policy::FILES`는 현재 앞의 8개 파일을 읽습니다. `community`·`partner`·`integration`·`platform`은 그 도메인의 오퍼레이션을 구현할 때 로더 목록에 더합니다.

## 4. 백엔드가 쓰는 방식

```
정책 로더 (acom-policy)
  └─ 기동 시 policies/*.yml 로드 → 파일별 타입 검증 → 스토어 덮어쓰기 병합 → PolicySet(+ 버전 해시)
쓰기 프리뷰
  └─ 체크 평가 → checks[{rule, passed, detail, fix_tool}]   (rule = 정책 키)
  └─ 승인 판정 → needs_approval, approval_reason            (approval.yml)
혜택 계산 (promotion.calculate_benefits)
  └─ discount_layers · max_discount · member_benefits_per_order · price_floor로 주문 혜택 계산
감시 작업 (platform.job)
  └─ watchers 기준으로 데이터를 확인 → 할 일(task) 생성·갱신
get_policy
  └─ 키별 저장 값과 사람이 읽는 표기(display)를 반환 (에이전트·화면이 같은 값을 인용)
```

- **식 언어 없음.** YAML 안에 `"amount > 200"` 같은 문자열 식을 두지 않습니다. 조건은 구조화된 데이터입니다.
  ```yaml
  when: { field: amount, op: gt, value: { value: 200.00, currency: USD } }
  when: { all: [ { field: liability, op: eq, value: customer_fault }, { field: extra_charge, op: gt, value: { value: 0.00, currency: USD } } ] }
  when: { field: days_since_delivery, op: gt, value_from: return_window_days }   # 다른 정책 값을 기준으로
  ```
  `op`는 `eq` · `ne` · `gt` · `gte` · `lt` · `lte` · `in`. `field`는 프리뷰·선택지 계산이 만드는 값의 이름이고, 작은 평가기(`policy::cond`)가 해석합니다. 계산식(판매가 = 정가 × (1 − 할인율), 하한 = 원가 × 배수)은 코드에 있고 YAML에는 그 파라미터만 둡니다.
- **스토어별 덮어쓰기**: 기본 정책 + 스토어별 차이. 맵은 키 단위로 병합하고, 리스트와 값은 통째로 바꿉니다.
- **정책 스냅샷**: 프리뷰와 실행이 적용된 정책 버전을 기록합니다(감사·분쟁 대응).

## 5. 정책 키 ↔ check `rule` ↔ 오퍼레이션

check `rule`은 정책 키 그 자체(파일 이름을 뺀 키 경로)입니다. 대표 대응은 아래와 같고, 전체 오퍼레이션 목록은 각 도메인 파일의 `정책:` 줄에 있습니다.

| check `rule` (정책 키) | 파일 | 적용 오퍼레이션 | 실패 시 `fix_tool` |
|---|---|---|---|
| `max_discount` | promotion.yml | `set_sale_prices`, `create_promotion` | `set_sale_prices` |
| `price_floor` | catalog.yml | `set_sale_prices`, `create_promotion`, `set_price`, `schedule_price_change`, `import_products` | `set_sale_prices` / `set_price` |
| `promo_min_stock` | promotion.yml | `set_sale_prices` | `set_sale_prices` |
| `promotion_overlap` | promotion.yml | `set_sale_prices`, `create_promotion` | `list_active_promotions` |
| `member_benefits_per_order` | promotion.yml | `create_coupon` | `create_coupon` |
| `slot_free` | merchandising.yml | `schedule_banner`, `schedule_display` | `list_display_slots` |
| `refund.to`, `refund.coupon_deducted` | fulfillment.yml | `refund_return`, `approve_claim`, `retry_refund` | `refund_return` |
| `checkout_validation.*` | order.yml | `prepare_order`, `prepare_subscription`, `prepare_gift_order` | `get_payment_methods`, `get_product`, `get_cart` |
| `preference.about_shopper_only`, `preference.no_sensitive_data` | customer.yml | `update_my_preferences` | — |
| `consent.marketing_gate` | customer.yml | `send_bulk`, `create_campaign` | `get_consent` |
| `change_set.irreversible` | approval.yml | `refund_return`, `send_customer_message`, `send_bulk` | — |
| `thresholds.*`, `customer_messages` | approval.yml | 모든 W·W$의 승인 판정(체크가 아님) | (사람 승인) |

## 6. 검증과 관리

- 정책 파일은 로더의 타입 검증(필수 키, 타입, 열거 값)을 통과해야 서비스가 뜹니다. CI는 `cargo test -p acom-policy`로 기본 파일을 읽어 봅니다.
- 상태·종류 단어는 [../docs/api/enums.md](../docs/api/enums.md)의 목록을 씁니다.
- 변경은 `version`을 올리고 변경 이력을 남깁니다.
- 계정·토큰·URL 같은 비밀은 정책 파일에 쓰지 않습니다.

데이터 타입과 명명 규칙은 [../docs/api/data-model.md](../docs/api/data-model.md), 전체 오퍼레이션 인덱스는 [../docs/api/README.md](../docs/api/README.md)를 보세요.
