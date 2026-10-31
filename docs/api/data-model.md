# 데이터 모델 (schema.org 우선)

> **원칙: 모든 엔티티와 속성의 이름은 schema.org를 먼저 따릅니다.** schema.org에 있으면 그 이름을 그대로 쓰고, 없을 때만 `acom:` 네임스페이스로 확장합니다. 설계 배경은 [../architecture.md](../architecture.md) §5, 데모 데이터는 [../scenarios.md](../scenarios.md)입니다.
> 이 문서가 데이터의 기준 모델입니다. 도메인 API의 입력·출력([README.md](README.md))과 Rust 타입(`acom-core`, `acom-contract`, 도메인 크레이트, [../backend/README.md](../backend/README.md) §2), 그리고 그 타입에서 생성하는 JSON Schema·OpenAPI는 여기서 나옵니다. 상태·종류·사유의 값 목록은 [enums.md](enums.md)가 단일 출처이고 이 문서는 그것을 가리키기만 합니다.

---

## 1. 네임스페이스와 명명 규칙

```
schema:  https://schema.org/            (표준, 먼저 사용)
acom:    https://metahackai.github.io/a-commerce/vocab#   (우리 확장, vocab/acom.jsonld가 정의. 규칙은 vocabulary.md)
```

1. **타입**: schema.org 타입 이름을 PascalCase 그대로 씁니다(`ProductGroup`, `Product`, `Offer`, `Order`, `OrderItem`, `Person`, `PostalAddress`, `ParcelDelivery`, `MerchantReturnPolicy`). schema.org 타입의 한 종류는 그 타입에 `additionalType`을 답니다(쿠폰 = `Offer` + `acom:Coupon`, §4).
2. **속성**: schema.org 속성 이름을 쓰되, API(wire)와 DB에서는 **snake_case**로 적습니다(`validFrom` → `valid_from`, `priceCurrency` → `price_currency`, `validForMemberTier` → `valid_for_member_tier`). 응답은 JSON-LD 컨텍스트가 붙지 않은 보통 JSON이고, 이름이 어느 용어인지는 DB 주석에서 만든 데이터 사전(`describe_data`, [../backend/schema.generated.md](../backend/schema.generated.md))이 알려 줍니다.
3. **열거값**: schema.org 열거 멤버를 snake_case 단어로 씁니다(`InStock` → `in_stock`, `OrderDelivered` → `delivered`, `FreeReturn` → `free_return`, `SalePrice` → `sale_price`). schema.org에 없으면 `acom:` 열거를 정의합니다. **코드값은 쓰지 않습니다.** 값 목록은 [enums.md](enums.md).
4. **확장**: schema.org 타입을 먼저 쓰고, 특수화는 `@type`(schema.org) + `additionalType`(`acom:`)으로 씁니다. 새 프로퍼티·타입은 마지막 수단이며 `vocab/acom.jsonld`에 상위 타입과 함께 정의합니다([vocabulary.md](vocabulary.md)). 전선 위에서는 확장 프로퍼티도 snake_case입니다(`percent_off`, `safety_stock`).
5. **식별자**: 접두 id(`prd_windbreaker`, `var_windbreaker_m_slate`, `cs_1042`). 접두 표와 규칙은 [conventions.md](conventions.md) §2·§9. 엔티티 자신의 id 필드는 `id`, 다른 엔티티를 가리키면 `<entity>_id`입니다. 회원 등급 id는 스토어가 정한 단어(`vip`, `gold`)이고 `"everyone"`은 예약어입니다([enums.md](enums.md) §5). 사람에게 보이는 번호는 따로 둡니다(`order_number: "20402"`, 변경셋 `number: 1042`).
6. **금액**: schema.org `MonetaryAmount` 형태 `{value, currency}`. 가격 필드는 schema.org를 따라 `price` + `price_currency`로 펼쳐 쓸 수 있습니다(`Offer`, `UnitPriceSpecification`). 정확한 십진수, 센트 반올림([conventions.md](conventions.md) §3).
7. **비율**: 툴 입력의 `percent_off`·`*_percent`는 0–100, 정책 파일과 `*_rate`는 0–1([conventions.md](conventions.md) §4).
8. **시각**: 출력은 RFC 3339 + 스토어 오프셋(데모 스토어는 America/New_York, 11월 기준 `-05:00`), 날짜만이면 `YYYY-MM-DD`. DB에는 UTC로 저장합니다([conventions.md](conventions.md) §5).

