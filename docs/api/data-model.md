# 데이터 모델 (schema.org 우선)

> **원칙: 모든 엔티티와 속성의 이름은 schema.org를 먼저 따릅니다.** schema.org에 있으면 그 이름을 그대로 쓰고, 없을 때만 `acom:` 네임스페이스로 확장합니다. 설계 배경은 [../architecture.md](../architecture.md) §5, MVP 계약은 [../plan.md](../plan.md) §3–4입니다.
> 이 문서가 데이터의 기준 모델입니다. 도메인 API의 입력·출력([README.md](README.md))과 두 단계 구현(Python, Rust)의 타입은 여기서 나옵니다.

---

## 1. 네임스페이스와 명명 규칙

```
schema:  https://schema.org/            (표준, 먼저 사용)
acom:    https://a-commerce.dev/ns#     (우리 확장, schema.org에 없는 것만)
```

1. **타입**: schema.org 타입 이름을 PascalCase 그대로 씁니다(`Product`, `Offer`, `Order`, `OrderItem`, `Person`, `PostalAddress`, `ParcelDelivery`, `MerchantReturnPolicy`). 없으면 `acom:Coupon`처럼 확장합니다.
2. **속성**: schema.org 속성 이름을 쓰되, API(wire)와 DB에서는 **snake_case**로 적습니다(`validFrom` → `valid_from`, `priceCurrency` → `price_currency`, `validForMemberTier` → `valid_for_member_tier`). 공개하는 JSON-LD 컨텍스트가 각 이름을 schema.org 용어로 되돌려 매핑하므로, 모든 응답은 그대로 유효한 JSON-LD입니다.
3. **열거값**: schema.org 열거 멤버를 snake_case 단어로 씁니다(`InStock` → `in_stock`, `OrderDelivered` → `delivered`, `FreeReturn` → `free_return`, `SalePrice` → `sale_price`). schema.org에 없으면 `acom:` 열거를 정의합니다. **코드값은 쓰지 않습니다.** 값은 항상 의미 있는 영단어입니다.
4. **확장**: 맞는 것이 없을 때만 `acom:` 네임스페이스에 만들고, 가장 가까운 schema.org 타입의 subtype으로 선언해 그 속성을 재사용합니다. 확장 속성도 snake_case입니다(`acom:percent_off`, `acom:safety_stock`).
5. **식별자**: 읽을 수 있는 접두 id(`prd_windbreaker`, `promo_1042`, `tier_vip`)를 `identifier`에 둡니다. DB 키는 따로 있습니다.
6. **금액**: 소수 둘째 자리 숫자 + `price_currency`(데모 스토어는 USD). 예: `"price": 71.20, "price_currency": "USD"`. 정책 파일과 집계 응답에서는 `MonetaryAmount` 형태 `{value, currency}`를 씁니다.
7. **시각**: ISO 8601, 스토어 시간대(데모 스토어는 US Eastern). DB에는 UTC로 저장합니다.

---

## 2. 엔티티 ↔ schema.org 매핑 (도메인별)

표기: ⚠️ = `acom:` 확장

### 2.1 catalog
| 엔티티 | schema.org 타입 | 핵심 속성 | 비고 |
|---|---|---|---|
| 상품(변형을 묶는 모델) | `ProductGroup` | `name`, `description`, `brand`, `category`, `image`, `has_variant`, `varies_by` | |
| 변형(구매 단위) | `Product` | `sku`, `name`, `color`, `size`, `image`, `offers`, `additional_property`, `is_variant_of` | |
| 옵션 축·값 | `PropertyValue` | `name`, `value` | `varies_by`의 색·사이즈 |
| 판매 조건 | `Offer` | `price`, `price_currency`, `availability`, `inventory_level`, `seller`, `price_specification` | 변형별 가격·가용성 |
| 정가 | `UnitPriceSpecification` | `price`, `price_currency`, `price_type`(`list_price`) | |
| 재고 수준 | `acom:InventoryLevel` ⚠️ (subtype of `QuantitativeValue`) | `value`(판매 가능 수량), `acom:safety_stock` | 변형 단위로만 관리 |
| 브랜드 | `Brand` | `name`, `logo` | |
| 원가 | `acom:CostSpecification` ⚠️ (subtype of `PriceSpecification`) | `price`, `price_currency` | **내부 전용, 쇼퍼에게 노출 안 함**. 가격 하한(원가 + 15%)의 기준 |
| 리뷰·평점 | `Review`, `AggregateRating` | `review_rating`, `author`, `review_body`, `rating_value`, `review_count` | |
| 상품 문의 | `Question`, `Answer` | `text`, `accepted_answer` | |
| 승인 상태 | `acom:approval_status` ⚠️ (속성) | `draft`, `pending_review`, `active`, `rejected`, `discontinued` | |

