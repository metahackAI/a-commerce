# API 명세 — `catalog` (상품·변형·가격·재고·브랜드·리뷰·문의)

> 계층: `catalog.{resource}.{operation}`. 공통 형식(읽기 봉투, 쓰기의 프리뷰 → `preview_id` 실행, 시간·금액·id)은 [conventions.md](conventions.md), 에러 코드는 [errors.md](errors.md), 상태 값은 [enums.md](enums.md), 데이터 타입은 [data-model.md](data-model.md), 정책은 [catalog.yml](../../policies/catalog.yml)·[approval.yml](../../policies/approval.yml). 예시 값은 데모 스토어([../scenarios.md](../scenarios.md))입니다.
>
> **설계 축**: ① 상품 = `ProductGroup`(`prd_`, 사람이 말하는 상품) → 변형 = `Product`(`var_`, 사이즈·색, SKU 단위). 다른 엔티티는 상품을 `product_id`, 변형을 `variant_id`로 가리킵니다([conventions.md](conventions.md) §9). ② 재고·가격은 변형 단위, 상품의 재고·상태는 변형에서 파생(읽기 전용). ③ 원가와 마진은 운영자에게만 보이고 쇼퍼에게는 보이지 않음(`pricing.cost_visibility`). ④ 설명·이미지 같은 콘텐츠 변경은 승인 없이, 가격·구조·단종은 승인 대상. ⑤ `price_floor`(원가 × 1.15) 아래 판매는 서버가 막음.

