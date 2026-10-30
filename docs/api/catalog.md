# API 명세 — `catalog` (상품·변형·가격·재고·브랜드·리뷰·문의)

> 계층: `catalog.{resource}.{operation}`. 공통 규약(응답 형식·에러·프리뷰/실행·멱등·헤더)은 [README.md](README.md) §3, 데이터 타입은 [data-model.md](data-model.md), 정책은 [catalog.yml](policies/catalog.yml)·[approval.yml](policies/approval.yml). MVP 툴(`get_product`, `get_product_measurements`, `get_review_summary`)의 시그니처는 [../plan.md](../plan.md) §3과 같고, 계층 위치는 [../architecture.md](../architecture.md) §3.4를 따릅니다.
>
> **설계 축**: ① 상품 = `ProductGroup`(모델) → 변형 = `Product`(구매 단위, SKU). ② 재고·가격은 변형 단위, 상품 상태는 파생(읽기 전용). ③ 원가와 마진은 운영자에게만 보이고 쇼퍼에게는 보이지 않음. ④ 설명·이미지 같은 콘텐츠 변경은 승인 없이, 가격·구조·삭제는 승인 대상. ⑤ `price_floor`(원가 + 15%) 아래 판매는 서버가 막음.
>
> **응답 형식**: 읽기 `{items, total}`, 프리뷰 `{preview_id, changes[{op, entity, id, from, to}], checks[{rule, passed, detail, fix_tool}], needs_approval, approval_reason}`, 실행 `{ok, id, undo{tool, until}}`, 에러 `{error: {code, message, next_tool}}`. 목록은 `limit`·`offset`. 금액은 USD 소수 둘째 자리.