### 2.2 promotion
| 엔티티 | schema.org 타입 | 핵심 속성 | 비고 |
|---|---|---|---|
| 회원 판매가 | `UnitPriceSpecification` | `price`, `price_currency`, `price_type`(`sale_price`), `valid_for_member_tier`, `valid_from`, `valid_through`, `acom:percent_off` | `set_sale_prices`가 만드는 것. 정가는 바꾸지 않음 |
| 등급 프로그램 | `MemberProgram` | `name`, `has_tiers`(MemberProgramTier[]), `hosting_organization` | 등급 수·이름·순서는 스토어가 정한다(고정 목록 없음). 예시(데모 스토어): Member, Gold, VIP |
| 등급 | `MemberProgramTier` | `identifier`(예시 `tier_vip`), `name`, `acom:rank`(정렬 순서), `has_tier_requirement`, `has_tier_benefit`, `membership_points_earned` | `valid_for_member_tier`가 가리키는 대상. 정책의 등급별 값(할인 상한, 적립 배율 등)은 이 `identifier`를 키로 참조 |
| 등급 요건 | `has_tier_requirement`의 값. 단순하면 `MonetaryAmount`, 그 밖에는 `acom:TierRequirement` ⚠️ (subtype of `StructuredValue`) | `acom:metric`(`spend`, `order_count`, `points` 등), `min_value`, `acom:window_months` | 스토어가 요건 종류·기준을 정의 |
| 등급 혜택 | `has_tier_benefit`의 값. `acom:TierBenefit` ⚠️ (subtype of `Offer`) | `name`, `acom:benefit_kind`(`discount`, `loyalty_multiplier`, `free_shipping`, `coupon_bundle`, `custom`), `acom:benefit_value` | 혜택 종류도 레지스트리 |
| 프로모션 | `acom:Promotion` ⚠️ (subtype of `Offer`) | `name`, `valid_from`, `valid_through`, `valid_for_member_tier`, `price_specification`(회원 판매가[]), `acom:promotion_status` | 기간 동안의 회원 판매가 묶음 |
| 쿠폰 | `acom:Coupon` ⚠️ (subtype of `Offer`) | `identifier`, `acom:code`, `acom:promotion`, `valid_for_member_tier`, `acom:auto_apply`, `acom:stacking`, `acom:coupon_status` | 주문에는 `discount_code`로 남음. 같은 혜택을 주문에 기록할 뿐 할인을 더하지 않음 |
| 쿠폰 상태 | `acom:coupon_status` ⚠️ | `issued`, `available`, `used`, `restored`, `expired`, `revoked` | |
| 적립금 거래 | `acom:LoyaltyTransaction` ⚠️ (subtype of `MoneyTransfer`) | `amount`, `acom:accrual_type`, `acom:expires`, `acom:source_order` | 이벤트로 쌓음 |
| 적립금 잔액 | `MonetaryAmount` | `value`, `currency` | 거래에서 파생 |
| 사은품 조건 | `acom:ConditionalGift` ⚠️ (subtype of `Offer`) | `acom:min_subtotal`, `acom:gift_product`, `valid_for_member_tier` | |
| 이벤트 | `Event` | `name`, `start_date`, `end_date`, `acom:event_state`, `acom:entry_mechanic`, `acom:selection_method`, `acom:reward_tiers`, `acom:eligibility`, `acom:entry_limit` | 응모 방식·당첨 방식은 아래 레지스트리의 id |
| 이벤트 유형 레지스트리 | `acom:EventTypeDefinition` ⚠️ (subtype of `DefinedTerm`) in `DefinedTermSet` | `term_code`(snake_case id), `name`, `acom:kind`(`entry_mechanic` \| `selection_method`), `acom:builtin`, `acom:enabled`, `acom:entry_payload_schema` | 기본 제공 항목 + 스토어 정의 항목 |
| 보상 종류 레지스트리 | `acom:RewardTypeDefinition` ⚠️ (subtype of `DefinedTerm`) in `DefinedTermSet` | `term_code`, `name`, `acom:fulfillment`(`ship` \| `issue` \| `grant` \| `email` \| `manual`), `acom:has_value`, `acom:enabled` | |
| 응모 | `acom:EventEntry` ⚠️ (subtype of `JoinAction`) | `agent`, `object`(Event), `acom:entry_payload`, `acom:entry_status`, `start_time` | |
| 보상 | `acom:EventReward` ⚠️ (subtype of `Offer`) | `acom:rank`, `acom:reward_type`, `acom:reward_ref`, `acom:reward_value`(MonetaryAmount), `acom:reward_status` | 지급은 `acom:fulfillment`에 따라 배송·쿠폰·적립금 원장·이메일 |
| 추첨 기록 | `acom:DrawRecord` ⚠️ (subtype of `ChooseAction`) | `agent`, `object`(Event), `acom:method`, `acom:seed_commitment`, `acom:seed`, `acom:entrant_snapshot_digest`, `result` | 추가만, 고치지 않음 |
| 컴플라이언스 규칙 | `acom:ComplianceRule` ⚠️ (subtype of `PropertyValue`) | `acom:jurisdiction`, `acom:applies_when`, `acom:actions`, `acom:winner_info_required` | 금액·세율은 스토어가 관할별로 정의 |
| 세그먼트 | `acom:Segment` ⚠️ (subtype of `Audience`) | `audience_type`, `acom:conditions`, `version` | |

