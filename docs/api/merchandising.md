# API 명세 — `merchandising` (카테고리·진열 슬롯·배너·팝업·기획전·캐시)

> 계층: `merchandising.{resource}.{operation}`. 공통 규약(응답 형식·에러·프리뷰/실행·멱등·헤더)은 [README.md](README.md) §3, 데이터 타입은 [data-model.md](data-model.md), 정책은 [merchandising.yml](policies/merchandising.yml)·[approval.yml](policies/approval.yml). MVP 툴(`list_display_slots`, `schedule_banner`)의 시그니처는 [../plan.md](../plan.md) §3.2와 같고, 계층 위치는 [../architecture.md](../architecture.md) §3.4를 따릅니다.
>
> **설계 축**: ① 표준 카테고리(내부 분류)와 전시 카테고리(쇼퍼에게 보이는 트리)를 나누고 `acom:CategoryMapping`으로 N:M 연결. ② 노출 여부는 배치로 켜고 끄지 않고 `valid_from`·`valid_through`로 그때그때 계산. 시각 비교는 툴 안에서만 하고 에이전트가 직접 하지 않음. ③ 노출 = 기간 ∧ 재고 ∧ 판매 가능 상태. ④ 쇼퍼에게 닿는 공개(배너 예약, 슬롯 예약, 기획전 공개)는 `W$`(변경셋 승인). ⑤ 전시 변경은 캐시를 명시적으로 무효화해야 쇼퍼 화면에 반영.
>
> **응답 형식**: 읽기 `{items, total}`, 프리뷰 `{preview_id, changes[{op, entity, id, from, to}], checks[{rule, passed, detail, fix_tool}], needs_approval, approval_reason}`, 실행 `{ok, id, undo{tool, until}}`, 에러 `{error: {code, message, next_tool}}`. 목록은 `limit`·`offset`. 날짜는 ISO 8601, 스토어 시간(US Eastern).

