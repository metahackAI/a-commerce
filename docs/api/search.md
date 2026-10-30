# API 명세 — `search` (검색·비교·자동완성·패싯·사전·색인)

> 계층: `search.{resource}.{operation}`. 공통 규약(응답 형식·에러·프리뷰/실행·멱등·헤더)은 [README.md](README.md) §3, 데이터 타입은 [data-model.md](data-model.md), 정책은 [search.yml](policies/search.yml). MVP 툴(`search_products`, `compare_products`)의 시그니처는 [../plan.md](../plan.md) §3.2·§3.4와 같고, 계층 위치는 [../architecture.md](../architecture.md) §3.4를 따릅니다.
>
> **설계 축**: ① `search_products`는 운영자와 쇼퍼가 함께 쓰는 첫 툴로, 후속 툴의 입력 `product_id[]`를 만듭니다(툴 그래프의 맨 앞, [README.md](README.md) §5). ② 키워드와 벡터를 섞은 랭킹, 질의 확장, 실패 시 키워드 검색으로 내려가도 같은 정렬 유지. ③ 스토어 격리와 등급 필터는 쿼리 안에서 적용(결과를 받은 뒤 거르지 않음). ④ 동의어·신조어 변경은 재색인 후 반영(새 색인을 다 채운 뒤 별칭을 바꿔 무중단). 이 모듈은 Rust 전환 1순위입니다([../architecture.md](../architecture.md) §2.3).
>
> **응답 형식**: 읽기 `{items, total}`, 프리뷰 `{preview_id, changes[{op, entity, id, from, to}], checks[{rule, passed, detail, fix_tool}], needs_approval, approval_reason}`, 실행 `{ok, id, undo{tool, until}}`, 에러 `{error: {code, message, next_tool}}`. 목록은 `limit`·`offset`. 금액은 USD 소수 둘째 자리.