### 2.3 sales
| 엔티티 | schema.org 타입 | 핵심 속성 | 비고 |
|---|---|---|---|
| 장바구니 | `acom:Cart` ⚠️ (subtype of `Order`) | `ordered_item`(OrderItem[]), `customer` | 결제 전(`payment_due` 이전)의 주문 |
| 주문 | `Order` | `order_number`, `order_date`, `order_status`, `customer`, `seller`, `ordered_item`, `payment_method`, `discount`, `discount_code`, `part_of_invoice` | |
| 주문 항목 | `OrderItem` | `ordered_item`(Product), `order_quantity`, `order_item_status` | 라인별 상태 독립 |
| 주문 상태 | `OrderStatus` | `payment_due`, `processing`, `in_transit`, `delivered`, `returned`, `cancelled`, `problem` | schema.org 멤버 그대로 |
| 결제 수단 | `PaymentMethod` | `name`, `acom:card_last4` | 카드 번호는 끝 4자리만 |
| 청구 | `Invoice` | `total_payment_due`, `payment_status`, `customer`, `provider` | |
| 결제 상태 | `PaymentStatusType` | `payment_due`, `payment_complete`, `payment_declined`, `payment_past_due` | |

### 2.4 fulfillment
| 엔티티 | schema.org 타입 | 핵심 속성 | 비고 |
|---|---|---|---|
| 배송 | `ParcelDelivery` | `delivery_address`, `tracking_number`, `tracking_url`, `carrier`, `expected_arrival_until`, `part_of_order`, `has_delivery_method` | |
| 배송 이벤트 | `DeliveryEvent` | `start_time`, `acom:delivery_state` | 이벤트로 쌓음 |
| 배송 상태 | `acom:delivery_state` ⚠️ | `label_created`, `picked_up`, `in_transit`, `delivered`, `delivery_failed`, `waiting_for_stock` | |
| 반품 정책 | `MerchantReturnPolicy` | `merchant_return_days`(30), `return_policy_category`(`finite_return_window`), `return_fees`, `return_method`, `refund_type`(`full_refund`) | 불량은 `free_return`, 단순 변심은 `return_fees_customer_responsibility` |
| 반품 요청 | `ReturnAction` | `object`(주문 라인), `agent`(고객), `action_status`, `acom:reason`, `acom:evidence`(사진), `result`(환불) | 쇼퍼가 `POST /returns`로 보내면 `potential` 상태로 대기열에 들어감 |
| 취소 | `CancelAction` | `object`(Order 또는 OrderItem), `agent`, `action_status`, `acom:reason` | |
| 교환 | `acom:ExchangeAction` ⚠️ (subtype of `ReturnAction`) | `object`, `acom:replacement`, `result` | 회수 후 재배송 |
| 환불 | `acom:Refund` ⚠️ (subtype of `MoneyTransfer`) | `amount`, `acom:refund_method`(`original_payment`), `acom:refund_status` | |
| 검수 결과 | `acom:ConditionAssessment` ⚠️ (subtype of `AssessAction`) | `object`, `result`(`as_new`, `damaged`, `missing_parts`) | |
| 배송비 | `OfferShippingDetails`, `ShippingRateSettings` | `shipping_destination`, `shipping_rate`, `acom:free_shipping_min_subtotal` | 데모: $80.00 이상 무료(회원 혜택 적용 전 금액 기준) |