리소스: [category](#category) · [display_slot](#display_slot) · [banner](#banner) · [popup](#popup) · [collection_page](#collection_page) · [cache](#cache)

---

## category

> 표준 카테고리와 전시 카테고리는 서로 다른 `DefinedTermSet`입니다. 상품은 표준 카테고리에 한 번 분류되고, 전시 트리는 자유롭게 구성되며 `acom:CategoryMapping`(N:M)으로 표준과 연결됩니다. 삭제는 하위 카테고리·매핑·상품이 없을 때만 됩니다.

### `list_categories` — Kind: R
- 논리주소/REST: `merchandising.category.list` · `GET /v1/merchandising/categories`
- 요약: 카테고리 트리를 돌려줍니다. 부모가 비활성이면 자식도 비활성으로 보입니다.
- consumes → produces: → `category_id[]`
- 입력: `term_set`(`standard` | `display`), `parent_id?`, `depth?`, `fields?[]`, `limit=50`, `offset=0`
- 출력: `{items: [{id: "cat_summer_outerwear", name: "Summer outerwear", term_code: "summer_outerwear", in_defined_term_set: "standard", acom:parent: "cat_outerwear", acom:display_order: 3, acom:active: true}], total: 24}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_term_set` → `list_categories`
- 멱등성: 읽기(부작용 없음)

### `get_category` — Kind: R
- 논리주소/REST: `merchandising.category.get` · `GET /v1/merchandising/categories/{category_id}`
- 요약: 카테고리 하나를 매핑 요약과 상품 수와 함께 돌려줍니다.
- consumes → produces: `category_id` → category
- 입력: `category_id`, `fields?[]`
- 출력: `{items: [{..., acom:mappings: CategoryMapping[], acom:product_count: 42}], total: 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `category_not_found` → `list_categories`
- 멱등성: 읽기(부작용 없음)

### `create_category` — Kind: W
- 논리주소/REST: `merchandising.category.create` · `POST /v1/merchandising/categories`
- 요약: 표준 또는 전시 카테고리 노드를 만듭니다.
- consumes → produces: `parent_id?` → `category_id`
- 입력: `term_set`(`standard` | `display`), `name`, `acom:parent?`, `acom:display_order?`, `term_code?`(의미 있는 영단어, 예 `"summer_outerwear"`), `preview`
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "cat_summer_outerwear", undo}`
- 프리뷰: `changes`=`create` `category`. checks `max_depth`, `term_code_unique`. `needs_approval: false`.
- 정책: [merchandising.yml](policies/merchandising.yml) `category`
- 에러: `parent_not_found` → `list_categories` · `depth_exceeded` → `update_category` · `duplicate_term_code` → `create_category`
- 멱등성: `Idempotency-Key`

### `update_category` — Kind: W
- 논리주소/REST: `merchandising.category.update` · `PATCH /v1/merchandising/categories/{category_id}`
- 요약: 이름·표시 순서·부모·활성 여부를 바꿉니다. 비활성(`acom:active: false`)은 하위·매핑·상품이 없을 때만.
- consumes → produces: `category_id` → `category_id`
- 입력: `category_id`, `name?`, `acom:display_order?`, `acom:parent?`, `acom:active?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: checks `has_children`, `has_mappings`, `has_products`(하나라도 `passed: false`면 비활성 불가), `no_cycle`. `needs_approval: false`.
- 정책: [merchandising.yml](policies/merchandising.yml) `category.delete_only_if`, `category.child_inherits_inactive_parent`
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
  | display_category_ids | Text[] | ✓ | 전시 카테고리, 예 `["dsp_new_in", "dsp_travel"]` |
  | is_primary | Boolean | | 표준 카테고리마다 대표 전시 카테고리 하나(상세의 경로 표시에 씀) |
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "map_221", undo}`
- 프리뷰: `changes`=`create` `category_mapping`. checks `cross_set_only`(표준 → 전시 방향만), `single_primary`(대표 매핑 하나). `detail`에 새로 보이게 될 상품 수. `needs_approval: false`.
- 정책: [merchandising.yml](policies/merchandising.yml) `category.mapping`, `category.primary_per_standard_per_mall`
- 에러: `cross_set_violation` → `map_category` · `primary_conflict` → `unmap_category` · `display_category_inactive` → `update_category`
- 멱등성: `(standard_category_id, display_category_id)`로 한 건

### `unmap_category` — Kind: W
- 논리주소/REST: `merchandising.category.unmap` · `DELETE /v1/merchandising/category-mappings/{category_mapping_id}`
- 요약: 표준·전시 매핑을 지웁니다. 대표 매핑을 지우면 남은 매핑 중 하나를 대표로 다시 정해야 합니다. 실행 후 `invalidate_display_cache`.
- consumes → produces: `category_mapping_id` →
- 입력: `category_mapping_id`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: `changes`=`delete` `category_mapping`. `detail`에 더는 보이지 않을 상품 수. `needs_approval: false`.
- 정책: [merchandising.yml](policies/merchandising.yml) `category`
- 에러: `mapping_not_found` → `list_categories` · `primary_needs_reassign` → `map_category`
- 멱등성: 이미 지운 매핑은 같은 결과

---

## display_slot

> 진열 슬롯은 `acom:DisplaySlot`(`ItemList`의 하위 타입)입니다. `item_list_element`에 상품·배너·기획전을 순서대로 담고, 예약(booking)마다 `valid_from`·`valid_through`와 `valid_for_member_tier`를 가집니다. 노출 = 기간 ∧ 재고 ∧ 판매 가능 상태로 그때그때 계산합니다.

### `list_display_slots` — Kind: R  (MVP)
- 논리주소/REST: `merchandising.display_slot.list` · `GET /v1/merchandising/display-slots`
- 요약: 한 페이지의 슬롯과 기간 동안 무엇이 누구에게 예약돼 있는지 돌려줍니다. `schedule_banner` 앞에서 빈 슬롯을 찾을 때 씁니다.
- consumes → produces: → `slot_id`
- 입력: `page`(`"main"`), `valid_from`, `valid_through`, `limit=20`, `offset=0`
- 출력:
```jsonc
{"items": [{
  "slot_id": "slot_main_hero", "page": "main", "name": "Hero",
  "bookings": [
    {"display_id": "dsp_fall_layers", "headline": "Fall layers", "valid_for_member_tier": "everyone",
     "valid_from": "2026-11-06T08:10:00", "valid_through": null}],
  "free_for": ["vip"]
 }], "total": 4}
```
- 프리뷰: 해당 없음(읽기)
- 정책: [merchandising.yml](policies/merchandising.yml) `display`
- 에러: `page_not_found` → `list_display_slots` · `invalid_period` → `list_display_slots`
- 멱등성: 읽기(부작용 없음)

### `get_display_slot` — Kind: R
- 논리주소/REST: `merchandising.display_slot.get` · `GET /v1/merchandising/display-slots/{slot_id}`
- 요약: 슬롯 하나를 담긴 항목과 특정 시각 기준 노출 상태와 함께 돌려줍니다.
- consumes → produces: `slot_id` → display_slot
- 입력: `slot_id`, `as_of?`(DateTime), `fields?[]`
- 출력: `{items: [{slot_id, item_list_element: [], bookings: [], acom:live_now: true, acom:hidden_items: [{product_id, reason: "out_of_stock"}]}], total: 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: [merchandising.yml](policies/merchandising.yml) `display.visibility_formula`
- 에러: `slot_not_found` → `list_display_slots`
- 멱등성: 읽기(부작용 없음)

### `arrange_display_slot` — Kind: W
- 논리주소/REST: `merchandising.display_slot.arrange` · `PUT /v1/merchandising/display-slots/{slot_id}/items`
- 요약: 슬롯 구성(`item_list_element`)을 더하고 빼고 순서를 바꿉니다. 기간과 대상은 건드리지 않습니다(그것은 `schedule_display`). 공개를 바꾸지 않으므로 `W`.
- consumes → produces: `slot_id`, `product_id[]` → `slot_id`
- 입력: `slot_id`, `item_list_element`(`{position, item}` 배열), `acom:slot_type?`(`product_grid` | `brand_strip` | `banner_board` | `collection_feed`), `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: checks `max_items`, `no_discontinued_items`. `needs_approval: false`.
- 정책: [merchandising.yml](policies/merchandising.yml) `display`
- 에러: `item_discontinued` → `search_products` · `duplicate_position` → `arrange_display_slot` · `slot_type_mismatch` → `get_display_slot`
- 멱등성: `Idempotency-Key`. 같은 구성은 다시 적용해도 변경 없음

### `schedule_display` — Kind: W$
- 논리주소/REST: `merchandising.display_slot.schedule` · `POST /v1/merchandising/display-slots/{slot_id}/bookings`
- 요약: 슬롯 구성 전체를 기간·대상 등급·매체에 예약합니다. 쇼퍼에게 닿으므로 항상 변경셋 승인을 거칩니다. 상태(`scheduled` → `active` → `ended`)는 기간으로 계산합니다. 배너 하나만 걸 때는 `schedule_banner`를 씁니다.
- consumes → produces: `slot_id` → `display_id`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명·예 |
  |------|----------------------|:--:|--------|
  | slot_id | Text | ✓ | 예 `"slot_main_hero"` |
  | valid_for_member_tier | Text | ✓ | `"everyone"` \| `"vip"` … |
  | valid_from | DateTime | ✓ | 과거 불가 |
  | valid_through | DateTime | ✓ | |
  | acom:media | enum[] | | `web` \| `mobile` \| `app` |
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "dsp_2210", undo: {tool: "undo_change_set", until: "..."}}`
- 프리뷰: `changes`=`create` `booking`. checks `slot_free`, `valid_from_not_in_past`, `no_discontinued_items`; `detail`에 숨겨질 품절 항목 수. 항상 `needs_approval: true`, 예 `approval_reason: "Shows on the main page to VIP members"`.
- 정책: [merchandising.yml](policies/merchandising.yml) `display`, `reaches_customers`; [approval.yml](policies/approval.yml) `display_schedule`
- 에러: `slot_taken` → `list_display_slots` · `valid_from_in_past` → `schedule_display` · `empty_slot` → `arrange_display_slot` · `tier_not_found` → `list_tiers`
- 멱등성: `Idempotency-Key = hash(slot_id + valid_for_member_tier + valid_from + valid_through)`

---

## banner

### `schedule_banner` — Kind: W$  (MVP)
- 논리주소/REST: `merchandising.banner.schedule` · `POST /v1/merchandising/banners`
- 요약: 슬롯에 배너(작은 머리글·제목·버튼·링크)를 대상 등급과 기간으로 예약합니다. 프로모션과 함께 걸면 그 프로모션 기간에만 보입니다. 쇼퍼에게 닿으므로 항상 변경셋 승인을 거치고, 승인 후 실행기가 `preview_id`로 실행합니다.
- consumes → produces: `slot_id`, `promotion_id` → `display_id`
- 입력:
  | 필드 | 타입(schema.org/acom) | 필수 | 설명·예 |
  |------|----------------------|:--:|--------|
  | slot_id | Text | ✓ | `list_display_slots`가 준 슬롯, 예 `"slot_main_hero"` |
  | promotion_id | Text | | 연결할 프로모션, 예 `"promo_1042"` |
  | valid_for_member_tier | Text | ✓ | `"vip"`. 다른 등급은 기존 배너를 계속 봄 |
  | valid_from | DateTime | ✓ | `"2026-11-07T00:00:00"` |
  | valid_through | DateTime | ✓ | `"2026-11-08T23:59:00"` |
  | eyebrow | Text | ✓ | `"VIP WEEKEND"` |
  | headline | Text | ✓ | `"20% off summer outerwear"` |
  | cta | Text | ✓ | `"Shop the sale"` |
  | link | URL | ✓ | `"/sale/vip-weekend"` |
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력(프리뷰):
```jsonc
{"preview_id": "pv_b71e",
 "changes": [{"op": "schedule", "entity": "banner", "id": "slot_main_hero", "from": null,
              "to": {"valid_for_member_tier": "vip", "headline": "20% off summer outerwear",
                     "valid_from": "2026-11-07T00:00:00", "valid_through": "2026-11-08T23:59:00"}}],
 "checks": [{"rule": "slot_free", "passed": true, "detail": "Hero slot open Nov 7–8", "fix_tool": null}],
 "needs_approval": true,
 "approval_reason": "Shows on the main page to 1,284 VIP members"}
