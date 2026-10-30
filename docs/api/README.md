# A-Commerce API 명세

A-Commerce의 전체 API(에이전트가 호출하는 툴과 사람 전용 엔드포인트)를 계층 구조로 정리한 명세입니다. 설계 배경은 [../architecture.md](../architecture.md), 해커톤 MVP 계약(툴 시그니처·시드·UI 블록)은 [../plan.md](../plan.md)를 따릅니다. **MVP 툴의 이름·입출력은 plan.md §3과 같고**, 이 명세는 그 툴들을 전체 계층 안의 자리에 두고 나머지 도메인까지 확장합니다.

- 데이터 모델(엔티티 ↔ schema.org, DB 명명): [data-model.md](data-model.md)
- 정책 데이터(규칙을 YAML로): [policies/](policies/README.md)

---

## 1. 계층 구조

모든 오퍼레이션은 **group → domain → resource → operation** 4단계로 주소화합니다.

```
a-commerce API
├── Offer      판매하는 것
│   ├── catalog          상품·변형·가격·재고·브랜드·리뷰·문의
│   ├── merchandising    카테고리·진열 슬롯·배너·팝업·기획전
│   └── search           검색·비교·자동완성·사전·색인
├── Transact   장바구니에서 배송까지
│   ├── sales            장바구니·주문 준비·주문·결제·영수증·구독·선물
│   └── fulfillment      클레임(취소·반품·교환)·검수·환불·배송·배송비
├── Engage     고객과 혜택
│   ├── promotion        프로모션·회원가·쿠폰·혜택 계산·적립금·이벤트·세그먼트
│   ├── customer         회원·등급·주소·동의·기억(선호)·위시리스트·상담·보상·FAQ
│   ├── messaging        고객 메시지·템플릿·캠페인·알림
│   └── community        게시판·글·댓글
└── Operate    스토어 운영
    ├── platform         인증·스토어(테넌트)·의미 enum·정책·변경셋·액션 로그·권한
    ├── analytics        매출·고객·상담 집계(읽기)
    ├── partner          입점 판매자·계약·등급
    └── integration      외부 채널 연계·대량 가져오기·택배/결제 이벤트 수신
```

| 수준 | 의미 | 예 |
|---|---|---|
| group | 비즈니스의 큰 부분 | Engage |
| domain | 자기 규칙·정책을 가진 영역 | `promotion` |
| resource | schema.org 타입 또는 그 확장 | `offer`(회원가 = `UnitPriceSpecification`) |
| operation | 하나의 비즈니스 동사 = 툴 | `set_sale_prices` |

하나의 오퍼레이션 정의에서 세 가지 이름이 생성됩니다.

| 표면 | 형식 | 예 |
|---|---|---|
| 논리 주소 | `domain.resource.operation` | `promotion.offer.set_sale_prices` |
| 툴 이름(모델이 보는 이름) | `verb_noun`, 평이한 영어 | `set_sale_prices` |
| REST 경로 | `METHOD /v1/{domain}/{resources}/...` | `POST /v1/promotion/sale-prices` |

툴 검색은 이 계층을 씁니다. 요청 → 해당 도메인 → 리소스 → 후보 툴 몇 개 + 툴 그래프 이웃. 모델은 200개 목록이 아니라 소수 후보만 봅니다.

## 2. 오퍼레이션 종류

| 표기 | 종류 | 누가 부르나 | 동작 |
|---|---|---|---|
| **R** | read | 에이전트 | 부작용 없음. 목록은 `items`와 `total`을 한 번에. `fields`로 응답 축소 |
| **W** | write | 에이전트 | `preview=true` 먼저. 프리뷰는 변경셋에 들어가고, 정책상 승인이 필요 없으면(예: 장바구니 담기, 쇼퍼 선호 기억) 바로 실행 |
| **W$** | write, 항상 승인 | 에이전트 | 돈이 움직이거나 고객에게 닿는 쓰기. 프리뷰 → 변경셋 → **사람 승인** 후 실행기가 `preview_id`로 실행 |
| **사람 전용** | HTTP 엔드포인트 | UI 버튼만 | 변경셋 승인·되돌리기, 주문하기, 반품 요청 보내기, 기억 되돌리기 등. **툴이 아니며 모델은 보지 못함** |