### 2.5 customer · messaging
| 엔티티 | schema.org 타입 | 핵심 속성 | 비고 |
|---|---|---|---|
| 회원 | `Person` | `name`, `email`, `telephone`, `address`, `member_of` | 개인정보 암호화·마스킹 |
| 사업자 회원 | `Organization` | `legal_name`, `tax_id`, `contact_point` | |
| 멤버십 | `ProgramMembership` | `program`(MemberProgram), `membership_number`, `membership_points_earned`, `acom:tier` | |
| 주소 | `PostalAddress` | `street_address`, `address_locality`, `address_region`, `postal_code`, `address_country`, `acom:is_primary` | |
| 연락처 | `ContactPoint` | `telephone`, `email`, `contact_type` | |
| 기억한 선호 | `acom:Preference` ⚠️ (subtype of `PropertyValue`) | `name`, `value`, `acom:source`(`conversation`), `acom:undo` | 쇼퍼에게 보이고 되돌릴 수 있음 |
| 동의 기록 | `acom:ConsentRecord` ⚠️ (subtype of `Action`) | `acom:consent_type`, `acom:granted`, `start_time` | 바꾸지 않는 이력 |
| 상담 문의 | `acom:CustomerInquiry` ⚠️ (subtype of `Message`) | `acom:inquiry_type`, `acom:inquiry_status`, `about`(Order) | |
| 보상 | `acom:Remedy` ⚠️ (subtype of `Action`) | `acom:remedy_type`, `action_status` | |
| FAQ · 공지 | `FAQPage`, `Question`, `Answer`, `NewsArticle` | `headline`, `article_body`, `date_published` | |
| 고객 메시지 | `Message` | `recipient`, `text`, `about`, `acom:message_kind`(`ask_for_photos`, `decline`, `refund_done`) | 정책상 보내기 전 항상 승인 |

### 2.6 merchandising · search · community
| 엔티티 | schema.org 타입 | 핵심 속성 | 비고 |
|---|---|---|---|
| 카테고리 | `DefinedTerm` in `DefinedTermSet` | `name`, `term_code`, `in_defined_term_set`, `acom:parent` | 내부용·전시용 두 세트 |
| 진열 슬롯 | `acom:DisplaySlot` ⚠️ (subtype of `ItemList`) | `item_list_element`, `acom:page`(`main`), `acom:slot_type`(`hero`) | |
| 배너 | `ImageObject` | `content_url`, `headline`, `acom:eyebrow`, `acom:cta`, `url`, `valid_for_member_tier`, `valid_from`, `valid_through` | |
| 기획전 | `CollectionPage`, `ItemList` | `name`, `item_list_element` | |
| 검색 | `SearchAction` | `query`, `acom:filters`, `result` | |
| 검색 결과 | `ItemList` | `item_list_element`, `number_of_items` | |
| 게시글 · 댓글 | `DiscussionForumPosting`, `Comment` | `headline`, `article_body`, `text`, `author`, `parent_item` | |