```
- 출력(실행): `{ok: true, id: "dsp_1042", undo: {tool: "undo_change_set", until: "2026-11-09T23:59:00"}}`
- 프리뷰: `changes`=`schedule` `banner` 한 건. check `slot_free`: 같은 슬롯·같은 등급·겹치는 기간에 다른 예약이 없어야 함. 실패하면 `detail`에 겹치는 배너, `fix_tool: "list_display_slots"`. 항상 `needs_approval: true`.
- 정책: [merchandising.yml](policies/merchandising.yml) `display`, `reaches_customers`; [approval.yml](policies/approval.yml) `display_schedule`
- 에러: `slot_taken` → `list_display_slots` · `slot_not_found` → `list_display_slots` · `valid_from_in_past` → `schedule_banner` · `promotion_not_found` → `list_active_promotions` · `tier_not_found` → `list_tiers`
- 멱등성: `Idempotency-Key = hash(slot_id + valid_for_member_tier + valid_from + valid_through)`. 다시 보내면 첫 결과

### `list_banners` — Kind: R
- 논리주소/REST: `merchandising.banner.list` · `GET /v1/merchandising/banners`
- 요약: 배너 목록(이미지·링크·매체·우선순위·기간).
- consumes → produces: → `banner_id[]`
- 입력: `page?`, `media?`, `live_at?`(DateTime), `limit=20`, `offset=0`
- 출력: `{items: [{id, content_url, link, acom:media, acom:priority, valid_from, valid_through, valid_for_member_tier}], total}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `page_not_found` → `list_banners`
- 멱등성: 읽기(부작용 없음)