사람 전용 엔드포인트([plan.md §3.6](../plan.md)와 같은 목록):

| 엔드포인트 | 누가 | 무엇 |
|---|---|---|
| `POST /change-sets/{id}/approve` `{version}` | 운영자 | 실행 시작. 최신 버전이 아니면 거부 |
| `POST /change-sets/{id}/undo` | 운영자 | 변경셋의 모든 변경을 함께 되돌림 |
| `POST /orders` `{order_preview_id}` | 쇼퍼 | 주문하기 |
| `POST /returns` `{order_id, line_id, option}` | 쇼퍼 | 반품 요청 보내기 |
| `POST /memories/{id}/undo` | 쇼퍼 | 기억한 선호 삭제 |
| `POST /orders/{order_id}/cancel` | 쇼퍼 | 주문 전체·일부 취소 요청(`get_cancel_options` 결과 기반) |
| `POST /exchanges` | 쇼퍼 | 교환 요청(`get_exchange_options` 결과 기반) |
| `POST /claims/{claim_id}/pay-charge` | 쇼퍼 | 클레임 진행 전 추가 비용 결제 |
| `POST /claims/{claim_id}/withdraw` | 쇼퍼 | 반품·교환 요청 철회 |
| `POST /me/withdrawal` | 회원 본인 | 탈퇴(재인증 필수, `check_withdrawal_eligibility` 결과 기반) |
| `POST /password-reset/requests`, `POST /password-reset/confirm` | 회원 본인 | 비밀번호 재설정(계정 존재 여부와 무관하게 같은 응답) |
| `POST /account-recovery/login-id` | 회원 본인 | 로그인 아이디 찾기 |
| `PUT /me/password` | 회원 본인 | 로그인 상태에서 비밀번호 변경 |

취소·반품·교환 요청은 금액이 서버 재계산과 다르면 `estimate_changed`로 거부됩니다. 에이전트는 `get_*_options`로 준비만 하고, 보내는 것은 사람입니다. 비밀번호는 어떤 툴도 다루지 않습니다.

## 3. 공통 규약

### 3.1 이름
- 타입은 schema.org 이름(PascalCase), 확장은 `acom:` 네임스페이스.
- 필드는 schema.org 프로퍼티 이름을 **snake_case**로(`validFrom` → `valid_from`). JSON-LD 컨텍스트가 schema.org 용어로 되돌려 매핑합니다.
- enum 값은 schema.org 열거 멤버를 snake_case 단어로(`InStock` → `in_stock`). **코드값 금지.**
- 식별자는 읽을 수 있는 접두 id(`prd_windbreaker`, `promo_1042`).
- 금액은 소수 둘째 자리 숫자 + `price_currency`(데모 스토어 USD). 날짜는 ISO 8601, 스토어 시간대.
- 경로 버전은 `/v1`. 파괴적 변경만 `/v2`로 올리고, 같은 기능의 버전 복제본은 두지 않습니다.

### 3.2 공통 헤더
| 헤더 | 필수 | 설명 |
|---|---|---|
| `Authorization: Bearer <jwt>` | ✓ | 역할·스코프 포함 |
| `X-Tenant-Id` | ✓ | 스토어 식별. 토큰 클레임과 일치해야 함 |
| `Idempotency-Key` | write | 재시도 시 첫 결과 반환 |
| `X-Request-Id` | 권장 | 추적·액션 로그 상관관계 |

### 3.3 응답 형식 ([plan.md §3.1](../plan.md)과 동일)

```jsonc
// read
{"items": [...], "total": 42}

// write, preview=true
{"preview_id": "pv_8f2c",
 "changes": [{"op": "update", "entity": "price", "id": "prd_windbreaker", "from": 89.00, "to": 71.20}],
 "checks": [{"rule": "price_floor", "passed": true, "detail": "Overshirt moved to 15% to stay at cost + 15%", "fix_tool": null}],
 "needs_approval": true,
 "approval_reason": "Changes 40 prices"}

// write, preview_id로 실행 (승인 후 실행기만 보냄)
{"ok": true, "id": "promo_1042", "undo": {"tool": "undo_change_set", "until": "2026-11-09T23:59:00"}}

// error
{"error": {"code": "below_price_floor", "message": "At 20% the Seersucker overshirt would sell below cost + 15%.", "next_tool": "set_sale_prices"}}
```

