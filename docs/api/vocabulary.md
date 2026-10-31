# 어휘: schema.org와 우리 확장

A-Commerce의 모든 타입·프로퍼티·값은 schema.org 어휘 위에 있습니다. 이 문서는 **schema.org에 없는 것을 어떻게 표현하는가**의 규칙입니다. 기계가 읽는 정의는 [`vocab/acom.jsonld`](../../vocab/acom.jsonld)이고, `cargo xtask check-schema --write`가 손으로 쓴 [`vocab/terms.json`](../../vocab/terms.json), 마이그레이션 주석, `acom-core` enum에서 생성합니다. DB에 나오는 우리 용어는 모두 이 파일에 정의가 있습니다.

```
schema:  https://schema.org/
acom:    https://metahackai.github.io/a-commerce/vocab#      (우리 확장. 저장소의 vocab/acom.jsonld가 정의)
```

---

## 1. 확장을 고르는 순서

새 개념을 만나면 위에서부터 차례로 시도하고, 처음 맞는 것에서 멈춥니다.

1. **schema.org 타입을 그대로 씁니다.** 이미 있는 타입이 그 개념을 뜻하면 확장하지 않습니다. 검수는 `CheckAction`, 진열 슬롯은 `WebPageElement`, 결제는 `PayAction`, 배송은 `ParcelDelivery`, 대화는 `Conversation`입니다.
2. **schema.org 타입 + `additionalType`.** 개념이 기존 타입의 한 종류이면 `@type`은 schema.org 타입으로 두고, 우리 용어를 `additionalType`에 답니다. schema.org만 아는 에이전트·검색엔진·외부 도구도 그것이 `Offer`라는 것은 압니다.
   ```json
   {"@type": "Offer", "additionalType": "acom:Coupon", "discountCode": "VIPWEEKEND20", "validForMemberTier": "vip"}
   ```
3. **schema.org 프로퍼티를 다른 타입에도 씁니다.** schema.org는 `domainIncludes`를 기대 범위로 둘 뿐 금지하지 않습니다. 쿠폰 코드는 새 프로퍼티 대신 `discountCode`를 씁니다.
4. **우리 프로퍼티(`acom:`)를 만듭니다.** 기존 프로퍼티로 뜻이 서지 않을 때만 만듭니다. 도메인·범위는 생성기가 DB에서 채우고, 뜻이 가까운 schema.org 프로퍼티가 있으면 `vocab/terms.json`에 `rdfs:subPropertyOf`를 적습니다(`acom:startedAt` ⊂ `schema:startTime`, `acom:title` ⊂ `schema:name`). schema.org만 아는 에이전트도 뜻을 추론할 수 있습니다.
5. **우리 타입(`acom:`)을 만듭니다.** 위 어느 것으로도 안 될 때의 마지막 수단입니다. 반드시 `rdfs:subClassOf`로 가장 가까운 schema.org 타입 아래에 둡니다.

**값 목록**(상태, 사유)은 schema.org 열거가 있으면 그 멤버를 쓰고(`OrderStatus`, `ItemAvailability`, `ActionStatusType`, `PaymentStatusType`, `ReturnFeesEnumeration`), 없으면 `acom:` 열거 타입(`schema:Enumeration`의 하위 타입)과 멤버로 정의합니다. 전선과 DB에서는 멤버의 snake_case 단어를 씁니다([enums.md](enums.md)).

## 2. 이 방식이 `acom:Coupon` 같은 새 타입보다 나은 이유

| | 새 타입만 (`@type: acom:Coupon`) | schema.org 타입 + `additionalType` |
|---|---|---|
| schema.org만 아는 에이전트·검색엔진 | 무엇인지 모름 | `Offer`로 이해하고 가격·기간·대상 등급을 읽음 |
| 모델의 사전 지식 | 새 단어를 추측해야 함 | 이미 아는 타입과 프로퍼티 |
| JSON-LD 검증 도구 | 모르는 타입 | 표준 타입으로 검증, 추가 정보는 무시되지 않고 보존 |
| 우리 확장의 정의 | 이름만 있음 | `vocab/acom.jsonld`에 상위 타입·설명·범위가 있음 |
| 나중에 schema.org에 같은 개념이 생기면 | 데이터 이전 필요 | `additionalType`만 바꾸거나 그대로 둠 |

## 3. 대응표