---

## 2. 엔티티 ↔ schema.org 매핑 (도메인별)

표기: ⚠️ = `acom:` 확장(`@type`은 괄호 안의 schema.org 타입, 우리 용어는 `additionalType`). 값 목록 열은 [enums.md](enums.md)의 절을 가리킵니다. 이 표는 핵심 엔티티만 담습니다. 모든 테이블의 타입과 컬럼은 마이그레이션 주석에서 생성한 [../backend/schema.generated.md](../backend/schema.generated.md)에 있습니다(`cargo xtask check-schema`).

### 2.1 catalog
| 엔티티 | schema.org 타입 | 핵심 속성 | 비고 |
|---|---|---|---|
| 상품(사람이 말하는 상품, `prd_`) | `ProductGroup` | `name`, `description`, `brand`, `category`, `image`, `has_variant`, `varies_by` | 사람과 모델은 상품으로 이야기함 |
| 변형(사이즈·색, SKU 단위, `var_`) | `Product` | `sku`, `name`, `color`, `size`, `image`, `offers`, `additional_property`, `is_variant_of` | 장바구니·재고·주문 품목은 변형을 씀 |
| 옵션 축·값 | `PropertyValue` | `name`, `value` | `varies_by`의 색·사이즈 |
| 판매 조건(`off_`) | `Offer` | `price`, `price_currency`, `availability`, `inventory_level`, `price_specification` | 변형별 가격·가용성. `availability`는 enums §1.10. 입점 판매자의 상품은 partner의 연결(`acom:SellerProduct`)로 |
| 정가 | `UnitPriceSpecification` | `price`, `price_currency`, `price_type`(`list_price`) | |
| 재고 수준 | `Offer`의 속성 | `inventory_level`(판매 가능 수량), `acom:safety_stock` | 변형 단위로만 관리. 프리뷰의 `changes[].entity`는 `inventory_level` |
| 브랜드(`brn_`) | `Brand` | `name`, `logo` | |
| 원가 | `Offer`의 속성 `acom:inventory_cost` ⚠️ | DB `cost_value` + `cost_currency` | **내부 전용, 쇼퍼에게 노출 안 함**. 가격 하한(`price_floor`)의 기준 |
| 리뷰·평점(`rev_`) | `Review`, `AggregateRating` | `review_rating`, `author`, `review_body`, `rating_value`, `review_count` | |
| 상품 문의(`qst_`/`ans_`) | `Question`, `Answer` | `text`, `accepted_answer` | |
| 승인 상태 | `acom:approval_status` ⚠️ (속성) | enums §1.10 | |