### `set_banner` — Kind: W
- 논리주소/REST: `merchandising.banner.set` · `PUT /v1/merchandising/banners/{banner_id}`
- 요약: 배너 이미지 자산(`ImageObject`)을 등록·수정합니다. 공개는 `schedule_banner`·`schedule_display`로 일어나므로 자산 등록은 `W`입니다.
- consumes → produces: → `banner_id`
- 입력: `banner_id?`(없으면 새로), `content_url`, `link?`, `acom:media`(`web` | `mobile` | `app`), `acom:priority?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id: "bnr_vip_weekend", undo}`
- 프리뷰: checks `image_spec`(크기·용량), `priority_unique`. 노출 중인 배너를 바꾸면 쇼퍼에게 닿으므로 `needs_approval: true`, 아니면 `false`.
- 정책: [merchandising.yml](policies/merchandising.yml) `reaches_customers`
- 에러: `image_spec_violation` → `set_banner` · `priority_conflict` → `list_banners`
- 멱등성: `Idempotency-Key` + `banner_id`

---

## popup

### `list_popups` — Kind: R
- 논리주소/REST: `merchandising.popup.list` · `GET /v1/merchandising/popups`
- 요약: 팝업 목록(종류·대상 화면·우선순위·기간).
- consumes → produces: → `popup_id[]`
- 입력: `popup_type?`, `page?`, `live_at?`, `limit=20`, `offset=0`
- 출력: `{items: [{id, popup_type, pages, acom:priority, valid_from, valid_through}], total}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_popup_type` → `list_popups`
- 멱등성: 읽기(부작용 없음)

