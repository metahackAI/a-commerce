# 정책 데이터 (Policy as Data)

> **규칙은 코드가 아니라 데이터입니다.** 할인 상한, 가격 하한, 반품 기간, 승인 임계 같은 비즈니스 규칙을 YAML 파일에 두고, 백엔드(툴 레이어)가 읽어 강제합니다. 프롬프트나 코드 상수에 숨기지 않습니다. 설계 배경은 [../../architecture.md](../../architecture.md) §6.4, API에서 쓰는 방식은 [../README.md](../README.md) §6입니다.

---

## 1. 데모 스토어 값

이 파일들의 데모 스토어(Northwind Apparel) 값은 **[../../plan.md](../../plan.md) §4.1 정책 표와 같습니다.** 해커톤 MVP의 `policy.yaml`은 아래 키들을 모은 것이고, 값이 다르면 plan.md §4.1이 기준입니다.

| 키 | 값 | 파일 |
|---|---|---|
| `max_discount.vip` | 0.30 (30%) | [promotion.yml](promotion.yml) |
| `max_discount.everyone` | 0.20 (20%) | [promotion.yml](promotion.yml) |
| `price_floor` | `cost * 1.15` (원가 + 15%) | [catalog.yml](catalog.yml) |
| `promo_min_stock` | 10 (미만은 세일에서 제외) | [promotion.yml](promotion.yml) |
| `member_benefits_per_order` | 1 (쿠폰은 등급 쿠폰과 중복 안 됨) | [promotion.yml](promotion.yml) |
| `refund_without_person_max` | `{value: 200.00, currency: USD}` | [fulfillment.yml](fulfillment.yml), [approval.yml](approval.yml) |
| `return_window_days` | 30 | [fulfillment.yml](fulfillment.yml) |
| `defect_refund` | 전액 환불, 반품 배송비 무료 | [fulfillment.yml](fulfillment.yml) |
| `changed_mind_return_shipping` | 고객 부담 | [fulfillment.yml](fulfillment.yml) |
| `damage_claim_needs_photo` | true | [fulfillment.yml](fulfillment.yml) |
| `size_exchange_days` | 30, 무료 | [fulfillment.yml](fulfillment.yml) |
| `gift_exchange_until` | 12월 31일 | [fulfillment.yml](fulfillment.yml) |
| `customer_messages` | `always_ask` | [approval.yml](approval.yml) |
| `undo_window_after_end_days` | 1 | [approval.yml](approval.yml) `change_set` |
| `free_shipping_min_subtotal` | `{value: 80.00, currency: USD}`, 회원 혜택 적용 전 소계 기준 | [order.yml](order.yml) |
| `loyalty_grant` 승인 임계 | `{value: 50.00, currency: USD}` 초과 | [approval.yml](approval.yml) |
| `pricing_rule` (P2) | `cost * 2.5`, 끝자리 9로 반올림 | [catalog.yml](catalog.yml) |

## 2. 왜 데이터인가

- **에이전트가 규칙을 모른 채 어기지 않게.** 위반은 읽을 수 있는 이유와 고칠 툴(`fix_tool`)로 돌아옵니다.
- **두 단계 구현이 같은 판정.** Python 구현과 Rust 구현이 같은 YAML을 읽고, 같은 계약 테스트를 통과합니다.
- **배포 없이 규칙 변경.** 정책 파일의 버전을 올려 바꾸고, 변경은 감사 기록에 남습니다.
- **프리뷰·실행·검증이 같은 규칙.** 미리보기에서 통과한 규칙으로 실행하고 읽어 확인합니다.

## 3. 파일 구성