### 2.2 promotion
| 엔티티 | schema.org 타입 | 핵심 속성 | 비고 |
|---|---|---|---|
| 회원 판매가(`ups_`) | `UnitPriceSpecification` | `price`, `price_currency`, `price_type`(`sale_price`), `valid_for_member_tier`, `valid_from`, `valid_through`, `acom:percent_off` | `set_sale_prices`가 변형 `Offer`마다 만드는 것. 정가는 바꾸지 않음 |
| 등급 프로그램(`prg_`) | `MemberProgram` | `name`, `description`, `acom:evaluation`(평가 일정), `acom:movement`(등급 이동 규칙). 등급은 `MemberProgramTier`가 `is_tier_of`로 가리킴 | 등급 수·이름·순서는 스토어가 정한다(고정 목록 없음). 데모 스토어는 [../scenarios.md](../scenarios.md) §2.1 |
| 등급 | `MemberProgramTier` | `id`(스토어가 정한 단어, 예 `vip`), `name`, `acom:rank`(정렬 순서), `has_tier_requirement`, `has_tier_benefit`, `membership_points_earned` | `valid_for_member_tier`가 가리키는 대상. 정책의 등급별 값(할인 상한, 적립 배율 등)은 이 id를 키로 참조 |
| 등급 요건 | `has_tier_requirement`의 값. 단순하면 `MonetaryAmount`, 그 밖에는 `acom:TierRequirement` ⚠️ (`StructuredValue`) | `acom:metric`(enums §1.13 등급 조건 지표), `threshold`, `retain_threshold`, `window` | 스토어가 요건 종류·기준을 정의 |
| 등급 혜택 | `has_tier_benefit`의 값(JSON 목록) | `type`(enums §1.13 등급 혜택 종류)과 종류별 값(예 `recurring_coupon`의 `coupon_id`, `every`) | `member_price` → `TierBenefitLoyaltyPrice` 등 |
| 프로모션(`promo_`) | `acom:Promotion` ⚠️ (`OfferCatalog`) | `name`, `valid_from`, `valid_through`, `valid_for_member_tier`, `acom:promotion_type`, `acom:percent_off`, `acom:discount_layer`, `acom:promotion_status` | 회원 판매가는 `promotion_id`로 이 프로모션을 가리킴. 상태·할인 층은 enums §1.11 |
| 쿠폰(`cpn_`) | `acom:Coupon` ⚠️ (`Offer`) | `discount_code`, `acom:promotion_id`, `valid_for_member_tier`, `acom:application`, `acom:stacking`, `valid_from`, `valid_through` | 주문에는 `discount_code`로 남음. 같은 혜택을 주문에 기록할 뿐 할인을 더하지 않음. 적용 방식은 enums §1.11 |
| 회원 쿠폰(`mcp_`) | `acom:MemberCoupon` ⚠️ (`OwnershipInfo`) | `acom:coupon_id`, `customer`, `acom:coupon_status`, `owned_from`, `owned_through` | 회원에게 발급된 쿠폰 한 장. 상태는 enums §1.11 |
| 적립금 거래(`lyt_`) | `acom:LoyaltyTransaction` ⚠️ (`MoneyTransfer`) | `amount`, `acom:loyalty_transaction_type`, `expires`, `acom:order_id` | 원장에 추가만 |
| 적립금 잔액 | `MonetaryAmount` | `value`, `currency` | 거래에서 파생 |
| 사은품 조건(`cgf_`) | `acom:ConditionalGift` ⚠️ (`Offer`) | `acom:gift_tiers`(기준 금액과 사은품), `valid_for_member_tier`, `valid_from`, `valid_through` | |
| 이벤트(`evt_`) | `Event` | `name`, `start_date`, `end_date`, `acom:event_status`, `acom:entry_mechanic`, `acom:selection_method`, `acom:reward_tiers`, `acom:eligibility`, `acom:entry_limit` | 상태는 enums §1.11, 응모·당첨 방식은 아래 레지스트리의 id |
| 이벤트 유형 레지스트리 | 정책 데이터(테이블 없음) | `id`(snake_case), `enabled`, `entry_payload_schema` | [promotion.yml](../../policies/promotion.yml) `event.entry_mechanics`·`event.selection_methods`의 기본 제공 항목 + 스토어 정의 항목. `list_event_types`가 읽음 |
| 보상 종류 레지스트리 | 정책 데이터(테이블 없음) | `id`, `fulfillment`(`ship` \| `issue` \| `grant` \| `email` \| `manual`), `has_value`, `enabled` | [promotion.yml](../../policies/promotion.yml) `event.reward_types`. `list_reward_types`가 읽음 |
| 응모(`ent_`) | `acom:EventEntry` ⚠️ (`JoinAction`) | `agent`, `object`(Event), `acom:entry_payload`, `acom:entry_status`, `start_time` | |
| 보상(`rwd_`) / 일괄 지급(`rwb_`) | `acom:EventReward` ⚠️ (`Offer`) | `acom:rank`, `acom:reward_type`, `acom:reward_ref`, `acom:reward_value`(MonetaryAmount), `acom:reward_status` | 지급은 `acom:fulfillment`에 따라 배송·쿠폰·적립금 원장·이메일 |
| 추첨 기록(`drw_`) | `acom:DrawRecord` ⚠️ (`ChooseAction`) | `agent`, `object`(Event), `acom:method`, `acom:seed_commitment`, `acom:seed`, `acom:entrant_snapshot_digest`, `result` | 추가만, 고치지 않음 |
| 컴플라이언스 규칙 | `acom:ComplianceRule` ⚠️ (`PropertyValue`), 정책 데이터 | `jurisdiction`, `applies_when`(구조화 조건), `actions`, `winner_info_required` | [promotion.yml](../../policies/promotion.yml) `event.compliance.rules`. 금액·세율은 스토어가 관할별로 정의 |
| 세그먼트(`seg_`) | `acom:Segment` ⚠️ (`Audience`) | `name`, `acom:conditions`, `acom:estimated_size` | |