리소스: [product_group](#product_group) · [product](#product) · [offer](#offer) · [inventory](#inventory) · [reorder](#reorder) · [approval](#approval) · [brand](#brand) · [review](#review) · [question](#question) · [bundle](#bundle) · [restock_alert](#restock_alert)

> 상품 검색 `search_products`는 [search.md](search.md#product)에 한 번만 정의합니다(`search.product.search`).

---

## product_group

> 리소스 이름은 schema.org 타입(`ProductGroup`)을 따르지만, 이 리소스의 엔티티가 사람이 말하는 "상품"입니다. id는 `prd_`이고 참조 필드는 `product_id`입니다.

### `search_product_groups` — Kind: R
- 논리주소/REST: `catalog.product_group.search` · `GET /v1/catalog/product-groups`
- 요약: 승인 상태·브랜드처럼 카탈로그 관리 조건으로 상품을 찾습니다(초안·심사 중 포함). 쇼퍼에게 보이는 검색은 `search_products`입니다.
- consumes → produces: → `product_id[]`
- 입력: `query?`(Text), `brand_id?`, `category_id?`, `approval_status?`(`draft` | `pending_review` | `active` | `rejected` | `discontinued`), `availability?`(`in_stock` | `out_of_stock` | `pre_order` | `back_order` | `discontinued`), `import_id?`(`imp_…`, 그 가져오기가 만든 초안만), `draft_status?`(`ready` | `needs_you` | `working`, 공급사 시트 초안의 진행), `sort?`(`relevance` | `newest` | `price` | `-price`), `fields?[]`, `limit?`(기본 20, 최대 100), `cursor?`
- 출력: `{items: ProductGroup[], total, next_cursor?}`. 공급사 시트에서 온 상품은 운영자에게 `acom:draft: {import_id, row, supplier_code, draft_status, open_fields[], note, supplier_text}`가 붙습니다.
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_filter` → `search_product_groups`
- 멱등성: 읽기(부작용 없음)

### `get_product_group` — Kind: R
- 논리주소/REST: `catalog.product_group.get` · `GET /v1/catalog/product-groups/{product_id}`
- 요약: 상품 하나를 카탈로그 관리 관점(승인 상태, 옵션 축 `varies_by`, 변형 전체, 보류 중인 변경)으로 돌려줍니다. 판매 관점의 가격·재고 요약은 `get_product`.
- consumes → produces: `product_id` → product_group
- 입력: `product_id`, `fields?[]`
- 출력: `ProductGroup` 한 건(단건 읽기라 목록 봉투 없음). 예 `{id: "prd_windbreaker", name: "Packable windbreaker", approval_status: "active", varies_by: ["color", "size"], has_variant: [...], pending_changes: []}`. 운영자에게는 공급사 시트 초안이면 `acom:draft`(`search_product_groups`와 같은 모양), 변형마다 예약된 정가 `scheduled_prices[]{unit_price_specification_id, price, valid_from, valid_through}`가 붙습니다.
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `product_not_found` → `search_product_groups`
- 멱등성: 읽기(부작용 없음)

### `create_product_group` — Kind: W
- 논리주소/REST: `catalog.product_group.create` · `POST /v1/catalog/product-groups`
- 요약: 상품(모델)을 `draft`로 등록합니다. 변형은 `set_variants`로 따로 만듭니다.
- consumes → produces: `brand_id?`, `category_id` → `product_id`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명·예 |
  |------|----------------------|:--:|--------|
  | name | Text | ✓ | "Packable windbreaker" |
  | description | Text | | 상세 설명 |
  | brand_id | Text | | 예 `"brn_northwind"` |
  | category_id | Text | ✓ | 표준 카테고리, 예 `"cat_summer_outerwear"` |
  | image | ImageObject[] | | 대표·상세 이미지 URL 목록. 첫 번째가 대표 이미지 |
  | varies_by | PropertyValue[] | | 옵션 축 단어 목록(`color`, `size`) |
  | material | Text | | 주 소재(쇼퍼에게 보임), 예 `"Recycled nylon"` |
  | acom:min_age | Integer | | 구매 가능 최소 연령(세). 없거나 `0`이면 제한 없음. 허용 값·확인 방식은 스토어 설정([catalog.yml](../../policies/catalog.yml) `product.age_restriction`). **아직 저장하지 않음**(입력에서 빠짐) |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "prd_windbreaker", undo: {by: "person", target: "cs_1044", until: "2026-11-07T23:59:00-05:00"}}`. id는 이름에서 만들고(`prd_trail_shell`), 이미 있으면 `_2`, `_3`
- 프리뷰: `changes`=`create` `product`(`id: null`, `to.product_id`에 만들 id). checks `product.required_on_create`(판매자·카테고리·이름; 판매자는 스토어), `variant.max_option_axes`(스토어가 상한을 정했을 때). 콘텐츠 등록이라 `needs_approval: false`. 카테고리는 활성 표준 카테고리여야 함(전시 카테고리는 `invalid_category`). 되돌리기는 아직 변형이 없는 초안일 때만 상품을 지웁니다.
- 정책: [catalog.yml](../../policies/catalog.yml) `product.required_on_create`, `product.age_restriction`, `variant.max_option_axes`
- 에러: `missing_required_field` → `create_product_group` · `invalid_category` → `list_categories` · `brand_not_found` → `list_brands`
- 멱등성: `Idempotency-Key`

### `update_product_group` — Kind: W
- 논리주소/REST: `catalog.product_group.update` · `PATCH /v1/catalog/product-groups/{product_id}`
- 요약: 상품 속성을 고칩니다. 설명·이미지 변경은 승인 없이, 카테고리 같은 구조 변경은 승인 대상입니다.
- consumes → produces: `product_id` → `product_id`
- 입력: `product_id`, 바꿀 필드(`name`, `description`, `category_id`, `brand_id`, `image`, `varies_by`, `material`). `description`의 빈 문자열은 설명을 지움
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` `product` 필드별 `from`/`to`. 쇼퍼가 볼 수 있는 상품(`active`, `pending_review`)에 구조 변경(카테고리·옵션 축)이 있으면 `needs_approval: true`, `approval_reason: "Changes the category"`([approval.yml](../../policies/approval.yml) `thresholds.product_approve`). 초안·반려 상품은 고객에게 닿지 않으므로 같은 변경도 콘텐츠로 판정해 승인 없이 실행합니다. 공급사 시트 초안의 열린 필드(`material`, `image`)를 채우면 `acom:open_fields`에서 빠지고, 다 채우면 `acom:draft_status`가 `ready`가 됩니다(checks `import.required_fields`). checks `product.approval`(왜 승인이 필요한지/아닌지).
- 정책: [catalog.yml](../../policies/catalog.yml) `product.approval`
- 에러: `invalid_state` → `get_product_group`
- 멱등성: `Idempotency-Key`

---

## product

### `get_product` — Kind: R  (데모 장면)
- 논리주소/REST: `catalog.product.get` · `GET /v1/catalog/products/{product_id}`
- 요약: 상품(`prd_`, `ProductGroup`) 하나를 판매 관점으로 돌려줍니다: 정가, 부르는 사람에게 해당하는 회원가, 상품 재고 합계, 변형별 재고, 비교에 쓰는 속성. 운영자에게는 원가(`cost`)도 보이고, 쇼퍼에게는 그 쇼퍼 자신의 회원가가 붙습니다. 장바구니에 담을 변형 id(`variant_id`)도 여기서 나옵니다.
- consumes → produces: `product_id` → product, `variant_id[]`
- 입력 (닫힌 객체):
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | product_id | Text `^prd_[a-z0-9_]+$` | ✓ | 상품 id(`search_products`) | `"prd_windbreaker"` |
  | valid_for_member_tier | Text | | **운영자만.** 회원가를 계산할 등급 id 또는 `"everyone"`. 기본 `"everyone"` | `"vip"` |
  | as_of | DateTime | | **운영자만.** 회원가를 계산할 시각. 기본 지금 | `"2026-11-07T00:00:00-05:00"` |
  | fields | Text[] | | 돌려줄 필드만 | `["id", "price", "has_variant"]` |
  - **운영자만** 필드를 쇼퍼 토큰으로 보내면 `forbidden`(`reason: scope`).
- 출력(운영자 SJ, Fri Nov 6 11:02, 세일 전): 단건 읽기라 목록 봉투 없이 엔티티 그대로
```jsonc
{"id": "prd_windbreaker", "@type": "ProductGroup", "name": "Packable windbreaker",
 "brand_id": "brn_northwind", "category_id": "cat_summer_outerwear", "approval_status": "active",
 "varies_by": ["color", "size"],
 "keywords": ["packable", "travel"],
 "price": 89.00, "price_currency": "USD",
 "member_price": null,
 "cost": 36.00,
 "inventory_level": 214, "availability": "in_stock",
 "additional_property": [
   {"name": "material", "value": "Recycled nylon"},
   {"name": "weight", "value": 190, "unit_code": "g", "size": "M"},
   {"name": "packing", "value": "own_pocket"},
   {"name": "packed_size", "value": [14, 18], "unit_code": "cm"},
   {"name": "warmth", "value": "wind_layer"},
   {"name": "rain", "value": true}],
 "has_variant": [
   {"variant_id": "var_windbreaker_m_slate", "sku": "wb-slate-m", "size": "M", "color": "Slate", "inventory_level": 18, "availability": "in_stock"},
   {"variant_id": "var_windbreaker_l_slate", "sku": "wb-slate-l", "size": "L", "color": "Slate", "inventory_level": 12, "availability": "in_stock"},
   {"variant_id": "var_windbreaker_m_black", "sku": "wb-black-m", "size": "M", "color": "Black", "inventory_level": 9, "availability": "in_stock"}]}
   // has_variant: 5색 × XS–XL = 25개 중 일부
```
- 출력(쇼퍼 Jamie, VIP, Sat Nov 7 10:00): 같은 모양에서 `cost` 키가 없고, `member_price`가 Jamie의 등급 기준으로 채워집니다.
```jsonc
{"id": "prd_windbreaker", "name": "Packable windbreaker", "price": 89.00, "price_currency": "USD",
 "member_price": {"price": 71.20, "name": "VIP weekend", "valid_for_member_tier": "vip", "promotion_id": "promo_1042",
                  "valid_through": "2026-11-08T23:59:00-05:00"},
 "inventory_level": 214, "availability": "in_stock", "has_variant": [/* … */]}
```
  - `keywords`는 상품 태그(`catalog.product_group.keywords`)입니다. 검색 필터(`search_products`의 `tags`)와 같은 단어이고, 상품 페이지의 JSON-LD `keywords`가 됩니다.
  - `price`는 정가(`list_price`), `member_price`는 그 시각 적용되는 회원·기간 한정 판매가(`UnitPriceSpecification`, `price_type: sale_price`)이고 없으면 `null`입니다. 가격 계산은 `search_products`와 같습니다.
  - `cost`는 운영자 주체에게만 있는 필드입니다. 쇼퍼·외부 에이전트 응답에는 키 자체가 없습니다.
  - `additional_property`는 상품의 사실입니다. 이름은 `compare_products`의 속성(`weight`, `packed_size`, `packing`, `warmth`, `rain`, `material`)과 상품 사실 속성(`fiber_content`, `materials`(신발·가방 등 부위별 소재), `fit`, `care`, `fabric`, `fabric_weight`, `knit_gauge`, `finish`, `insulation`, `closure`, `sleeve_length`, `neckline`, `hood`, `support`, `features`, `dimensions`, `capacity`, `width`, `fits_shoe_size`, `fits_waist`, `product_type`, `google_product_category`)입니다. 단위가 있으면 `unit_code`(예: `g`, `cm`, `g/m2`), 사이즈마다 다르면 `size`(예: `weight`는 M 기준)를 함께 둡니다.
- 프리뷰: 해당 없음(읽기)
- 정책: [catalog.yml](../../policies/catalog.yml) `pricing.cost_visibility`, `pricing.sale_price`, `inventory.product_status`
- 에러: `not_found`(`detail.entity: product`) → `search_products` · `tier_not_found` → `list_tiers` · `forbidden`(쇼퍼가 운영자 전용 필드를 보냄) → null
- 멱등성: 읽기(부작용 없음)

### `get_product_measurements` — Kind: R  (데모 장면)
- 논리주소/REST: `catalog.product.get_measurements` · `GET /v1/catalog/products/{product_id}/measurements?sizes=M,L`
- 요약: 사이즈별 실측(cm)을 돌려줍니다. 쇼퍼가 "M과 L 중 무엇이 맞을까", "후드티 위에 입으려면" 같은 질문을 할 때 쓰고, 화면은 `fit_check` 블록의 `measurements`([chat.md](chat.md) §3.3)입니다. 쇼퍼가 전에 산 상품의 실측과 나란히 놓을 수 있도록 상품마다 같은 측정 이름을 씁니다.
- consumes → produces: `product_id` → measurements
- 입력 (닫힌 객체):
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | product_id | Text `^prd_[a-z0-9_]+$` | ✓ | 상품 id | `"prd_wool_overcoat"` |
  | sizes | Text[] | | 이 사이즈만. 기본 전부. 상품의 `varies_by` 사이즈 값 | `["M", "L"]` |
- 출력: 단건 읽기라 목록 봉투 없이
```jsonc
{"product_id": "prd_wool_overcoat",
 "unit": "cm",
 "measurements": ["chest", "shoulder", "sleeve", "length"],
 "sizes": [
   {"size": "M", "chest": 56, "shoulder": 46, "sleeve": 64, "length": 98},
   {"size": "L", "chest": 59, "shoulder": 48, "sleeve": 65.5, "length": 100}],
 "how_measured": "Garment laid flat. Chest is armpit to armpit; length is from the highest point of the shoulder."}
```
  - `measurements[]`는 이 상품에 있는 측정 이름의 순서이고 값은 enum `chest` \| `underbust` \| `shoulder` \| `sleeve` \| `length` \| `waist` \| `hip` \| `inseam` \| `rise` \| `foot_length`입니다. `sizes[]`의 각 항목은 `size`와 이 이름들을 키로 가진 숫자(`unit` 단위, 소수 한 자리까지)입니다.
  - 예: `prd_windbreaker` M은 `{"size": "M", "chest": 57, "shoulder": 47, "sleeve": 64, "length": 71}`(F3에서 Jamie가 가진 windbreaker M과 비교).
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `not_found`(`detail.entity: product`) → `search_products` · `size_not_found`(`detail.allowed[]`에 있는 사이즈) → `get_product` · `not_found`(`detail.entity: measurement_chart`, 사이즈표가 없음) → `get_review_summary`
- 멱등성: 읽기(부작용 없음)

### `create_product` — Kind: W
- 논리주소/REST: `catalog.product.create` · `POST /v1/catalog/variants`
- 요약: 변형(`var_`) 하나를 `draft`로 만듭니다. 상품 없이 단품으로 등록하면 단품 상품(`prd_`)을 함께 만들고, 기존 상품에 변형을 더할 수도 있습니다. 여러 변형은 `set_variants`가 낫습니다.
- consumes → produces: `product_id?`, `brand_id?` → `variant_id`, `product_id`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명·예 |
  |------|----------------------|:--:|--------|
  | product_id | Text | | 소속 상품(`is_variant_of`). 없으면 단품 상품을 자동 생성 |
  | sku | Text | ✓ | 스토어 안에서 유일 |
  | name | Text | ✓ | 변형 이름 |
  | category_id | Text | (✓) | 표준 카테고리. `product_id`가 없을 때(단품 상품을 함께 만들 때)만 필요 |
  | brand_id | Text | | |
  | image | ImageObject[] | | |
  | additional_property | PropertyValue[] | | 옵션 조합 객체 `{color: "Slate", size: "M"}` |
  | offers | Offer | | 초기 가격 `{price: 89.00, price_currency: "USD"}`. 재고 0·`out_of_stock`으로 시작하고 재고는 `set_variant_stock`. 생략하면 가격 없는 변형(오퍼 없음)이고 `set_variants`로 가격을 줌 |
  | acom:cost | acom:CostSpecification | | 원가(운영자 전용) |
  | acom:managed | Boolean | | 재고 관리 여부(기본 `true`) |
  | acom:sale_type | enum | | `always`(기본) \| `pre_order` \| `subscription` \| `gift_card` |
  | acom:min_age | Integer | | 구매 가능 최소 연령(세). 없거나 `0`이면 제한 없음. 허용 값·확인 방식은 스토어 설정([catalog.yml](../../policies/catalog.yml) `product.age_restriction`) |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "var_windbreaker_m_slate", undo: {by: "person", target: "cs_1044", until: "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: `changes`=`create` `variant`(+ `offer`, 단품이면 `product`), 모두 `id: null`. checks `product.required_on_create`, `product.sku_unique`, `price_floor`(가격이 있으면 원가 × 1.15 이상, 실패 시 `fix_tool: "set_price"`), `gift_card.disallow`(`gift_card`일 때 금지 조합). 쇼퍼가 볼 수 있는 상품에 가격 있는 변형을 더하면 구조 변경으로 `needs_approval: true`([approval.yml](../../policies/approval.yml) `thresholds.product_approve`), 초안이면 `false`. id는 SKU에서(`wb-sand-m` → `var_wb_sand_m`, `off_wb_sand_m`).
- 정책: [catalog.yml](../../policies/catalog.yml) `product.required_on_create`, `product.sku_unique`, `product.age_restriction`, `price_floor`, `variant.option_source`, `gift_card.disallow`
- 에러: `duplicate_sku` → `get_product` · `below_price_floor` → `set_price` · `min_age_not_allowed` → `get_policy` · `missing_required_field` → `create_product` · `invalid_sale_type` → `update_product`
- 멱등성: `Idempotency-Key = hash(tenant + sku)`. 같은 SKU로 다시 보내면 첫 결과를 돌려줌

### `update_product` — Kind: W
- 논리주소/REST: `catalog.product.update` · `PATCH /v1/catalog/variants/{variant_id}`
- 요약: 변형 속성을 고칩니다. 콘텐츠는 승인 없이, 가격·구조는 승인 대상입니다.
- consumes → produces: `variant_id` → `variant_id`
- 입력: `variant_id`, 바꿀 필드(`name`, `image`, `additional_property`(`{color, size}`), `acom:managed`, `acom:sale_type`). 가격은 `set_price`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` `variant` 필드별 `from`/`to`. checks `product.approval`, `gift_card.disallow`. 쇼퍼가 볼 수 있는 상품의 색·사이즈·판매 방식 변경은 `needs_approval: true`([approval.yml](../../policies/approval.yml) `thresholds.product_approve`).
- 정책: [catalog.yml](../../policies/catalog.yml) `product.approval`, `price_floor`
- 에러: `invalid_state` → `get_product` · `below_price_floor` → `set_price`
- 멱등성: `Idempotency-Key`

### `set_variants` — Kind: W
- 논리주소/REST: `catalog.product.set_variants` · `PUT /v1/catalog/product-groups/{product_id}/variants`
- 요약: 옵션 축을 정하고 조합별 변형을 한 번에 만들거나 맞춥니다(추가·수정·판매 중지). 새 옵션 값은 백엔드가 중복 없이 등록합니다.
- consumes → produces: `product_id` → `variant_id[]`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명·예 |
  |------|----------------------|:--:|--------|
  | product_id | Text | ✓ | 상품 id, 예 `"prd_windbreaker"` |
  | varies_by | PropertyValue[] | ✓ | 축과 값 객체 `{color: [Slate, Sand], size: [S, M, L]}`. 축 수 상한은 스토어 설정 `variant.max_option_axes`(기본 제한 없음), 조합 수는 `variant.max_variants_per_group` |
  | has_variant | Product[] | ✓ | 조합별 `{sku, additional_property: {color, size}, offers?: {price, price_currency}, acom:cost?, acom:managed?}`. 이 상품의 기존 SKU면 고침, 새 SKU면 만듦 |
  | missing_combination | enum | | `create_all`(목록에 없는 조합도 가격 없이 만듦) \| `listed_only`(기본) |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "prd_windbreaker", undo: {by: "person", target: "cs_1044", until: "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: `changes`=조합별 `create`/`update` `variant`. 입력에서 빠진 기존 변형은 지우지 않고 `update`(`availability` → `discontinued`, `variant.missing_from_input`). checks `variant.max_option_axes`(설정했을 때), `variant.max_variants_per_group`, `product.sku_unique`, `price_floor`(가격이 있는 변형). 판매 중인 상품이면 구조 변경으로 `needs_approval: true`.
- 정책: [catalog.yml](../../policies/catalog.yml) `variant.max_option_axes`, `variant.max_variants_per_group`, `variant.option_source`, `variant.missing_from_input`, `product.approval.structural_changes`, `price_floor`
- 에러: `too_many_option_axes` → `set_variants` · `too_many_variants` → `set_variants` · `duplicate_sku` → `get_product` · `incomplete_combination` → `set_variants` · `below_price_floor` → `set_price`
- 멱등성: `Idempotency-Key = hash(product_id + variants_digest)`. 같은 조합을 다시 적용하면 변경 없음

### `discontinue_product` — Kind: W
- 논리주소/REST: `catalog.product.discontinue` · `POST /v1/catalog/variants/{variant_id}/discontinue`
- 요약: 변형을 단종합니다(`availability` → `discontinued`, 되돌릴 수 없음). 모든 변형이 단종되면 상품도 단종으로 보입니다.
- consumes → produces: `variant_id` → `variant_id`
- 입력: `variant_id`, `reason?`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo: null}`
- 프리뷰: `changes`=`update` `variant` `availability` `in_stock` → `discontinued`. checks `product.discontinue`(진행 중 구독이 있으면 `passed: false`, `fix_tool: "list_subscriptions"`). 되돌릴 수 없는 구조 변경이라 `needs_approval: true`([approval.yml](../../policies/approval.yml) `thresholds.product_approve`).
- 정책: [catalog.yml](../../policies/catalog.yml) `product.discontinue`, `inventory.availability_values`
- 에러: `invalid_state` → `get_product` · `has_open_subscriptions` → `list_subscriptions`
- 멱등성: 이미 단종이면 첫 결과를 돌려줌

### `get_return_policy` — Kind: R
- 논리주소/REST: `catalog.product.get_return_policy` · `GET /v1/catalog/products/{product_id}/return-policy`
- 요약: 상품에 적용되는 반품 정책(`MerchantReturnPolicy`)을 상세 화면용으로 돌려줍니다.
- consumes → produces: `product_id` → return_policy
- 입력: `product_id`
- 출력: `{product_id: "prd_wool_overcoat", merchant_return_days: 30, return_fees: "free_return", acom:non_returnable_when: [], acom:size_exchange_days: 30}` (단건 읽기라 목록 봉투 없음)
- 프리뷰: 해당 없음(읽기)
- 정책: [fulfillment.yml](../../policies/fulfillment.yml)
- 에러: `product_not_found` → `search_products`
- 멱등성: 읽기(부작용 없음)

### `get_delivery_policy` — Kind: R
- 논리주소/REST: `catalog.product.get_delivery_policy` · `GET /v1/catalog/products/{product_id}/delivery-policy`
- 요약: 상품의 배송 조건(`OfferShippingDetails`)을 돌려줍니다.
- consumes → produces: `product_id` → delivery_policy
- 입력: `product_id`
- 출력: `{product_id: "prd_windbreaker", shipping_fee_type: "free_over_min_subtotal", shipping_rate: {value: 6.00, currency: "USD"}, free_shipping_min_subtotal: {value: 80.00, currency: "USD"}, handling_days: 1}` (단건 읽기라 목록 봉투 없음)
- 프리뷰: 해당 없음(읽기)
- 정책: [fulfillment.yml](../../policies/fulfillment.yml), [order.yml](../../policies/order.yml) `free_shipping_min_subtotal`
- 에러: `product_not_found` → `search_products`
- 멱등성: 읽기(부작용 없음)

### `batch_import` — Kind: W$
- 논리주소/REST: `catalog.product.batch_import` · `POST /v1/catalog/products/batch-import`
- 요약: 공급사 시트(스프레드시트)를 상품 초안으로 만듭니다(장면 B4). 모든 행을 프리뷰로 검사한 뒤 변경셋 승인("초안 저장")을 거쳐 실행합니다. 초안은 쇼퍼에게 보이지 않고, 게시는 상품마다 `approve_product`의 별도 변경셋입니다.
- consumes → produces: `upload_id` → `import_id`, `product_id[]`
- 입력: `acom:file`(MediaObject, 업로드 id `upl_…`; 에이전트에는 대화에 첨부한 시트의 id가 슬롯으로 들어감), `acom:mode`(`create`: 모든 행을 새 초안으로 | `upsert`: 앞선 가져오기가 만든 초안은 시트 값으로 고침), `acom:publish_on_success?`(`false`만 허용: 게시는 상품마다 따로)
- 시트: `catalog.data_feed.data_feed_element`의 행(열 이름 → 셀 문자열). 필수 열 `supplier_code`, `sku`, `style_name`, `category`, `cost`; 그 밖 `color`, `brand`, `sizes`(쉼표로 나눈 사이즈), `units_per_size`, `material`, `image`, `supplier_text`. 행마다:
  - 이름 = `style_name` + `, ` + 색 소문자(`import.name_format: style_color`): "Wool blend overcoat, charcoal"
  - 카테고리 = 시트의 단어를 활성 표준 카테고리의 이름·term code와 대조(`import.category_match`)
  - 정가 = 원가에 `pricing_rule`(원가 × 2.5 → 가장 가까운 x9, 동률이면 위로): $96 → $239, $52 → $129, $33.60 → $84 → $89. 시트의 가격은 쓰지 않음
  - 사이즈마다 변형 하나(SKU `<sku>-<size>`), 오퍼(정가·원가·`units_per_size` 재고), 표준 카테고리 배치
  - 열린 필드: `import.required_fields`(`material`, `image`)가 비었거나, 두 행이 같은 사진을 쓰면(`import.one_product_per_image`, "사진에 상품이 둘") 그 필드가 열림 → `acom:draft_status: needs_you`, 아니면 `ready`
  - id는 SKU에서: `FW26-OT-014-CHR` → `prd_fw26_ot_014_chr`, `var_fw26_ot_014_chr_m`. 가져오기 id는 시트에서: `upl_fw26_supplier` → `imp_fw26_supplier`(다음 가져오기는 `_2`)
- **설명(정함)**: 가져오기는 결정적이어서 설명을 쓰지 않습니다. 초안마다 공급사의 글(`acom:supplier_text`, `catalog.data_feed_item.supplier_text`)을 남기고 `description`은 비워 둡니다. 스토어 어조는 정책 `content.copy`(두 문장 + 관리 한 줄)이고, 설명은 그 어조로 사람이나 에이전트가 `update_product_group`으로 씁니다. 설명이 없으면 게시할 수 없습니다(`product.publish_requires: description`). `ready`는 "시트에서 와야 할 필드가 다 있음"이고 설명은 따로 표시합니다. `working`은 가져오기가 아직 채우는 중인 초안의 자리이고, 실행이 끝난 가져오기에는 없습니다.
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "imp_fw26_supplier", import_id, product_id: [...], undo: {by: "person", target: "cs_1044", until: "2026-11-07T23:59:00-05:00"}}`. 결과는 `search_product_groups(import_id, draft_status?)`로 읽음
- 프리뷰: `changes[0]`=`create` `product_import` `{import_id, upload_id, file_name, rows, drafts, ready, needs_you, working, rules[]{rule, label, text}}`(쓴 규칙: 가격·하한·어조·카테고리 대조); 그 뒤 행별 `create`(`upsert`로 고치는 초안은 `update`) `product` `{row, product_id, supplier_code, sku, name, category_id, cost, price, price_floor, sizes, variants, material, image, acom:draft_status, acom:open_fields, note, acom:supplier_text, description: null}`. checks:
  - `product.required_on_create`: 이름·카테고리가 없거나 카테고리가 맞지 않는 행(`fix_tool: list_categories`)
  - `product.sku_unique`: 시트 안에서 겹치거나 카탈로그에 이미 있는 SKU(`fix_tool: get_product`)
  - `price_floor`: 가격이 하한 아래이거나 원가가 없는 행(`fix_tool: set_price`)
  - `import.required_fields`: 열린 필드가 있는 초안 수와 이유. `on_open_field: save_as_draft`이면 `passed: true`인 경고(초안은 저장되고 게시 전에 채움, `fix_tool: update_product_group`), `reject_row`면 실패
  - 항상 `needs_approval: true`, 예 `approval_reason: "Creates 30 product drafts"`. 기준(B4): 30행 → 초안 30, 중복 SKU 없음, 30개 모두 하한 이상, 필수 필드 4건 미해결(충전재 비율 2, 두 상품이 찍힌 사진 2)
- 되돌리기: 아직 `draft`·`rejected`인 초안과 그 변형·오퍼·배치·시트 행, 가져오기 기록을 지웁니다. 이미 심사·공개된 초안이 있으면 거부(먼저 그 변경셋을 되돌림). `upsert`로 고친 초안은 이전 값으로.
- 정책: [catalog.yml](../../policies/catalog.yml) `import`, `product.required_on_create`, `product.sku_unique`, `price_floor`, `pricing_rule`, `content.copy`; [approval.yml](../../policies/approval.yml) `thresholds.bulk_catalog_change`
- 에러: `invalid_file_format`(행 없음, `import.max_rows` 초과, 필수 열 없음) → `batch_import` · `not_found`(`detail.entity: upload`) → `batch_import` · `below_price_floor` → `set_price`
- 멱등성: 같은 프리뷰는 한 번만 실행. 같은 시트를 `create`로 다시 가져오면 모든 SKU가 `product.sku_unique`에 걸리므로 `upsert`로

### `generate_image` — Kind: W$
- 논리주소/REST: `catalog.product.generate_image` · `POST /v1/catalog/products/{product_id}/generate-image`
- 요약: 상품 연출 이미지를 생성합니다. 생성물은 초안으로 두고, 상세 화면 노출은 승인 후에 합니다.
- consumes → produces: `product_id` → `image_id[]`
- 입력: `product_id`, `acom:prompt`, `acom:style?`(`studio` | `lifestyle` | `flat_lay`), `acom:count?`(≤ `content.max_images_per_request`)
- 출력: 프리뷰 형식(`changes`=이미지마다 `create` `image` `{image_id, content_url, caption, style, acom:ai_generated: true, generator}`) → 실행 `{ok: true, id: "img_windbreaker_01", image_id: ["img_windbreaker_01"], undo: {by: "person", target: "cs_1044", until: "2026-11-07T23:59:00-05:00"}}`. id는 상품에서 `img_<상품>_<순번>`, 같은 요청은 같은 프리뷰(승인 때 다시 프리뷰해도 같음)
- **생성기(정함)**: 연결된 이미지 생성 모델이 없습니다(`content.generator: placeholder`). `content_url`은 상품 이름·연출·id를 적고 "Placeholder · No image generator is connected"라고 말하는 자리표시 SVG(`data:` URL)이고, 같은 입력이면 같은 그림입니다. 승인 후 실행하면 `catalog.image_object`에 초안으로 저장하고 상세 화면에는 올리지 않습니다(쓰려면 `update_product_group`의 `image`). 되돌리기는 저장한 초안을 지웁니다.
- 프리뷰: checks `content.brand_guideline`(프롬프트에 `content.prompt_blocked_words`의 단어가 있으면 실패), `content.ai_content_disclosure`(`acom:ai_generated: true` 표기). 고객에게 닿으므로 항상 `needs_approval: true`. `acom:count`가 `content.max_images_per_request`를 넘으면 `invalid_input`.
- 정책: [catalog.yml](../../policies/catalog.yml) `content`, `product.approval`
- 에러: `generation_failed` → `generate_image` · `brand_guideline_violation` → `generate_image`
- 멱등성: `Idempotency-Key = hash(product_id + prompt + style)`

### `generate_copy` — Kind: W
- 논리주소/REST: `catalog.product.generate_copy` · `POST /v1/catalog/products/{product_id}/generate-copy`
- 요약: 상품명·요약·상세 설명 초안을 생성합니다. 콘텐츠 변경이라 승인 없이 초안으로 저장하고, 반영은 `update_product_group`으로 합니다.
- consumes → produces: `product_id` → `copy_draft_id`
- 입력: `product_id`, `acom:copy_type`(`title` | `summary` | `description` | `seo`), `acom:tone?`, `acom:keywords?[]`
- 출력: 프리뷰 형식(`changes`=`create` `copy_draft`, `to`가 생성 문구 `{product_id, copy_type, text, keywords, tone, acom:ai_generated: true, generator}`) → 실행 `{ok, id: "cpd_…", copy_draft_id, text, undo}`. 되돌리기는 초안을 지움
- **생성기(정함)**: 연결된 글쓰기 모델이 없습니다(`content.generator: placeholder`). 초안은 상품 자신의 자료(이름·브랜드·소재·색·사이즈·접는 방식·보온·비)를 스토어 어조 규칙(`content.copy`: 문장 수, 관리 한 줄)의 모양으로 엮어 만들고, 같은 입력이면 같은 글입니다. `title`은 `content.copy.title_max_chars`, `seo`는 `content.copy.seo_max_chars` 안에서 단어 경계로 자릅니다. `acom:tone`은 초안에 기록만 합니다. 초안은 `catalog.copy_draft`에 남고 상세 화면에는 `update_product_group`으로 올립니다.
- 프리뷰: checks `content.ai_content_disclosure`, `product.approval`(문구는 콘텐츠라 승인 없음). `needs_approval: false`.
- 정책: [catalog.yml](../../policies/catalog.yml) `content.ai_content_disclosure`, `product.approval.content_changes`
- 에러: `generation_failed` → `generate_copy`
- 멱등성: `Idempotency-Key`

---

## offer

### `get_price_breakdown` — Kind: R
- 논리주소/REST: `catalog.offer.get_breakdown` · `GET /v1/catalog/variants/{variant_id}/price-breakdown`
- 요약: 변형의 지금 적용되는 가격을 나눠 보여줍니다(정가, 회원·기간 한정가와 그 출처). 원가와 마진은 운영자에게만.
- consumes → produces: `variant_id` → price breakdown
- 입력: `variant_id`, `valid_for_member_tier?`(운영자만, 등급 id 또는 `"everyone"`), `as_of?`(운영자만), `order_quantity?`
- 출력: `{variant_id: "var_windbreaker_m_slate", list_price: 89.00, member_price: 71.20, price_currency: "USD", applied: [{promotion_id: "promo_1042", name: "VIP weekend", valid_for_member_tier: "vip"}], cost: 36.00, price_floor: 41.40}` (단건 읽기라 목록 봉투 없음. `cost`·`price_floor`는 운영자에게만)
- 프리뷰: 해당 없음(읽기)
- 정책: [catalog.yml](../../policies/catalog.yml) `pricing.cost_visibility`, `price_floor`
- 에러: `product_not_found` → `search_products`
- 멱등성: 읽기(부작용 없음)

### `set_price` — Kind: W
- 논리주소/REST: `catalog.offer.set_price` · `PUT /v1/catalog/variants/{variant_id}/price`
- 요약: 변형의 정가·원가를 정합니다. 가격은 구조 변경이라 승인 대상이고, `price_floor` 아래 가격은 서버가 막습니다. 회원·기간 한정 판매가는 `set_sale_prices`(promotion)가 맡습니다.
- consumes → produces: `variant_id` → `offer_id`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명·예 |
  |------|----------------------|:--:|--------|
  | variant_id | Text | ✓ | 예 `"var_windbreaker_m_slate"` |
  | price | Number + price_currency | ✓ | `85.00`, `"USD"`. 센트 단위 |
  | acom:list_price | UnitPriceSpecification | | 참조 정가(`price_type: "list_price"`) |
  | acom:cost | acom:CostSpecification | | 원가(운영자 전용) |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "off_windbreaker_m_slate", undo: {by: "person", target: "cs_1044", until: "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: `changes`=`update` `offer` `id: "off_windbreaker_m_slate"` `from: {price: 89.00, price_currency: "USD"}` → `to: {price: 85.00, price_currency: "USD"}`(원가를 바꾸면 `cost`도). checks `price_floor`(원가 × 1.15 이상, 실패 시 `fix_tool: "set_price"`). `needs_approval: true`, 예 `approval_reason: "Changes 1 price"`.
- 정책: [catalog.yml](../../policies/catalog.yml) `price_floor`, `product.approval.structural_changes`; [approval.yml](../../policies/approval.yml) `thresholds.price_change_bulk`
- 에러: `below_price_floor` → `set_price`
- 멱등성: `Idempotency-Key = hash(variant_id + price)`

### `schedule_price_change` — Kind: W
- 논리주소/REST: `catalog.offer.schedule_price_change` · `POST /v1/catalog/variants/{variant_id}/price/schedule`
- 요약: 정가 변경을 미래 시점에 예약합니다. 적용 여부는 시각으로 그때그때 계산합니다.
- consumes → produces: `variant_id` → `offer_id`
- 입력: `variant_id`, `price`, `price_currency?`(오퍼의 통화), `valid_from`, `valid_through?`
- 출력: 프리뷰 형식 → 실행 `{ok, id: "off_…", unit_price_specification_id: "ups_…", undo}`
- 프리뷰: `changes`=`schedule` `unit_price_specification`(`price_type: list_price`, `id: null`). checks `price_floor`, `pricing.scheduled_change`(`valid_from`이 지금 이후). 승인 여부는 [approval.yml](../../policies/approval.yml) `thresholds.price_change_bulk`(한 번에 10개 이상)를 따릅니다. 예약은 `catalog.unit_price_specification`에 저장되고, 정가를 읽는 곳(상품 카드·`get_product`·`get_price_breakdown`·장바구니)이 그 시각에 계산해 적용합니다. `valid_through` 뒤에는 오퍼의 원래 정가.
- 정책: [catalog.yml](../../policies/catalog.yml) `pricing.scheduled_change`, `price_floor`
- 에러: `below_price_floor` → `set_price` · `valid_from_in_past` → `schedule_price_change`
- 멱등성: `Idempotency-Key`

---

## inventory

### `get_inventory` — Kind: R
- 논리주소/REST: `catalog.inventory.get` · `GET /v1/catalog/products/{product_id}/inventory`
- 요약: 상품의 변형별 가용 재고, 안전재고, 파생된 `availability`를 돌려줍니다.
- consumes → produces: `product_id` → inventory
- 입력: `product_id`, `fields?[]`
- 출력: `{product_id: "prd_chore_coat", inventory_level: 7, variants: [{variant_id: "var_chore_coat_s_olive", sku: "cc-olive-s", inventory_level: 3, acom:safety_stock: 2, acom:managed: true, availability: "in_stock", on_order: 0}, {variant_id: "var_chore_coat_m_olive", sku: "cc-olive-m", inventory_level: 0, acom:safety_stock: 2, acom:managed: true, availability: "out_of_stock", on_order: 40}], reorders: [{id: "ro_01jd2k8", order_status: "processing", total_units: 40, expected_arrival_until: "2026-11-13T23:59:00-05:00"}]}` (단건 읽기라 목록 봉투 없음)
  - `on_order`: 받지 않은 재주문([reorder](#reorder), `processing`·`in_transit`)에 든 수량. 아직 팔 수 있는 재고가 아니므로 `inventory_level`에 더하지 않습니다. `reorders[]`는 그 재주문들(도착 예정 빠른 순).
- 프리뷰: 해당 없음(읽기)
- 정책: [catalog.yml](../../policies/catalog.yml) `inventory`
- 에러: `product_not_found` → `search_products`
- 멱등성: 읽기(부작용 없음)

### `set_variant_stock` — Kind: W
- 논리주소/REST: `catalog.inventory.set_stock` · `PUT /v1/catalog/variants/{variant_id}/inventory`
- 요약: 변형의 가용 수량을 정합니다. 0이 되면 자동으로 `out_of_stock`, 자동 전환된 경우에만 회복 시 `in_stock`으로 돌아갑니다.
- consumes → produces: `variant_id` → `variant_id`
- 입력: `variant_id`, `inventory_level`(Integer), `acom:managed?`, `reason?`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` `inventory_level` `from`/`to`, 바뀌는 `availability`. 품절이던 변형이 판매 중으로 돌아오고 재입고 알림을 기다리는 쇼퍼가 있으면(`inventory.auto_restore.notify_restock_alerts`) `send` `restock_alert` `{variant_id, shoppers, by_channel, message_kind}`가 붙습니다: 실행하면 그 알림들을 보낸 것으로 표시하고 아웃박스(`restock_alert.notify`)로 messaging에 넘기며, 이 발송은 되돌리지 않습니다. `needs_approval: false`.
- 정책: [catalog.yml](../../policies/catalog.yml) `inventory.managed_at`, `inventory.auto_out_of_stock`, `inventory.auto_restore`
- 에러: `negative_stock` → `set_variant_stock` · `invalid_state` → `get_product`
- 멱등성: 절대값 설정이므로 `Idempotency-Key`로 같은 결과

### `set_safety_stock` — Kind: W
- 논리주소/REST: `catalog.inventory.set_safety_stock` · `PUT /v1/catalog/variants/{variant_id}/inventory/safety`
- 요약: 안전재고 임계치를 정합니다. 알림 전용이며 판매 상태에는 영향이 없습니다.
- consumes → produces: `variant_id` → `variant_id`
- 입력: `variant_id`, `acom:safety_stock`(Integer), `enabled?`(`false`면 0으로 두어 알림 끔)
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` `inventory_level`(`safety_stock`). `needs_approval: false`.
- 정책: [catalog.yml](../../policies/catalog.yml) `inventory.safety_stock`
- 에러: `negative_value` → `set_safety_stock`
- 멱등성: `Idempotency-Key`

---

## reorder

공급처에 재고를 더 주문하는 것(재주문). 스토어가 사는 쪽인 schema.org `Order`(`ro_`)이고, 줄은 변형별 수량과 단가(그 변형의 원가)입니다. 주문한 재고는 받을 때 팔 수 있는 재고가 되므로, 그 전까지는 `get_inventory`의 `on_order`로만 보입니다. 상태는 주문 상태 단어([enums.md](enums.md) §1.1)를 씁니다: `processing`(주문함) → `in_transit`(공급처가 보냄) → `delivered`(받음), 받기 전 취소는 `cancelled`.

### `suggest_reorder` — Kind: R
- 논리주소/REST: `catalog.reorder.suggest` · `GET /v1/catalog/products/{product_id}/reorder-suggestion`
- 요약: 상품의 변형마다 최근 판매량, 품절까지 남은 일수, 권장 재주문 수량을 돌려줍니다. "품절될 사이즈를 재주문해"에 답하는 읽기이며, `lines`는 그대로 `create_reorder`의 `lines`가 됩니다.
- consumes → produces: `product_id` → `variant_id[]`
- 입력: `product_id`(Text, `^prd_`, ✓)
- 출력: `{product_id, name, sales_window: {valid_from: "2026-10-30", valid_through: "2026-11-05", days: 7}, lead_time_days: 7, cover_days: 14, variants: [{variant_id, size, color, inventory_level: 4, acom:safety_stock: 2, on_order: 0, units_sold: 10, days_until_stockout: 2.8, order_quantity: 28}], lines: [{variant_id, order_quantity: 28}], total_units: 28}` (단건 읽기라 목록 봉투 없음)
  - `units_sold`: 판매 창(`sales_window`, 오늘을 뺀 최근 스토어 날짜들) 동안 주문된 그 변형의 수량(취소 줄 제외). 창 길이는 품절 예측 감시와 같은 [platform.yml](../../policies/platform.yml) `watchers.stock_out_forecast.lookback_days`라서, 그 감시가 연 할 일의 "about N days"와 숫자가 맞습니다. sales가 연결되지 않은 스택에서는 `sales_window: null`, `units_sold: 0`.
  - `days_until_stockout`: 가용 재고 ÷ 하루 판매량(소수 한 자리). 팔린 게 없으면 `null`.
  - `order_quantity`: 리드타임과 커버 일수 동안 팔릴 수량(`units_sold × (lead_time_days + cover_days) ÷ days`, 올림) + 안전재고 − 가용 재고 − `on_order`. 0 아래는 0, `reorder.max_units_per_line`에서 자릅니다. 재고를 추적하지 않거나(`acom:managed: false`) `reorder.orderable_availability`에 없는 변형(단종)은 0. 판매 창에 팔린 게 없는 변형은 안전재고까지만 채웁니다.
  - `lines`: `order_quantity`가 1 이상인 변형만. 비어 있으면 지금 재주문할 변형이 없습니다.
- 프리뷰: 해당 없음(읽기)
- 정책: [catalog.yml](../../policies/catalog.yml) `reorder.lead_time_days`, `reorder.cover_days`, `reorder.max_units_per_line`, `reorder.orderable_availability`, [platform.yml](../../policies/platform.yml) `watchers.stock_out_forecast.lookback_days`
- 에러: `product_not_found` → `search_products`
- 멱등성: 읽기(부작용 없음)

### `create_reorder` — Kind: W
- 논리주소/REST: `catalog.reorder.create` · `POST /v1/catalog/reorders`
- 요약: 한 상품의 변형별 수량을 공급처에 재주문합니다. 단가는 각 변형의 원가, 도착 예정은 오늘(스토어 날짜) + 정책의 리드타임입니다. 실행하면 재주문이 `processing`으로 남고, 재고는 받을 때 늘어납니다.
- consumes → produces: `product_id`, `variant_id[]` → `reorder_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | product_id | Text, `^prd_` | ✓ | 재주문할 상품 | `"prd_linen_shirt"` |
  | lines | Object[] (1–50) | ✓ | 변형별 수량 `{variant_id ✓, order_quantity ✓ ≥1}`. 같은 변형은 한 번만 | `[{"variant_id": "var_linen_shirt_s_white", "order_quantity": 70}]` |
  | description | Text (≤500) | | 공급처에 남길 말이나 이유 | `"Sells out in about 4 days at the current rate."` |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "ro_01jd2k8", undo: {by: "person", target: "cs_1041", until: "…"}}`
- 프리뷰: `changes`=`create` `reorder`(`id: null`, `to: {product_id, name, total_units, total_payment_due: {value, currency}, expected_arrival_until}`)와 줄마다 `create` `reorder_item`(`to: {variant_id, size, color, order_quantity, price, price_currency}`). checks `reorder.max_units_per_line`(줄마다 상한), `reorder.variant_orderable`(`reorder.orderable_availability`에 없는 변형, 예 단종이 있으면 실패). 원가 없는 변형은 단가를 정할 수 없어 프리뷰가 `invalid_state`(→ `set_price`)로 거절합니다. 원가 합계가 [approval.yml](../../policies/approval.yml) `thresholds.reorder_spend`를 넘으면 `needs_approval: true`, 예 `approval_reason: "Reorder cost over $1000.00"`.
  - 예: Linen shirt(White) S 70 · M 90 · L 80 = 240 units, 단가 $26.00, 합계 $6,240.00, Fri Nov 6에 주문하면 Fri Nov 13 도착 예정(B 시나리오의 재주문 할 일, #1041).
- 정책: [catalog.yml](../../policies/catalog.yml) `reorder.lead_time_days`, `reorder.max_units_per_line`, `reorder.orderable_availability`, [approval.yml](../../policies/approval.yml) `thresholds.reorder_spend`
- 에러: `product_not_found`(상품이나 변형이 없음) → `search_products` · `invalid_input`(그 상품의 변형이 아님 `detail.reason: other_product`, 같은 변형 두 번 `duplicate`) → `get_inventory` · `invalid_state`(원가 없는 변형) → `set_price`
- 되돌리기: 받기 전(`processing`)이면 재주문을 `cancelled`로 바꿉니다. 공급처가 보냈거나 받은 재주문은 되돌리지 않습니다(`invalid_state`).
- 멱등성: 프리뷰 → 실행(`preview_id`)

### `receive_reorder` — Kind: W
- 논리주소/REST: `catalog.reorder.receive` · `POST /v1/catalog/reorders/{reorder_id}/receive`
- 요약: 공급처에서 온 재주문을 받습니다. 주문한 수량이 각 변형의 팔 수 있는 재고에 더해지고(품절로 자동 전환됐던 변형은 판매 중으로 돌아옴), 재주문은 `delivered`가 됩니다. 실제로 온 수량이 다르면 받은 뒤 `set_variant_stock`으로 고칩니다.
- consumes → produces: `reorder_id` → `variant_id[]`
- 입력: `reorder_id`(Text, `^ro_`, ✓)
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "ro_01jd2k8", undo: {by: "person", target: "cs_…", until: "…"}}`
- 프리뷰: `changes`=`update` `reorder` `order_status` `processing` → `delivered`, 그리고 줄마다 `update` `inventory_level`(`from`/`to`, 바뀌면 `availability`), 판매 중으로 돌아오는 변형에 재입고 알림 신청자가 있으면 `send` `restock_alert`(`set_variant_stock`과 같음). checks는 판매 상태가 바뀌는 변형마다 `inventory.auto_restore`. `needs_approval: false`.
- 정책: [catalog.yml](../../policies/catalog.yml) `inventory.auto_restore`
- 에러: `not_found`(`detail.entity: reorder`) → `get_inventory` · `invalid_state`(이미 받았거나 취소된 재주문) → `get_inventory`
- 되돌리기: 그 뒤로 재고가 바뀌지 않았으면 재고를 받기 전으로, 재주문을 받기 전 상태로 돌립니다. 그새 팔렸거나 고쳐졌으면 `invalid_state`(→ `set_variant_stock`).
- 멱등성: 프리뷰 → 실행(`preview_id`)

---

## approval

### `submit_for_review` — Kind: W
- 논리주소/REST: `catalog.approval.submit` · `POST /v1/catalog/products/{product_id}/submit`
- 요약: 상품을 심사 대기로 보냅니다(`draft` | `rejected` → `pending_review`).
- consumes → produces: `product_id` → `product_id`
- 입력: `product_id`, `note?`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` `product` `approval_status` `draft` → `pending_review`. checks `product.publish_requires`(공개에 필요한 것이 다 있는지: 변형·정가·원가·열린 필드 없음·설명; 실패 시 `fix_tool`이 채울 툴), `price_floor`. 체크가 실패하면 실행되지 않으므로 덜 된 초안은 심사로 가지 않습니다. `needs_approval: false`.
- 정책: [catalog.yml](../../policies/catalog.yml) `product.approval`, `product.publish_requires`
- 에러: `invalid_state` → `get_product` · `nothing_to_review`(변형이 없음) → `set_variants`
- 멱등성: `Idempotency-Key`

### `approve_product` — Kind: W$
- 논리주소/REST: `catalog.approval.approve` · `POST /v1/catalog/products/{product_id}/approve`
- 요약: 심사 중인 상품을 공개합니다(`pending_review` → `active`). 고객에게 닿으므로 항상 변경셋 승인을 거칩니다. 승인 후 실행기가 `preview_id`로 실행합니다.
- consumes → produces: `product_id` → `product_id`
- 입력: `product_id`, `valid_from?`(공개일. 상품의 `release_date`가 되어 "newest" 정렬에 쓰임, 없으면 오늘. 쇼퍼에게는 변경셋이 실행되는 즉시 보임), `note?`
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "prd_windbreaker", undo: {by: "person", target: "cs_1044", until: "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: `changes`=`update` `product` `approval_status` `pending_review` → `active`와 보류된 가격·변형. checks `price_floor`(공개 직전 재검사), `product.required_on_create`, `product.publish_requires`(변형·정가·원가가 다 있는지), `gift_card.disallow`. 항상 `needs_approval: true`, 예 `approval_reason: "Publishes 1 product"`.
- 정책: [catalog.yml](../../policies/catalog.yml) `product.approval`, `product.publish_requires`, `price_floor`; [approval.yml](../../policies/approval.yml) `thresholds.product_approve`
- 에러: `invalid_state` → `get_change_history` · `below_price_floor` → `set_price`
- 멱등성: `Idempotency-Key`. 다시 보내도 한 번만 공개

### `reject_product` — Kind: W
- 논리주소/REST: `catalog.approval.reject` · `POST /v1/catalog/products/{product_id}/reject`
- 요약: 심사를 반려합니다(`pending_review` → `rejected`, 이후 `draft`로 다시 편집).
- consumes → produces: `product_id` → `product_id`
- 입력: `product_id`, `reason`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` `product` `approval_status`. `needs_approval: false`.
- 정책: [catalog.yml](../../policies/catalog.yml) `product.approval`
- 에러: `invalid_state` → `get_change_history`
- 멱등성: `Idempotency-Key`

### `get_change_history` — Kind: R
- 논리주소/REST: `catalog.approval.get_history` · `GET /v1/catalog/products/{product_id}/history`
- 요약: 상품의 변경 이력(추가만 되는 기록)을 돌려줍니다. 같은 기록은 액션 로그(`search_action_log`)에도 있습니다.
- consumes → produces: `product_id` → history
- 입력: `product_id`, `limit?`(기본 20), `cursor?`
- 출력: `{items: [{action_id: "act_7710", agent, change_type: "content" | "structural", from, to, end_time: "2026-11-02T14:10:00-05:00"}], total, next_cursor?}`
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
- 입력: `name?`, `limit?`(기본 20), `cursor?`
- 출력: `{items: Brand[], total, next_cursor?}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_filter` → `list_brands`
- 멱등성: 읽기(부작용 없음)

### `get_brand` — Kind: R
- 논리주소/REST: `catalog.brand.get` · `GET /v1/catalog/brands/{brand_id}`
- 요약: 브랜드 하나(`name`, `logo`).
- consumes → produces: `brand_id` → brand
- 입력: `brand_id`
- 출력: `{id: "brn_northwind", name: "Northwind", logo: "https://cdn.northwind.example/logo.svg"}` (단건 읽기라 목록 봉투 없음)
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `brand_not_found` → `list_brands`
- 멱등성: 읽기(부작용 없음)

### `create_brand` — Kind: W
- 논리주소/REST: `catalog.brand.create` · `POST /v1/catalog/brands`
- 요약: 브랜드를 만듭니다.
- consumes → produces: → `brand_id`
- 입력: `name`, `logo?`(ImageObject)
- 출력: 프리뷰 형식 → 실행 `{ok, id: "brn_northwind", undo: {by: "person", target: "cs_1044", until: "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: `changes`=`create` `brand`(`id: null`). `needs_approval: false`
- 정책: 없음
- 에러: `duplicate_brand` → `list_brands`
- 멱등성: `Idempotency-Key`

### `update_brand` — Kind: W
- 논리주소/REST: `catalog.brand.update` · `PATCH /v1/catalog/brands/{brand_id}`
- 요약: 브랜드 이름·로고를 고칩니다.
- consumes → produces: `brand_id` → `brand_id`
- 입력: `brand_id`, `name?`, `logo?`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` `brand`. `needs_approval: false`
- 정책: 없음
- 에러: `brand_not_found` → `list_brands`
- 멱등성: `Idempotency-Key`

---

## review

### `get_review_summary` — Kind: R  (데모 장면)
- 논리주소/REST: `catalog.review.summarize` · `GET /v1/catalog/products/{product_id}/reviews/summary?height_min_cm=175&height_max_cm=182`
- 요약: 리뷰를 요약해 핏 투표와 반복되는 표현, 근거가 되는 리뷰 몇 개를 돌려줍니다. 키 범위를 주면 그 범위의 리뷰어만 셉니다. 숨긴 리뷰는 세지 않습니다. 화면은 `fit_check` 블록의 `reviews`([chat.md](chat.md) §3.3)입니다.
- consumes → produces: `product_id` → review summary
- 입력 (닫힌 객체):
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | product_id | Text `^prd_[a-z0-9_]+$` | ✓ | 상품 id | `"prd_wool_overcoat"` |
  | height_min_cm | Integer 100–230 | | 리뷰어 키 하한(포함). `height_max_cm`와 함께 | `175` |
  | height_max_cm | Integer 100–230 | | 리뷰어 키 상한(포함) | `182` |
  | size | Text | | 리뷰어가 산 사이즈만 | `"M"` |
  | sample_limit | Integer 0–5 | | 함께 돌려줄 리뷰 수. 기본 3 | `3` |
- 출력: 단건 읽기라 목록 봉투 없이
```jsonc
{"product_id": "prd_wool_overcoat",
 "filter": {"height_min_cm": 175, "height_max_cm": 182, "size": null},
 "review_count": 41,
 "rating_value": 4.6,
 "fit_votes": [
   {"fit": "true_to_size", "label": "True to size", "count": 29},
   {"fit": "runs_small", "label": "A bit snug", "count": 12},
   {"fit": "runs_large", "label": "Runs large", "count": 0}],
 "mentions": [
   {"text": "size up for a hoodie", "count": 9}],
 "sample": [
   {"review_id": "rev_8812", "review_rating": 5, "size_bought": "L", "height_cm": 180,
    "excerpt": "Size up if you wear a hoodie underneath.", "date_published": "2026-10-12"}]}
```
  - `fit_votes[].fit`는 enum `true_to_size` \| `runs_small` \| `runs_large`이고 세 값이 항상 다 나옵니다(0건 포함). `label`은 리뷰에서 가장 많이 쓴 표현입니다.
  - `mentions[]`는 리뷰에 반복해서 나온 표현과 횟수, 많은 순입니다.
- 프리뷰: 해당 없음(읽기)
- 정책: [catalog.yml](../../policies/catalog.yml) `review.moderation`(숨긴 리뷰는 제외)
- 에러: `not_found`(`detail.entity: product`) → `search_products` · `no_reviews`(조건에 맞는 리뷰 0건) → `get_product_measurements` · `invalid_input`(`height_min_cm` > `height_max_cm`, 한쪽만 줌) → `get_review_summary`
- 멱등성: 읽기(부작용 없음)

### `list_reviews` — Kind: R
- 논리주소/REST: `catalog.review.list` · `GET /v1/catalog/reviews`
- 요약: 리뷰 목록(`review_rating`, `author`, `review_body`).
- consumes → produces: `product_id?` → `review_id[]`
- 입력: `product_id?`, `rating?`, `status?`(`visible` | `hidden` | `reported`), `sort?`(`newest` | `rating`), `limit?`(기본 20), `cursor?`
- 출력: `{items: Review[], total, next_cursor?}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_filter` → `list_reviews`
- 멱등성: 읽기(부작용 없음)

### `moderate_review` — Kind: W
- 논리주소/REST: `catalog.review.moderate` · `POST /v1/catalog/reviews/{review_id}/moderate`
- 요약: 리뷰를 숨기거나 되살립니다.
- consumes → produces: `review_id` → `review_id`
- 입력: `review_id`, `action`(`hide` | `restore`), `reason?`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` `review` `status`(`visible` ↔ `hidden`). `needs_approval: false`.
- 정책: [catalog.yml](../../policies/catalog.yml) `review.moderation`
- 에러: `invalid_state` → `list_reviews`
- 멱등성: `Idempotency-Key`

### `create_review` — Kind: W
- 논리주소/REST: `catalog.review.create` · `POST /v1/catalog/reviews`
- 요약: 쇼퍼가 구매한 상품에 리뷰(별점·본문·사진)를 씁니다. 구매 확인 후에만 가능하며, 바로 보이고 사후에 `moderate_review`로 심사합니다.
- consumes → produces: `product_id`, `order_id` → `review_id`
- 입력: `product_id`, `order_id`, `review_rating`(1–5), `review_body`, `image?[]`, `fit?`(`true_to_size` | `runs_small` | `runs_large`), `height_cm?`. 작성자는 로그인한 쇼퍼
- 출력: 프리뷰 형식 → 실행 `{ok, id: "rev_8812", undo: null}` — 쇼퍼의 쓰기라 변경셋 밖이고, 지우기는 `delete_review`
- 프리뷰: `changes`=`create` `review`(`id: null`). checks `review.verified_purchase_only`, `review.one_per_order_item`, `review.profanity_filter`. `needs_approval: false`. 리뷰에 남는 작성자 이름은 `author_masking`대로 가려 저장합니다(`Jamie` → `Ja***`, `to.author`).
- 정책: [catalog.yml](../../policies/catalog.yml) `review`, `author_masking`
- 에러: `not_purchased` → `get_my_orders` · `already_reviewed` → `update_review` · `profanity_blocked` → `create_review`
- 멱등성: `(order_id, product_id, 작성자)`로 한 건

### `update_review` — Kind: W
- 논리주소/REST: `catalog.review.update` · `PATCH /v1/catalog/reviews/{review_id}`
- 요약: 쇼퍼가 자기 리뷰를 고칩니다.
- consumes → produces: `review_id` → `review_id`
- 입력: `review_id`, `review_rating?`, `review_body?`, `image?[]`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo: null}`
- 프리뷰: `changes`=`update` `review`(바뀌는 필드만 `from`/`to`; 주지 않은 필드는 그대로, `image: []`은 사진을 모두 뺌). checks `review.profanity_filter`(`hold_for_review`면 보이던 리뷰가 `reported`로). `needs_approval: false`
- 정책: 작성자 본인만(가져온 리뷰처럼 작성자가 없는 리뷰는 아무도 못 고침). [catalog.yml](../../policies/catalog.yml) `review.profanity_filter`
- 에러: `not_author` → `list_reviews` · `review_not_found` → `list_reviews` · `profanity_blocked` → `update_review`
- 멱등성: `Idempotency-Key`

### `delete_review` — Kind: W
- 논리주소/REST: `catalog.review.delete` · `DELETE /v1/catalog/reviews/{review_id}`
- 요약: 쇼퍼가 자기 리뷰를 지웁니다.
- consumes → produces: `review_id` →
- 입력: `review_id`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo: null}`
- 프리뷰: `changes`=`delete` `review`(`from`에 지울 리뷰). 리뷰의 "도움이 됐어요" 투표와 신고도 함께 지워지고, 리뷰 요약에서 빠집니다. `needs_approval: false`.
- 정책: 작성자 본인만
- 에러: `not_author` → `list_reviews` · `review_not_found` → `list_reviews`
- 멱등성: 같은 프리뷰를 다시 실행하면 같은 결과(이미 지운 리뷰를 지우는 것은 아무것도 바꾸지 않음). 지운 뒤의 새 프리뷰는 `review_not_found`

### `vote_review_helpful` — Kind: W
- 논리주소/REST: `catalog.review.vote_helpful` · `POST /v1/catalog/reviews/{review_id}/helpful`
- 요약: "도움이 됐어요" 투표를 켜거나 끕니다.
- consumes → produces: `review_id` → `review_id`
- 입력: `review_id`, `helpful`(Boolean)
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo: null}`
- 프리뷰: `changes`=`update` `review`(`helpful_count`). `needs_approval: false`
- 정책: 없음
- 에러: `review_not_found` → `list_reviews`
- 멱등성: `(review_id, 투표자)`로 상태 하나

### `report_review` — Kind: W
- 논리주소/REST: `catalog.review.report` · `POST /v1/catalog/reviews/{review_id}/report`
- 요약: 부적절한 리뷰를 신고해 심사 대기열에 넣습니다. 후속은 `moderate_review`.
- consumes → produces: `review_id` → `review_id`
- 입력: `review_id`, `reason`(`acom:report_reason`: `spam` | `abuse` | `obscene` | `advertisement` | `fake` | `irrelevant` | `other`), `note?`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo: null}`
- 프리뷰: `changes`=`create` `report`(`id: null`, `rpt_`)와, 보이던 리뷰면 `update` `review` `status` `visible` → `reported`(`review.report_holds_review`: 심사 대기열. 쇼퍼에게는 계속 보이고 `moderate_review`가 숨기거나 되살림). checks `review.report_reasons`. 숨긴 리뷰는 신고할 수 없음(`review_not_found`). `needs_approval: false`
- 정책: [catalog.yml](../../policies/catalog.yml) `review.report_reasons`, `review.report_holds_review`
- 에러: `review_not_found` → `list_reviews` · `already_reported` → `list_reviews`
- 멱등성: `(review_id, 신고자)`로 한 건

---

## question

### `list_questions` — Kind: R
- 논리주소/REST: `catalog.question.list` · `GET /v1/catalog/questions`
- 요약: 상품 문의 목록(`text`, `accepted_answer`).
- consumes → produces: `product_id?` → `question_id[]`
- 입력: `product_id?`, `answered?`(Boolean), `status?`(`visible` | `hidden`), `limit?`(기본 20), `cursor?`
- 출력: `{items: Question[], total, next_cursor?}`, 최신 순. 항목 `{id: "qst_311", @type: "Question", product_id, product_name, text, author: {name: "Pr***"}, acom:private, status, date_created, answer_count, accepted_answer: {id: "ans_311", @type: "Answer", text, author: {@type: "Organization", name: "The store"}, date_created} | null}`
  - 쇼퍼는 보이는(`visible`) 문의만, 비공개 문의는 자기가 남긴 것만 봅니다. 스토어(운영자·운영 에이전트)는 숨긴 문의와 모든 비공개 문의도 봅니다.
- 프리뷰: 해당 없음(읽기)
- 정책: [catalog.yml](../../policies/catalog.yml) `question.private_allowed`
- 에러: `invalid_filter`(쇼퍼가 `status: hidden`) → `list_questions`
- 멱등성: 읽기(부작용 없음)

### `answer_question` — Kind: W
- 논리주소/REST: `catalog.question.answer` · `POST /v1/catalog/questions/{question_id}/answer`
- 요약: 문의에 답(`Answer`, `ans_`)을 답니다.
- consumes → produces: `question_id` → `answer_id`
- 입력: `question_id`, `text`
- 출력: 프리뷰 형식 → 실행 `{ok, id: "ans_311", answer_id, question_id, undo: null}` — 쇼퍼가 읽은 답은 되돌리지 않습니다. 문의 하나에 답 하나이므로, 잘못된 답은 `moderate_question`으로 문의째 숨깁니다
- 누가 답하나(정함): `question.answered_by`. 기본값 `[operator]`는 스토어(운영자·운영 에이전트·시스템)만이고, 스토어가 `shopper`를 더하면 문의한 사람이 아닌 로그인 쇼퍼가 보이는 공개 문의에 답할 수 있습니다. 스토어의 답은 작성자가 스토어로, 쇼퍼의 답은 `author_masking`대로 가린 이름으로 보입니다.
- 프리뷰: `changes`=`create` `answer`(`id: null`, `to`에 쇼퍼가 읽을 문장 그대로와 문의 원문). checks `question.answered_by`, `question.profanity_filter`(답은 보류할 곳이 없어 막는 단어가 있으면 `hold_for_review`여도 실패). [approval.yml](../../policies/approval.yml)에 이 툴의 임계가 없으므로 기본 `needs_approval: false`.
- 정책: [catalog.yml](../../policies/catalog.yml) `question`; [approval.yml](../../policies/approval.yml)
- 에러: `already_answered` → `list_questions` · `forbidden`(`question.answered_by`가 허락하지 않음) → null · `not_found`(`detail.entity: question`) → `list_questions` · `profanity_blocked` → `answer_question`
- 멱등성: `Idempotency-Key`

### `moderate_question` — Kind: W
- 논리주소/REST: `catalog.question.moderate` · `POST /v1/catalog/questions/{question_id}/moderate`
- 요약: 문의·답을 숨기거나 되살립니다.
- consumes → produces: `question_id` → `question_id`
- 입력: `question_id`, `action`(`hide` | `restore`), `reason?`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` `question` `status`(`visible` ↔ `hidden`; 답이 있으면 함께 숨거나 보임). 스토어만 부를 수 있습니다(쇼퍼는 `forbidden`). `needs_approval: false`. 되돌리기는 이전 상태로.
- 정책: 없음
- 에러: `invalid_state` → `list_questions` · `not_found`(`detail.entity: question`) → `list_questions`
- 멱등성: `Idempotency-Key`

### `ask_question` — Kind: W
- 논리주소/REST: `catalog.question.ask` · `POST /v1/catalog/questions`
- 요약: 쇼퍼가 상품 문의를 남깁니다. 비공개로 지정할 수 있습니다.
- consumes → produces: `product_id` → `question_id`
- 입력: `product_id`, `text`, `acom:private?`(Boolean)
- 출력: 프리뷰 형식 → 실행 `{ok, id: "qst_311", question_id, undo: null}`
- 프리뷰: `changes`=`create` `question`(`id: null`, `to.author`는 `author_masking`대로 가린 이름). checks `question.profanity_filter`(`block`이면 실패, `hold_for_review`면 통과하되 `hidden`으로 저장해 스토어가 `moderate_question`으로 공개), `question.private_allowed`(비공개를 받지 않는 스토어면 실패). `needs_approval: false`.
- 정책: [catalog.yml](../../policies/catalog.yml) `question`, `author_masking`
- 에러: `profanity_blocked` → `ask_question` · `product_not_found` → `search_products`
- 멱등성: `Idempotency-Key`

### `update_question` — Kind: W
- 논리주소/REST: `catalog.question.update` · `PATCH /v1/catalog/questions/{question_id}`
- 요약: 쇼퍼가 답이 달리기 전의 자기 문의를 고칩니다.
- consumes → produces: `question_id` → `question_id`
- 입력: `question_id`, `text?`, `acom:private?`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo: null}`
- 프리뷰: `changes`=`update` `question`(바뀌는 필드만). checks `question.edit_until`, `question.profanity_filter`, `question.private_allowed`. `needs_approval: false`
- 정책: 작성자 본인만. [catalog.yml](../../policies/catalog.yml) `question.edit_until`
- 에러: `not_author` → `list_questions` · `already_answered` → `list_questions` · `profanity_blocked` → `update_question` · `not_found`(`detail.entity: question`) → `list_questions`
- 멱등성: `Idempotency-Key`

### `delete_question` — Kind: W
- 논리주소/REST: `catalog.question.delete` · `DELETE /v1/catalog/questions/{question_id}`
- 요약: 쇼퍼가 자기 문의를 지웁니다.
- consumes → produces: `question_id` →
- 입력: `question_id`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo: null}`
- 프리뷰: `changes`=`delete` `question`, 답이 있으면 `delete` `answer`(답도 함께 지워짐). `needs_approval: false`
- 정책: 작성자 본인만
- 에러: `not_author` → `list_questions` · `not_found`(`detail.entity: question`) → `list_questions`
- 멱등성: 같은 프리뷰를 다시 실행하면 같은 결과(이미 지운 문의를 지우는 것은 아무것도 바꾸지 않음)

---

## bundle

### `create_bundle` — Kind: W
- 논리주소/REST: `catalog.bundle.create` · `POST /v1/catalog/bundles`
- 요약: 묶음 상품을 만듭니다. 가격·재고는 구성품에서 파생하고, 구성품은 같은 판매자·같은 연령 제한(`acom:min_age`)이어야 합니다.
- consumes → produces: `variant_id[]` → `product_id`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명·예 |
  |------|----------------------|:--:|--------|
  | name | Text | ✓ | "Denim bundle" |
  | is_related_to | Product[] | ✓ | 구성품 `{variant_id, order_quantity}` |
  | category_id | Text | ✓ | 표준 카테고리 |
  | acom:bundle_discount | UnitPriceSpecification | | 묶음 할인 `{percent_off}`(0–100, 구성품 정가 합에서 뺌, 상한 `bundle.max_discount`) |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "prd_denim_bundle", product_id, variant_id: "var_denim_bundle", undo: {by: "person", target: "cs_1044", until: "2026-11-07T23:59:00-05:00"}}`
- 파생(정함): 정가 = 구성품 정가 × 수량의 합 − 묶음 할인(센트 반올림), 원가 = 구성품 원가 × 수량의 합(원가 없는 구성품이 있으면 없음), 재고 = 재고를 관리하는 구성품마다 `재고 ÷ 수량`(버림)의 최솟값. 묶음은 상품(`prd_`, 이름에서 id) 하나와 변형(`var_`, SKU는 상품 id에서) 하나, 그 오퍼로 만들고, 구성품은 `catalog.type_and_quantity_node`에 남깁니다. 재고는 묶음을 만들 때의 값으로 정하고, 묶음이 팔려도 구성품 재고는 아직 줄이지 않습니다.
- 프리뷰: `changes`=`create` `product`(`id: null`, 구성품·정가 합·할인·가격·원가·하한·재고 포함). checks `bundle.same_seller`(판매자는 모두 스토어), `bundle.same_age_restriction`(`acom:min_age`는 아직 저장하지 않아 모두 같음), `price_floor`(파생 가격 ≥ 파생 원가 × 1.15), `bundle.components_must_be_active`(구성품 상품이 `active`이고 오퍼가 단종이 아님). 구성품이 `bundle.min_components`보다 적거나 같은 변형이 두 번이면 `invalid_input`. 가격이 정해지므로 `needs_approval: true`([approval.yml](../../policies/approval.yml) `thresholds.product_approve`, `new_product: true`)이고, 승인되어 실행되면 바로 판매 중(`active`, 공개일 오늘)입니다. 되돌리기는 지우지 않고 판매를 멈춥니다(상품 `discontinued`, 오퍼 단종): 그새 팔렸을 수 있기 때문입니다.
- 정책: [catalog.yml](../../policies/catalog.yml) `bundle`, `price_floor`
- 에러: `mixed_seller` → `create_bundle` · `mixed_age_restriction` → `create_bundle` · `below_price_floor` → `set_price` · `component_inactive` → `get_product` · `product_not_found`(구성품 변형이 없음) → `get_product` · `invalid_category` → `list_categories`
- 멱등성: `Idempotency-Key = hash(components + name)`

---

## restock_alert

### `subscribe_restock` — Kind: W
- 논리주소/REST: `catalog.restock_alert.subscribe` · `POST /v1/catalog/products/{product_id}/restock-alerts`
- 요약: 품절된 변형의 재입고 알림을 신청합니다. 재고가 돌아오면 거래성 알림으로 보냅니다(마케팅 아님).
- consumes → produces: `product_id` → `restock_alert_id`
- 입력: `product_id`, `mode`(`product_options`: `size`·`color`로 변형을 찾음 | `variant`: `variant_id`를 그대로), `size?`, `color?`, `variant_id?`, `channel`(`email` | `sms` | `push` | `in_app`). 신청자는 로그인한 쇼퍼
- 출력: 프리뷰 형식 → 실행 `{ok, id: "rsa_204", variant_id: "var_chore_coat_m_olive", undo: null}`
- 프리뷰: `changes`=`create` `restock_alert`(`id: null`, `to: {product_id, variant_id, size, color, channel, availability, expires}`). checks `restock_alert.channels`(스토어가 그 채널로 보내는지), `inventory.auto_restore`(`notify_restock_alerts`: 스토어가 재입고 때 알리는지). 신청은 `restock_alert.subscribable_availability`(기본 `out_of_stock`, `back_order`)의 변형만이고, `restock_alert.expires_after_days` 뒤 만료됩니다. `needs_approval: false`
- 보내는 때(정함): 재고 변경(`set_variant_stock`, `receive_reorder`, 반품 재입고)으로 변형이 판매 중으로 돌아오면 그 트랜잭션에서 기다리던 알림을 보낸 것으로 표시하고(`notified_at`) 아웃박스 `restock_alert.notify`(`{restock_alert_id, customer_id, product_id, variant_id, size, color, channel, message_kind}`)로 messaging에 넘깁니다. 실제 발송은 messaging의 일입니다.
- 정책: [catalog.yml](../../policies/catalog.yml) `restock_alert`, `inventory.auto_restore`
- 에러: `already_in_stock` → `add_to_cart` · `already_subscribed` → `list_restock_subscriptions` · `invalid_state`(단종된 변형) → `search_products` · `invalid_input`(그 상품에 없는 사이즈·색, 여럿이 맞음; `detail.allowed`에 변형 목록) → `get_product`
- 멱등성: `(variant_id, 신청자, channel)`로 한 건

### `list_restock_subscriptions` — Kind: R
- 논리주소/REST: `catalog.restock_alert.list` · `GET /v1/catalog/restock-alerts`
- 요약: 로그인한 쇼퍼의 재입고 알림 신청 목록.
- consumes → produces: → `restock_alert_id[]`
- 입력: `limit?`(기본 20), `cursor?`
- 출력: `{items: [{id: "rsa_204", product_id: "prd_chore_coat", product_name: "Cotton chore coat", variant_id: "var_chore_coat_m_olive", size: "M", color: "Olive", channel: "email", availability: "out_of_stock", start_time: "2026-11-05T09:12:00-05:00", expires: "2027-02-03T09:12:00-05:00", notified_at: null}], total: 1}`. 최신 순이고, 알림이 나간 것(`notified_at`)과 만료된 것도 보입니다. 로그인하지 않으면 `forbidden`
- 프리뷰: 해당 없음(읽기)
- 정책: 본인 것만
- 에러: `invalid_filter` → `list_restock_subscriptions`
- 멱등성: 읽기(부작용 없음)
