# API 명세 — `merchandising` (카테고리·진열 슬롯·배너·팝업·기획전·캐시)

> 계층: `merchandising.{resource}.{operation}`. 공통 형식(읽기 봉투, 쓰기의 프리뷰 → `preview_id` 실행, 시간·금액·id)은 [conventions.md](conventions.md), 에러 코드는 [errors.md](errors.md), 상태 값은 [enums.md](enums.md), 데이터 타입은 [data-model.md](data-model.md), 정책은 [merchandising.yml](../../policies/merchandising.yml)·[approval.yml](../../policies/approval.yml). 예시 값은 데모 스토어([../scenarios.md](../scenarios.md))입니다.
>
> **설계 축**: ① 표준 카테고리(내부 분류)와 전시 카테고리(쇼퍼에게 보이는 트리)를 나누고 `acom:CategoryMapping`으로 N:M 연결. ② 노출 여부는 배치로 켜고 끄지 않고 `valid_from`·`valid_through`로 그때그때 계산. 시각 비교는 툴 안에서만 하고 에이전트가 직접 하지 않음. ③ 노출 = 기간 ∧ 재고 ∧ 판매 가능 상태. ④ 쇼퍼에게 닿는 공개(배너 예약, 슬롯 예약, 기획전 공개, 노출 중인 배너·팝업 수정)는 변경셋 승인. ⑤ 전시 변경은 캐시를 명시적으로 무효화해야 쇼퍼 화면에 반영.
>
> **대상(audience)**: 예약마다 대상이 하나입니다. 등급 id(`list_tiers`, 예 `vip`) 또는 예약어 `everyone`이고, 저장할 때는 `valid_for_member_tier`(everyone = NULL)로 남습니다([enums.md](enums.md) §5). 등급 대상 예약은 그 등급에게만 보이고 나머지 고객은 기존 예약을 계속 봅니다([merchandising.yml](../../policies/merchandising.yml) `slot_free`).