### 2.3 sales
| 엔티티 | schema.org 타입 | 핵심 속성 | 비고 |
|---|---|---|---|
| 장바구니(`cart_`) / 장바구니 줄(`cln_`) | `acom:Cart` ⚠️ (`Order`) | `ordered_item`(OrderItem[]), `customer` | 결제 전의 주문. 줄은 변형을 가리킴 |
| 주문(`ord_`) | `Order` | `order_number`, `order_date`, `order_status`, `customer`, `ordered_item`(주문 품목), `discount`, `discount_code`, `total_payment_due` | `order_status`는 enums §1.1. 결제는 따로(아래) |
| 주문 품목(`oln_`) | `OrderItem` | `ordered_item`(Product = 변형), `order_quantity`, `order_item_status`, `acom:fulfillment_stage` | 품목별 상태 독립(enums §1.1, 출고 단계 §1.2) |
| 결제 수단 | `PaymentMethod` | `name`, `acom:card_last4` | 카드 번호는 끝 4자리만. 수단 값은 enums §3 |
| 결제(`pay_`) | `PayAction` | `payment_method`, `payment_status`, `amount`, `acom:card_last4` | `payment_status`는 enums §1.4 |
| 영수증(`rct_`) | `acom:Receipt` ⚠️ (`Invoice`) | `references_order`, `acom:receipt_type`, `total_payment_due` | |
| 프리뷰(`pv_`) | `Action`(`action_status: potential`) | `operation`, `changes`, `checks`, `needs_approval`, `expires_at`, `acom:preview_state` | 주문 준비·구독 준비도 프리뷰. 상태는 enums §1.9 |

### 2.4 fulfillment
| 엔티티 | schema.org 타입 | 핵심 속성 | 비고 |
|---|---|---|---|
| 배송(`dlv_`) | `ParcelDelivery` | `delivery_address`, `tracking_number`, `tracking_url`, `carrier`, `expected_arrival_until`, `part_of_order`, `has_delivery_method`, `acom:delivery_state` | 상태는 enums §1.3 |
| 배송 이벤트 | `Action` | `acom:from_state`, `acom:to_state`, `start_time` | 이벤트로 쌓음 |
| 반품 정책 | `MerchantReturnPolicy` | `merchant_return_days`, `return_policy_category`(`finite_return_window`), `return_fees`, `return_method`, `refund_type`(`full_refund`) | 반품 배송비 값은 enums §3, 기간은 `fulfillment.yml` `return_window_days` |
| 클레임(`clm_`: 취소·반품·교환) | `acom:Claim` ⚠️ (`ReturnAction`: 반품·교환, `CancelAction`: 취소) | `object`(주문 품목), `agent`(고객), `acom:claim_type`, `acom:claim_status`, `acom:claim_reason`, `acom:liability`, `acom:evidence`(사진), `result`(환불) | 상태·사유·귀책은 enums §1.5·§2.1–§2.3. 쇼퍼가 `POST /v1/fulfillment/returns`로 보내면 `requested` |
| 교환 | 클레임(`claim_type: exchange`) | 클레임 품목(`acom:ClaimItem`)의 `acom:replacement` | 회수 후 재배송. 재배송 진행은 배송 엔티티에 |
| 환불(`rfd_`) | `acom:Refund` ⚠️ (`MoneyTransfer`) | `amount`, `acom:refund_method`, `acom:refund_status` | enums §1.6 |
| 검수 결과(`ins_`) | `CheckAction` | `acom:claim_id`, `acom:inspection_condition`, `acom:liability` | enums §2.5 |
| 배송비·구역(`zone_`) | `acom:ShippingZone` ⚠️ (`ShippingRateSettings`) | `shipping_destination`, `shipping_rate`, `free_shipping_threshold` | 배송비 종류는 enums §3. 무료 기준은 `order.yml` `free_shipping_min_subtotal` |