### 2.7 platform
| 엔티티 | schema.org 타입 | 핵심 속성 | 비고 |
|---|---|---|---|
| 변경셋 | `acom:ChangeSet` ⚠️ (subtype of `ItemList`) | `item_list_element`(액션[]), `version`, `acom:change_set_status`, `acom:undo_until` | 상태: `drafting`, `waiting_for_approval`, `running`, `verifying`, `live`, `failed`, `rejected`, `undone` |
| 액션 로그 | 각 schema.org `Action` | `agent`, `object`, `instrument`, `action_status`, `result`, `error`, `start_time`, `end_time` | §3 |
| 정책 | `acom:Policy` ⚠️ (subtype of `PropertyValue`) | `name`, `value`, `description` | [policies/](policies/README.md) |

---

## 3. 오퍼레이션은 schema.org 액션

명사는 schema.org 사물이고, **동사는 schema.org 액션**입니다. 모든 오퍼레이션은 자기가 수행하는 액션 타입으로 분류되고, 호출·프리뷰·승인·실행은 모두 액션 인스턴스로 기록됩니다.

| 오퍼레이션 | schema.org 액션 |
|---|---|
| `search_products`, `list_*` | `SearchAction` |
| `evaluate_return_request`, `verify_change_set` | `AssessAction` |
| `compare_products` | `acom:CompareAction` (subtype of `AssessAction`) |
| `set_sale_prices` | `Offer` 가격에 대한 `UpdateAction` |
| `create_coupon` | `CreateAction` |
| `schedule_banner` | `ScheduleAction` |
| `add_to_cart` | `AddAction` |
| `refund_return` | `acom:RefundAction` (subtype of `PayAction`) |
| `send_customer_message` | `Message`를 보내는 `SendAction` |
| `prepare_order` | `potential` 상태의 `OrderAction`. 쇼퍼가 버튼을 눌러야 완료 |
| 반품 요청 보내기(사람 전용) | `ReturnAction` |
| 변경셋 승인(사람 전용) | `Person`의 `AuthorizeAction` |

`action_status`(schema.org `ActionStatusType`)가 수명 주기 전체를 하나의 어휘로 표현합니다.

| 단계 | `action_status` |
|---|---|
| 프리뷰, 승인 대기 | `potential` |
| 실행 중 | `active` |
| 실행되고 읽어 확인됨 | `completed` + `result` |
| 읽어 보니 다름 | `failed` + `error` |

```jsonc
// 변경셋 #1042 안의 변경 하나 (액션 로그에 저장된 모습)
{
  "@type": "UpdateAction",
  "identifier": "act_1042_03",
  "action_status": "potential",
  "agent": {"@type": "SoftwareApplication", "name": "muse-glimmer-30b"},
  "instrument": "set_sale_prices",
  "object": {"@type": "Offer", "identifier": "off_windbreaker_m_slate"},
  "result": {
    "@type": "UnitPriceSpecification",
    "price": 71.20, "price_currency": "USD", "price_type": "sale_price",
    "valid_for_member_tier": "tier_vip",
    "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00"
  }
}
```

---

## 4. `acom:` 확장 요약

schema.org에 없어 새로 만든 것은 대부분 정책·상태·커머스 특유의 개념입니다. 각 확장은 가장 가까운 schema.org 타입의 subtype이며 그 속성을 최대한 재사용합니다.