- 목록 읽기는 `limit`·`offset`. 큰 목록은 선택적으로 `next_cursor`.
- 에러 `code`는 소문자 snake_case 단어, `message`는 사람이 읽는 문장, `next_tool`은 다음에 시도할 툴.
- `checks[].rule`은 [policies/](policies/README.md)의 정책 키에 대응합니다.

### 3.4 오퍼레이션 명세 형식
도메인 파일의 각 오퍼레이션은 다음 항목을 씁니다: 논리 주소 · REST · 요약 · consumes → produces · 입력 · 출력 · 프리뷰(쓰기) · 정책 · 에러 · 멱등성.

## 4. 전체 오퍼레이션 인덱스

**굵게** 표시한 것은 해커톤 MVP 툴입니다(27개, [plan.md §3](../plan.md)). 위치는 [architecture.md §3.4](../architecture.md)와 같습니다.

### Offer

**[catalog](catalog.md)**
| resource | operations |
|---|---|
| product_group | `search_product_groups` R · `get_product_group` R · `create_product_group` W · `update_product_group` W |
| product | **`get_product`** R · **`get_product_measurements`** R · `create_product` W · `update_product` W · `set_variants` W · `discontinue_product` W · `get_return_policy` R · `get_delivery_policy` R · `batch_import` W$ · `generate_image` W$ · `generate_copy` W |
| offer | `get_price_breakdown` R · `set_price` W · `schedule_price_change` W |
| inventory | `get_inventory` R · `set_variant_stock` W · `set_safety_stock` W |
| approval | `submit_for_review` W · `approve_product` W$ · `reject_product` W · `get_change_history` R |
| brand | `list_brands` R · `get_brand` R · `create_brand` W · `update_brand` W |
| review | **`get_review_summary`** R · `list_reviews` R · `moderate_review` W · `create_review` W · `update_review` W · `delete_review` W · `vote_review_helpful` W · `report_review` W |
| question | `list_questions` R · `answer_question` W · `moderate_question` W · `ask_question` W · `update_question` W · `delete_question` W |
| bundle | `create_bundle` W |
| restock_alert | `subscribe_restock` W · `list_restock_subscriptions` R |

**[merchandising](merchandising.md)**
| resource | operations |
|---|---|
| category | `list_categories` R · `get_category` R · `create_category` W · `update_category` W · `map_category` W · `unmap_category` W |
| display_slot | **`list_display_slots`** R · `get_display_slot` R · `arrange_display_slot` W · `schedule_display` W$ |
| banner | **`schedule_banner`** W$ · `list_banners` R · `set_banner` W |
| popup | `list_popups` R · `set_popup` W |
| collection_page | `list_curations` R · `get_curation` R · `create_curation` W · `update_curation` W · `publish_curation` W$ |
| cache | `invalidate_display_cache` W |

**[search](search.md)**
| resource | operations |
|---|---|
| product | **`search_products`** R · **`compare_products`** R · `autocomplete` R · `get_facets` R |
| lexicon | `get_lexicon` R · `update_lexicon` W |
| index | `reindex` W · `get_index_status` R |

### Transact

**[sales](sales.md)**
| resource | operations |
|---|---|
| cart | **`get_cart`** R · **`add_to_cart`** W · `update_cart_item` W · `remove_cart_item` W · `clear_cart` W |
| checkout | `get_payment_methods` R |
| order | **`prepare_order`** W(프리뷰만) · **`get_my_orders`** R · **`get_order`** R · `search_orders` R · `verify_guest` W · `change_shipping_address` W · 사람 전용 `POST /orders` |
| payment | `get_payment` R · `confirm_virtual_account_deposit` W$ |
| receipt | `issue_receipt` W · `get_receipt` R |
| subscription | `prepare_subscription` W(프리뷰만) · `pause_subscription` W · `resume_subscription` W · `cancel_subscription` W · `update_schedule` W · `list_subscriptions` R · 사람 전용 `POST /subscriptions` |
| installment | `set_installment` W |
| gift_order | `prepare_gift_order` W · 사람 전용 `POST /gift-orders/{id}/accept`(받는 사람) |