| 파일 | 도메인 | 핵심 내용 |
|---|---|---|
| [catalog.yml](catalog.yml) | catalog | **가격 하한(원가 + 15%)**, 가격 규칙(P2), 콘텐츠 자동·구조 변경 승인, 변형 단위 재고, 번들·상품권 제약 |
| [promotion.yml](promotion.yml) | promotion | **할인 상한**(VIP 30%, 전체 20%), **세일 최소 재고 10**, **주문당 회원 혜택 1개**, 프로모션 겹침, 쿠폰 상태·복원, 적립금, 사은품, 이벤트 |
| [order.yml](order.yml) | sales | **무료 배송 $80.00**(혜택 전 소계), 장바구니, 주문 검증 목록, 결제, 재고 예약, `OrderStatus` 이벤트 이력 |
| [fulfillment.yml](fulfillment.yml) | fulfillment | **반품 30일**, **$200.00 이하 무인 환불**, 불량·변심·파손 규칙, 반품 제안 규칙, 원 결제 수단 환불·쿠폰 차감, 취소·교환, 책임별 배송비, 배송 상태 |
| [approval.yml](approval.yml) | platform | **승인 임계**(환불 $200.00, 적립금 $50.00 등), 고객 메시지 항상 승인, 변경셋 되돌리기 기한(종료 + 1일), 사람 전용 엔드포인트 |
| [customer.yml](customer.yml) | customer | 회원 상태, 등급(Member·Gold·VIP), 동의 게이트, 개인정보 마스킹, 쇼퍼 기억, 탈퇴, 상담·보상 |
| [merchandising.yml](merchandising.yml) | merchandising | **슬롯 중복 예약 금지**, 카테고리 매핑, 진열 기간 계산·대상, 기획전 공개 = W$ |
| [search.yml](search.yml) | search | 검색 모드와 확장, 스토어 격리, 등급 필터를 쿼리 안에서, 랭킹 신호, 색인 교체 |

## 4. 백엔드가 쓰는 방식

```
정책 로더 (platform.policy)
  └─ 기동 시 policies/*.yml 로드 → 스키마 검증 → 메모리 (+ 스토어별 덮어쓰기)
쓰기 프리뷰
  └─ 규칙 평가 → checks[{rule, passed, detail, fix_tool}]
  └─ 승인 판정 → needs_approval, approval_reason   (approval.yml)
혜택 계산 (promotion.calculate_benefits)
  └─ max_discount · member_benefits_per_order · price_floor로 주문 혜택 계산
get_policy
  └─ 키별 값과 한 줄 설명을 반환 (에이전트·화면이 같은 값을 인용)
```

- **스토어별 덮어쓰기**: 기본 정책 + 스토어별 차이. 병합 규칙은 명시적입니다.
- **정책 스냅샷**: 주문·프로모션을 커밋할 때 적용된 정책 버전을 결과에 남깁니다(감사·분쟁 대응).

## 5. 정책 키 ↔ check `rule` ↔ 오퍼레이션

| 정책 키 | check `rule` | 적용 오퍼레이션 | 실패 시 `fix_tool` |
|---|---|---|---|
| `promotion.max_discount` | `max_discount` | `set_sale_prices`, `create_promotion` | `set_sale_prices` |
| `catalog.price_floor` | `price_floor` | `set_sale_prices`, `set_price`, `import_products` | `set_sale_prices` |
| `promotion.promo_min_stock` | `min_stock` | `set_sale_prices` | `set_sale_prices` |
| `promotion.promotion_overlap` | `promotion_overlap` | `set_sale_prices`, `create_promotion` | `list_active_promotions` |
| `promotion.member_benefits_per_order`, `promotion.coupon.stacking` | `coupon_stacking` | `create_coupon` | `create_coupon` |
| `merchandising.slot_free` | `slot_free` | `schedule_banner` | `list_display_slots` |
| `fulfillment.refund.to` | `refund_to_original_payment` | `refund_return` | `refund_return` |
| `fulfillment.refund.coupon_deducted` | `coupon_deducted` | `refund_return` | `refund_return` |
| `customer.consent.marketing_gate` | `consent_required` | `send_bulk`, `create_campaign` | `set_consent` |
| `approval.thresholds.*`, `approval.customer_messages` | (승인 판정) | 모든 W·W$ | (사람 승인) |

## 6. 검증과 관리

- 정책 파일은 스키마 검증(필수 키, 타입, 열거 값)을 CI에서 통과해야 합니다.
- 변경은 `version`을 올리고 변경 이력을 남깁니다.
- 계정·토큰·URL 같은 비밀은 정책 파일에 쓰지 않습니다.

데이터 타입과 명명 규칙은 [../data-model.md](../data-model.md), 전체 오퍼레이션 인덱스는 [../README.md](../README.md)를 보세요.