| 확장 | subtype of | schema.org만으로 부족한 이유 |
|---|---|---|
| `acom:Promotion` | `Offer` | 기간 동안의 회원 판매가를 묶는 프로모션이 없음 |
| `acom:Coupon` | `Offer` | 주문의 `discount_code`만 있고 중복 적용·발급 규칙이 없음 |
| `acom:Cart` | `Order` | 장바구니 타입이 없음. 결제 전의 주문으로 표현 |
| `acom:Refund` | `MoneyTransfer` | 수단과 상태를 가진 환불 타입이 없음 |
| `acom:ChangeSet` | `ItemList` | 검사와 되돌리기 기한을 가진, 버전 있는 액션 묶음 |
| `acom:Preference` | `PropertyValue` | 알게 된 경로(`source`)와 되돌리기를 가진 쇼퍼 선호 기억 |
| `acom:CompareAction`, `acom:RefundAction` | `AssessAction`, `PayAction` | `compare_products`, `refund_return`의 동사 |
| `acom:ExchangeAction` | `ReturnAction` | 회수와 재배송을 함께 가진 교환 |
| `acom:InventoryLevel`, `acom:CostSpecification` | `QuantitativeValue`, `PriceSpecification` | 안전 재고, 내부 전용 원가 |
| `acom:DisplaySlot`, `acom:Segment` | `ItemList`, `Audience` | 페이지의 진열 자리, 조건으로 정의한 고객 묶음 |
| `acom:percent_off` | `UnitPriceSpecification`의 속성 | 계산된 판매가 뒤에 있는 요청 할인율 |
| `acom:coupon_status`, `acom:delivery_state`, `acom:approval_status` | 열거 | schema.org에 해당 상태 열거가 없음 |
| `acom:EventTypeDefinition`, `acom:RewardTypeDefinition` | `DefinedTerm` | 스토어가 늘릴 수 있는 이벤트·보상 유형 레지스트리 |
| `acom:EventEntry`, `acom:EventReward`, `acom:DrawRecord` | `JoinAction`, `Offer`, `ChooseAction` | 응모, 등수별 보상, 재현 가능한 추첨 감사 기록 |
| `acom:TierRequirement`, `acom:TierBenefit` | `StructuredValue`, `Offer` | 스토어가 정의하는 등급 요건·혜택 |
| `acom:ComplianceRule` | `PropertyValue` | 관할별 경품 신고·원천징수·정보 수집 요구 |

schema.org 열거(`OrderStatus`, `ItemAvailability`, `ActionStatusType`, `PriceTypeEnumeration`, `ReturnFeesEnumeration`)는 그대로 재사용합니다. 확장 정의는 JSON-LD 컨텍스트 파일로 공개합니다.

---

## 5. DB 명명 규칙

해커톤 단계는 SQLite, 다음 단계는 도메인별 스키마(`catalog`, `sales`, `fulfillment` …)를 둔 PostgreSQL입니다. 이름 규칙은 같습니다.

### 5.1 규칙
1. **테이블**: schema.org 타입의 snake_case, 단수(`product_group`, `product`, `offer`, `unit_price_specification`, `order_item`, `merchant_return_policy`, `parcel_delivery`, `return_action`).
2. **SQL 예약어**: 도메인 접두를 붙입니다(`Order` → `sales_order`).
3. **컬럼**: schema.org 속성의 snake_case(`price_currency`, `valid_from`, `valid_for_member_tier`, `order_status`). 확장 속성은 접두 없이 snake_case로 두고(`percent_off`, `safety_stock`) 문서에서 `acom:`임을 표시합니다.
4. **키**: PK는 `id`(시간순 UUID). 읽을 수 있는 식별자(`prd_windbreaker`, `order_number`)는 별도 unique 컬럼이며 PK로 쓰지 않습니다. FK는 `<type>_id`(`product_id`, `order_id`).
5. **열거**: 열거 단어를 저장합니다(`'in_stock'`, `'delivered'`, `'sale_price'`). 코드값은 쓰지 않습니다.
6. **금액**: `numeric(12,2)` + 통화 컬럼(`price_currency char(3)`). float 금지.
7. **시각**: UTC로 저장(`timestamptz`). 모든 행에 `created_at`, `updated_at`.
8. **스토어 범위**: 스토어에 속한 모든 테이블에 `tenant_id`(not null), PostgreSQL에서는 행 수준 보안. MVP는 스토어 하나.
9. **상태 이력**: 이벤트 테이블(`order_status_event`, `return_action_event`, `action_log`)에 추가만 하고 고치지 않습니다. 현재 상태는 여기서 파생합니다.
10. **가변 속성**: 옵션 조합, 세그먼트 조건, 정책 스냅샷처럼 형태가 바뀌는 값은 `jsonb`(SQLite는 JSON 텍스트). 자주 조회하는 값은 일반 컬럼으로.

### 5.2 예시 DDL (명명 규칙 예시)

