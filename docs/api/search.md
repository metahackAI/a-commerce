# API 명세 — `search` (검색·비교·자동완성·패싯·사전·색인)

> 계층: `search.{resource}.{operation}`. 공통 형식(읽기 봉투, 쓰기의 프리뷰 → `preview_id` 실행, 시간·금액·id)은 [conventions.md](conventions.md), 에러 코드는 [errors.md](errors.md), 상태 값은 [enums.md](enums.md), 데이터 타입은 [data-model.md](data-model.md), 정책은 [search.yml](../../policies/search.yml). 예시 값은 데모 스토어([../scenarios.md](../scenarios.md))입니다.
>
> **설계 축**: ① `search_products`는 운영자와 쇼퍼가 함께 쓰는 첫 툴로, 후속 툴의 입력 `product_id[]`를 만듭니다(툴 그래프의 맨 앞, [README.md](README.md) §5). ② Postgres 전문 검색으로 순위를 매기고 사전의 동의어로 질의를 넓힘. 검색 문서가 아직 없으면 카탈로그 글에서 같은 방식으로 찾음. 임베딩 모델(바깥 모델)은 연결하지 않아 벡터는 쓰지 않음(`reindex`의 `embeddings`도 그렇게 답함). ③ 스토어 격리와 등급·재고 필터는 질의 안에서 적용(결과를 받은 뒤 거르지 않음). ④ 동의어·신조어 변경은 재색인 후 반영(재색인은 트랜잭션 하나라 커밋 순간 새 문서로 바뀌어 무중단, [index](#index)).
>
> **상품과 변형**: 사람과 모델은 상품(`prd_`, schema.org `ProductGroup`)으로 이야기하고, 재고·장바구니는 변형(`var_`, `Product`)을 씁니다([conventions.md](conventions.md) §9). 검색 결과는 상품 단위이며 변형은 `has_variant[]`에 담깁니다.

리소스: [product](#product) · [lexicon](#lexicon) · [index](#index)

---

## product

### `search_products` — Kind: R  (데모 장면, 운영자·쇼퍼 공용)
- 논리주소/REST: `search.product.search` · `GET /v1/search/products`
- 요약: 질의·카테고리·재고·가격 조건으로 상품을 찾습니다. 상품마다 정가(`price`), 부르는 사람에게 해당하는 회원가(`member_price`), 재고(`inventory_level`), 변형(`has_variant[]`)을 붙입니다. 운영자는 `valid_for_member_tier`·`as_of`로 "그 등급이 그 시각에 보는 가격"을 볼 수 있고 원가(`cost`)도 받습니다. 쇼퍼가 부르면 가격은 그 쇼퍼의 등급과 지금 시각 기준이고 `cost`는 응답에 없습니다.
- consumes → produces: `category_id`? → `product_id[]`
- 입력 (닫힌 객체, 모두 선택):
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | query | Text | | 자연어 질의. 비우면 필터만으로 찾음 | `"light jacket for spring trips"` |
  | category_id | Text `^cat_[a-z0-9_]+$` | | 표준 또는 전시 카테고리(`list_categories`). 하위 카테고리 포함 | `"cat_summer_outerwear"` |
  | tags | Text[] | | 상품 태그. 모두 가진 상품만 | `["packable"]` |
  | availability | enum[] | | `in_stock`: 재고 있음; `out_of_stock`: 품절; `pre_order`: 예약 판매; `back_order`: 입고 대기 판매; `discontinued`: 단종. 기본: 운영자 전체, 쇼퍼 `search.yml query.shopper_default_availability` | `["in_stock"]` |
  | min_inventory_level | Integer ≥ 0 | | **운영자만.** 변형 합계 재고가 이 수 이상인 상품 | `10` |
  | max_inventory_level | Integer ≥ 0 | | **운영자만.** 변형 합계 재고가 이 수 이하인 상품(재고 부족 찾기) | `9` |
  | min_price | MonetaryAmount | | 정가 하한 `{value, currency}` | `{"value": 50.00, "currency": "USD"}` |
  | max_price | MonetaryAmount | | 정가 상한 | `{"value": 150.00, "currency": "USD"}` |
  | valid_for_member_tier | Text | | **운영자만.** 회원가를 계산할 등급 id(`list_tiers`) 또는 `"everyone"`. 기본 `"everyone"` | `"vip"` |
  | as_of | DateTime | | **운영자만.** 회원가를 계산할 시각. 기본 지금(스토어 시계) | `"2026-11-07T00:00:00-05:00"` |
  | sort | enum | | `relevance`(기본, 질의가 있을 때) \| `price` \| `-price` \| `newest` \| `inventory_level` \| `-inventory_level` | `"-inventory_level"` |
  | fields | Text[] | | 돌려줄 필드만([conventions.md](conventions.md) §6). 예: 변형이 필요 없으면 `has_variant`를 뺌 | `["id", "name", "price", "inventory_level"]` |
  | limit | Integer 1–100 | | 기본 `20` | `50` |
  | cursor | Text | | 이전 응답의 `next_cursor` | `"c2"` |
  - **운영자만** 표시 필드는 쇼퍼 매니페스트에 없고, 쇼퍼 토큰으로 보내면 `forbidden`(`reason: scope`).
- 출력(운영자 SJ, Fri Nov 6 11:02, `category_id: "cat_summer_outerwear"`, `valid_for_member_tier: "vip"`, 세일 전이라 `member_price`는 `null`):
```jsonc
{"items": [
  {"id": "prd_windbreaker", "name": "Packable windbreaker", "category_id": "cat_summer_outerwear", "brand_id": "brn_northwind",
   "price": 89.00, "price_currency": "USD", "member_price": null, "cost": 36.00,
   "inventory_level": 214, "availability": "in_stock",
   "has_variant": [
     {"variant_id": "var_windbreaker_m_slate", "sku": "wb-slate-m", "size": "M", "color": "Slate", "inventory_level": 18, "availability": "in_stock"},
     {"variant_id": "var_windbreaker_l_slate", "sku": "wb-slate-l", "size": "L", "color": "Slate", "inventory_level": 12, "availability": "in_stock"}]},
     // … 25개 변형(5색 × XS–XL) 중 일부
  {"id": "prd_chore_coat", "name": "Cotton chore coat", "category_id": "cat_summer_outerwear", "brand_id": "brn_northwind",
   "price": 142.00, "price_currency": "USD", "member_price": null, "cost": 60.00,
   "inventory_level": 7, "availability": "in_stock",
   "has_variant": [
     {"variant_id": "var_chore_coat_s_olive", "sku": "cc-olive-s", "size": "S", "color": "Olive", "inventory_level": 3, "availability": "in_stock"},
     {"variant_id": "var_chore_coat_m_olive", "sku": "cc-olive-m", "size": "M", "color": "Olive", "inventory_level": 0, "availability": "out_of_stock"},
     {"variant_id": "var_chore_coat_l_olive", "sku": "cc-olive-l", "size": "L", "color": "Olive", "inventory_level": 0, "availability": "out_of_stock"},
     {"variant_id": "var_chore_coat_xl_olive", "sku": "cc-olive-xl", "size": "XL", "color": "Olive", "inventory_level": 4, "availability": "in_stock"}]},
  {"id": "prd_rain_parka", "name": "Light rain parka", "category_id": "cat_summer_outerwear", "brand_id": "brn_northwind",
   "price": 118.00, "price_currency": "USD", "member_price": null, "cost": 50.00,
   "inventory_level": 4, "availability": "in_stock", "has_variant": [/* S 2, M 0, L 0, XL 2 */]}],
 "total": 42,
 "next_cursor": "c2"}
```
- 출력(쇼퍼 Jamie, VIP, Sat Nov 7 10:00, `query: "light jacket for spring trips"`): 같은 모양에서 `cost`가 없고, `member_price`가 Jamie의 등급 기준으로 채워집니다.
```jsonc
{"items": [
  {"id": "prd_windbreaker", "name": "Packable windbreaker", "category_id": "cat_summer_outerwear", "brand_id": "brn_northwind",
   "price": 89.00, "price_currency": "USD",
   "member_price": {"price": 71.20, "name": "VIP weekend", "valid_for_member_tier": "vip", "promotion_id": "promo_1042",
                    "valid_through": "2026-11-08T23:59:00-05:00"},
   "inventory_level": 214, "availability": "in_stock", "has_variant": [/* … */]},
  {"id": "prd_field_jacket", "name": "Linen blend field jacket", "price": 128.00, "price_currency": "USD",
   "member_price": {"price": 102.40, "name": "VIP weekend", "valid_for_member_tier": "vip", "promotion_id": "promo_1042",
                    "valid_through": "2026-11-08T23:59:00-05:00"},
   "inventory_level": 86, "availability": "in_stock", "has_variant": [/* … */]}],
 "total": 12, "next_cursor": "c2"}
```
  - `member_price`: 부르는 사람(운영자는 `valid_for_member_tier`·`as_of`)에게 그 시각 적용되는 회원·기간 한정 판매가(`UnitPriceSpecification`, `price_type: sale_price`). 없으면 `null`. `price`는 언제나 정가입니다.
  - `cost`: 운영자 주체에게만 있는 필드. 쇼퍼·외부 에이전트 응답에는 키 자체가 없습니다(`pricing.cost_visibility`).
  - `inventory_level`(상품)은 변형 재고의 합, `availability`(상품)는 변형에서 파생(하나라도 `in_stock`이면 `in_stock`).
- 프리뷰: 해당 없음(읽기)
- 정책: [search.yml](../../policies/search.yml) `query.tenant_isolation`, `query.member_tier_filter_at_query_time`, `query.expansion_fallback_keeps_ranking`, `query.shopper_default_availability`, `ranking.signals`; 원가 노출은 [catalog.yml](../../policies/catalog.yml) `pricing.cost_visibility`
- 에러: `invalid_input`(필터 값·범위가 틀림, `detail.field`) → `get_facets` · `tier_not_found` → `list_tiers` · `not_found`(`detail.entity: category`) → `list_categories` · `forbidden`(쇼퍼가 운영자 전용 필드를 보냄) → null · `index_unavailable`(검색 색인 장애) → `get_index_status`
- 멱등성: 읽기(부작용 없음). 같은 입력의 결과는 캐시됨

### `compare_products` — Kind: R  (데모 장면, 쇼퍼)
- 논리주소/REST: `search.product.compare` · `GET /v1/search/products/compare?product_ids=prd_windbreaker,prd_field_jacket&attributes=weight,packed_size` (배열은 쉼표로 구분)
- 요약: 2–6개 상품을 같은 속성 축으로 나란히 놓습니다. 항목 하나가 상품 하나이고, 상품마다 요청한 속성 값을 같은 순서로 담습니다. 쇼퍼가 "어느 게 가장 작게 접히나"를 물을 때 근거로 쓰고, 화면은 `compare_table` 블록([chat.md](chat.md) §3.3)입니다. 가격은 부르는 쇼퍼의 가격입니다.
- consumes → produces: `product_id[]` → comparison
- 입력 (닫힌 객체):
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | product_ids | Text[] `^prd_[a-z0-9_]+$`, 2–6개, 중복 불가 | ✓ | 비교할 상품(`search_products` 결과) | `["prd_windbreaker", "prd_field_jacket", "prd_seersucker_overshirt"]` |
  | attributes | enum[] | | 비교할 속성, 요청 순서대로 돌려줌. `weight`: 무게(M 기준, g); `packed_size`: 접은 크기(cm, 짧은 변·긴 변); `packing`: 접는 방식; `warmth`: 보온; `rain`: 비 막음; `material`: 소재; `price`: 부르는 쇼퍼의 가격. 기본: 전부(`search.yml compare.attributes`) | `["packed_size", "weight", "rain", "price"]` |
  | sort_by | enum | | `attributes`의 값 하나. 그 속성 오름차순으로 항목 정렬(값이 없는 상품은 뒤로). 기본: 입력 순서 | `"packed_size"` |
- 값 형식: 모든 값은 `{attribute, value, unit, display}`.
  | attribute | `value` 타입 | `unit` | `display` 예 |
  |---|---|---|---|
  | `weight` | Number | `g` | `"190 g"` |
  | `packed_size` | Number[2] (짧은 변, 긴 변) 또는 `null`(전용 접힘 없음) | `cm` | `"14 × 18 cm"` |
  | `packing` | enum `own_pocket` \| `rolls_into_hood` \| `folds_flat` \| `none` | `null` | `"Packs into its own pocket"` |
  | `warmth` | enum `wind_layer` \| `light` \| `medium` \| `warm` | `null` | `"Wind layer only"` |
  | `rain` | Boolean | `null` | `"Light rain"` |
  | `material` | Text | `null` | `"Recycled nylon"` |
  | `price` | MonetaryAmount(부르는 쇼퍼가 내는 가격) | `null` | `"$71.20 (VIP weekend)"` |
- 출력(Jamie, Sat Nov 7, `attributes: ["packed_size", "weight", "rain", "price"]`, `sort_by: "packed_size"`):
```jsonc
{"items": [
  {"product_id": "prd_windbreaker", "name": "Packable windbreaker", "color": "Slate", "material": "Recycled nylon",
   "price": 89.00, "member_price": 71.20, "price_currency": "USD",
   "values": [
     {"attribute": "packed_size", "value": [14, 18], "unit": "cm", "display": "Packs into its own pocket, 14 × 18 cm"},
     {"attribute": "weight", "value": 190, "unit": "g", "display": "190 g"},
     {"attribute": "rain", "value": true, "unit": null, "display": "Light rain"},
     {"attribute": "price", "value": {"value": 71.20, "currency": "USD"}, "unit": null, "display": "$71.20 (VIP weekend)"}]},
  {"product_id": "prd_field_jacket", "name": "Linen blend field jacket", "color": "Sand", "material": "Linen, cotton",
   "price": 128.00, "member_price": 102.40, "price_currency": "USD",
   "values": [
     {"attribute": "packed_size", "value": null, "unit": "cm", "display": "Folds flat"},
     {"attribute": "weight", "value": 340, "unit": "g", "display": "340 g"},
     {"attribute": "rain", "value": false, "unit": null, "display": "No"},
     {"attribute": "price", "value": {"value": 102.40, "currency": "USD"}, "unit": null, "display": "$102.40 (VIP weekend)"}]},
  {"product_id": "prd_seersucker_overshirt", "name": "Seersucker overshirt", "color": "Clay", "material": "Cotton",
   "price": 79.00, "member_price": 67.15, "price_currency": "USD",
   "values": [
     {"attribute": "packed_size", "value": null, "unit": "cm", "display": "Folds flat"},
     {"attribute": "weight", "value": 360, "unit": "g", "display": "360 g"},
     {"attribute": "rain", "value": false, "unit": null, "display": "No"},
     {"attribute": "price", "value": {"value": 67.15, "currency": "USD"}, "unit": null, "display": "$67.15 (VIP weekend)"}]}],
 "total": 3}
```
  - `total`은 비교한 상품 수입니다. 상품에 값이 없는 속성은 `value: null`, `display: "Not listed"`로 채워 칸이 비지 않습니다.
- 프리뷰: 해당 없음(읽기)
- 정책: [search.yml](../../policies/search.yml) `compare.max_products`, `compare.attributes`, `query.tenant_isolation`, `query.member_tier_filter_at_query_time`. 가격은 부르는 쇼퍼의 가격([catalog.yml](../../policies/catalog.yml) `pricing.sale_price`)
- 에러: `too_many_products`(7개 이상, `detail.limit: 6`) → `compare_products` · `invalid_input`(1개만 줌, 중복 id) → `search_products` · `not_found`(`detail.entity: product`) → `search_products` · `unknown_attribute`(`detail.allowed[]`에 허용 값) → `compare_products`
- 멱등성: 읽기(부작용 없음)

### `autocomplete` — Kind: R
- 논리주소/REST: `search.product.autocomplete` · `GET /v1/search/autocomplete`
- 요약: 입력 중인 접두어로 추천어를 돌려줍니다(키워드·상품·브랜드·카테고리). 동의어·신조어와 언어별 부스트·제외를 반영합니다.
- consumes → produces: → suggestions
- 입력: `prefix`(Text, 1자 이상), `suggest_type?`(`keyword` | `product` | `brand` | `category`, 기본 `keyword`), `limit?`(기본 10, 최대 20)
- 출력: `{items: [{text: "windbreaker", suggest_type: "keyword", score: 0.92}], total: 5}`
- 프리뷰: 해당 없음(읽기)
- 정책: [search.yml](../../policies/search.yml) `boost_exclude`
- 에러: `invalid_prefix` → `autocomplete` · `index_unavailable` → `get_index_status`
- 멱등성: 읽기(부작용 없음)

### `get_facets` — Kind: R
- 논리주소/REST: `search.product.get_facets` · `GET /v1/search/facets`
- 요약: 지금 질의·필터에서 걸 수 있는 필터 차원과 값별 건수를 돌려줍니다. 화면의 필터 패널과 에이전트의 필터 제안에 씁니다. 격리된 상품은 세지 않습니다.
- consumes → produces: → facets
- 입력: `query?`, `category_id?`, `tags?[]`, `facet_fields?[]`(`category` | `brand` | `price_range` | `color` | `size` | `availability`)
- 출력: `{items: [{name: "price_range", value: "50.00-100.00", count: 18}, {name: "availability", value: "in_stock", count: 40}], total: 2}`
- 프리뷰: 해당 없음(읽기)
- 정책: [search.yml](../../policies/search.yml) `query.tenant_isolation`
- 에러: `invalid_filter` → `get_facets` · `index_unavailable` → `get_index_status`
- 멱등성: 읽기(부작용 없음)

---

## lexicon

### `get_lexicon` — Kind: R
- 논리주소/REST: `search.lexicon.get` · `GET /v1/search/lexicon`
- 요약: 스토어·언어별 동의어·신조어 사전과 분석기 버전을 돌려줍니다. 가장 최근 버전과, 지금 색인이 쓰는 버전이 같은지(재색인이 남았는지)도 알려 줍니다.
- consumes → produces: → lexicon
- 입력: `in_language`(✓, 예 `"en"`)
- 출력: `{in_language: "en", synonyms: [{terms: ["jacket", "coat"], bidirectional: true}], neologisms: [], boost_rules: [], analyzer_version: "v12", source: "saved", saved_at: "2026-11-06T09:40:00-05:00", in_use_version: "v12", reindex_pending: false}` (단건 읽기라 목록 봉투 없음). `v1`은 정책([search.yml](../../policies/search.yml) `lexicon.synonyms`)의 기본 사전(`source: policy_default`)이고, `update_lexicon`이 저장할 때마다 다음 버전이 됩니다.
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `lexicon_not_found` → `update_lexicon`
- 멱등성: 읽기(부작용 없음)

### `update_lexicon` — Kind: W
- 논리주소/REST: `search.lexicon.update` · `PUT /v1/search/lexicon`
- 요약: 동의어·신조어 사전을 통째로 바꿉니다(부분 수정 없음, 분석 결과가 언제나 같도록). 분석기 버전이 올라가고, 반영하려면 `reindex`가 필요합니다.
- consumes → produces: → `analyzer_version`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | in_language | Text | ✓ | `"en"`. 스토어 검색이 분석하는 언어만([search.yml](../../policies/search.yml) `lexicon.languages`, 기본 `[en]`) |
  | synonyms | object[] | ✓ | 전체 집합, 예 `[{terms: ["windbreaker", "wind jacket"], bidirectional: true}]`. `bidirectional: false`면 첫 단어만 나머지로 넓힘 |
  | neologisms | object[] | | 예 `[{surface: "packable", normalize: "packs into pocket"}]`. 그 단어로 찾으면 뜻의 단어로도 찾음 |
  | boost_rules | object[] | | 언어별 부스트·제외 `[{term, effect: boost \| exclude}]`. 부스트어가 있는 상품이 위로, 제외어가 든 질의는 결과 0건 |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "v13", undo: {by: "person", target: "cs_1044", until: "2026-11-07T23:59:00-05:00"}}`. 실행 뒤 다음 툴은 `reindex`
- 프리뷰: `changes`=`update` `lexicon`(`from`/`to`에 추가·삭제한 동의어). checks `lexicon.no_circular_synonym`, `lexicon.no_self_reference`, `index.lexicon_change_requires_reindex`(`detail: "Takes effect after reindex"`, `fix_tool: "reindex"`). [approval.yml](../../policies/approval.yml)에 이 툴의 임계가 없으므로 `needs_approval: false`.
- 정책: [search.yml](../../policies/search.yml) `lexicon`, `index.lexicon_change_requires_reindex`, `boost_exclude`
- 에러: `circular_synonym`(한 방향 동의어·신조어가 돌아 제자리로) → `update_lexicon` · `unsupported_language` → `get_lexicon` · `reindex_pending`(이전 사전이 아직 재색인 전) → `get_index_status` · `invalid_input`(`detail.rule: lexicon.no_self_reference`, 같은 단어를 두 번 쓰거나 자기 자신을 뜻하는 항목) → `update_lexicon`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 같은 사전을 다시 보내면 버전을 올리지 않고 첫 결과(프리뷰 `note`가 그렇게 말함)
- 되돌리기: 아직 어떤 색인도 쓰지 않은 버전만 지웁니다. 재색인이 이미 쓴 버전은 `not_revocable`(사전을 다시 저장해 바꿈).

---

## index

### `reindex` — Kind: W
- 논리주소/REST: `search.index.reindex` · `POST /v1/search/index/reindex`
- 요약: 색인을 다시 만듭니다. 전체 재색인은 새 색인을 다 채우고 확인한 뒤 별칭을 한 번에 바꿔 검색이 멈추지 않습니다. 증분은 바뀐 문서만 반영합니다. 스토어마다 한 번에 하나만 돌고, 사전 변경 뒤에는 꼭 불러야 합니다.
- consumes → produces: `analyzer_version?` → `reindex_job_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | mode | enum | ✓ | `full` \| `incremental` |
  | product_ids | Text[] | | 증분 범위 |
  | since | DateTime | | 증분 범위 |
  | analyzer_version | Text | | 없으면 최신 사전 |
  | embeddings | object | | 벡터 재생성 `{text: true, image: true}`(비용 큼) |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "rij_42", undo: null}`. 진행은 `get_index_status`
- 프리뷰: `changes`=`create` `reindex_job`(`to`에 모드, 색인 이름, 사전 버전, 다시 쓸 문서 수. 한 번에 끝나므로 시간은 없음). checks `index.distributed_lock`(진행 중 작업이 없는지), `index.resource_budget`(오늘의 전체 재색인 횟수, `full_reindex_max_per_day`). [approval.yml](../../policies/approval.yml)에 임계가 없으므로 `needs_approval: false`.
- 구현: 바깥 검색 엔진 없이 Postgres 검색 문서(`search.search_document`)를 다시 씁니다. 재색인은 트랜잭션 하나라 검색은 커밋까지 이전 문서를 읽고, 커밋 순간 새 문서로 바뀝니다(별칭 교체와 같은 효과). 증분은 `product_ids`, `since` 이후 바뀐 상품, 둘 다 없으면 문서가 없거나 상품보다 오래된 상품만. 검색은 최신 문서만 색인에서 읽고, 없거나 오래된 상품은 같은 질의 안에서 카탈로그 글로 순위를 매기므로 색인이 낡아도 상품이 빠지지 않습니다. 임베딩 모델은 연결되어 있지 않아 `embeddings`를 청해도 벡터는 그대로이고 결과가 그렇게 말합니다. 실행이 끝나면 그 사전 버전이 쓰이기 시작합니다(검색 질의 확장).
- 정책: [search.yml](../../policies/search.yml) `index.reindex`, `index.distributed_lock`, `index.incremental`, `index.resource_budget`
- 에러: `reindex_in_progress` → `get_index_status` · `analyzer_version_not_found` → `get_lexicon` · `swap_failed` → `get_index_status`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과(같은 작업). 스토어마다 잠금 하나라 동시에 둘이 돌지 않습니다(`reindex_in_progress`). 문서 쓰기에 실패하면 트랜잭션이 되돌아가 이전 색인을 그대로 씀(`swap_failed`)
- 되돌리기: 없음(`undo: null`). 다시 재색인합니다.

### `get_index_status` — Kind: R
- 논리주소/REST: `search.index.get_status` · `GET /v1/search/index/status`
- 요약: 지금 쓰는 색인·별칭과 재색인 작업 상태를 돌려줍니다.
- consumes → produces: `reindex_job_id?` → index status
- 입력: `reindex_job_id?`
- 출력: `{active_alias: "products-store_northwind-en", active_index: "products-store_northwind-en-v42", in_language: "en", analyzer_version: "v12", latest_analyzer_version: "v13", reindex_pending: true, serving: "index", products: 400, documents: 400, out_of_date: 3, note: "...", job: {id: "rij_42", mode: "full", action_status: "completed", progress: 1.0, analyzer_version: "v12", index: "products-store_northwind-en-v42", documents: 400, start_time: "2026-11-06T09:10:00-05:00", end_time: "2026-11-06T09:10:00-05:00"}}` (단건 읽기라 목록 봉투 없음). 재색인 전에는 `active_index: null`, `serving: "catalog_text"`(검색이 카탈로그 글을 바로 순위 매김). `job`은 준 작업 또는 가장 최근 작업
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `job_not_found` → `reindex`
- 멱등성: 읽기(부작용 없음)