### `set_popup` — Kind: W
- 논리주소/REST: `merchandising.popup.set` · `PUT /v1/merchandising/popups/{popup_id}`
- 요약: 팝업(`acom:Popup`, `WebPageElement`의 하위 타입)을 등록·수정합니다. "오늘 그만 보기" 일수를 정할 수 있습니다.
- consumes → produces: → `popup_id`
- 입력: `popup_id?`, `popup_type`(`image` | `notice` | `event` | `consent`), `content`(ImageObject | Text), `pages[]`, `acom:priority?`, `acom:suppress_days?`, `valid_from`, `valid_through`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: checks `max_concurrent_per_page`, `priority_unique`, `valid_from_not_in_past`. 쇼퍼에게 닿으므로 `needs_approval: true`.
- 정책: [merchandising.yml](policies/merchandising.yml) `reaches_customers`
- 에러: `too_many_popups` → `list_popups` · `priority_conflict` → `list_popups` · `valid_from_in_past` → `set_popup`
- 멱등성: `Idempotency-Key` + `popup_id`

---

## collection_page

> 기획전은 `CollectionPage` + 섹션별 `ItemList`입니다. 상태(`draft` → `scheduled` → `active` → `ended`)는 기간으로 계산합니다.

### `list_curations` — Kind: R
- 논리주소/REST: `merchandising.collection_page.list` · `GET /v1/merchandising/collection-pages`
- 요약: 기획전 목록(상태·기간·대상 등급).
- consumes → produces: → `collection_page_id[]`
- 입력: `status?`(`draft` | `scheduled` | `active` | `ended`), `valid_for_member_tier?`, `live_at?`, `limit=20`, `offset=0`
- 출력: `{items: [{id, name, status, valid_for_member_tier, valid_from, valid_through}], total}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_status` → `list_curations`
- 멱등성: 읽기(부작용 없음)