### 2.5 customer · messaging
| 엔티티 | schema.org 타입 | 핵심 속성 | 비고 |
|---|---|---|---|
| 회원(`cus_`) | `Person` | `name`, `email`, `telephone`, `address`, `member_of`, `acom:member_state` | 개인정보 암호화·마스킹. 상태는 enums §1.13 |
| 사업자 회원 | `Person`(`acom:member_type: business`) | `legal_name`, `tax_id`, `acom:business_verification` | 개인 회원과 같은 테이블(`customer.person`) |
| 멤버십 | `ProgramMembership` | `program`(MemberProgram), `membership_number`, `acom:tier_id`, `acom:tier_since` | |
| 주소(`adr_`) | `PostalAddress` | `street_address`, `address_locality`, `address_region`, `postal_code`, `address_country`, `acom:is_primary`, `acom:usage` | 용도는 enums §1.13 |
| 연락처 | `ContactPoint` | `telephone`, `email`, `contact_type` | |
| 기억한 선호(`mem_`) | `acom:Preference` ⚠️ (`PropertyValue`) | `acom:preference_key`, `value`, `description`(한 줄), `acom:source`(`conversation`) | 쇼퍼에게 보이고, 쇼퍼가 사람 전용 `POST /v1/customer/me/memories/{memory_id}/undo`로 되돌림 |
| 동의 기록(`cns_`) | `acom:ConsentRecord` ⚠️ (`Action`) | `acom:consent_type`, `acom:granted`, `start_time` | 바꾸지 않는 이력. 종류는 enums §1.13 |
| 상담 문의(`inq_`) | `acom:CustomerInquiry` ⚠️ (`Message`) | `acom:inquiry_type`, `acom:inquiry_status`, `about`(Order) | enums §1.13 |
| 보상(`rmd_`) | `acom:Remedy` ⚠️ (`Action`) | `acom:remedy_type`, `acom:remedy_status`, `acom:liability` | enums §1.13·§2.1 |
| FAQ · 공지 | `FAQPage`, `Question`, `Answer`, `NewsArticle` | `headline`, `article_body`, `date_published` | |
| 고객 메시지(`msg_`) | `Message` | `recipient`, `headline`, `text`, `about`, `acom:message_kind`(`ask_for_photos` \| `decline` \| `refund_done` \| `custom` \| `campaign`), `acom:message_channel`, `date_sent`, `date_read` | 정책상 보내기 전 항상 승인. 회원이 받은 알림(`list_notifications`)도 이 행. 채널은 enums §3 |

### 2.6 merchandising · search · community
| 엔티티 | schema.org 타입 | 핵심 속성 | 비고 |
|---|---|---|---|
| 카테고리(`cat_`) | `DefinedTerm` in `DefinedTermSet` | `name`, `term_code`, `in_defined_term_set`(`acom:term_set`), `acom:broader`(상위) | 표준·전시 두 묶음(enums §1.12) |
| 진열 슬롯(`slot_`) | `WebPageElement` | `item_list_element`, `acom:page`(`main` 등), `acom:slot_type`, `position` | 종류는 enums §1.12. 데모: `slot_home_hero` |
| 진열 예약(`dsp_`) | `acom:DisplayBooking` ⚠️ (`Schedule`) | `acom:slot_id`, `valid_for_member_tier`(등급 id, everyone = NULL), `valid_from`, `valid_through`, `acom:banner_id`, `acom:display_status` | 배너는 `acom:Banner` ⚠️ (`CreativeWork`, `bnr_`: `headline`, `alternative_headline`, `acom:eyebrow`, `acom:cta_label`, `url`, `image`) |
| 기획전(`col_`) | `CollectionPage` | `name`, `has_part`, `valid_for_member_tier`, `valid_from`, `valid_through`, `acom:display_status` | |
| 검색 | `SearchAction` | `query`, `acom:filters`, `result` | |
| 검색 결과 | `ItemList` | `item_list_element`, `number_of_items` | |
| 게시글 · 댓글(`post_`/`rpl_`) | `DiscussionForumPosting`, `Comment` | `headline`, `article_body`, `text`, `author`, `parent_item` | |