```sql
-- catalog.product  ← Product (변형 단위)
create table catalog.product (
  id               uuid primary key,
  tenant_id        uuid not null,
  identifier       text not null,                                 -- identifier: 'prd_windbreaker_m_slate'
  product_group_id uuid references catalog.product_group(id),     -- is_variant_of
  sku              text not null,                                 -- sku
  name             text not null,                                 -- name
  color            text,                                          -- color
  size             text,                                          -- size
  approval_status  text not null default 'draft'                  -- acom:approval_status
                   check (approval_status in ('draft','pending_review','active','rejected','discontinued')),
  created_at       timestamptz not null default now(),
  updated_at       timestamptz not null default now(),
  unique (tenant_id, identifier),
  unique (tenant_id, sku)
);

-- catalog.offer  ← Offer (변형별 판매 조건)
create table catalog.offer (
  id              uuid primary key,
  tenant_id       uuid not null,
  product_id      uuid not null references catalog.product(id),
  price           numeric(12,2) not null,                         -- price: 89.00
  price_currency  char(3) not null default 'USD',                 -- price_currency
  availability    text not null default 'in_stock'                -- availability (ItemAvailability)
                  check (availability in ('in_stock','out_of_stock','pre_order','discontinued')),
  inventory_level integer not null default 0,                     -- inventory_level
  created_at      timestamptz not null default now(),
  updated_at      timestamptz not null default now()
);

-- promotion.unit_price_specification  ← UnitPriceSpecification (회원 판매가)
create table promotion.unit_price_specification (
  id                    uuid primary key,
  tenant_id             uuid not null,
  promotion_id          uuid not null references promotion.promotion(id),
  offer_id              uuid not null references catalog.offer(id),
  price                 numeric(12,2) not null,                   -- price: 71.20
  price_currency        char(3) not null default 'USD',           -- price_currency
  price_type            text not null default 'sale_price',       -- price_type (PriceTypeEnumeration)
  valid_for_member_tier text,                                     -- valid_for_member_tier: 'tier_vip' (null = everyone)
  valid_from            timestamptz not null,                     -- valid_from
  valid_through         timestamptz not null,                     -- valid_through
  percent_off           numeric(5,2) not null,                    -- acom:percent_off: 20.00
  created_at            timestamptz not null default now(),
  updated_at            timestamptz not null default now()
);

-- sales.sales_order  ← Order (예약어라 접두)
create table sales.sales_order (
  id            uuid primary key,
  tenant_id     uuid not null,
  order_number  text not null,                                    -- order_number: '20402'
  order_date    timestamptz not null,                             -- order_date
  order_status  text not null,                                    -- order_status (OrderStatus): 'delivered'
  customer_id   uuid references customer.person(id),              -- customer
  discount_code text,                                             -- discount_code: 'VIPWEEKEND20'
  created_at    timestamptz not null default now(),
  updated_at    timestamptz not null default now(),
  unique (tenant_id, order_number)
);

-- fulfillment.return_action  ← ReturnAction (반품 요청)
create table fulfillment.return_action (
  id            uuid primary key,
  tenant_id     uuid not null,
  order_item_id uuid not null references sales.order_item(id),    -- object
  agent_id      uuid not null references customer.person(id),     -- agent
  action_status text not null default 'potential'                 -- action_status (ActionStatusType)
                check (action_status in ('potential','active','completed','failed')),
  reason        text not null,                                    -- acom:reason: 'defect'
  created_at    timestamptz not null default now(),
  updated_at    timestamptz not null default now()
);

-- sales.order_status_event  ← 상태 이력(추가만, 덮어쓰지 않음)
create table sales.order_status_event (
  id          uuid primary key,
  tenant_id   uuid not null,
  order_id    uuid not null references sales.sales_order(id),
  from_status text,
  to_status   text not null,
  agent_id    uuid,
  reason      text,
  start_time  timestamptz not null default now()
);
```

---

## 6. 연결

- 도메인 API의 입력·출력은 이 모델의 타입과 속성을 씁니다([README.md](README.md)).
- 정책(할인 상한, 가격 하한, 반품 기간, 승인 임계)은 데이터이며 [policies/](policies/README.md)에 있습니다. 상태와 규칙은 이 모델의 이벤트와 제약으로 강제됩니다.
- 스토어 범위·스키마 분리·행 수준 보안은 [../architecture.md](../architecture.md) §2.4, §5.6을 따릅니다.