**[fulfillment](fulfillment.md)**
| resource | operations |
|---|---|
| claim | **`search_return_requests`** R · **`evaluate_return_request`** R · **`get_return_options`** R · `get_cancel_options` R · `get_exchange_options` R · `search_claims` R · `get_claim` R · `cancel_order_lines` W$(스토어가 시작하는 취소) · `approve_claim` W$ · `reject_claim` W · `convert_exchange_to_return` W · `upload_evidence` W · 사람 전용 `POST /orders/{id}/cancel`, `POST /exchanges`, `POST /returns`, `POST /claims/{id}/pay-charge`, `POST /claims/{id}/withdraw` |
| inspection | `assess_return_condition` W |
| refund | **`refund_return`** W$ · `get_refund` R · `retry_refund` W$ |
| parcel_delivery | **`get_delivery_dates`** R · `get_delivery` R · `track_shipment` R · `schedule_shipment` W · `confirm_shipment` W · `register_shortage` W |
| shipping_rate | `list_shipping_zones` R · `manage_shipping_zone` W · `set_shipping_rate` W |

### Engage

**[promotion](promotion.md)**
| resource | operations |
|---|---|
| promotion | **`list_active_promotions`** R · `search_promotions` R · `get_promotion` R · `create_promotion` W$ · `update_promotion` W · `pause_promotion` W · `end_promotion` W |
| offer | **`set_sale_prices`** W$ (회원·기간 한정 판매가) |
| coupon | **`create_coupon`** W$ · `issue_coupon` W$ · `redeem_coupon` W · `restore_coupon` W · `revoke_coupon` W · `search_coupons` R · `get_coupon` R · `list_member_coupons` R |
| benefit | `calculate_benefits` R (장바구니 혜택 계산의 단일 기준) |
| conditional_gift | `create_conditional_gift` W · `list_conditional_gifts` R |
| loyalty | `get_loyalty_balance` R · `list_loyalty_transactions` R · `grant_loyalty` W$ · `spend_loyalty` W · `expire_loyalty` W |
| event | `list_event_types` R · `list_reward_types` R · `search_events` R · `get_event` R · `create_event` W · `update_event` W · `enter_event` W · `list_my_entries` R · `run_draw` W$ · `get_draw_record` R · `dispatch_reward` W$ — 참여 방식·당첨 방식·보상 종류는 확장 가능한 레지스트리 |
| segment | `define_segment` W · `preview_segment` R · `list_segments` R |

**[customer](customer.md)**
| resource | operations |
|---|---|
| person | **`get_my_profile`** R · **`get_return_history`** R · `search_members` R · `get_member` R · `register_member` W · `update_member` W · `block_member` W$ · `unblock_member` W$ · `verify_identity` W · `check_availability` R · `verify_business` W$ · `check_withdrawal_eligibility` R · `send_password_reset` W$ · 사람 전용 `POST /me/withdrawal`, `POST /password-reset/requests`, `POST /password-reset/confirm`, `POST /account-recovery/login-id`, `PUT /me/password` |
| membership | **`list_tiers`** R · `get_tier` R · `get_member_tier` R · `define_tier_program` W$ · `preview_tier_evaluation` R · `run_tier_evaluation` W$ · `set_member_tier` W$ · `list_tier_history` R — 등급 수·조건·혜택은 스토어가 데이터로 정의 |
| address | `list_addresses` R · `add_address` W · `update_address` W · `set_primary_address` W |
| consent | `get_consent` R · `set_consent` W |
| preference | **`update_my_preferences`** W(승인 없음) · `get_preferences` R · 사람 전용 `POST /memories/{id}/undo` |
| wishlist | `list_wishlist` R · `add_to_wishlist` W · `remove_from_wishlist` W |
| employee | `register_employee` W · `set_discount_limit` W |
| inquiry | `search_inquiries` R · `get_inquiry` R · `create_inquiry` W · `assign_inquiry` W · `respond_inquiry` W$ · `close_inquiry` W |
| remedy | `request_remedy` W · `approve_remedy` W$ · `pay_remedy` W$ |
| faq_notice | `list_faqs` R · `get_faq` R · `list_notices` R · `get_notice` R · `publish_notice` W |

**[messaging](messaging.md)**
| resource | operations |
|---|---|
| message | **`send_customer_message`** W$ · `send_bulk` W$ |
| template | `create_message_template` W · `list_message_templates` R |
| campaign | `create_campaign` W$ · `search_campaigns` R · `get_campaign` R |
| notification | `list_notifications` R |