| 개념 | 예전 표기 | 지금 | 메모 |
|---|---|---|---|
| 프로모션(기간·대상이 있는 판매가 묶음) | `acom:Promotion` ⊂ Offer | `OfferCatalog` + `acom:Promotion` | 판매가(`UnitPriceSpecification`)를 담은 제안 목록 |
| 회원가·기간가 | — | `UnitPriceSpecification` | `validForMemberTier`, `priceType: SalePrice`, `acom:percentOff` |
| 쿠폰 | `acom:Coupon` ⊂ Offer | `Offer` + `acom:Coupon` | 코드는 `discountCode` |
| 회원에게 발급된 쿠폰 | `acom:MemberCoupon` | `OwnershipInfo` + `acom:MemberCoupon` | `ownedFrom`, `ownedThrough`, `acom:couponId` |
| 장바구니 | `acom:Cart` ⊂ Order | `Order` + `acom:Cart` | 주문 전의 주문 |
| 장바구니 줄 | — | `OrderItem` + `acom:CartLine` | |
| 결제 | `acom:Payment` | `PayAction` | `paymentMethod`, `paymentStatus`, `amount` |
| 반품·교환 요청 | `acom:Claim` | `ReturnAction` + `acom:Claim` | 교환도 반품의 한 형태 |
| 취소 요청 | `acom:Claim` | `CancelAction` + `acom:Claim` | `claim_type`가 정함 |
| 검수 | `acom:Inspection` | `CheckAction` | schema.org: "inspects … condition" |
| 환불 | `acom:Refund` ⊂ MoneyTransfer | `MoneyTransfer` + `acom:Refund` | `amount`, `acom:refundMethod`, `refundType` |
| 진열 슬롯 | `acom:DisplaySlot` | `WebPageElement` | `acom:page`, `acom:slotType`, `position` |
| 배너 | `acom:Banner` ⊂ CreativeWork | `CreativeWork` + `acom:Banner` | `headline`, `alternativeHeadline`, `image`, `url` |
| 진열 예약 | `acom:DisplayBooking` | `Schedule` + `acom:DisplayBooking` | 슬롯 × 대상 × 기간 |
| 카테고리 | — | `DefinedTerm` (`inDefinedTermSet`) | |
| 기억(쇼퍼 선호) | `acom:Preference` ⊂ PropertyValue | `PropertyValue` + `acom:Preference` | `propertyID`, `value` |
| 변경셋 | `acom:ChangeSet` ⊂ ItemList | `ItemList` + `acom:ChangeSet` | 항목은 `ListItem` → Action |
| 프리뷰 | `acom:Preview` | `Action` (`PotentialActionStatus`) | 오퍼레이션이 Action 하위 타입을 정함(`UpdateAction` 등) |
| 할 일 | `acom:Task` | `Action` + `acom:Task` | `acom:taskState`, `acom:taskOrigin`, `object` |
| 액션 로그·이벤트 | — | `Action` | |
| 스토어(테넌트) | — | `OnlineStore` | |
| 운영자 | — | `Person` | 역할은 `Role` |
| 대화 / 턴 / 단계 | — | `Conversation` / `Message` / `Action` | |

## 4. 우리 용어 목록

정식 정의(상위 타입, 범위, 설명)는 `vocab/acom.jsonld`에 있습니다.

| 종류 | 용어 |
|---|---|
| 타입(모두 `additionalType`으로만 씀) | `acom:Promotion`, `acom:Coupon`, `acom:MemberCoupon`, `acom:Cart`, `acom:CartLine`, `acom:Claim`, `acom:Refund`, `acom:Banner`, `acom:DisplayBooking`, `acom:Preference`, `acom:ChangeSet`, `acom:Task` 등 |
| 프로퍼티 | `acom:percentOff`, `acom:inventoryCost`, `acom:fulfillmentStage`, `acom:deliveryState`, `acom:claimType`, `acom:claimStatus`, `acom:claimReason`, `acom:liability`, `acom:inspectionCondition`, `acom:refundStatus`, `acom:refundMethod`, `acom:couponApplication`, `acom:discountLayer`, `acom:promotionStatus`, `acom:displayStatus`, `acom:slotType`, `acom:memberState`, `acom:preferenceKey`, `acom:changeSetState`, `acom:taskState`, `acom:taskOrigin`, `acom:rank`, `acom:confirmedAt` 등 |
| 열거 타입 | `acom:ClaimStatus`, `acom:ClaimReason`, `acom:Liability`, `acom:FulfillmentStage`, `acom:DeliveryState` 등 ([enums.md](enums.md)의 `acom:` 목록 전부) |

## 5. 쓰는 곳

- **API(JSON):** 필드는 snake_case이고, 응답에 JSON-LD 컨텍스트(`valid_from` → `schema:validFrom`)는 붙지 않습니다. 이름이 어느 용어인지는 데이터 사전 툴 `describe_data`가 답합니다(API 이름으로도 찾음, 예 `api_name: variant_id`).
- **DB:** 테이블 주석의 첫 토큰이 `schema:<Type>` 또는 `schema:<Type>/acom:<Term>`입니다. 앞은 `@type`, 뒤는 `additionalType`입니다([../backend/database.md](../backend/database.md) §6). 맞는 schema.org 타입이 없는 연결 테이블 둘은 `acom:<Term>`만 씁니다(`acom:CategoryPlacement`, `acom:SellerProduct`), 기술 테이블은 `-`.
- **화면:** 스토어프론트 상품 페이지(`/p/{product_id}`)가 schema.org JSON-LD(`ProductGroup`·`Product`, `Offer`, 회원가 `UnitPriceSpecification`, `AggregateRating`)를 냅니다. 우리 용어는 넣지 않습니다.
- **네임스페이스 주소:** 위 IRI는 저장소의 GitHub Pages로 공개할 주소입니다. 공개 전에는 저장소의 `vocab/acom.jsonld`가 정의입니다.