### 2.7 platform
| 엔티티 | schema.org 타입 | 핵심 속성 | 비고 |
|---|---|---|---|
| 변경셋(`cs_`) | `acom:ChangeSet` ⚠️ (`ItemList`) | 항목(`ListItem` → 프리뷰 액션), `acom:number`, `version`, `acom:change_set_state`, `acom:undo_until`, `acom:interpretation` | 상태는 enums §1.8. API는 [platform.md](platform.md#change_set) |
| 할 일(`task_`) | `acom:Task` ⚠️ (`Action`) | `acom:title`, `acom:task_state`, `acom:task_origin`(`you_asked` \| `agent_noticed` \| `automation`), `acom:change_set_id`, `acom:watcher` | 상태는 enums §1.14. 감시 기준은 `platform.yml` `watchers` |
| 액션 로그(`act_`) | 각 schema.org `Action` | `agent`, `object`, `instrument`, `action_status`, `result`, `error`, `start_time`, `end_time` | §3 |
| 정책 | 정책 데이터(테이블 없음) | `key`, `value`, `display` | YAML 파일([policies/](../../policies/README.md)) 위에 스토어 덮어쓰기(`platform.policy_override`). `get_policy`가 병합해 읽음 |

---

## 3. 오퍼레이션은 schema.org 액션

명사는 schema.org 사물이고, **동사는 schema.org 액션**입니다. 모든 오퍼레이션은 자기가 수행하는 액션 타입으로 분류되고, 호출·프리뷰·승인·실행은 모두 액션 인스턴스로 기록됩니다.

| 오퍼레이션 | schema.org 액션 |
|---|---|
| `search_products`, `list_*` | `SearchAction` |
| `evaluate_return_request`, `verify_change_set` | `AssessAction` |
| `compare_products` | `SearchAction` |
| `set_sale_prices` | `Offer` 가격에 대한 `UpdateAction` |
| `create_coupon` | `CreateAction` |
| `schedule_banner` | `ScheduleAction` |
| `add_to_cart` | `AddAction` |
| `refund_return` | `PayAction`(결과는 `MoneyTransfer` + `acom:Refund`) |
| `send_customer_message` | `Message`를 보내는 `SendAction` |
| `prepare_order` | `potential` 상태의 `OrderAction`. 쇼퍼가 사람 전용 `POST /v1/sales/orders`를 눌러야 완료 |
| 반품 요청 보내기(사람 전용) | `ReturnAction` |
| 변경셋 승인(사람 전용) | `Person`의 `AuthorizeAction` |

`action_status`(schema.org `ActionStatusType`, enums §1.7)가 수명 주기 전체를 하나의 어휘로 표현합니다.

| 단계 | `action_status` |
|---|---|
| 프리뷰, 승인 대기 | `potential` |
| 실행 중 | `active` |
| 실행되고 읽어 확인됨 | `completed` + `result` |
| 읽어 보니 다름 | `failed` + `error` |

```jsonc
// 변경셋 #1042 안의 변경 하나 (액션 로그에 저장된 모습, 승인 전)
{
  "@type": "UpdateAction",
  "id": "act_1042_03",
  "action_status": "potential",
  "agent": {"@type": "SoftwareApplication", "name": "SJ", "acom:on_behalf_of": "adm_sj"},   // 에이전트가 대신한 사람. 모델 이름은 턴 기록에
  "instrument": "set_sale_prices",
  "object": {"@type": "Offer", "id": "off_windbreaker_m_slate"},
  "result": {
    "@type": "UnitPriceSpecification",
    "price": 71.20, "price_currency": "USD", "price_type": "sale_price",
    "valid_for_member_tier": "vip",
    "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00"
  },
  "acom:change_set_id": "cs_1042", "acom:preview_id": "pv_8f2c"
}
```

---

## 4. 확장 (`acom:`)

확장 규칙은 [vocabulary.md](vocabulary.md)에 있습니다.
1. schema.org 타입이 있으면 그대로 씁니다.
2. 그 타입의 한 종류이면 `@type`은 schema.org 타입으로 두고 우리 용어는 `additionalType`에 답니다.
3. 새 프로퍼티와 새 타입은 마지막 수단입니다.

정식 정의는 [`vocab/acom.jsonld`](../../vocab/acom.jsonld)에 있습니다.

| 개념 | `@type` (schema.org) | `additionalType` | 메모 |
|---|---|---|---|
| 프로모션 | `OfferCatalog` | `acom:Promotion` | 기간·대상 등급이 있는 판매가 묶음 |
| 회원가·기간가 | `UnitPriceSpecification` | — | `valid_for_member_tier`, `price_type: sale_price`, `acom:percent_off` |
| 쿠폰 | `Offer` | `acom:Coupon` | 코드는 `discount_code`(schema:discountCode), 중복·적용 규칙은 정책 |
| 발급된 쿠폰 | `OwnershipInfo` | `acom:MemberCoupon` | `owned_from`, `owned_through` |
| 장바구니 / 줄 | `Order` / `OrderItem` | `acom:Cart` / `acom:CartLine` | 결제 전의 주문 |
| 결제 | `PayAction` | — | |
| 반품·교환 / 취소 | `ReturnAction` / `CancelAction` | `acom:Claim` | `claim_type`이 `@type`을 정함 |
| 검수 | `CheckAction` | — | |
| 환불 | `MoneyTransfer` | `acom:Refund` | 오퍼레이션의 액션은 `PayAction` |
| 비교 | `SearchAction` | — | `compare_products`의 액션 |
| 진열 슬롯 | `WebPageElement` | — | |
| 배너 | `CreativeWork` | `acom:Banner` | `headline`, `alternative_headline`, `image`, `url` |
| 진열 예약 | `Schedule` | `acom:DisplayBooking` | 슬롯 × 대상 × 기간 |
| 세그먼트 | `Audience` | `acom:Segment` | 조건으로 정의한 고객 묶음 |
| 원가 / 안전 재고 | — | — | `Offer`의 프로퍼티 `acom:inventory_cost`(MonetaryAmount), `acom:safety_stock` |
| 기억(쇼퍼 선호) | `PropertyValue` | `acom:Preference` | 키는 `acom:preference_key` |
| 변경셋 | `ItemList` | `acom:ChangeSet` | 항목은 `ListItem` → `Action` |
| 프리뷰 | `Action` (PotentialActionStatus) | — | 오퍼레이션이 Action 하위 타입을 정함 |
| 할 일 | `Action` | `acom:Task` | |
| 이벤트 유형·보상 유형 레지스트리 | — | — | 정책 데이터([promotion.yml](../../policies/promotion.yml) `event.entry_mechanics`·`selection_methods`·`reward_types`), 테이블 없음 |
| 응모 | `JoinAction` | `acom:EventEntry` | |
| 등수별 보상 | `Offer` | `acom:EventReward` | |
| 추첨 기록 | `ChooseAction` | `acom:DrawRecord` | 재현 가능한 감사 기록 |
| 등급 요건 / 혜택 | `StructuredValue` / `TierBenefitEnumeration` 멤버 | `acom:TierRequirement` / — | 혜택 종류는 schema.org `TierBenefitLoyaltyPrice` 등, 세부는 우리 프로퍼티 |
| 경품 컴플라이언스 규칙 | `PropertyValue` | `acom:ComplianceRule` | |
| 상태·사유 열거 | `Enumeration` 하위 | — | schema.org 열거(`OrderStatus`, `ItemAvailability`, `ActionStatusType`, `PriceTypeEnumeration`, `ReturnFeesEnumeration`, `PaymentStatusType`, `RefundTypeEnumeration`)는 그대로, 나머지는 `acom:ClaimStatus` 등 |

API 응답에 붙는 JSON-LD 컨텍스트는 없습니다. JSON-LD를 내는 곳은 스토어프론트 상품 페이지뿐입니다([vocabulary.md](vocabulary.md) §5).

---

## 5. 데이터베이스

물리 설계(스키마, 테이블·컬럼 이름, 공통 컬럼, 주석, RLS, 이벤트 테이블, 검색)는 [../backend/database.md](../backend/database.md)에 있습니다. 요점:

- 테이블은 schema.org 타입, 컬럼은 schema.org 프로퍼티의 snake_case. 모든 테이블·컬럼에 schema.org 용어가 적힌 주석.
- 기본 키는 API와 같은 접두 id(`text`) 하나. 따로 숨은 키가 없습니다.
- API와 이름이 다른 곳: 상품 `product_id`(`prd_`)는 DB `catalog.product_group`, 변형 `variant_id`(`var_`)는 DB `catalog.product`(schema.org `Product`)입니다. 전체 대응표는 [../backend/database.md](../backend/database.md) §11.
- 실제 DDL은 `migrations/*.sql`, 컬럼 사전은 [../backend/schema.generated.md](../backend/schema.generated.md).

---

## 6. 연결

- 도메인 API의 입력·출력은 이 모델의 타입과 속성을 씁니다([README.md](README.md)). 형식 규약은 [conventions.md](conventions.md), 값 목록은 [enums.md](enums.md), 에러는 [errors.md](errors.md).
- 정책(할인 상한, 가격 하한, 반품 기간, 승인 임계)은 데이터이며 [policies/](../../policies/README.md)에 있습니다. 상태와 규칙은 이 모델의 이벤트와 제약으로 강제됩니다.
- 스토어 범위·스키마 분리·행 수준 보안은 [../backend/database.md](../backend/database.md)를 따릅니다.