**[community](community.md)**
| resource | operations |
|---|---|
| post | `list_posts` R · `get_post` R · `create_post` W · `update_post` W · `delete_post` W · `report_post` W |
| reply | `list_replies` R · `create_reply` W · `update_reply` W · `delete_reply` W |
| board | `list_boards` R · `configure_board` W |

### Operate

**[platform](platform.md)**
| resource | operations |
|---|---|
| policy | **`get_policy`** R · `list_policies` R |
| change_set | **`verify_change_set`** R · 사람 전용 `approve`, `undo` |
| action_log | `search_action_log` R · `get_action_log` R |
| enum | `list_enums` R |
| auth | `login` W · `refresh_token` W · `logout` W |
| tenant | `get_tenant` R · `list_tenants` R |
| role | `list_roles` R · `get_role` R · `create_role` W · `assign_role` W$ · `list_permissions` R |
| menu | `list_menus` R · `set_menu` W · `set_favorite` W |
| admin_user | `search_admin_users` R · `create_admin_user` W$ · `set_admin_status` W$ |
| org | `list_orgs` R · `set_org` W |
| api_key | `list_api_keys` R · `issue_api_key` W$ · `revoke_api_key` W$ |

**[analytics](analytics.md)** (읽기 전용)
| resource | operations |
|---|---|
| sales | `explain_sales_change` R · `get_sales_overview` R · `get_sales_trend` R |
| customer | `get_customer_overview` R |
| service | `get_cs_overview` R |

**[partner](partner.md)**
| resource | operations |
|---|---|
| seller | `search_sellers` R · `get_seller` R · `onboard_seller` W$ · `update_seller` W · `set_seller_status` W$ |
| contract | `get_contract` R · `set_contract` W$ |
| grade | `get_seller_grade` R · `set_seller_grade` W |
| dashboard | `get_seller_overview` R |

**[integration](integration.md)**
| resource | operations |
|---|---|
| product_import | `import_products` W$ · `get_import_status` R |
| order | `lookup_external_order` R · `sync_order_status` W |
| claim | `lookup_external_claim` R · `sync_claim_status` W |
| channel | `list_channels` R · `register_channel` W$ |
| event | `receive_carrier_event` W · `receive_payment_event` W |

## 5. 툴 그래프

각 오퍼레이션의 `consumes → produces`에서 생성됩니다. MVP 흐름의 핵심 간선:

```
search_products ──product_id[]──▶ set_sale_prices ──promotion_id──▶ create_coupon
list_tiers ─────────tier────────▶ set_sale_prices                   │
list_display_slots ──slot_id──▶ schedule_banner ◀──promotion_id─────┘
set_sale_prices, create_coupon, schedule_banner ──change_set_id──▶ verify_change_set

search_products ──▶ compare_products ──▶ add_to_cart ──cart_id──▶ get_cart ──▶ prepare_order ──▶ (사람) POST /orders
search_return_requests ──▶ evaluate_return_request ──▶ refund_return ──▶ send_customer_message
get_my_orders ──order_id──▶ get_return_options ──▶ (사람) POST /returns
```

## 6. 정책과의 연결

쓰기 프리뷰의 `checks`는 정책 파일에서 옵니다. 대표 대응:

| check `rule` | 정책 | 적용 오퍼레이션 |
|---|---|---|
| `max_discount` | [promotion.yml](policies/promotion.yml) | `set_sale_prices`, `create_promotion` |
| `price_floor` | [catalog.yml](policies/catalog.yml) | `set_sale_prices`, `set_price` |
| `min_stock` | [promotion.yml](policies/promotion.yml) | `set_sale_prices` |
| `promotion_overlap` | [promotion.yml](policies/promotion.yml) | `set_sale_prices`, `create_promotion` |
| `coupon_stacking` | [promotion.yml](policies/promotion.yml) | `create_coupon` |
| `slot_free` | [merchandising.yml](policies/merchandising.yml) | `schedule_banner` |
| `refund_to_original_payment`, `coupon_deducted` | [fulfillment.yml](policies/fulfillment.yml) | `refund_return` |
| 승인 필요 여부 | [approval.yml](policies/approval.yml) | 모든 W·W$ |