### `get_curation` — Kind: R
- 논리주소/REST: `merchandising.collection_page.get` · `GET /v1/merchandising/collection-pages/{collection_page_id}`
- 요약: 기획전 하나(섹션·상품·연결 배너·검색 키워드·현재 상태).
- consumes → produces: `collection_page_id` → collection_page
- 입력: `collection_page_id`, `fields?[]`
- 출력: `{items: [{id, name, has_part: ItemList[], status, valid_for_member_tier, valid_from, valid_through, acom:banner_ids, keywords, acom:hidden_items}], total: 1}`
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
  | has_part | ItemList[] | ✓ | 섹션마다 `item_list_element` = `product_id[]` |
  | valid_for_member_tier | Text | | `"everyone"` 기본 |
  | valid_from / valid_through | DateTime | | 공개 때 확정 |
  | acom:banner_ids | Text[] | | 연결 배너 |
  | keywords | Text[] | | 검색 키워드 |
  | preview | Boolean | ✓ | `true` 먼저 |
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "col_winter_edit", undo}`
- 프리뷰: `changes`=`create` `collection_page`, 섹션별 `create`. checks `no_discontinued_items`, `no_empty_section`. `needs_approval: false`.
- 정책: [merchandising.yml](policies/merchandising.yml) `curation`
- 에러: `product_discontinued` → `search_products` · `empty_section` → `update_curation` · `banner_not_found` → `list_banners`
- 멱등성: `Idempotency-Key`

### `update_curation` — Kind: W
- 논리주소/REST: `merchandising.collection_page.update` · `PATCH /v1/merchandising/collection-pages/{collection_page_id}`
- 요약: 초안·예정 기획전의 섹션·상품·배너·키워드·대상을 고칩니다. 상태 전환은 `publish_curation`으로만.
- consumes → produces: `collection_page_id` → `collection_page_id`
- 입력: `collection_page_id`, `name?`, `has_part?`, `valid_for_member_tier?`, `acom:banner_ids?`, `keywords?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok, id, undo}`
- 프리뷰: checks `no_discontinued_items`. `needs_approval: false`(이미 공개 중이면 `true`).
- 정책: [merchandising.yml](policies/merchandising.yml) `curation`
- 에러: `product_discontinued` → `search_products` · `already_ended` → `create_curation`
- 멱등성: `Idempotency-Key`

### `publish_curation` — Kind: W$
- 논리주소/REST: `merchandising.collection_page.publish` · `POST /v1/merchandising/collection-pages/{collection_page_id}/publish`
- 요약: 기획전 기간을 확정하고 공개를 예약합니다(`draft` → `scheduled`, 시작이 지금이면 `active`). 쇼퍼에게 닿으므로 항상 변경셋 승인.
- consumes → produces: `collection_page_id` → `collection_page_id`
- 입력: `collection_page_id`, `valid_from`(과거 불가), `valid_through`, `valid_for_member_tier?`, `acom:media?[]`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "col_winter_edit", undo: {tool: "undo_change_set", until: "..."}}`
- 프리뷰: `changes`=`update` `status` `draft` → `scheduled`. checks `valid_from_not_in_past`, `no_empty_section`, `no_discontinued_items`; `detail`에 섹션 수·상품 수·품절로 숨겨질 수. 항상 `needs_approval: true`, 예 `approval_reason: "Publishes a page with 86 products to everyone"`.
- 정책: [merchandising.yml](policies/merchandising.yml) `curation.publish_reaches_customers`; [approval.yml](policies/approval.yml) `display_schedule`
- 에러: `valid_from_in_past` → `update_curation` · `empty_curation` → `update_curation` · `already_published` → `get_curation` · `tier_not_found` → `list_tiers`
- 멱등성: `Idempotency-Key = hash(collection_page_id + valid_from + valid_through + valid_for_member_tier)`

---

## cache

### `invalidate_display_cache` — Kind: W
- 논리주소/REST: `merchandising.cache.invalidate` · `POST /v1/merchandising/cache/invalidate`
- 요약: 전시 캐시를 명시적으로 비웁니다. 카테고리 매핑·슬롯·배너·팝업·기획전 변경은 이 호출 뒤에 쇼퍼 화면에 반영됩니다. 범위를 좁혀 불필요한 재조회를 줄입니다. 승인된 변경셋의 실행기는 실행 끝에 자동으로 부릅니다.
- consumes → produces: `slot_id?`, `category_id?`, `collection_page_id?` → invalidation
- 입력: `resource_type`(`category` | `display_slot` | `banner` | `popup` | `collection_page` | `all`), `resource_id?`, `pages?[]`, `reason?`, `preview`
- 출력: 프리뷰 형식 → 실행 `{ok: true, id: "inv_5521", undo: null}`
- 프리뷰: `changes`=`invalidate` 대상 키 수. checks `scope_allowed`, `rate_limit`(`all` 남용 방지). `needs_approval: false`.
- 정책: [merchandising.yml](policies/merchandising.yml) `display.cache_invalidation`
- 에러: `invalid_scope` → `invalidate_display_cache` · `rate_limited` → `invalidate_display_cache`
- 멱등성: 같은 범위를 다시 비워도 무해