리소스: [product_group](#product_group) · [product](#product) · [offer](#offer) · [inventory](#inventory) · [approval](#approval) · [brand](#brand) · [review](#review) · [question](#question) · [bundle](#bundle) · [restock_alert](#restock_alert)

> 상품 검색 `search_products`는 [search.md](search.md#product)에 한 번만 정의합니다(`search.product.search`).

---

## product_group

### `search_product_groups` — Kind: R
- 논리주소/REST: `catalog.product_group.search` · `GET /v1/catalog/product-groups`
- 요약: 조건으로 상품(모델)을 찾습니다. 후속 툴 입력 `product_group_id[]`를 만듭니다.
- consumes → produces: → `product_group_id[]`
- 입력: `query?`(Text), `brand?`, `category?`, `approval_status?`(`draft` | `pending_review` | `active` | `rejected`), `availability?`(`in_stock` | `out_of_stock` | `pre_order` | `discontinued`), `sort?`(`relevance` | `newest` | `price_asc`), `fields?[]`, `limit=20`, `offset=0`
- 출력: `{items: ProductGroup[], total}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_filter` → `search_product_groups`
- 멱등성: 읽기(부작용 없음)

### `get_product_group` — Kind: R
- 논리주소/REST: `catalog.product_group.get` · `GET /v1/catalog/product-groups/{product_group_id}`
- 요약: 모델 하나를 변형(`has_variant`), 옵션 축(`varies_by`), 브랜드, 카테고리와 함께 돌려줍니다.
- consumes → produces: `product_group_id` → product_group
- 입력: `product_group_id`, `fields?[]`
- 출력: `{items: [ProductGroup], total: 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `product_group_not_found` → `search_product_groups`
- 멱등성: 읽기(부작용 없음)

### `create_product_group` — Kind: W
- 논리주소/REST: `catalog.product_group.create` · `POST /v1/catalog/product-groups`
- 요약: 상품(모델)을 `draft`로 등록합니다. 변형은 `set_variants`로 따로 만듭니다.
- consumes → produces: `brand_id?`, `category` → `product_group_id`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명·예 |
  |------|----------------------|:--:|--------|
  | name | Text | ✓ | "Packable windbreaker" |
  | description | Text | | 상세 설명 |
  | brand | Brand(ref) | | `brand_id` |
  | category | DefinedTerm | ✓ | 표준 카테고리, 예 `"summer_outerwear"` |
  | image | ImageObject[] | | 대표·상세 이미지 |
  | varies_by | PropertyValue[] | | 옵션 축(`color`, `size`) |
  | acom:min_age | Integer | | 구매 가능 최소 연령(세). 없거나 `0`이면 제한 없음. 허용 값·확인 방식은 스토어 설정([catalog.yml](policies/catalog.yml) `product.age_restriction`) |
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "pgr_windbreaker", undo}`
- 프리뷰: `changes`=`create` `product_group`. checks `required_on_create`(판매자·카테고리·이름). 콘텐츠 등록이라 `needs_approval: false`.
- 정책: [catalog.yml](policies/catalog.yml) `product.required_on_create`, `product.age_restriction`, `variant.max_option_axes`
- 에러: `missing_required_field` → `create_product_group` · `invalid_category` → `list_categories`
- 멱등성: `Idempotency-Key`

### `update_product_group` — Kind: W
- 논리주소/REST: `catalog.product_group.update` · `PATCH /v1/catalog/product-groups/{product_group_id}`
- 요약: 모델 속성을 고칩니다. 설명·이미지 변경은 승인 없이, 카테고리 같은 구조 변경은 승인 대상입니다.
- consumes → produces: `product_group_id` → `product_group_id`
- 입력: `product_group_id`, 바꿀 필드(`name`, `description`, `category`, `image`, `varies_by`), `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` 필드별 `from`/`to`. 구조 변경이 있으면 `needs_approval: true`, `approval_reason: "Changes the category"`.
- 정책: [catalog.yml](policies/catalog.yml) `product.approval`
- 에러: `invalid_state` → `get_product_group`
- 멱등성: `Idempotency-Key`

---

## product

### `get_product` — Kind: R  (MVP)
- 논리주소/REST: `catalog.product.get` · `GET /v1/catalog/products/{product_id}`
- 요약: 상품 하나를 변형과 사이즈별 재고와 함께 돌려줍니다. 운영자에게는 원가도 보입니다.
- consumes → produces: `product_id` → product
- 입력: `product_id`(예 `"prd_windbreaker"`), `fields?[]`
- 출력:
```jsonc
{"items": [{
  "id": "prd_windbreaker", "@type": "ProductGroup", "name": "Packable windbreaker",
  "category": "summer_outerwear", "price": 89.00, "price_currency": "USD", "cost": 35.60,
  "has_variant": [
    {"sku": "wb-slate-m", "color": "slate", "size": "M", "inventory_level": 61, "availability": "in_stock"},
    {"sku": "wb-slate-l", "color": "slate", "size": "L", "inventory_level": 48, "availability": "in_stock"}]
 }], "total": 1}
```
- 프리뷰: 해당 없음(읽기)
- 정책: 원가(`cost`)는 운영자에게만([catalog.yml](policies/catalog.yml) `pricing.cost_visibility`)
- 에러: `product_not_found` → `search_products`
- 멱등성: 읽기(부작용 없음)

### `get_product_measurements` — Kind: R  (MVP, P2)
- 논리주소/REST: `catalog.product.get_measurements` · `GET /v1/catalog/products/{product_id}/measurements`
- 요약: 사이즈별 실측(가슴·어깨·소매·길이, cm)을 돌려줍니다. 쇼퍼가 "M과 L 중 무엇이 맞을까" 물을 때 씁니다.
- consumes → produces: `product_id` → measurements
- 입력: `product_id`(예 `"prd_wool_overcoat"`), `sizes[]`(예 `["M", "L"]`)
- 출력:
```jsonc
{"items": [
  {"size": "M", "chest": 56, "shoulder": 46, "sleeve": 64, "length": 98, "unit_text": "cm"},
  {"size": "L", "chest": 59, "shoulder": 48, "sleeve": 65.5, "length": 100, "unit_text": "cm"}],
 "total": 2}
```
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `product_not_found` → `search_products` · `size_not_found` → `get_product`
- 멱등성: 읽기(부작용 없음)

### `create_product` — Kind: W
- 논리주소/REST: `catalog.product.create` · `POST /v1/catalog/products`
- 요약: 변형 하나를 `draft`로 만듭니다. 모델 없이 단품으로 등록하거나 기존 모델에 변형을 더합니다. 여러 변형은 `set_variants`가 낫습니다.
- consumes → produces: `product_group_id?`, `brand_id?` → `product_id`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명·예 |
  |------|----------------------|:--:|--------|
  | is_variant_of | ProductGroup(ref) | | 소속 모델. 없으면 단품 모델을 자동 생성 |
  | sku | Text | ✓ | 스토어 안에서 유일 |
  | name | Text | ✓ | 변형 이름 |
  | category | DefinedTerm | ✓ | 표준 카테고리 |
  | brand | Brand(ref) | | `brand_id` |
  | image | ImageObject[] | | |
  | additional_property | PropertyValue[] | | 옵션 조합, 예 `{color: "slate", size: "M"}` |
  | offers | Offer | | 초기 가격 `{price: 89.00, price_currency: "USD", availability: "in_stock"}`. 생략하면 `set_price`로 |
  | acom:cost | acom:CostSpecification | | 원가(운영자 전용) |
  | acom:managed | Boolean | | 재고 관리 여부(기본 `true`) |
  | acom:sale_type | enum | | `always` \| `pre_order` \| `subscription` \| `gift_allowed` \| `digital_voucher` |
  | acom:min_age | Integer | | 구매 가능 최소 연령(세). 없거나 `0`이면 제한 없음. 허용 값·확인 방식은 스토어 설정([catalog.yml](policies/catalog.yml) `product.age_restriction`) |
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "prd_windbreaker", undo}`
- 프리뷰: `changes`=`create` `product`(+ `offer`, `inventory`). checks `required_on_create`, `min_age_allowed`(`acom:min_age`가 스토어 허용 값인지), `price_floor`(가격이 있으면 원가 + 15% 이상, 실패 시 `fix_tool: "set_price"`), `digital_voucher_disallow`(`digital_voucher`일 때 금지 조합). 가격이 없으면 `needs_approval: false`, 있으면 구조 변경으로 `true`.
- 정책: [catalog.yml](policies/catalog.yml) `product.required_on_create`, `product.age_restriction`, `pricing.margin_protection`, `variant.option_source`, `digital_voucher.disallow`
- 에러: `duplicate_sku` → `get_product` · `below_price_floor` → `set_price` · `min_age_not_allowed` → `get_policy` · `missing_required_field` → `create_product` · `invalid_sale_type` → `update_product`
- 멱등성: `Idempotency-Key = hash(tenant + sku)`. 같은 SKU로 다시 보내면 첫 결과를 돌려줌

### `update_product` — Kind: W
- 논리주소/REST: `catalog.product.update` · `PATCH /v1/catalog/products/{product_id}`
- 요약: 변형 속성을 고칩니다. 콘텐츠는 승인 없이, 가격·구조는 승인 대상입니다.
- consumes → produces: `product_id` → `product_id`
- 입력: `product_id`, 바꿀 필드, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: 필드별 `from`/`to`. 가격·구조 변경이 있으면 `needs_approval: true`.
- 정책: [catalog.yml](policies/catalog.yml) `product.approval`, `pricing.margin_protection`
- 에러: `invalid_state` → `get_product` · `below_price_floor` → `set_price`
- 멱등성: `Idempotency-Key`

### `set_variants` — Kind: W
- 논리주소/REST: `catalog.product.set_variants` · `PUT /v1/catalog/product-groups/{product_group_id}/variants`
- 요약: 옵션 축을 정하고 조합별 변형을 한 번에 만들거나 맞춥니다(추가·수정·비활성). 새 옵션 값은 백엔드가 중복 없이 등록합니다.
- consumes → produces: `product_group_id` → `product_id[]`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명·예 |
  |------|----------------------|:--:|--------|
  | product_group_id | Text | ✓ | |
  | varies_by | PropertyValue[] | ✓ | 축과 값, 예 `color: [slate, sand]`, `size: [S, M, L]`. 축 수 상한은 스토어 설정 `variant.max_option_axes`(기본 제한 없음), 조합 수는 `variant.max_variants_per_group` |
  | has_variant | Product[] | ✓ | 조합별 `{sku, additional_property, offers?, acom:cost?, acom:managed?}` |
  | missing_combination | enum | | `create_all` \| `listed_only` |
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "pgr_windbreaker", undo}`
- 프리뷰: `changes`=조합별 `create`/`update`/`deactivate`. 입력에서 빠진 기존 변형은 삭제가 아니라 비활성. checks `max_option_axes`(설정했을 때), `max_variants`, `sku_unique`, `price_floor`(가격이 있는 변형). 판매 중인 상품이면 구조 변경으로 `needs_approval: true`.
- 정책: [catalog.yml](policies/catalog.yml) `variant.max_option_axes`, `variant.max_variants_per_group`, `variant.option_source`, `product.approval.structural_changes`, `pricing.margin_protection`
- 에러: `too_many_option_axes` → `set_variants` · `too_many_variants` → `set_variants` · `duplicate_sku` → `get_product` · `incomplete_combination` → `set_variants` · `below_price_floor` → `set_price`
- 멱등성: `Idempotency-Key = hash(product_group_id + variants_digest)`. 같은 조합을 다시 적용하면 변경 없음

### `discontinue_product` — Kind: W
- 논리주소/REST: `catalog.product.discontinue` · `POST /v1/catalog/products/{product_id}/discontinue`
- 요약: 변형을 단종합니다(`availability` → `discontinued`, 되돌릴 수 없음).
- consumes → produces: `product_id` → `product_id`
- 입력: `product_id`, `reason?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo: null}`
- 프리뷰: `changes`=`update` `availability` `in_stock` → `discontinued`. checks `open_subscriptions`(진행 중 구독이 있으면 `passed: false`, `fix_tool: "list_subscriptions"`). 되돌릴 수 없으므로 `needs_approval: true`.
- 정책: [catalog.yml](policies/catalog.yml) `inventory.availability_values`
- 에러: `invalid_state` → `get_product` · `has_open_subscriptions` → `list_subscriptions`
- 멱등성: 이미 단종이면 첫 결과를 돌려줌

### `get_return_policy` — Kind: R
- 논리주소/REST: `catalog.product.get_return_policy` · `GET /v1/catalog/products/{product_id}/return-policy`
- 요약: 상품에 적용되는 반품 정책(`MerchantReturnPolicy`)을 상세 화면용으로 돌려줍니다.
- consumes → produces: `product_id` → return_policy
- 입력: `product_id`
- 출력: `{items: [{merchant_return_days: 30, return_fees: "free_return", acom:non_returnable_when: [], acom:size_exchange_days: 30}], total: 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: [fulfillment.yml](policies/fulfillment.yml)
- 에러: `product_not_found` → `search_products`
- 멱등성: 읽기(부작용 없음)

### `get_delivery_policy` — Kind: R
- 논리주소/REST: `catalog.product.get_delivery_policy` · `GET /v1/catalog/products/{product_id}/delivery-policy`
- 요약: 상품의 배송 조건(`OfferShippingDetails`)을 돌려줍니다.
- consumes → produces: `product_id` → delivery_policy
- 입력: `product_id`
- 출력: `{items: [{acom:shipping_fee_type: "conditional_free", acom:free_shipping_min_subtotal: 80.00, price_currency: "USD", acom:handling_days: 1}], total: 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: [fulfillment.yml](policies/fulfillment.yml)
- 에러: `product_not_found` → `search_products`
- 멱등성: 읽기(부작용 없음)

### `batch_import` — Kind: W$
- 논리주소/REST: `catalog.product.batch_import` · `POST /v1/catalog/products/batch-import`
- 요약: 스프레드시트(CSV·엑셀)로 상품을 대량 등록·수정합니다. 모든 행을 프리뷰로 검사한 뒤 변경셋 승인을 거쳐 실행합니다.
- consumes → produces: file → `import_job_id`
- 입력: `acom:file`(MediaObject, 업로드 파일 ref), `acom:mode`(`create` | `upsert`), `acom:publish_on_success?`(Boolean), `preview`
- 출력: 프리뷰 형식(`changes`는 행별 `create`/`update`) → 실행 `{ok: true, id: "imp_1107", undo}`. 진행 상황은 `get_import_status`
- 프리뷰: checks `required_on_create`, `price_floor`, `sku_unique`. 실패한 행은 `detail`에 행 번호와 이유. 항상 `needs_approval: true`, 예 `approval_reason: "Creates 128 products"`.
- 정책: [catalog.yml](policies/catalog.yml) `product.required_on_create`, `pricing.margin_protection`; [approval.yml](policies/approval.yml)
- 에러: `invalid_file_format` → `batch_import` · `row_validation_failed` → `batch_import` · `below_price_floor` → `set_price`
- 멱등성: `Idempotency-Key = hash(tenant + file_digest)`. 같은 파일은 다시 적용하지 않음

### `generate_image` — Kind: W$
- 논리주소/REST: `catalog.product.generate_image` · `POST /v1/catalog/products/{product_id}/generate-image`
- 요약: 상품 연출 이미지를 생성합니다. 생성물은 초안으로 두고, 상세 화면 노출은 승인 후에 합니다.
- consumes → produces: `product_id` → `image_id[]`
- 입력: `product_id`, `acom:prompt`, `acom:style?`(`studio` | `lifestyle` | `flat_lay`), `acom:count?`(≤ 4), `preview`
- 출력: 프리뷰 형식(`changes`=`create` `image`, 초안 URL 포함) → 실행 `{ok: true, id: "img_wb_01", undo}`
- 프리뷰: checks `brand_guideline`, `ai_content_disclosure`(`acom:ai_generated: true` 표기). 고객에게 닿으므로 항상 `needs_approval: true`.
- 정책: [catalog.yml](policies/catalog.yml) `product.approval`; [approval.yml](policies/approval.yml)
- 에러: `generation_failed` → `generate_image` · `brand_guideline_violation` → `generate_image`
- 멱등성: `Idempotency-Key = hash(product_id + prompt + style)`

### `generate_copy` — Kind: W
- 논리주소/REST: `catalog.product.generate_copy` · `POST /v1/catalog/products/{product_id}/generate-copy`
- 요약: 상품명·요약·상세 설명 초안을 생성합니다. 콘텐츠 변경이라 승인 없이 초안으로 저장하고, 반영은 `update_product`로 합니다.
- consumes → produces: `product_id` → copy draft
- 입력: `product_id`, `acom:copy_type`(`title` | `summary` | `description` | `seo`), `acom:tone?`, `acom:keywords?[]`, `preview`
- 출력: 프리뷰 형식(`changes`의 `to`가 생성 문구) → 실행 `{ok, id, undo}`
- 프리뷰: checks `ai_content_disclosure`. `needs_approval: false`.
- 정책: [catalog.yml](policies/catalog.yml) `product.approval.content_changes`
- 에러: `generation_failed` → `generate_copy`
- 멱등성: `Idempotency-Key`

---

## offer

### `get_price_breakdown` — Kind: R
- 논리주소/REST: `catalog.offer.get_breakdown` · `GET /v1/catalog/products/{product_id}/price-breakdown`
- 요약: 변형의 지금 적용되는 가격을 나눠 보여줍니다(정가, 판매가, 회원·기간 한정가). 원가와 마진은 운영자에게만.
- consumes → produces: `product_id` → price breakdown
- 입력: `product_id`, `valid_for_member_tier?`(`"everyone"` | `"vip"` …), `order_quantity?`
- 출력: `{items: [{list_price: 89.00, sale_price: 71.20, applied: [{promotion_id: "promo_1042", name: "VIP weekend"}], price_currency: "USD"}], total: 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: [catalog.yml](policies/catalog.yml) `pricing.cost_visibility`
- 에러: `product_not_found` → `search_products`
- 멱등성: 읽기(부작용 없음)

### `set_price` — Kind: W
- 논리주소/REST: `catalog.offer.set_price` · `PUT /v1/catalog/products/{product_id}/price`
- 요약: 변형의 정가·원가를 정합니다. 가격은 구조 변경이라 승인 대상이고, `price_floor` 아래 가격은 서버가 막습니다. 회원·기간 한정 판매가는 `set_sale_prices`(promotion)가 맡습니다.
- consumes → produces: `product_id` → `offer_id`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명·예 |
  |------|----------------------|:--:|--------|
  | product_id | Text | ✓ | |
  | price | Number + price_currency | ✓ | `89.00`, `"USD"`. 센트 단위 |
  | acom:list_price | UnitPriceSpecification | | 참조 정가(`price_type: "list_price"`) |
  | acom:cost | acom:CostSpecification | | 원가(운영자 전용) |
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "off_windbreaker_m_slate", undo}`
- 프리뷰: `changes`=`update` `price` `from: 89.00` → `to: 85.00`. checks `price_floor`(원가 + 15% 이상, 실패 시 `fix_tool: "set_price"`). `needs_approval: true`, 예 `approval_reason: "Changes 1 price"`.
- 정책: [catalog.yml](policies/catalog.yml) `pricing.margin_protection`, `product.approval.structural_changes`; [approval.yml](policies/approval.yml) `price_change_bulk`
- 에러: `below_price_floor` → `set_price`
- 멱등성: `Idempotency-Key = hash(product_id + price)`

### `schedule_price_change` — Kind: W
- 논리주소/REST: `catalog.offer.schedule_price_change` · `POST /v1/catalog/products/{product_id}/price/schedule`
- 요약: 정가 변경을 미래 시점에 예약합니다. 적용 여부는 시각으로 그때그때 계산합니다.
- consumes → produces: `product_id` → `offer_id`
- 입력: `product_id`, `price`, `valid_from`, `valid_through?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: checks `price_floor`, `valid_from_in_future`. `needs_approval: true`.
- 정책: [catalog.yml](policies/catalog.yml) `pricing.time_limited_offer`
- 에러: `below_price_floor` → `set_price` · `valid_from_in_past` → `schedule_price_change`
- 멱등성: `Idempotency-Key`

---

## inventory

### `get_inventory` — Kind: R
- 논리주소/REST: `catalog.inventory.get` · `GET /v1/catalog/products/{product_id}/inventory`
- 요약: 변형의 가용 재고, 안전재고, 파생된 `availability`를 돌려줍니다.
- consumes → produces: `product_id` → inventory
- 입력: `product_id`, `fields?[]`
- 출력: `{items: [{sku: "wb-slate-m", inventory_level: 61, acom:safety_stock: 10, acom:managed: true, availability: "in_stock"}], total: 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: [catalog.yml](policies/catalog.yml) `inventory`
- 에러: `product_not_found` → `search_products`
- 멱등성: 읽기(부작용 없음)

### `set_variant_stock` — Kind: W
- 논리주소/REST: `catalog.inventory.set_stock` · `PUT /v1/catalog/products/{product_id}/inventory`
- 요약: 변형의 가용 수량을 정합니다. 0이 되면 자동으로 `out_of_stock`, 자동 전환된 경우에만 회복 시 `in_stock`으로 돌아갑니다.
- consumes → produces: `product_id` → inventory
- 입력: `product_id`, `inventory_level`(Integer), `acom:managed?`, `reason?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` `inventory_level` `from`/`to`, 바뀌는 `availability`. `needs_approval: false`.
- 정책: [catalog.yml](policies/catalog.yml) `inventory.managed_at`, `auto_out_of_stock_when`, `auto_restore_when`
- 에러: `negative_stock` → `set_variant_stock` · `invalid_state` → `get_product`
- 멱등성: 절대값 설정이므로 `Idempotency-Key`로 같은 결과

### `set_safety_stock` — Kind: W
- 논리주소/REST: `catalog.inventory.set_safety_stock` · `PUT /v1/catalog/products/{product_id}/inventory/safety`
- 요약: 안전재고 임계치를 정합니다. 알림 전용이며 판매 상태에는 영향이 없습니다.
- consumes → produces: `product_id` → inventory
- 입력: `product_id`, `acom:safety_stock`(Integer), `enabled?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `needs_approval: false`
- 정책: [catalog.yml](policies/catalog.yml) `inventory.safety_stock`
- 에러: `negative_value` → `set_safety_stock`
- 멱등성: `Idempotency-Key`

---

## approval

### `submit_for_review` — Kind: W
- 논리주소/REST: `catalog.approval.submit` · `POST /v1/catalog/products/{product_id}/submit`
- 요약: 상품을 심사 대기로 보냅니다(`draft` | `rejected` → `pending_review`).
- consumes → produces: `product_id` → `product_id`
- 입력: `product_id`, `note?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` `approval_status` `draft` → `pending_review`. `needs_approval: false`.
- 정책: [catalog.yml](policies/catalog.yml) `product.approval`
- 에러: `invalid_state` → `get_product` · `nothing_to_review` → `update_product`
- 멱등성: `Idempotency-Key`

### `approve_product` — Kind: W$
- 논리주소/REST: `catalog.approval.approve` · `POST /v1/catalog/products/{product_id}/approve`
- 요약: 심사 중인 상품을 공개합니다(`pending_review` → `active`). 고객에게 닿으므로 항상 변경셋 승인을 거칩니다. 승인 후 실행기가 `preview_id`로 실행합니다.
- consumes → produces: `product_id` → `product_id`
- 입력: `product_id`, `valid_from?`(공개 시점, 없으면 즉시), `note?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "prd_windbreaker", undo: {tool: "undo_change_set", until: "..."}}`
- 프리뷰: `changes`=`update` `approval_status` `pending_review` → `active`와 보류된 가격·변형. checks `price_floor`(공개 직전 재검사), `required_on_create`, `variant_integrity`, `digital_voucher_disallow`. 항상 `needs_approval: true`, 예 `approval_reason: "Publishes 1 product"`.
- 정책: [catalog.yml](policies/catalog.yml) `product.approval`, `pricing.margin_protection`; [approval.yml](policies/approval.yml) `product_approve`
- 에러: `invalid_state` → `get_change_history` · `below_price_floor` → `set_price` · `preview_expired` → `approve_product`
- 멱등성: `Idempotency-Key`. 다시 보내도 한 번만 공개

### `reject_product` — Kind: W
- 논리주소/REST: `catalog.approval.reject` · `POST /v1/catalog/products/{product_id}/reject`
- 요약: 심사를 반려합니다(`pending_review` → `rejected`, 이후 `draft`로 다시 편집).
- consumes → produces: `product_id` → `product_id`
- 입력: `product_id`, `reason`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` `approval_status`. `needs_approval: false`.
- 정책: [catalog.yml](policies/catalog.yml) `product.approval`
- 에러: `invalid_state` → `get_change_history`
- 멱등성: `Idempotency-Key`

### `get_change_history` — Kind: R
- 논리주소/REST: `catalog.approval.get_history` · `GET /v1/catalog/products/{product_id}/history`
- 요약: 상품의 변경 이력(추가만 되는 기록)을 돌려줍니다.
- consumes → produces: `product_id` → history
- 입력: `product_id`, `limit=20`, `offset=0`
- 출력: `{items: [{agent, change_type: "content" | "structural", from, to, end_time}], total}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `product_not_found` → `search_products`
- 멱등성: 읽기(부작용 없음)

---

## brand

### `list_brands` — Kind: R
- 논리주소/REST: `catalog.brand.list` · `GET /v1/catalog/brands`
- 요약: 브랜드 목록.
- consumes → produces: → `brand_id[]`
- 입력: `name?`, `limit=20`, `offset=0`
- 출력: `{items: Brand[], total}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_filter` → `list_brands`
- 멱등성: 읽기(부작용 없음)

### `get_brand` — Kind: R
- 논리주소/REST: `catalog.brand.get` · `GET /v1/catalog/brands/{brand_id}`
- 요약: 브랜드 하나(`name`, `logo`).
- consumes → produces: `brand_id` → brand
- 입력: `brand_id`
- 출력: `{items: [Brand], total: 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `brand_not_found` → `list_brands`
- 멱등성: 읽기(부작용 없음)

### `create_brand` — Kind: W
- 논리주소/REST: `catalog.brand.create` · `POST /v1/catalog/brands`
- 요약: 브랜드를 만듭니다.
- consumes → produces: → `brand_id`
- 입력: `name`, `logo?`(ImageObject), `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id: "brd_northwind", undo}`
- 프리뷰: `needs_approval: false`
- 정책: 없음
- 에러: `duplicate_brand` → `list_brands`
- 멱등성: `Idempotency-Key`

### `update_brand` — Kind: W
- 논리주소/REST: `catalog.brand.update` · `PATCH /v1/catalog/brands/{brand_id}`
- 요약: 브랜드 이름·로고를 고칩니다.
- consumes → produces: `brand_id` → `brand_id`
- 입력: `brand_id`, `name?`, `logo?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `needs_approval: false`
- 정책: 없음
- 에러: `brand_not_found` → `list_brands`
- 멱등성: `Idempotency-Key`

---

## review

### `get_review_summary` — Kind: R  (MVP, P2)
- 논리주소/REST: `catalog.review.summarize` · `GET /v1/catalog/products/{product_id}/reviews/summary`
- 요약: 리뷰를 요약해 핏 투표와 반복되는 표현을 돌려줍니다. 키 범위를 주면 그 범위 리뷰어만 셉니다.
- consumes → produces: `product_id` → review summary
- 입력: `product_id`(예 `"prd_wool_overcoat"`), `height_cm_range?`(예 `{min: 175, max: 182}`)
- 출력:
```jsonc
{"items": [{
  "product_id": "prd_wool_overcoat", "review_count": 41, "height_cm_range": {"min": 175, "max": 182},
  "fit_votes": {"true_to_size": 29, "a_bit_snug": 12},
  "repeated_phrases": [{"text": "size up for a hoodie", "count": 9}]
 }], "total": 1}
```
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `product_not_found` → `search_products` · `no_reviews` → `get_product_measurements`
- 멱등성: 읽기(부작용 없음)

### `list_reviews` — Kind: R
- 논리주소/REST: `catalog.review.list` · `GET /v1/catalog/reviews`
- 요약: 리뷰 목록(`review_rating`, `author`, `review_body`).
- consumes → produces: `product_id?` → `review_id[]`
- 입력: `product_id?`, `rating?`, `status?`(`visible` | `hidden` | `reported`), `sort?`(`newest` | `rating`), `limit=20`, `offset=0`
- 출력: `{items: Review[], total}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_filter` → `list_reviews`
- 멱등성: 읽기(부작용 없음)

### `moderate_review` — Kind: W
- 논리주소/REST: `catalog.review.moderate` · `POST /v1/catalog/reviews/{review_id}/moderate`
- 요약: 리뷰를 숨기거나 되살립니다.
- consumes → produces: `review_id` → `review_id`
- 입력: `review_id`, `action`(`hide` | `restore`), `reason?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` `status`. `needs_approval: false`.
- 정책: [catalog.yml](policies/catalog.yml)
- 에러: `invalid_state` → `list_reviews`
- 멱등성: `Idempotency-Key`

### `create_review` — Kind: W
- 논리주소/REST: `catalog.review.create` · `POST /v1/catalog/reviews`
- 요약: 쇼퍼가 구매한 상품에 리뷰(별점·본문·사진)를 씁니다. 구매 확인 후에만 가능하며, 바로 보이고 사후에 `moderate_review`로 심사합니다.
- consumes → produces: `product_id`, `order_id` → `review_id`
- 입력: `product_id`, `order_id`, `review_rating`(1–5), `review_body`, `image?[]`, `preview`. 작성자는 로그인한 쇼퍼
- 출력: 프리뷰 형식 → 실행 `{ok, id: "rev_8812", undo}`
- 프리뷰: checks `verified_purchase`, `one_per_order_item`, `profanity`. `needs_approval: false`.
- 정책: [catalog.yml](policies/catalog.yml)
- 에러: `not_purchased` → `get_my_orders` · `already_reviewed` → `update_review` · `profanity_blocked` → `create_review`
- 멱등성: `(order_id, product_id, 작성자)`로 한 건

### `update_review` — Kind: W
- 논리주소/REST: `catalog.review.update` · `PATCH /v1/catalog/reviews/{review_id}`
- 요약: 쇼퍼가 자기 리뷰를 고칩니다.
- consumes → produces: `review_id` → `review_id`
- 입력: `review_id`, `review_rating?`, `review_body?`, `image?[]`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `needs_approval: false`
- 정책: 작성자 본인만
- 에러: `not_author` → `list_reviews` · `review_not_found` → `list_reviews`
- 멱등성: `Idempotency-Key`

### `delete_review` — Kind: W
- 논리주소/REST: `catalog.review.delete` · `DELETE /v1/catalog/reviews/{review_id}`
- 요약: 쇼퍼가 자기 리뷰를 지웁니다.
- consumes → produces: `review_id` →
- 입력: `review_id`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`delete` `review`. `needs_approval: false`.
- 정책: 작성자 본인만
- 에러: `not_author` → `list_reviews`
- 멱등성: 이미 지운 리뷰는 같은 결과

### `vote_review_helpful` — Kind: W
- 논리주소/REST: `catalog.review.vote_helpful` · `POST /v1/catalog/reviews/{review_id}/helpful`
- 요약: "도움이 됐어요" 투표를 켜거나 끕니다.
- consumes → produces: `review_id` → `review_id`
- 입력: `review_id`, `helpful`(Boolean), `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `needs_approval: false`
- 정책: 없음
- 에러: `review_not_found` → `list_reviews`
- 멱등성: `(review_id, 투표자)`로 상태 하나

### `report_review` — Kind: W
- 논리주소/REST: `catalog.review.report` · `POST /v1/catalog/reviews/{review_id}/report`
- 요약: 부적절한 리뷰를 신고해 심사 대기열에 넣습니다. 후속은 `moderate_review`.
- consumes → produces: `review_id` → `review_id`
- 입력: `review_id`, `reason`(`spam` | `abuse` | `irrelevant` | `fake`), `note?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `needs_approval: false`
- 정책: 없음
- 에러: `review_not_found` → `list_reviews`
- 멱등성: `(review_id, 신고자)`로 한 건

---

## question

### `list_questions` — Kind: R
- 논리주소/REST: `catalog.question.list` · `GET /v1/catalog/questions`
- 요약: 상품 문의 목록(`text`, `accepted_answer`).
- consumes → produces: `product_id?` → `question_id[]`
- 입력: `product_id?`, `answered?`(Boolean), `status?`, `limit=20`, `offset=0`
- 출력: `{items: Question[], total}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_filter` → `list_questions`
- 멱등성: 읽기(부작용 없음)

### `answer_question` — Kind: W
- 논리주소/REST: `catalog.question.answer` · `POST /v1/catalog/questions/{question_id}/answer`
- 요약: 문의에 답(`Answer`)을 답니다.
- consumes → produces: `question_id` → `question_id`
- 입력: `question_id`, `text`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`create` `answer`, 쇼퍼가 읽을 문장 그대로. `needs_approval`은 [approval.yml](policies/approval.yml)에 따름.
- 정책: [approval.yml](policies/approval.yml)
- 에러: `already_answered` → `list_questions`
- 멱등성: `Idempotency-Key`

### `moderate_question` — Kind: W
- 논리주소/REST: `catalog.question.moderate` · `POST /v1/catalog/questions/{question_id}/moderate`
- 요약: 문의·답을 숨기거나 되살립니다.
- consumes → produces: `question_id` → `question_id`
- 입력: `question_id`, `action`(`hide` | `restore`), `reason?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `needs_approval: false`
- 정책: 없음
- 에러: `invalid_state` → `list_questions`
- 멱등성: `Idempotency-Key`

### `ask_question` — Kind: W
- 논리주소/REST: `catalog.question.ask` · `POST /v1/catalog/questions`
- 요약: 쇼퍼가 상품 문의를 남깁니다. 비공개로 지정할 수 있습니다.
- consumes → produces: `product_id` → `question_id`
- 입력: `product_id`, `text`, `acom:private?`(Boolean), `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id: "qst_311", undo}`
- 프리뷰: checks `profanity`. `needs_approval: false`.
- 정책: 없음
- 에러: `profanity_blocked` → `ask_question`
- 멱등성: `Idempotency-Key`

### `update_question` — Kind: W
- 논리주소/REST: `catalog.question.update` · `PATCH /v1/catalog/questions/{question_id}`
- 요약: 쇼퍼가 답이 달리기 전의 자기 문의를 고칩니다.
- consumes → produces: `question_id` → `question_id`
- 입력: `question_id`, `text?`, `acom:private?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `needs_approval: false`
- 정책: 작성자 본인만
- 에러: `not_author` → `list_questions` · `already_answered` → `list_questions`
- 멱등성: `Idempotency-Key`

### `delete_question` — Kind: W
- 논리주소/REST: `catalog.question.delete` · `DELETE /v1/catalog/questions/{question_id}`
- 요약: 쇼퍼가 자기 문의를 지웁니다.
- consumes → produces: `question_id` →
- 입력: `question_id`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `needs_approval: false`
- 정책: 작성자 본인만
- 에러: `not_author` → `list_questions`
- 멱등성: 이미 지운 문의는 같은 결과

---

## bundle

### `create_bundle` — Kind: W
- 논리주소/REST: `catalog.bundle.create` · `POST /v1/catalog/bundles`
- 요약: 묶음 상품을 만듭니다. 가격·재고는 구성품에서 파생하고, 구성품은 같은 판매자·같은 연령 제한(`acom:min_age`)이어야 합니다.
- consumes → produces: `product_id[]` → `product_id`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명·예 |
  |------|----------------------|:--:|--------|
  | name | Text | ✓ | "Denim bundle" |
  | is_related_to | Product[] | ✓ | 구성품 `{product_id, order_quantity}` |
  | category | DefinedTerm | ✓ | |
  | acom:bundle_discount | UnitPriceSpecification | | 묶음 할인 |
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "prd_denim_bundle", undo}`
- 프리뷰: `changes`=`create` `product`(파생 가격·재고 포함). checks `same_seller`, `same_age_restriction`, `price_floor`(파생 가격), `component_active`. 가격이 정해지므로 `needs_approval: true`.
- 정책: [catalog.yml](policies/catalog.yml) `bundle`, `pricing.margin_protection`
- 에러: `mixed_seller` → `create_bundle` · `mixed_age_restriction` → `create_bundle` · `below_price_floor` → `set_price` · `component_inactive` → `get_product`
- 멱등성: `Idempotency-Key = hash(components + name)`

---

## restock_alert

### `subscribe_restock` — Kind: W
- 논리주소/REST: `catalog.restock_alert.subscribe` · `POST /v1/catalog/products/{product_id}/restock-alerts`
- 요약: 품절된 변형의 재입고 알림을 신청합니다. 재고가 돌아오면 거래성 알림으로 보냅니다(마케팅 아님).
- consumes → produces: `product_id` → `restock_alert_id`
- 입력: `product_id`, `size?`, `color?`, `channel`(`sms` | `email` | `push`), `preview`. 신청자는 로그인한 쇼퍼
- 출력: 프리뷰 형식 → 실행 `{ok, id: "rsa_204", undo}`
- 프리뷰: `needs_approval: false`
- 정책: [catalog.yml](policies/catalog.yml) `inventory.auto_restore_when`
- 에러: `already_in_stock` → `add_to_cart` · `already_subscribed` → `list_restock_subscriptions`
- 멱등성: `(product_id, 신청자, channel)`로 한 건

### `list_restock_subscriptions` — Kind: R
- 논리주소/REST: `catalog.restock_alert.list` · `GET /v1/catalog/restock-alerts`
- 요약: 로그인한 쇼퍼의 재입고 알림 신청 목록.
- consumes → produces: → `restock_alert_id[]`
- 입력: `limit=20`, `offset=0`
- 출력: `{items: [{id, product_id, size, channel, availability, start_time}], total}`
- 프리뷰: 해당 없음(읽기)
- 정책: 본인 것만
- 에러: `invalid_filter` → `list_restock_subscriptions`
- 멱등성: 읽기(부작용 없음)