리소스: [category](#category) · [display_slot](#display_slot) · [banner](#banner) · [popup](#popup) · [collection_page](#collection_page) · [cache](#cache)

---

## category

> 표준 카테고리와 전시 카테고리는 서로 다른 `DefinedTermSet`이고 `term_set`(`standard` | `display`)으로 구분합니다. id는 둘 다 `cat_`입니다. 상품은 표준 카테고리에 한 번 분류되고, 전시 트리는 자유롭게 구성되며 `acom:CategoryMapping`(N:M, `map_`)으로 표준과 연결됩니다. 삭제는 하위 카테고리·매핑·상품이 없을 때만 됩니다.

### `list_categories` — Kind: R
- 논리주소/REST: `merchandising.category.list` · `GET /v1/merchandising/categories`
- 요약: 카테고리 트리를 돌려줍니다. 부모가 비활성이면 자식도 비활성으로 보입니다.
- consumes → produces: → `category_id[]`
- 입력: `term_set`(`standard` | `display`), `parent_id?`, `depth?`, `fields?[]`, `limit?`(기본 50, 최대 100), `cursor?`
- 출력: `{items: [{id: "cat_summer_outerwear", name: "Summer outerwear", term_code: "summer_outerwear", term_set: "standard", parent_id: "cat_outerwear", display_order: 3, active: true}], total: 24}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_term_set` → `list_categories`
- 멱등성: 읽기(부작용 없음)

### `get_category` — Kind: R
- 논리주소/REST: `merchandising.category.get` · `GET /v1/merchandising/categories/{category_id}`
- 요약: 카테고리 하나를 매핑 요약과 상품 수와 함께 돌려줍니다.
- consumes → produces: `category_id` → category
- 입력: `category_id`, `fields?[]`
- 출력: `{id: "cat_summer_outerwear", name: "Summer outerwear", term_set: "standard", mappings: [{id: "map_221", display_category_id: "cat_new_in", is_primary: true}], product_count: 42}` (단건 읽기라 목록 봉투 없음)
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `category_not_found` → `list_categories`
- 멱등성: 읽기(부작용 없음)

### `create_category` — Kind: W
- 논리주소/REST: `merchandising.category.create` · `POST /v1/merchandising/categories`
- 요약: 표준 또는 전시 카테고리 노드를 만듭니다.
- consumes → produces: `parent_id?` → `category_id`
- 입력: `term_set`(`standard` | `display`), `name`, `parent_id?`, `display_order?`, `term_code?`(의미 있는 영단어, 예 `"summer_outerwear"`)
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "cat_summer_outerwear", undo: {by: "person", target: "cs_1044", until: "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: `changes`=`create` `category`(`id: null`). checks `category.max_depth`, `category.term_code_unique`. `needs_approval: false`.
- 정책: [merchandising.yml](../../policies/merchandising.yml) `category.max_depth`, `category.term_code_unique`
- 에러: `parent_not_found` → `list_categories` · `depth_exceeded` → `update_category` · `duplicate_term_code` → `create_category`
- 멱등성: `Idempotency-Key`

### `update_category` — Kind: W
- 논리주소/REST: `merchandising.category.update` · `PATCH /v1/merchandising/categories/{category_id}`
- 요약: 이름·표시 순서·부모·활성 여부를 바꿉니다. 비활성(`active: false`)은 하위·매핑·상품이 없을 때만.
- consumes → produces: `category_id` → `category_id`
- 입력: `category_id`, `name?`, `display_order?`, `parent_id?`, `active?`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` `category`. checks `category.delete_only_if`(하위·매핑·상품 중 하나라도 있으면 비활성 불가, `detail`에 무엇이 남았는지), `category.no_cycle`. `needs_approval: false`.
- 정책: [merchandising.yml](../../policies/merchandising.yml) `category.delete_only_if`, `category.no_cycle`, `category.child_inherits_inactive_parent`
- 에러: `has_children` → `list_categories` · `has_mapped_products` → `unmap_category` · `cycle_detected` → `update_category`
- 멱등성: `Idempotency-Key`

### `map_category` — Kind: W
- 논리주소/REST: `merchandising.category.map` · `POST /v1/merchandising/category-mappings`
- 요약: 표준 카테고리를 전시 카테고리 여러 개에 연결합니다. 전시 트리에 보이는 상품은 이 매핑에서 나오므로, 실행 후 `invalidate_display_cache`를 부릅니다.
- consumes → produces: `category_id`(표준), `category_id[]`(전시) → `category_mapping_id[]`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명·예 |
  |------|----------------------|:--:|--------|
  | standard_category_id | Text | ✓ | 표준 카테고리, 예 `"cat_summer_outerwear"` |
  | display_category_ids | Text[] | ✓ | 전시 카테고리, 예 `["cat_new_in", "cat_travel"]` |
  | is_primary | Boolean | | 표준 카테고리마다 대표 전시 카테고리 하나(상세의 경로 표시에 씀) |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "map_221", undo: {by: "person", target: "cs_1044", until: "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: `changes`=`create` `category_mapping`(`id: null`). checks `category.mapping`(표준 → 전시 방향만), `category.primary_display_per_standard`(대표 매핑 하나). `detail`에 새로 보이게 될 상품 수. `needs_approval: false`.
- 정책: [merchandising.yml](../../policies/merchandising.yml) `category.mapping`, `category.primary_display_per_standard`
- 에러: `cross_set_violation` → `map_category` · `primary_conflict` → `unmap_category` · `display_category_inactive` → `update_category`
- 멱등성: `(standard_category_id, display_category_id)`로 한 건

### `unmap_category` — Kind: W
- 논리주소/REST: `merchandising.category.unmap` · `DELETE /v1/merchandising/category-mappings/{category_mapping_id}`
- 요약: 표준·전시 매핑을 지웁니다. 대표 매핑을 지우면 남은 매핑 중 하나를 대표로 다시 정해야 합니다. 실행 후 `invalidate_display_cache`.
- consumes → produces: `category_mapping_id` →
- 입력: `category_mapping_id`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`delete` `category_mapping`. `detail`에 더는 보이지 않을 상품 수. checks `category.primary_display_per_standard`. `needs_approval: false`.
- 정책: [merchandising.yml](../../policies/merchandising.yml) `category.mapping`, `category.primary_display_per_standard`
- 에러: `mapping_not_found` → `list_categories` · `primary_needs_reassign` → `map_category`
- 멱등성: 이미 지운 매핑은 같은 결과

---

## display_slot

> 진열 슬롯은 `acom:DisplaySlot`(`ItemList`의 하위 타입, `slot_`)입니다. `item_list_element`에 상품·배너·기획전을 순서대로 담고, 예약(`display_booking`, `dsp_`)마다 대상(`audience`)·`valid_from`·`valid_through`를 가집니다. 노출 = 기간 ∧ 재고 ∧ 판매 가능 상태로 그때그때 계산합니다(`display.visible_when`). 슬롯 종류 `slot_type`은 `hero` | `banner_board` | `product_grid` | `brand_strip` | `collection_feed`, 예약 상태는 `draft` | `scheduled` | `active` | `ended`입니다([enums.md](enums.md) §1.12).

### `list_display_slots` — Kind: R  (MVP)
- 논리주소/REST: `merchandising.display_slot.list` · `GET /v1/merchandising/display-slots?page=main&valid_from=…&valid_through=…`
- 요약: 한 페이지의 슬롯마다 기간 안의 예약과 **대상별 빈 시간**을 돌려줍니다. `schedule_banner`·`schedule_display` 앞에서 "이 슬롯을 VIP에게 이 주말에 쓸 수 있나"를 확인할 때 씁니다. 빈 시간은 `slot_free` 체크와 같은 규칙으로 계산합니다.
- consumes → produces: → `slot_id`
- 입력 (닫힌 객체):
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | page | enum | ✓ | 슬롯이 있는 페이지. `main`: 메인; `category`: 카테고리 목록; `product`: 상품 상세; `search`: 검색 결과 | `"main"` |
  | valid_from | DateTime | ✓ | 볼 기간의 시작(포함). 오프셋이 없으면 스토어 시간대 | `"2026-11-07T00:00:00-05:00"` |
  | valid_through | DateTime | ✓ | 볼 기간의 끝(포함, 분 단위) | `"2026-11-08T23:59:00-05:00"` |
  | audience | Text | | 이 대상의 빈 시간만. 등급 id(`list_tiers`) 또는 `"everyone"`. 기본: `everyone`과 모든 등급 | `"vip"` |
  | slot_type | enum | | `hero` \| `banner_board` \| `product_grid` \| `brand_strip` \| `collection_feed` | `"hero"` |
  | limit | Integer 1–100 | | 기본 20 | `20` |
  | cursor | Text | | 이전 응답의 `next_cursor` | `"c2"` |
- 출력(Fri Nov 6 11:02, 위 예의 입력에서 `audience` 생략):
```jsonc
{"items": [{
  "id": "slot_home_hero", "page": "main", "name": "Home hero", "slot_type": "hero",
  "bookings": [
    {"display_booking_id": "dsp_fall_layers", "audience": "everyone", "headline": "Fall layers",
     "valid_from": "2026-11-06T10:00:00-05:00", "valid_through": null, "status": "active"}],
  "availability": [
    {"audience": "vip",    "free_from": "2026-11-07T00:00:00-05:00", "free_until": "2026-11-08T23:59:00-05:00"},
    {"audience": "gold",   "free_from": "2026-11-07T00:00:00-05:00", "free_until": "2026-11-08T23:59:00-05:00"},
    {"audience": "member", "free_from": "2026-11-07T00:00:00-05:00", "free_until": "2026-11-08T23:59:00-05:00"}]
 }],
 "total": 4}
```
  - `bookings[]`: 요청 기간과 겹치는 예약. `valid_through: null`은 끝을 정하지 않은 예약입니다. `status`는 지금 시각 기준 `acom:display_status`.
  - `availability[]`: 대상마다 요청 기간 안에서 예약이 없는 구간(`free_from` ~ `free_until`, 둘 다 포함). 빈 구간이 여럿이면 여러 줄, 빈 구간이 없는 대상은 나오지 않습니다(위 예에서 `everyone`은 "Fall layers"가 잡고 있어 없음). 등급 대상 예약은 `everyone` 예약과 겹쳐도 됩니다(`slot_free.everyone_blocks_tiers: false`).
- 프리뷰: 해당 없음(읽기)
- 정책: [merchandising.yml](../../policies/merchandising.yml) `slot_free`, `display.schedule_compute`
- 에러: `page_not_found` → `list_display_slots` · `invalid_period`(`valid_from` > `valid_through`) → `list_display_slots` · `tier_not_found` → `list_tiers`
- 멱등성: 읽기(부작용 없음)

### `get_display_slot` — Kind: R
- 논리주소/REST: `merchandising.display_slot.get` · `GET /v1/merchandising/display-slots/{slot_id}`
- 요약: 슬롯 하나를 담긴 항목과 특정 시각 기준 노출 상태와 함께 돌려줍니다.
- consumes → produces: `slot_id` → display_slot
- 입력: `slot_id`, `as_of?`(DateTime), `fields?[]`
- 출력: `{id: "slot_home_hero", slot_type: "hero", item_list_element: [], bookings: [], live_now: true, hidden_items: [{product_id: "prd_chore_coat", reason: "out_of_stock"}]}` (단건 읽기라 목록 봉투 없음)
- 프리뷰: 해당 없음(읽기)
- 정책: [merchandising.yml](../../policies/merchandising.yml) `display.visible_when`
- 에러: `slot_not_found` → `list_display_slots`
- 멱등성: 읽기(부작용 없음)

### `arrange_display_slot` — Kind: W
- 논리주소/REST: `merchandising.display_slot.arrange` · `PUT /v1/merchandising/display-slots/{slot_id}/items`
- 요약: 슬롯 구성(`item_list_element`)을 더하고 빼고 순서를 바꿉니다. 기간과 대상은 건드리지 않습니다(그것은 `schedule_display`). 공개를 바꾸지 않으므로 `W`.
- consumes → produces: `slot_id`, `product_id[]` → `slot_id`
- 입력: `slot_id`, `item_list_element`(`{position, item_id}` 배열, `item_id`는 `prd_`·`bnr_`·`col_`), `slot_type?`(`hero` | `banner_board` | `product_grid` | `brand_strip` | `collection_feed`)
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` `display_slot`. checks `display.max_items_per_slot`, `display.exclude_discontinued`. `needs_approval: false`.
- 정책: [merchandising.yml](../../policies/merchandising.yml) `display.max_items_per_slot`, `display.exclude_discontinued`
- 에러: `item_discontinued` → `search_products` · `duplicate_position` → `arrange_display_slot` · `slot_type_mismatch` → `get_display_slot`
- 멱등성: `Idempotency-Key`. 같은 구성은 다시 적용해도 변경 없음

### `schedule_display` — Kind: W$
- 논리주소/REST: `merchandising.display_slot.schedule` · `POST /v1/merchandising/display-slots/{slot_id}/bookings`
- 요약: 슬롯 구성 전체를 기간·대상·판매 채널에 예약합니다. 쇼퍼에게 닿으므로 항상 변경셋 승인을 거칩니다. 상태(`scheduled` → `active` → `ended`)는 기간으로 계산합니다. 배너 하나만 걸 때는 `schedule_banner`를 씁니다.
- consumes → produces: `slot_id` → `display_booking_id`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명·예 |
  |------|----------------------|:--:|--------|
  | slot_id | Text | ✓ | 예 `"slot_home_hero"` |
  | audience | Text | ✓ | 등급 id(`list_tiers`) 또는 `"everyone"`. 예 `"everyone"` |
  | valid_from | DateTime | ✓ | 과거 불가. 예 `"2026-11-13T00:00:00-05:00"` |
  | valid_through | DateTime | ✓ | 포함, 분 단위. 예 `"2026-11-20T23:59:00-05:00"` |
  | channels | enum[] | | 판매 채널 `web` \| `mobile_web` \| `app`. 기본 전부 |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "dsp_denim_bundle", undo: {by: "person", target: "cs_1047", until: "2026-11-21T23:59:00-05:00"}}`
- 프리뷰: `changes`=`schedule` `display_booking`(`id: null`). checks `slot_free`, `display.start_not_in_past`, `display.exclude_discontinued`; `detail`에 숨겨질 품절 항목 수. 항상 `needs_approval: true`, 예 `approval_reason: "Shows on the main page to everyone"`.
- 정책: [merchandising.yml](../../policies/merchandising.yml) `slot_free`, `display.start_not_in_past`, `display.exclude_discontinued`, `reaches_customers`; [approval.yml](../../policies/approval.yml) `thresholds.display_schedule`
- 에러: `slot_taken`(policy_violation, `detail.rule: slot_free`) → `list_display_slots` · `valid_from_in_past` → `schedule_display` · `empty_slot` → `arrange_display_slot` · `tier_not_found` → `list_tiers`
- 멱등성: `Idempotency-Key = hash(slot_id + audience + valid_from + valid_through)`

---

## banner

### `schedule_banner` — Kind: W$  (MVP)
- 논리주소/REST: `merchandising.banner.schedule` · `POST /v1/merchandising/banners` (프리뷰 `?preview=true`, 실행 `{"preview_id": …}`, [conventions.md](conventions.md) §7)
- 요약: 슬롯에 배너(작은 머리글·제목·버튼·링크·이미지)를 대상과 기간으로 예약합니다. 결과는 진열 예약(`display_booking`, `dsp_`) 하나입니다. 프로모션을 연결하면 배너 기간이 프로모션 기간 안에 있어야 합니다. 쇼퍼에게 닿으므로 항상 변경셋 승인을 거치고, 승인 후 실행기가 `preview_id`로 실행합니다. schema.org `ScheduleAction`.
- consumes → produces: `slot_id`, `tier_id`?, `promotion_id`? → `display_booking_id`
- 입력 (닫힌 객체):
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | slot_id | Text `^slot_[a-z0-9_]+$` | ✓ | `list_display_slots`가 준 슬롯 | `"slot_home_hero"` |
  | audience | Text | ✓ | 배너를 볼 대상. 등급 id(`list_tiers`) 또는 `"everyone"`. 다른 대상은 기존 배너를 계속 봄. `x-aliases`: "VIP members" → `vip` | `"vip"` |
  | valid_from | DateTime | ✓ | 시작(포함). 지금 이후. 오프셋이 없으면 스토어 시간대 | `"2026-11-07T00:00:00-05:00"` |
  | valid_through | DateTime | ✓ | 끝(포함, 분 단위). `valid_from`보다 뒤 | `"2026-11-08T23:59:00-05:00"` |
  | content | object | ✓ | 배너 문구와 이미지(아래 `content.*`, 닫힌 객체) | |
  | content.eyebrow | Text ≤ 24자 | | 제목 위 작은 머리글 | `"VIP WEEKEND"` |
  | content.headline | Text ≤ 60자 | ✓ | 제목 | `"20% off summer outerwear"` |
  | content.cta_label | Text ≤ 24자 | ✓ | 버튼 문구 | `"Shop the sale"` |
  | content.url | Text | ✓ | 버튼이 여는 스토어 안 경로(`/`로 시작) | `"/sale/vip-weekend"` |
  | content.image_source | enum | ✓ | `product`: 상품의 대표 이미지를 씀(`image_product_id` 필수, `image_color` 선택); `image`: 등록한 이미지를 씀(`image_id` 필수) | `"product"` |
  | content.image_product_id | Text `^prd_[a-z0-9_]+$` | | `image_source: product`일 때 | `"prd_field_jacket"` |
  | content.image_color | Text | | `image_source: product`일 때 그 색 변형의 사진. 기본 대표 색 | `"Sand"` |
  | content.image_id | Text `^(bnr\|img)_[a-z0-9_]+$` | | `image_source: image`일 때. `set_banner`(`bnr_`) 또는 `generate_image`(`img_`) 결과 | `"bnr_vip_weekend"` |
  | promotion_id | Text `^promo_[a-z0-9_]+$` | | 연결할 프로모션. 같은 변경셋의 `set_sale_prices`가 아직 실행 전이면 런타임이 그 항목에 `depends_on`으로 묶고 실행 때 실제 id로 바꿈 | `"promo_1042"` |
  - 모드별 필수 필드(`image_source`)를 어기면 `invalid_input`(`detail.field`). 문구 길이 상한은 [merchandising.yml](../../policies/merchandising.yml) `banner.text_max_length`.
- 출력(프리뷰, 변경셋 #1042 v2, Fri Nov 6 11:05):
```jsonc
{"preview_id": "pv_b71e",
 "expires_at": "2026-11-06T11:35:00-05:00",
 "change_set_id": "cs_1042",
 "changes": [
   {"op": "schedule", "entity": "display_booking", "id": null, "from": null,
    "to": {"slot_id": "slot_home_hero", "audience": "vip",
           "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00",
           "content": {"eyebrow": "VIP WEEKEND", "headline": "20% off summer outerwear", "cta_label": "Shop the sale",
                       "url": "/sale/vip-weekend", "image_source": "product", "image_product_id": "prd_field_jacket", "image_color": "Sand"},
           "promotion_id": "promo_1042"}}],
 "checks": [
   {"rule": "slot_free", "passed": true, "detail": "Hero slot open Nov 7–8 for VIP", "fix_tool": null}],
 "needs_approval": true,
 "approval_reason": "Shows on the main page to 1,284 VIP members"}
```
- 출력(실행): `{"ok": true, "id": "dsp_1042", "undo": {"by": "person", "target": "cs_1042", "until": "2026-11-09T23:59:00-05:00"}}` — 되돌리기 기한은 배너 기간 끝 + 1일 23:59([approval.yml](../../policies/approval.yml) `change_set.undo_window_after_end_days`). 실행은 예약 레코드를 만드는 것이고, 시각이 되면 작업 큐가 켜고 끕니다.
- 프리뷰: `changes`=`schedule` `display_booking` 한 건(`id`는 실행 전이라 `null`). check `slot_free`: 같은 슬롯·같은 대상·겹치는 기간에 다른 예약이 없어야 함. 실패하면 `passed: false`, `detail`에 겹치는 예약(예 `"dsp_fall_layers holds everyone"`), `fix_tool: "list_display_slots"`. 항상 `needs_approval: true`([approval.yml](../../policies/approval.yml) `thresholds.display_schedule`), `approval_reason`에 대상 인원.
- 정책: [merchandising.yml](../../policies/merchandising.yml) `slot_free`, `display.start_not_in_past`, `banner.text_max_length`, `reaches_customers`; [approval.yml](../../policies/approval.yml) `thresholds.display_schedule`
- 에러: `slot_taken`(policy_violation, `detail.rule: slot_free`, 겹치는 예약 id) → `list_display_slots` · `slot_not_found` → `list_display_slots` · `valid_from_in_past`(invalid_input) → `schedule_banner` · `invalid_period`(`valid_through` ≤ `valid_from`, 또는 연결한 프로모션 기간 밖) → `get_promotion` · `promotion_not_found` → `list_active_promotions` · `tier_not_found` → `list_tiers` · `not_found`(`detail.entity: product` 또는 `image`) → `search_products`
- 멱등성: `Idempotency-Key = hash(slot_id + audience + valid_from + valid_through)`. 같은 `preview_id`로 다시 실행하면 첫 결과

### `list_banners` — Kind: R
- 논리주소/REST: `merchandising.banner.list` · `GET /v1/merchandising/banners`
- 요약: 배너 이미지 자산 목록(이미지·링크·채널·우선순위)과 그 자산을 쓰는 예약.
- consumes → produces: → `banner_id[]`
- 입력: `page?`, `channel?`(`web` | `mobile_web` | `app`), `as_of?`(DateTime, 그 시각에 보이는 것만), `limit?`(기본 20), `cursor?`
- 출력: `{items: [{id: "bnr_vip_weekend", content_url: "https://cdn.northwind.example/b/vip-weekend.webp", url: "/sale/vip-weekend", channels: ["web", "app"], priority: 1, bookings: [{display_booking_id: "dsp_1042", audience: "vip", valid_from: "2026-11-07T00:00:00-05:00", valid_through: "2026-11-08T23:59:00-05:00"}]}], total: 6}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `page_not_found` → `list_banners`
- 멱등성: 읽기(부작용 없음)

### `set_banner` — Kind: W
- 논리주소/REST: `merchandising.banner.set` · `PUT /v1/merchandising/banners/{banner_id}`
- 요약: 배너 이미지 자산(`ImageObject`, `bnr_`)을 등록·수정합니다. 공개는 `schedule_banner`·`schedule_display`로 일어나므로 자산 등록은 `W`입니다.
- consumes → produces: → `banner_id`
- 입력: `banner_id?`(없으면 새로), `content_url`, `url?`(누르면 열 경로), `channels`(`web` | `mobile_web` | `app`), `priority?`
- 출력: 프리뷰 형식 → 실행 `{ok, id: "bnr_vip_weekend", undo: {by: "person", target: "cs_1048", until: "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: `changes`=`create`/`update` `banner_image`. checks `banner.image_spec`(크기·용량·형식), `display.priority_unique`. 노출 중인 배너를 바꾸면 쇼퍼에게 닿으므로 `needs_approval: true`([approval.yml](../../policies/approval.yml) `thresholds.display_schedule`), 아니면 `false`.
- 정책: [merchandising.yml](../../policies/merchandising.yml) `banner.image_spec`, `display.priority_unique`, `reaches_customers`
- 에러: `image_spec_violation` → `set_banner` · `priority_conflict` → `list_banners`
- 멱등성: `Idempotency-Key` + `banner_id`

---

## popup

### `list_popups` — Kind: R
- 논리주소/REST: `merchandising.popup.list` · `GET /v1/merchandising/popups`
- 요약: 팝업 목록(종류·대상 화면·우선순위·기간).
- consumes → produces: → `popup_id[]`
- 입력: `popup_type?`(`image` | `notice` | `event` | `consent`), `page?`, `as_of?`(DateTime), `limit?`(기본 20), `cursor?`
- 출력: `{items: [{id: "pop_holiday_hours", popup_type: "notice", pages: ["main"], priority: 1, suppress_days: 1, valid_from: "2026-12-20T00:00:00-05:00", valid_through: "2026-12-26T23:59:00-05:00"}], total: 2}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_popup_type` → `list_popups`
- 멱등성: 읽기(부작용 없음)

### `set_popup` — Kind: W
- 논리주소/REST: `merchandising.popup.set` · `PUT /v1/merchandising/popups/{popup_id}`
- 요약: 팝업(`acom:Popup`, `WebPageElement`의 하위 타입, `pop_`)을 등록·수정합니다. "오늘 그만 보기" 일수를 정할 수 있습니다.
- consumes → produces: → `popup_id`
- 입력: `popup_id?`, `popup_type`(`image` | `notice` | `event` | `consent`), `content`(ImageObject | Text), `pages[]`, `priority?`, `suppress_days?`(최대 `popup.suppress_days_max`), `valid_from`, `valid_through`
- 출력: 프리뷰 형식 → 실행 `{ok, id: "pop_holiday_hours", undo: {by: "person", target: "cs_1049", until: "2026-12-27T23:59:00-05:00"}}`
- 프리뷰: `changes`=`create`/`update` `popup`. checks `popup.max_concurrent_per_page`, `display.priority_unique`, `display.start_not_in_past`. 쇼퍼에게 닿으므로 `needs_approval: true`([approval.yml](../../policies/approval.yml) `thresholds.display_schedule`).
- 정책: [merchandising.yml](../../policies/merchandising.yml) `popup`, `display.priority_unique`, `display.start_not_in_past`, `reaches_customers`
- 에러: `too_many_popups` → `list_popups` · `priority_conflict` → `list_popups` · `valid_from_in_past` → `set_popup`
- 멱등성: `Idempotency-Key` + `popup_id`

---

## collection_page

> 기획전은 `CollectionPage`(`col_`) + 섹션별 `ItemList`입니다. 상태(`draft` → `scheduled` → `active` → `ended`)는 기간으로 계산합니다.

### `list_curations` — Kind: R
- 논리주소/REST: `merchandising.collection_page.list` · `GET /v1/merchandising/collection-pages`
- 요약: 기획전 목록(상태·기간·대상).
- consumes → produces: → `collection_page_id[]`
- 입력: `status?`(`draft` | `scheduled` | `active` | `ended`), `audience?`(등급 id 또는 `everyone`), `as_of?`, `limit?`(기본 20), `cursor?`
- 출력: `{items: [{id: "col_winter_edit", name: "Winter outerwear edit", status: "draft", audience: "everyone", valid_from: null, valid_through: null}], total: 3}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_status` → `list_curations`
- 멱등성: 읽기(부작용 없음)

### `get_curation` — Kind: R
- 논리주소/REST: `merchandising.collection_page.get` · `GET /v1/merchandising/collection-pages/{collection_page_id}`
- 요약: 기획전 하나(섹션·상품·연결 배너·검색 키워드·현재 상태).
- consumes → produces: `collection_page_id` → collection_page
- 입력: `collection_page_id`, `fields?[]`
- 출력: `{id: "col_winter_edit", name: "Winter outerwear edit", has_part: [{name: "Coats", item_list_element: ["prd_wool_overcoat"]}], status: "draft", audience: "everyone", valid_from: null, valid_through: null, banner_ids: [], keywords: ["winter coat"], hidden_items: []}` (단건 읽기라 목록 봉투 없음)
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `collection_page_not_found` → `list_curations`
- 멱등성: 읽기(부작용 없음)

### `create_curation` — Kind: W
- 논리주소/REST: `merchandising.collection_page.create` · `POST /v1/merchandising/collection-pages`
- 요약: 상품을 섹션으로 묶어 기획전을 `draft`로 만듭니다. 공개는 `publish_curation`.
- consumes → produces: `product_id[]` → `collection_page_id`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명·예 |
  |------|----------------------|:--:|--------|
  | name | Text | ✓ | "Winter outerwear edit" |
  | has_part | ItemList[] | ✓ | 섹션마다 `{name, item_list_element: product_id[]}` |
  | audience | Text | | 등급 id 또는 `"everyone"`(기본) |
  | valid_from / valid_through | DateTime | | 공개 때 확정 |
  | banner_ids | Text[] | | 연결 배너(`bnr_`) |
  | keywords | Text[] | | 검색 키워드 |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "col_winter_edit", undo: {by: "person", target: "cs_1050", until: "2026-11-07T23:59:00-05:00"}}`
- 프리뷰: `changes`=`create` `collection_page`(`id: null`), 섹션별 `create` `item_list`. checks `display.exclude_discontinued`, `curation.no_empty_section`. `needs_approval: false`.
- 정책: [merchandising.yml](../../policies/merchandising.yml) `curation`, `display.exclude_discontinued`
- 에러: `product_discontinued` → `search_products` · `empty_section` → `update_curation` · `banner_not_found` → `list_banners`
- 멱등성: `Idempotency-Key`

### `update_curation` — Kind: W
- 논리주소/REST: `merchandising.collection_page.update` · `PATCH /v1/merchandising/collection-pages/{collection_page_id}`
- 요약: 초안·예정 기획전의 섹션·상품·배너·키워드·대상을 고칩니다. 상태 전환은 `publish_curation`으로만.
- consumes → produces: `collection_page_id` → `collection_page_id`
- 입력: `collection_page_id`, `name?`, `has_part?`, `audience?`, `banner_ids?`, `keywords?`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`update` `collection_page`. checks `display.exclude_discontinued`, `curation.no_empty_section`. `needs_approval: false`(이미 공개 중이면 `true`, [approval.yml](../../policies/approval.yml) `thresholds.display_schedule`).
- 정책: [merchandising.yml](../../policies/merchandising.yml) `curation`, `display.exclude_discontinued`
- 에러: `product_discontinued` → `search_products` · `already_ended` → `create_curation`
- 멱등성: `Idempotency-Key`

### `publish_curation` — Kind: W$
- 논리주소/REST: `merchandising.collection_page.publish` · `POST /v1/merchandising/collection-pages/{collection_page_id}/publish`
- 요약: 기획전 기간을 확정하고 공개를 예약합니다(`draft` → `scheduled`, 시작이 지금이면 `active`). 쇼퍼에게 닿으므로 항상 변경셋 승인.
- consumes → produces: `collection_page_id` → `collection_page_id`
- 입력: `collection_page_id`, `valid_from`(과거 불가), `valid_through`, `audience?`, `channels?[]`(`web` | `mobile_web` | `app`)
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "col_winter_edit", undo: {by: "person", target: "cs_1051", until: "2026-12-01T23:59:00-05:00"}}` (입력 예 `valid_through: "2026-11-30T23:59:00-05:00"`, 기간 끝 + 1일)
- 프리뷰: `changes`=`update` `collection_page` `status` `draft` → `scheduled`. checks `display.start_not_in_past`, `curation.no_empty_section`, `display.exclude_discontinued`; `detail`에 섹션 수·상품 수·품절로 숨겨질 수. 항상 `needs_approval: true`, 예 `approval_reason: "Publishes a page with 86 products to everyone"`.
- 정책: [merchandising.yml](../../policies/merchandising.yml) `curation.publish_reaches_customers`, `display.start_not_in_past`; [approval.yml](../../policies/approval.yml) `thresholds.display_schedule`
- 에러: `valid_from_in_past` → `update_curation` · `empty_curation` → `update_curation` · `already_published` → `get_curation` · `tier_not_found` → `list_tiers`
- 멱등성: `Idempotency-Key = hash(collection_page_id + valid_from + valid_through + audience)`

---

## cache

### `invalidate_display_cache` — Kind: W
- 논리주소/REST: `merchandising.cache.invalidate` · `POST /v1/merchandising/cache/invalidate`
- 요약: 전시 캐시를 명시적으로 비웁니다. 카테고리 매핑·슬롯·배너·팝업·기획전 변경은 이 호출 뒤에 쇼퍼 화면에 반영됩니다. 범위를 좁혀 불필요한 재조회를 줄입니다. 승인된 변경셋의 실행기는 실행 끝에 자동으로 부릅니다.
- consumes → produces: `slot_id?`, `category_id?`, `collection_page_id?` → `cache_invalidation_id`
- 입력: `resource_type`(`category` | `display_slot` | `banner` | `popup` | `collection_page` | `all`), `resource_id?`, `pages?[]`, `reason?`
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "cin_5521", undo: null}`
- 프리뷰: `changes`=`create` `cache_invalidation`(`to`에 비울 키 수). checks `cache_invalidation.allowed_resource_types`, `cache_invalidation.all_scope_max_per_hour`(`all` 남용 방지). `needs_approval: false`.
- 정책: [merchandising.yml](../../policies/merchandising.yml) `cache_invalidation`
- 에러: `invalid_resource_type` → `invalidate_display_cache` · `rate_limited`(`all`을 한 시간에 너무 자주) → `invalidate_display_cache`
- 멱등성: 같은 범위를 다시 비워도 무해