리소스: [product](#product) · [lexicon](#lexicon) · [index](#index)

---

## product

### `search_products` — Kind: R  (MVP, 운영자·쇼퍼 공용)
- 논리주소/REST: `search.product.search` · `GET /v1/search/products`
- 요약: 질의·카테고리·태그·재고 조건으로 상품을 찾습니다. 각 상품에 가격, 재고, 그리고 `valid_for_member_tier`에 해당하는 가격을 붙입니다. 원가(`cost`)는 운영자에게만 보입니다. 쇼퍼 에이전트가 부르면 `valid_for_member_tier`는 무시되고 돌려주는 가격은 그 쇼퍼 자신의 가격입니다.
- consumes → produces: → `product_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | query | Text | | 자연어 질의, 예 `"light jacket for travel"`. 비우면 필터만으로 탐색 |
  | category | Text | | 예 `"summer_outerwear"` |
  | tags | Text[] | | 예 `["packable", "natural_fabric"]` |
  | min_stock | Integer | | 예 `10`(세일 대상 고르기) |
  | max_stock | Integer | | 예 `9`(재고 부족 상품 찾기) |
  | valid_for_member_tier | Text | | `"everyone"`(기본) \| `"vip"`. 운영자만 |
  | fields | Text[] | | 응답 필드 축소, 예 `["id", "name", "price", "stock"]` |
  | limit | Integer | | 기본 `20`, 최대 `100` |
  | offset | Integer | | 기본 `0` |
- 출력(운영자, `valid_for_member_tier: "vip"`):
```jsonc
{"items": [
  {"id": "prd_windbreaker", "name": "Packable windbreaker", "category": "summer_outerwear",
   "price": 89.00, "cost": 35.60, "stock": 214,
   "tier_price": {"valid_for_member_tier": "vip", "price": 89.00}, "price_currency": "USD"},
  {"id": "prd_chore_coat", "name": "Cotton chore coat", "category": "summer_outerwear",
   "price": 142.00, "cost": 56.80, "stock": 7,
   "tier_price": {"valid_for_member_tier": "vip", "price": 142.00}, "price_currency": "USD"}],
 "total": 42}
```
- 출력(쇼퍼 Jamie, VIP, Sat Nov 7): `price`는 Jamie가 내는 가격(`71.20`), `cost`는 없음, `tier_price` 대신 `list_price: 89.00`.
- 프리뷰: 해당 없음(읽기)
- 정책: [search.yml](policies/search.yml) `query.tenant_isolation`, `query.member_tier_filter_at_query_time`, `query.expansion_fallback_keeps_ranking`, `ranking.signals`; 원가 노출은 [catalog.yml](policies/catalog.yml) `pricing.cost_visibility`
- 에러: `invalid_filter` → `get_facets` · `tier_not_found` → `list_tiers` · `index_unavailable` → `get_index_status`
- 멱등성: 읽기(부작용 없음). 같은 입력의 결과는 캐시됨

### `compare_products` — Kind: R  (MVP, 쇼퍼)
- 논리주소/REST: `search.product.compare` · `GET /v1/search/products/compare`
- 요약: 여러 상품을 같은 속성 축으로 나란히 놓습니다. 속성 하나가 한 줄, 상품 하나가 한 칸입니다. 쇼퍼가 "어느 게 여행에 나을까"를 물을 때 근거로 씁니다.
- consumes → produces: `product_id[]` → comparison
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | product_ids | Text[] | ✓ | 2–10개, `search_products` 결과를 그대로 |
  | attributes | Text[] | | `"packed"` \| `"weight"` \| `"warmth"` \| `"light_rain"` \| `"price"`. 없으면 공통 속성 자동 선택 |
- 출력:
```jsonc
{"items": [
  {"attribute": "packed", "values": {"prd_windbreaker": "own pocket, 14 × 18 cm", "prd_field_jacket": "folds flat", "prd_overshirt": "folds flat"}},
  {"attribute": "weight", "values": {"prd_windbreaker": "190 g", "prd_field_jacket": "340 g", "prd_overshirt": "360 g"}},
  {"attribute": "light_rain", "values": {"prd_windbreaker": true, "prd_field_jacket": false, "prd_overshirt": false}},
  {"attribute": "price", "values": {"prd_windbreaker": 71.20, "prd_field_jacket": 102.40, "prd_overshirt": 67.15}}],
 "total": 4,
 "columns": ["prd_windbreaker", "prd_field_jacket", "prd_overshirt"]}
```
- 프리뷰: 해당 없음(읽기)
- 정책: [search.yml](policies/search.yml) `query.tenant_isolation`, `query.member_tier_filter_at_query_time`. 가격은 부르는 쇼퍼의 가격
- 에러: `too_many_products` → `compare_products` · `product_not_found` → `search_products` · `unknown_attribute` → `compare_products`
- 멱등성: 읽기(부작용 없음)

### `autocomplete` — Kind: R
- 논리주소/REST: `search.product.autocomplete` · `GET /v1/search/autocomplete`
- 요약: 입력 중인 접두어로 추천어를 돌려줍니다(키워드·상품·브랜드·카테고리). 동의어·신조어와 언어별 부스트·제외를 반영합니다.
- consumes → produces: → suggestions
- 입력: `prefix`, `suggest_type?`(`keyword` | `product` | `brand` | `category`, 기본 `keyword`), `limit=10`
- 출력: `{items: [{text: "windbreaker", suggest_type: "keyword", score: 0.92}], total: 5}`
- 프리뷰: 해당 없음(읽기)
- 정책: [search.yml](policies/search.yml) `boost_exclude`
- 에러: `invalid_prefix` → `autocomplete` · `index_unavailable` → `get_index_status`
- 멱등성: 읽기(부작용 없음)

### `get_facets` — Kind: R
- 논리주소/REST: `search.product.get_facets` · `GET /v1/search/facets`
- 요약: 지금 질의·필터에서 걸 수 있는 필터 차원과 값별 건수를 돌려줍니다. 화면의 필터 패널과 에이전트의 필터 제안에 씁니다. 격리된 상품은 세지 않습니다.
- consumes → produces: → facets
- 입력: `query?`, `category?`, `tags?[]`, `facet_fields?[]`(`category` | `brand` | `price_range` | `color` | `size` | `availability`)
- 출력: `{items: [{name: "price_range", value: "50.00-100.00", count: 18}, {name: "availability", value: "in_stock", count: 40}], total: 2}`
- 프리뷰: 해당 없음(읽기)
- 정책: [search.yml](policies/search.yml) `query.tenant_isolation`
- 에러: `invalid_filter` → `get_facets` · `index_unavailable` → `get_index_status`
- 멱등성: 읽기(부작용 없음)

---

## lexicon

### `get_lexicon` — Kind: R
- 논리주소/REST: `search.lexicon.get` · `GET /v1/search/lexicon`
- 요약: 스토어·언어별 동의어·신조어 사전과 분석기 버전을 돌려줍니다.
- consumes → produces: → lexicon
- 입력: `in_language`(예 `"en"`)
- 출력: `{items: [{in_language: "en", synonyms: [{terms: ["jacket", "coat"]}], neologisms: [], analyzer_version: "v12"}], total: 1}`
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
  | in_language | Text | ✓ | `"en"` \| `"ko"` |
  | synonyms | object[] | ✓ | 전체 집합, 예 `[{terms: ["windbreaker", "wind jacket"], bidirectional: true}]` |
  | neologisms | object[] | | 예 `[{surface: "packable", normalize: "packs into pocket"}]` |
  | boost_rules | object[] | | 언어별 부스트·제외. 제외어는 결과 0건 |
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "v13", undo}`. 실행 뒤 다음 툴은 `reindex`
- 프리뷰: `changes`=동의어 추가·삭제 목록. checks `no_circular_synonym`, `no_self_reference`, `reindex_required`(`detail: "Takes effect after reindex"`, `fix_tool: "reindex"`). 검색 품질 전체에 영향이 있어 `needs_approval`은 [approval.yml](policies/approval.yml)에 따름.
- 정책: [search.yml](policies/search.yml) `index.lexicon_change_requires_reindex`, `boost_exclude`
- 에러: `circular_synonym` → `update_lexicon` · `unsupported_language` → `get_lexicon` · `reindex_pending` → `get_index_status`
- 멱등성: `Idempotency-Key`. 같은 사전을 다시 보내면 버전을 올리지 않고 첫 결과

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
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "rij_42", undo: null}`. 진행은 `get_index_status`
- 프리뷰: `changes`=`create` `reindex_job`(`detail`에 예상 문서 수·시간·비용). checks `single_active_job`, `resource_budget`. `full`은 비용 때문에 `needs_approval`이 [approval.yml](policies/approval.yml)에 따름.
- 정책: [search.yml](policies/search.yml) `index.reindex`, `index.distributed_lock`, `index.incremental`
- 에러: `reindex_in_progress` → `get_index_status` · `analyzer_version_not_found` → `get_lexicon` · `swap_failed` → `get_index_status`
- 멱등성: `Idempotency-Key = hash(tenant + mode + scope + analyzer_version)`. 다시 보내면 진행 중인 작업을 돌려줌. 별칭 교체 실패 시 이전 색인을 그대로 씀

### `get_index_status` — Kind: R
- 논리주소/REST: `search.index.get_status` · `GET /v1/search/index/status`
- 요약: 지금 쓰는 색인·별칭과 재색인 작업 상태를 돌려줍니다.
- consumes → produces: `reindex_job_id?` → index status
- 입력: `reindex_job_id?`
- 출력: `{items: [{active_alias: "products_active", active_index: "products_v42", analyzer_version: "v12", job: {id: "rij_42", mode: "full", action_status: "active", progress: 0.72, start_time, end_time: null}}], total: 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `job_not_found` → `reindex`
- 멱등성: 읽기(부작용 없음)
