# API 명세 — `sales` (장바구니·주문 준비·주문·결제·영수증·구독·할부·선물 주문)

> 계층: `sales.{resource}.{operation}`. 공통 규약(응답 형식·에러·프리뷰/실행·멱등·헤더)은 [README.md](README.md) §3과 [conventions.md](conventions.md), 값 목록은 [enums.md](enums.md), 에러는 [errors.md](errors.md), 데이터 타입은 [data-model.md](data-model.md). 정책은 [order.yml](../../policies/order.yml) · [promotion.yml](../../policies/promotion.yml) · [approval.yml](../../policies/approval.yml). 예시의 사람·상품·금액은 데모 스토어([../scenarios.md](../scenarios.md))의 값이고, 장바구니·주문 확인 화면 블록은 [chat.md](chat.md) §3.3의 `cart`, `checkout_card`, `order_detail`입니다.
> **설계 축**: ① 에이전트는 `prepare_order`·`prepare_subscription`·`prepare_gift_order`로 **준비만** 하고(프리뷰만, `needs_approval: false`), 주문하기는 쇼퍼가 누르는 사람 전용 `POST /v1/sales/orders`(모델은 보지 못함, [README.md](README.md) §2), ② 가격·할인은 서버가 `calculate_benefits`로 다시 계산한 값만 신뢰, ③ 주문·결제 쓰기는 멱등, ④ 주문 상태는 이벤트로 쌓음(덮어쓰기 금지).
> 금액: 줄 단가·소계·할인·배송비·합계는 `{value, currency}`. 할인 금액은 **양수**(빼는 금액)이고 합계 = 소계 − 할인 합 + 배송비입니다.

리소스: [cart](#cart) · [checkout](#checkout) · [order](#order) · [payment](#payment) · [receipt](#receipt) · [subscription](#subscription) · [installment](#installment) · [gift_order](#gift_order)

---

## cart

장바구니는 호출한 사람(로그인 쇼퍼 또는 비회원 세션)의 것 하나입니다. 장바구니 쓰기는 [approval.yml](../../policies/approval.yml) `no_approval`이라 프리뷰 뒤 바로 실행되고 변경셋에 들어가지 않으며, `undo`는 `null`입니다(되돌리기는 `update_cart_item`·`remove_cart_item`).

### `get_cart` — Kind: R  (MVP)
- 논리주소/REST: `sales.cart.get` · `GET /v1/sales/carts/mine`
- 요약: 호출한 쇼퍼 **본인의** 장바구니. 줄, 소계, 할인(여러 줄 가능), 배송비, 합계, 배송 예정, 결제 수단, 주문에 기록될 쿠폰을 한 번에 돌려줍니다. 할인·배송비는 서버가 `calculate_benefits`로 계산하며, 필드 이름은 [chat.md](chat.md)의 `cart` 블록과 같습니다. 장바구니가 없으면 빈 장바구니(`lines: []`)를 돌려줍니다.
- consumes → produces: → `cart_id`, `cart_line_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | fields | Text[] | | 돌려받을 최상위 필드만(conventions.md §6). 값: `id`, `lines`, `subtotal`, `discounts`, `shipping`, `total`, `delivery`, `payment`, `coupons_recorded` | `["lines", "total"]` |
- 출력: 장바구니 한 건(`Order` + `additionalType: acom:Cart`, 목록 봉투 없음)
  ```jsonc
  {"id": "cart_jamie",
   "customer_id": "cus_jamie",
   "lines": [{"cart_line_id": "cln_1", "product_id": "prd_windbreaker", "variant_id": "var_windbreaker_m_slate",
              "name": "Packable windbreaker", "color": "Slate", "size": "M", "quantity": 1,
              "price": {"value": 89.00, "currency": "USD"}, "note": null}],
   "subtotal": {"value": 89.00, "currency": "USD"},
   "discounts": [{"label": "VIP weekend", "amount": {"value": 17.80, "currency": "USD"},
                  "promotion_id": "promo_1042", "discount_layer": "item_price"}],
   "shipping": {"value": 0.00, "currency": "USD"},          // 할인 전 소계 $89.00 ≥ $80.00이라 무료
   "total": {"value": 71.20, "currency": "USD"},
   "delivery": {"address_id": "adr_home", "address_label": "Home · 12 Harbor St, Apt 3", "arrives": "2026-11-10"},
   "payment": {"payment_method": "credit_card", "label": "Card ending 4821"},
   "coupons_recorded": [{"coupon_id": "cpn_vipweekend20", "code": "VIPWEEKEND20", "promotion_id": "promo_1042"}]}
  ```
  - `lines[].price`는 정가(할인 전 단가), 할인은 `discounts[]`에 프로모션마다 한 줄. `discount_layer`는 `item_price` | `order_discount` | `add_on`([enums.md](enums.md) §1.11).
  - `coupons_recorded[]`는 주문에 기록될 쿠폰입니다. 프로모션 쿠폰은 혜택을 기록할 뿐 두 번째 할인을 더하지 않습니다(`coupon.records_benefit_only`).
  - `delivery.arrives`는 날짜(`YYYY-MM-DD`), 저장된 기본 배송지 기준.
- 프리뷰: 해당 없음(읽기)
- 정책: 혜택 계산 `discount_layers`, `member_benefits_per_order`, 배송비 `free_shipping_min_subtotal`
- 에러: 없음(공통 에러만). 장바구니가 없으면 빈 장바구니.
- 멱등성: 읽기(부작용 없음).

### `add_to_cart` — Kind: W (승인 없음)  (MVP)
- 논리주소/REST: `sales.cart.add` · `POST /v1/sales/carts/mine/lines`
- 요약: 상품을 사이즈·색과 함께(또는 변형 id로) 쇼퍼 본인 장바구니에 담습니다. 장바구니가 없으면 만듭니다. 상품 + 옵션을 받으면 변형을 찾아 결과에 `variant_id`를 돌려줍니다(conventions.md §9). 정책상 승인이 필요 없어(`no_approval`) 우리 에이전트의 런타임이 프리뷰 뒤 바로 실행합니다. schema.org `AddAction`.
- consumes → produces: `product_id` 또는 `variant_id` → `cart_id`, `cart_line_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | mode | enum | ✓ | `product_options`: `product_id` + `size` + `color`로 변형을 찾음; `variant`: `variant_id`를 그대로 씀 | `"product_options"` |
  | product_id | Text, `^prd_[a-z0-9_]+$` | mode=`product_options`일 때 ✓ | 담을 상품(`search_products`, `get_product`) | `"prd_windbreaker"` |
  | size | Text | mode=`product_options`일 때 ✓ | 상품의 사이즈 값(`get_product`의 `has_variant[].size`) | `"M"` |
  | color | Text | mode=`product_options`일 때 ✓ | 상품의 색 값(`has_variant[].color`) | `"Slate"` |
  | variant_id | Text, `^var_[a-z0-9_]+$` | mode=`variant`일 때 ✓ | 담을 변형 | `"var_windbreaker_m_slate"` |
  | order_quantity | Integer | | 1 이상 `cart.max_quantity_per_line` 이하. 기본 1. 같은 변형이 이미 있으면 그 줄의 수량을 이 값으로 맞춤 | `1` |
- 출력(프리뷰):
  ```jsonc
  {"preview_id": "pv_c1a7", "expires_at": "2026-11-07T10:38:00-05:00",
   "changes": [{"op": "create", "entity": "cart_line", "id": null, "from": null,
                "to": {"cart_id": "cart_jamie", "variant_id": "var_windbreaker_m_slate", "quantity": 1}}],
   "checks": [{"rule": "cart.max_lines", "passed": true, "detail": "1 of 50 lines", "fix_tool": null},
              {"rule": "cart.no_mix", "passed": true, "detail": "Ships within the US like the rest of the cart", "fix_tool": null},
              {"rule": "checkout_validation.stock_available", "passed": true, "detail": "Windbreaker M Slate is in stock", "fix_tool": null}],
   "needs_approval": false, "approval_reason": null}
  ```
- 출력(실행): `{ok, id, cart, undo}` — `id`는 담긴 줄, `cart`는 `get_cart`와 같은 모양
  ```jsonc
  {"ok": true, "id": "cln_1", "cart_id": "cart_jamie", "cart_line_id": "cln_1", "variant_id": "var_windbreaker_m_slate",
   "cart": {"id": "cart_jamie", "lines": [{"cart_line_id": "cln_1", "product_id": "prd_windbreaker", "variant_id": "var_windbreaker_m_slate",
                                           "name": "Packable windbreaker", "color": "Slate", "size": "M", "quantity": 1,
                                           "price": {"value": 89.00, "currency": "USD"}, "note": null}],
            "subtotal": {"value": 89.00, "currency": "USD"},
            "discounts": [{"label": "VIP weekend", "amount": {"value": 17.80, "currency": "USD"}, "promotion_id": "promo_1042", "discount_layer": "item_price"}],
            "shipping": {"value": 0.00, "currency": "USD"}, "total": {"value": 71.20, "currency": "USD"},
            "delivery": {"address_id": "adr_home", "address_label": "Home · 12 Harbor St, Apt 3", "arrives": "2026-11-10"},
            "payment": {"payment_method": "credit_card", "label": "Card ending 4821"},
            "coupons_recorded": [{"coupon_id": "cpn_vipweekend20", "code": "VIPWEEKEND20", "promotion_id": "promo_1042"}]},
   "undo": null}
  ```
- 프리뷰: `changes`는 `create cart_line`(같은 변형이 있으면 `update cart_line` 수량, 장바구니가 없으면 `create cart` 한 줄 추가). checks `cart.max_lines`, `cart.no_mix`, `checkout_validation.stock_available`. `needs_approval: false`.
- 정책: `cart.max_lines`, `cart.max_quantity_per_line`, `cart.no_mix`, `checkout_validation.stock_available`, [approval.yml](../../policies/approval.yml) `no_approval`
- 에러: `out_of_stock`(그 변형 재고 없음) → `get_product` · `not_found`(`detail.entity`: `product` | `variant`, 그 사이즈·색 조합이 없음) → `get_product` · `purchase_limit_exceeded`(`cart.max_lines`·`cart.max_quantity_per_line` 또는 상품 구매 한도 초과) → `update_cart_item` · `mixed_order_type`(구독 상품과 일반 상품 혼합) → `remove_cart_item`
- 멱등성: `(cart_id, variant_id)` 기준 upsert. 같은 변형을 다시 담으면 수량을 `order_quantity`로 맞추고, 같은 `preview_id` 재실행은 첫 결과.

### `update_cart_item` — Kind: W
- 논리주소/REST: `sales.cart.update` · `PATCH /v1/sales/carts/mine/lines/{cart_line_id}`
- 요약: 장바구니 줄의 수량이나 변형(사이즈·색)을 바꿉니다(판매 중인 변형으로만). `no_approval`이라 바로 실행.
- consumes → produces: `cart_line_id` → `cart_line_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | cart_line_id | Text, `^cln_[a-z0-9_]+$` | ✓ | 바꿀 줄(`get_cart`) | `"cln_1"` |
  | order_quantity | Integer | | 새 수량(1 이상) | `2` |
  | size | Text | | 같은 상품의 다른 사이즈 | `"L"` |
  | color | Text | | 같은 상품의 다른 색 | `"Black"` |
- 출력(프리뷰): `{"preview_id": "pv_c1b2", "expires_at": "2026-11-07T10:40:00-05:00", "changes": [{"op": "update", "entity": "cart_line", "id": "cln_1", "from": {"variant_id": "var_windbreaker_m_slate", "quantity": 1}, "to": {"variant_id": "var_windbreaker_l_slate", "quantity": 1}}], "checks": [{"rule": "checkout_validation.stock_available", "passed": true, "detail": "Windbreaker L Slate is in stock", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "cln_1", "cart": {…get_cart 모양…}, "undo": null}`
- 프리뷰: `update cart_line`. `needs_approval: false`.
- 정책: `cart.max_quantity_per_line`, `checkout_validation.stock_available`
- 에러: `not_found`(`detail.entity: cart_line` 또는 그 옵션의 `variant`) → `get_cart` · `out_of_stock` → `get_product`
- 멱등성: 같은 값으로 다시 보내면 같은 결과.

### `remove_cart_item` — Kind: W
- 논리주소/REST: `sales.cart.remove` · `DELETE /v1/sales/carts/mine/lines/{cart_line_id}`
- 요약: 장바구니 줄 하나를 뺍니다. `no_approval`이라 바로 실행.
- consumes → produces: `cart_line_id` → `cart_id`
- 입력: `cart_line_id`(Text, `^cln_`, ✓, 예 `"cln_1"`)
- 출력(프리뷰): `{"preview_id": "pv_c1c3", "expires_at": "2026-11-07T10:41:00-05:00", "changes": [{"op": "delete", "entity": "cart_line", "id": "cln_1", "from": {"variant_id": "var_windbreaker_m_slate", "quantity": 1}, "to": null}], "checks": [], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "cart_jamie", "cart": {…}, "undo": null}`
- 정책: 없음
- 에러: 없음(이미 없는 줄을 빼는 것은 성공으로 처리)
- 멱등성: 이미 없는 줄 삭제는 같은 결과.

### `clear_cart` — Kind: W
- 논리주소/REST: `sales.cart.clear` · `DELETE /v1/sales/carts/mine/lines`
- 요약: 장바구니를 비웁니다. `no_approval`이라 바로 실행.
- consumes → produces: → `cart_id`
- 입력: 없음
- 출력(프리뷰): `changes`는 줄마다 `delete cart_line`. 출력(실행): `{"ok": true, "id": "cart_jamie", "cart": {"id": "cart_jamie", "lines": [], …}, "undo": null}`
- 정책: 없음
- 에러: 없음
- 멱등성: 반복 호출해도 같은 결과.

---

## checkout

### `get_payment_methods` — Kind: R
- 논리주소/REST: `sales.checkout.list_payment_methods` · `GET /v1/sales/checkout/payment-methods`
- 요약: 호출한 쇼퍼의 장바구니의 모든 상품에 **함께 쓸 수 있는 결제 수단**과 할부·적립금 사용 가능 여부. `prepare_order`의 `checkout_validation.common_payment_method_exists`가 실패했을 때 다음 툴입니다.
- consumes → produces: `cart_id` → payment methods
- 입력: 없음(호출한 쇼퍼의 장바구니)
- 출력: `{"items": [{"payment_method": "credit_card", "label": "Card ending 4821", "saved": true, "default": true, "installment_months": [0, 3, 6], "loyalty_usable": true}], "total": 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: `payment.methods`, `payment.primary_methods_per_order`, `payment.installment`
- 에러: `cart_empty` → `add_to_cart`
- 멱등성: 읽기.

---

## order

### `prepare_order` — Kind: W (프리뷰만)  (MVP)
- 논리주소/REST: `sales.order.prepare` · `POST /v1/sales/orders/prepare`
- 요약: 호출한 쇼퍼의 장바구니로 **주문할 수 있는 상태의 주문을 준비**합니다. 이 툴은 프리뷰만 만들고 실행 단계가 없으므로(전선 위에서는 `?preview=true`로만 부르고, 같은 경로에 `preview_id`를 보내면 `invalid_input`) `needs_approval`은 항상 `false`입니다([approval.yml](../../policies/approval.yml) `preview_only`). 주문하기는 쇼퍼가 확인 카드(`checkout_card`)에서 누르는 사람 전용 `POST /v1/sales/orders {preview_id}`뿐입니다. schema.org로는 `potential` 상태의 `OrderAction`이고 쇼퍼의 탭이 이를 완료합니다. 같은 장바구니로 다시 부르면 새 프리뷰를 만들고 이전 프리뷰는 `superseded`가 됩니다.
- consumes → produces: `cart_id` → `preview_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | delivery_mode | enum | ✓ | `saved`: 저장된 주소(`delivery_address_id`); `one_time`: 이번 주문에만 쓰는 주소(`delivery_address`, 저장하지 않음) | `"saved"` |
  | delivery_address_id | Text, `^adr_[a-z0-9_]+$` | delivery_mode=`saved`일 때 ✓ | `get_my_profile`·`list_addresses`의 주소 | `"adr_home"` |
  | delivery_address | PostalAddress | delivery_mode=`one_time`일 때 ✓ | `{name, street_address, address_locality, address_region, postal_code, address_country}` | `{"name": "Jamie", "street_address": "12 Harbor St, Apt 3", "address_locality": "Brooklyn", "address_region": "NY", "postal_code": "11201", "address_country": "US"}` |
  | payment_method | enum | | `credit_card` \| `bank_transfer` \| `virtual_account` \| `loyalty_points` \| `store_credit`. 생략하면 저장된 기본 수단 | `"credit_card"` |
  | client_total | MonetaryAmount | ✓ | 쇼퍼가 화면에서 본 합계. 서버 재계산 합계와 비교(`checkout_validation.server_total_matches_client`) | `{"value": 71.20, "currency": "USD"}` |
- 출력(프리뷰):
  ```jsonc
  {"preview_id": "pv_7731", "expires_at": "2026-11-07T10:42:00-05:00",
   "changes": [{"op": "create", "entity": "order", "id": null, "from": null,
                "to": {"lines": [{"cart_line_id": "cln_1", "variant_id": "var_windbreaker_m_slate", "name": "Packable windbreaker",
                                  "color": "Slate", "size": "M", "quantity": 1, "price": {"value": 89.00, "currency": "USD"}}],
                       "subtotal": {"value": 89.00, "currency": "USD"},
                       "discounts": [{"label": "VIP weekend", "amount": {"value": 17.80, "currency": "USD"}, "promotion_id": "promo_1042"}],
                       "shipping": {"value": 0.00, "currency": "USD"},
                       "total": {"value": 71.20, "currency": "USD"},
                       "delivery": {"address_id": "adr_home", "address_label": "Home · 12 Harbor St, Apt 3, Brooklyn, NY 11201", "arrives": "2026-11-10"},
                       "payment": {"payment_method": "credit_card", "label": "Card ending 4821"},
                       "coupons_recorded": [{"coupon_id": "cpn_vipweekend20", "code": "VIPWEEKEND20"}]}}],
   "checks": [{"rule": "checkout_validation.common_payment_method_exists", "passed": true, "detail": "Card ending 4821 works for every line", "fix_tool": null},
              {"rule": "checkout_validation.stock_available", "passed": true, "detail": "Windbreaker M Slate is in stock", "fix_tool": null},
              {"rule": "checkout_validation.product_sellable", "passed": true, "detail": "All lines are on sale", "fix_tool": null},
              {"rule": "checkout_validation.no_domestic_international_mix", "passed": true, "detail": "All lines ship within the US", "fix_tool": null},
              {"rule": "checkout_validation.server_total_matches_client", "passed": true, "detail": "Recalculated with calculate_benefits: 89.00 − 17.80 + 0.00 = 71.20, same as shown", "fix_tool": null},
              {"rule": "checkout_validation.price_floor_respected", "passed": true, "detail": "Sale price stays at or above cost + 15%", "fix_tool": null}],
   "needs_approval": false, "approval_reason": null}
  ```
  - 확인 카드의 버튼은 이 `preview_id`로 `POST /v1/sales/orders`를 부릅니다. 실패한 check는 `fix_tool`에 다음 툴을 담습니다(재고 → `get_product`, 결제 수단 → `get_payment_methods`, 합계 불일치 → `get_cart`).
- 프리뷰: `create order` 한 줄(`id: null`). checks는 `checkout_validation`의 여섯 항목. `needs_approval: false`(프리뷰만).
- 정책: `checkout_validation.common_payment_method_exists`, `checkout_validation.stock_available`, `checkout_validation.product_sellable`, `checkout_validation.no_domestic_international_mix`, `checkout_validation.server_total_matches_client`, `checkout_validation.price_floor_respected`, `free_shipping_min_subtotal`, [approval.yml](../../policies/approval.yml) `preview_only`
- 에러: `cart_empty` → `add_to_cart` · `no_common_payment_method` → `get_payment_methods` · `out_of_stock` → `get_cart` · `not_found`(`detail.entity: postal_address`) → `get_my_profile` · `address_not_deliverable` → `get_my_profile` · `mixed_order_type` → `remove_cart_item`
- 멱등성: 상태를 바꾸지 않음(재고 예약·결제 없음). 같은 장바구니로 다시 부르면 새 `preview_id`, 이전 프리뷰는 `superseded`.

### `POST /v1/sales/orders` — 사람 전용
- 경로·스코프: `POST /v1/sales/orders` `{"preview_id": "pv_7731"}` · `human:order`. **툴이 아니며 모델은 보지 못하고 부를 수 없습니다.** 쇼퍼가 `checkout_card`의 주문하기 버튼으로만 보냅니다([README.md](README.md) §2).
- 요약: 쇼퍼가 준비된 주문(`prepare_order` 또는 `prepare_gift_order`의 프리뷰)을 **주문합니다**. 서버는 `checkout_validation`을 다시 돌리고, 통과하면 재고를 예약하고 결제를 시작합니다.
- consumes → produces: `preview_id` → `order_id`
- 입력: 본문 `preview_id`(Text, `^pv_`, ✓). 헤더 `Idempotency-Key`, `X-CSRF-Token`.
- 출력: `{"ok": true, "id": "ord_20518", "order_number": "20518", "order_status": "processing", "undo": null}` — 주문 취소는 사람 전용 `POST /v1/fulfillment/orders/{order_id}/cancel`([fulfillment.md](fulfillment.md)). 선물 주문(`recipient_accepts`)이면 `gift_order_id`와 `accept_url`이 더해집니다.
- 정책: `inventory.reserve_on`(주문 시점 예약), `inventory.unpaid_ttl_minutes`(미결제 자동 취소), `checkout_validation`(다시 검사)
- 에러(사람 전용이라 `next_tool`은 모두 `null`, 화면이 다시 준비를 안내): `preview_expired`(프리뷰가 없거나 만료·대체됨) → null · `estimate_changed`(다시 계산한 합계가 프리뷰와 다름, `detail.new_preview`) → null · `out_of_stock` → null · `payment_declined` → null
- 멱등성: 같은 `preview_id`(또는 같은 `Idempotency-Key`)로 다시 보내면 첫 주문을 그대로 돌려줍니다. 주문은 하나만 생깁니다.

### `get_my_orders` — Kind: R  (MVP)
- 논리주소/REST: `sales.order.list_mine` · `GET /v1/sales/orders/mine`
- 요약: 호출한 쇼퍼 본인의 주문 목록(줄 요약과 배송 완료 시각 포함), 최근 주문부터. 반품·교환 흐름의 시작점(`get_return_options`)입니다.
- consumes → produces: → `order_id[]`, `order_item_id[]`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | order_status | enum[] | | `payment_due` \| `processing` \| `in_transit` \| `delivered` \| `returned` \| `cancelled` \| `problem`. 생략하면 전부 | `["delivered"]` |
  | limit | Integer | | 1–100, 기본 20 | `20` |
  | cursor | Text | | 이전 응답의 `next_cursor` | `"c_2"` |
- 출력:
  ```jsonc
  {"items": [{"id": "ord_20402", "order_number": "20402", "order_status": "delivered",
              "order_date": "2026-10-27T19:12:00-04:00",
              "ordered_item": [{"id": "oln_1", "product_id": "prd_chelsea_boots", "variant_id": "var_chelsea_boots_9_tan",
                                "name": "Leather chelsea boots", "size": "9", "color": "Tan", "order_quantity": 1,
                                "order_item_status": "delivered"}],
              "delivered_at": "2026-10-31T14:02:00-04:00",
              "total": {"value": 312.00, "currency": "USD"}}],
   "total": 1}
  ```
  - 예: Alex(`cus_alex`)가 Fri Nov 6 09:50에 부른 결과. `delivered_at`은 모든 줄이 배송 완료된 시각, 아니면 `null`.
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: 없음(주문이 없으면 `items: []`, `total: 0`).
- 멱등성: 읽기.

### `get_order` — Kind: R  (MVP)
- 논리주소/REST: `sales.order.get` · `GET /v1/sales/orders/{order_id}`
- 요약: 주문 한 건 — 줄, 배송, 결제, 합계, 기록된 쿠폰. 운영자는 모든 주문, 쇼퍼는 본인 주문만. 쇼퍼 화면 블록 `order_detail`과 반품 검토(`evaluate_return_request`의 근거)에 씁니다.
- consumes → produces: `order_id` → `order_item_id[]`, `delivery_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | order_id | Text, `^ord_[a-z0-9_]+$` | ✓ | 주문 id(`get_my_orders`, `search_orders`) | `"ord_20402"` |
  | fields | Text[] | | 돌려받을 최상위 필드만 | `["ordered_item", "payment"]` |
- 출력: `Order` 한 건(목록 봉투 없음)
  ```jsonc
  {"id": "ord_20402", "order_number": "20402", "order_status": "delivered",
   "order_date": "2026-10-27T19:12:00-04:00",
   "customer_id": "cus_alex",
   "ordered_item": [{"id": "oln_1", "variant_id": "var_chelsea_boots_9_tan", "product_id": "prd_chelsea_boots",
                     "name": "Leather chelsea boots", "size": "9", "color": "Tan", "order_quantity": 1,
                     "price": {"value": 312.00, "currency": "USD"},
                     "discount": {"value": 0.00, "currency": "USD"},
                     "order_item_status": "delivered"}],
   "delivery": {"id": "dlv_20402",
                "delivery_address": {"name": "Alex", "street_address": "220 Fore St", "address_locality": "Portland",
                                     "address_region": "ME", "postal_code": "04101", "address_country": "US"},
                "expected_arrival_until": "2026-10-31T20:00:00-04:00",
                "delivery_state": "delivered",
                "delivered_at": "2026-10-31T14:02:00-04:00"},
   "payment": {"payment_method": "credit_card", "card_last4": "0937", "payment_status": "payment_complete"},
   "totals": {"subtotal": {"value": 312.00, "currency": "USD"},
              "discounts": [],
              "shipping": {"value": 0.00, "currency": "USD"},
              "gift_wrap": null,
              "total": {"value": 312.00, "currency": "USD"}},
   "gift": null,
   "coupons_recorded": [],
   "claims": [{"id": "clm_20402", "claim_type": "return", "claim_status": "requested",
               "order_item_ids": ["oln_20402_1"], "requested_at": "2026-11-06T09:52:00-05:00"}]}
  ```
  - `gift`: 선물 주문이면 `{recipient_name, recipient_mode, card_message, gift_receipt, exchange_until, acceptance}`, 아니면 `null`. `acceptance`는 받는 사람이 링크로 수락하는 선물(`recipient_accepts`)일 때 `{gift_order_id, accept_url, action_status, expires}`이고, 기한이 지난 대기는 `failed`로 보입니다. 포장비는 `totals.gift_wrap`(포장하지 않았으면 `null`).
  - `claims[]`: 이 주문의 취소·반품·교환 요청(오래된 순)과 각각이 걸린 줄. 쇼퍼 화면은 열린 요청이 있는 줄에 "반품 요청 중"을 보여 주고 새 요청 버튼을 내지 않습니다. 상태는 [enums.md](enums.md) §1.5.
  - 예: Alex의 #20402(F5, B2). Fri Nov 6 기준 배송 완료 6일째. 쿠폰을 쓴 주문이면 `totals.discounts[]`와 `coupons_recorded[]`가 채워집니다(예 `[{"coupon_id": "cpn_vipweekend20", "code": "VIPWEEKEND20", "promotion_id": "promo_1042"}]`).
  - `order_status`·`order_item_status`는 [enums.md](enums.md) §1.1, `delivery_state`는 §1.3, `payment_status`는 §1.4. 줄마다 상태가 따로 움직입니다.
- 프리뷰: 해당 없음(읽기)
- 정책: 없음(읽기). 카드는 끝 4자리만.
- 에러: `not_found`(`detail.entity: order`) → `search_orders` · `forbidden`(쇼퍼가 남의 주문을 봄, `detail.reason: owner`) → `get_my_orders`
- 멱등성: 읽기.

### `search_orders` — Kind: R
- 논리주소/REST: `sales.order.search` · `GET /v1/sales/orders`
- 요약: 운영자용 주문 검색. 사람이 말하는 단위로 찾습니다: 주문 번호, 고객의 이메일·이름·전화번호.
- consumes → produces: → `order_id[]`
- 입력: `order_number`(Text, `^#?[0-9]{1,12}$`, `#` 있어도 됨), `customer`(Text, 2–100자: 이메일·이름·전화번호나 그 일부, 또는 고객 id. `search_members`와 같은 방식으로 맞춤), `order_status`(enum[], [enums.md](enums.md) §1.1), `customer_id`(`^cus_`), `order_date_from` / `order_date_to`(Date), `seller_id`(`^slr_`), `sort`(`order_date` \| `-order_date` \| `total` \| `-total`, 기본 `-order_date`), `limit`(기본 20, 최대 100), `cursor`, `fields`(Text[])
- 출력: `{"items": [{"id": "ord_20402", "order_number": "20402", "order_status": "delivered", "customer_id": "cus_alex", "order_date": "2026-10-27T19:12:00-04:00", "total": {"value": 312.00, "currency": "USD"}}], "total": 1}`
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_filter` → `list_enums` · `invalid_filter`(`detail.reason: too_broad`, `customer`가 500명 넘게 맞음, 이메일이나 더 긴 이름으로) → `search_members`
- 멱등성: 읽기.

### `verify_guest` — Kind: W
- 논리주소/REST: `sales.order.verify_guest` · `POST /v1/sales/orders/verify-guest`
- 요약: 로그인하지 않은 사람이 주문 번호와 그 주문의 연락처(이메일 또는 전화번호)로 주문 하나를 엽니다. 성공하면 그 주문만 볼 수 있는 짧은 토큰을 줍니다. 틀린 짝은 그런 주문이 있든 없든 **같은 답**이고 같은 방식으로 셉니다(주문이 있는지 알려 주지 않음). 다른 쓰기처럼 프리뷰 → 실행이며 승인은 없습니다(`no_approval` 대상은 아니지만 임계가 없음): 프리뷰가 연락처를 확인하고 내줄 토큰을 보여 주며, 틀린 시도는 그때 바로 셉니다(`login`의 틀린 비밀처럼, 추측을 막는 기록). 실행이 토큰을 한 번만 보여 주고(`shown_once`) 해시만 남깁니다(`sales.guest_lookup`).
- consumes → produces: `order_number`, `email` 또는 `telephone` → `guest_token`
- 입력: `order_number`(Text, `^#?[0-9]{1,12}$`, ✓, 예 `"20402"`), `email`(Text, 예 `"alex.kim@example.com"`, 대소문자·앞뒤 공백 무시), `telephone`(Text, 숫자만 비교). `email`과 `telephone` 중 하나 이상 ✓.
- 출력(프리뷰): `changes: [{"op": "create", "entity": "guest_token", "id": null, "from": null, "to": {"order_id": "ord_20402", "order_number": "20402", "expires_in": 1800}}]`, checks `guest_lookup.max_attempts`(앞선 틀린 시도 수), `needs_approval: false`
- 출력(실행): `{"ok": true, "id": "ord_20402", "order_number": "20402", "guest_token": "gt_…", "expires_in": 1800, "undo": null}` — 토큰은 이 응답에만 있습니다.
- 정책: `guest_lookup.max_attempts`(같은 번호에 `lock_minutes` 안의 연속 틀린 시도), `guest_lookup.lock_minutes`, `guest_lookup.token_ttl_seconds`
- 에러: `guest_match_failed`(번호·연락처가 맞지 않음, `detail.attempts_left`) → `verify_guest` · `too_many_attempts`(class `locked`, `guest_lookup.max_attempts` 초과로 잠김, `detail.until`) → null
- 멱등성: 같은 `preview_id` 재실행은 첫 결과(토큰 없이). 다시 인증하면 새 토큰이 이전 토큰을 대신합니다.
- 메모: 주문의 연락처는 주문한 회원(`customer.person`)의 이메일·전화번호입니다. 이 스토어의 체크아웃은 비회원 주문과 주문 비밀번호를 받지 않으므로 예전 입력 `order_password`는 두지 않습니다.

### `change_shipping_address` — Kind: W
- 논리주소/REST: `sales.order.change_shipping_address` · `POST /v1/sales/orders/{order_id}/delivery-address`
- 요약: 주문의 배송지를 바꿉니다(**출고 전만**, `change.allowed_before`). 지역 추가 배송비를 다시 계산합니다. 승인 임계에 걸리지 않으면 바로 실행.
- consumes → produces: `order_id` → `order_id`
- 입력: `order_id`(`^ord_`, ✓), `delivery_mode`(✓, `saved` \| `one_time`), `delivery_address_id`(saved일 때 ✓), `delivery_address`(PostalAddress, one_time일 때 ✓)
- 출력(프리뷰): `{"preview_id": "pv_a3d1", "expires_at": "2026-11-07T11:15:00-05:00", "changes": [{"op": "update", "entity": "order", "id": "ord_20518", "from": {"delivery_address": "12 Harbor St, Apt 3, Brooklyn, NY 11201"}, "to": {"delivery_address": "40 Pier Rd, Brooklyn, NY 11231"}}], "checks": [{"rule": "change.allowed_before", "passed": true, "detail": "Not shipped yet", "fix_tool": null}, {"rule": "remote_area_surcharge", "passed": true, "detail": "No surcharge for this address", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "ord_20518", "undo": null}` — 되돌리려면 이전 주소로 다시 `change_shipping_address`(출고 전까지). 변경셋 안에서는 셋 되돌리기로도 이전 주소로 돌아갑니다(출고 전까지).
- 프리뷰: `update order`. checks `change.allowed_before`(품목의 `acom:fulfillment_stage`와 배송이 `shipped` 전), `remote_area_surcharge`(주소가 속한 배송 지역(`list_shipping_zones`, 주를 지정한 지역이 나라 전체 지역보다 우선)의 추가비, 저장된 지역이 없으면 `fulfillment.delivery.default_zone`의 추가비; 새 주소가 더 비싸면 주문 뒤에는 받을 수 없어 `policy_violation`, `detail.zone`에 지역 이름).
- 동작: 주소는 주문 이벤트(`UpdateAction`, `result.delivery`)로 남고 이전 주소도 그대로 남습니다. `get_order`와 출고(fulfillment)는 마지막으로 기록된 주소를 씁니다. 받는 사람이 수락할 선물(`recipient_accepts`, 대기 중)은 받는 사람이 주소를 정하므로 `invalid_state`.
- 정책: `change.allowed_before`, `remote_area_surcharge`([fulfillment.yml](../../policies/fulfillment.yml))
- 에러: `already_shipped` → `get_order` · `address_not_deliverable` → `list_addresses` · `not_found`(`detail.entity: postal_address`, 그 쇼퍼의 저장 주소가 아님) → `list_addresses` · `invalid_state`(취소·문제 주문, 수락 대기 선물) → `get_order`
- 멱등성: `Idempotency-Key`. 같은 주소로 다시 보내면 같은 결과.

---

## payment

### `get_payment` — Kind: R
- 논리주소/REST: `sales.payment.get` · `GET /v1/sales/orders/{order_id}/payment`
- 요약: 주문의 결제 상태·수단·분할 내역.
- consumes → produces: `order_id` → `payment_id`
- 입력: `order_id`(`^ord_`, ✓)
- 출력: `{"id": "pay_20518", "order_id": "ord_20518", "payment_status": "payment_complete", "payment_method": "credit_card", "label": "Card ending 4821", "amount": {"value": 71.20, "currency": "USD"}, "installment_months": 0}` — `payment_status`는 [enums.md](enums.md) §1.4
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `not_found`(`detail.entity: order`) → `search_orders`
- 멱등성: 읽기.

### `confirm_virtual_account_deposit` — Kind: W$
- 논리주소/REST: `sales.payment.confirm_deposit` · `POST /v1/sales/orders/{order_id}/payment/confirm-deposit`
- 요약: 가상계좌 입금을 운영자가 수동으로 확인합니다(운영자만). 결제 상태 `payment_due → payment_complete`, 주문 `payment_due → processing`(출고 가능). 돈이 움직이므로 항상 사람 승인. 은행은 이 시스템 밖이라 묻지 않으며, 운영자의 확인을 그대로 기록하고 실행 결과의 `outside_system`(`{"system": "bank", "connected": false, …}`)이 그렇게 말합니다.
- consumes → produces: `order_id` → `payment_id`
- 입력: `order_id`(`^ord_`, ✓), `amount`(MonetaryAmount, ✓, 예 `{"value": 71.20, "currency": "USD"}`)
- 출력(프리뷰): `{"preview_id": "pv_v01d", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "update", "entity": "payment", "id": "pay_20519", "from": "payment_due", "to": "payment_complete"}], "checks": [{"rule": "payment.virtual_account_hold_days", "passed": true, "detail": "Deposit within 3 days", "fix_tool": null}], "needs_approval": true, "approval_reason": "Confirms a $71.20 payment", "change_set_id": "cs_1047"}`
- 출력(실행): `{"ok": true, "id": "pay_20519", "order_status": "processing", "outside_system": {"system": "bank", "connected": false, "note": "…"}, "undo": null}` — 결제 확정은 출고 지시로 이어지므로 되돌리지 않습니다.
- 프리뷰: `update payment`(`payment_due → payment_complete`)와 `update order`(`payment_due → processing`). 항상 `needs_approval: true`, `approval_reason`은 approval.yml 임계가 없으면 "Confirms a 71.20 USD payment".
- 정책: `payment.virtual_account_hold_days`(주문 시각 + 일수까지)
- 에러: `deadline_passed`(입금 기한 경과) → `get_order` · `invalid_state`(이미 `payment_complete`, 가상계좌가 아님, 수락 대기 선물) → `get_payment` · `amount_mismatch` → `get_payment` · `forbidden`(쇼퍼) → `get_payment`
- 예: 시드의 Priya #29001(가상계좌 $128.00, Thu Nov 5 16:20 주문, Sun Nov 8 16:20까지).
- 멱등성: `Idempotency-Key`. 이미 `payment_complete`면 첫 결과.

---

## receipt

### `issue_receipt` — Kind: W
- 논리주소/REST: `sales.receipt.issue` · `POST /v1/sales/orders/{order_id}/receipt`
- 요약: 결제가 끝난 주문의 영수증·증빙을 발급합니다. 종류마다 주문에 하나(취소되지 않은 것). 소득공제(`deduction`)·세금계산서(`tax_invoice`)를 신고할 세무 시스템은 이 시스템 밖이라 여기 기록만 하고, 결과의 `tax_filing`(`{"system": "tax_service", "connected": false, …}`)이 그렇게 말합니다. 쇼퍼는 본인 주문, 운영자는 모든 주문.
- consumes → produces: `order_id` → `receipt_id`
- 입력: `order_id`(`^ord_`, ✓), `receipt_type`(✓, `deduction` \| `proof` \| `tax_invoice`), `tax_id`(Text, `tax_invoice`일 때 ✓)
- 출력(프리뷰): `{"preview_id": "pv_r7c2", "expires_at": "2026-11-07T11:48:00-05:00", "changes": [{"op": "create", "entity": "receipt", "id": null, "from": null, "to": {"order_id": "ord_20518", "receipt_type": "proof"}}], "checks": [{"rule": "receipt.same_month_only", "passed": true, "detail": "Ordered this month", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "rct_01jd…", "receipt_type": "proof", "amount": {"value": 71.20, "currency": "USD"}, "tax_filing": null, "undo": null}` — 발급한 증빙은 되돌리지 않고, 금액이 바뀌면 취소 후 다시 발급(`receipt.after_claim`).
- 프리뷰: `create receipt`. 같은 종류가 있고 주문 금액이 그때와 다르면 `update receipt`(취소)가 앞에 붙고 check `receipt.after_claim`.
- 정책: `receipt.types`, `receipt.same_month_only`(스토어 시간대로 주문한 달), `receipt.after_claim`
- 에러: `business_id_required`(`tax_invoice`에 `tax_id` 없음) → `issue_receipt` · `already_issued`(같은 종류 발급됨, `detail.existing_id`) → `get_receipt` · `invalid_state`(결제 전, `detail.reason: not_paid`) → `get_payment` · `policy_violation`(`detail.rule: receipt.same_month_only`, 주문한 달이 지남) → `get_receipt`
- 멱등성: `(order_id, receipt_type)` 한 건.

### `get_receipt` — Kind: R
- 논리주소/REST: `sales.receipt.get` · `GET /v1/sales/orders/{order_id}/receipt`
- 요약: 발급된 영수증·증빙 조회. 쇼퍼는 본인 주문, 운영자는 모든 주문.
- consumes → produces: `order_id` → `receipt_id`
- 입력: `order_id`(`^ord_`, ✓), `receipt_type`(enum `deduction` \| `proof` \| `tax_invoice`, 생략하면 가장 최근 것)
- 출력: `{"id": "rct_20381", "order_id": "ord_20381", "order_number": "20381", "receipt_type": "proof", "issued_at": "2026-10-24T16:20:00-04:00", "amount": {"value": 64.00, "currency": "USD"}, "tax_id": null, "tax_filing": null, "other_receipts": []}` — 시드의 Jamie #20381
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `not_found`(`detail.entity: receipt`) → `issue_receipt`
- 멱등성: 읽기.

---

## subscription

### `prepare_subscription` — Kind: W (프리뷰만)
- 논리주소/REST: `sales.subscription.prepare` · `POST /v1/sales/subscriptions/prepare`
- 요약: 호출한 쇼퍼의 장바구니로 정기 주문을 **준비만** 합니다(주기, 첫 결제일, 결제 카드). 프리뷰만 만들고 `needs_approval`은 항상 `false`이며, 구독 시작은 쇼퍼의 사람 전용 `POST /v1/sales/subscriptions {preview_id}`뿐입니다. 장바구니의 줄 전부가 회차마다 반복됩니다(이 스토어의 카탈로그에는 `sale_type: subscription` 상품이 없으므로 상품 종류로 거르지 않음. 구독·일반 상품이 섞이면 `mixed_order_type`). 회차 금액은 소계 − `subscription.discount_layers`의 할인 + 배송비(`free_shipping_min_subtotal`)입니다.
- consumes → produces: `cart_id` → `preview_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | billing_cycle | enum | ✓ | `every_week` \| `every_2_weeks` \| `every_month` | `"every_month"` |
  | next_charge_date | Date | ✓ | 첫 결제일(오늘 이후) | `"2026-12-01"` |
  | delivery_mode | enum | ✓ | `saved` \| `one_time` | `"saved"` |
  | delivery_address_id | Text, `^adr_` | saved일 때 ✓ | 저장된 주소 | `"adr_home"` |
  | delivery_address | PostalAddress | one_time일 때 ✓ | 이번 구독에만 쓰는 주소 | — |
  | client_total | MonetaryAmount | ✓ | 쇼퍼가 본 회차 금액 | `{"value": 64.00, "currency": "USD"}` |
- 출력(프리뷰): `{"preview_id": "pv_2210", "expires_at": "2026-11-07T11:00:00-05:00", "changes": [{"op": "create", "entity": "subscription", "id": null, "from": null, "to": {"billing_cycle": "every_month", "next_charge_date": "2026-12-01", "amount": {"value": 64.00, "currency": "USD"}, "lines": [{"variant_id": "var_merino_tee_m_charcoal", "name": "Merino trail tee", "size": "M", "color": "Charcoal", "quantity": 1, "price": {"value": 58.00, "currency": "USD"}, "discount": {"value": 0.00, "currency": "USD"}}], "delivery": {"address_id": "adr_home", "address_label": "Home · 12 Harbor St, Apt 3, Brooklyn, NY 11201"}, "payment": {"payment_method": "credit_card", "label": "Card ending 4821"}}}], "checks": [{"rule": "payment.subscription_card_only", "passed": true, "detail": "Card ending 4821", "fix_tool": null}, {"rule": "checkout_validation.stock_available", "passed": true, "detail": "Merino trail tee M Charcoal is in stock", "fix_tool": null}, {"rule": "checkout_validation.server_total_matches_client", "passed": true, "detail": "64.00 as shown", "fix_tool": null}], "needs_approval": false, "approval_reason": null}` — merino tee $58.00는 무료 배송 기준($80.00) 아래라 배송비 $6.00이 회차마다 붙습니다.
- 프리뷰: `create subscription`(`id: null`). 할인은 `subscription.discount_layers`(회원가·기간가 층)만, 적립금 사용 불가.
- 정책: `payment.subscription_card_only`, `subscription.billing_cycles`, `subscription.cycle_length`, `subscription.discount_layers`, `subscription.loyalty_usable`, [approval.yml](../../policies/approval.yml) `preview_only`
- 에러: `not_card_payment`(저장된 카드 없음) → `get_payment_methods` · `invalid_input`(`next_charge_date`가 오늘이거나 과거 `detail.reason: past`, 형식 `format`, 스토어가 내지 않는 주기 `detail.field: billing_cycle`) → `prepare_subscription` · `cart_empty` → `add_to_cart` · `mixed_order_type` → `remove_cart_item` · `not_found`(`detail.entity: postal_address`) → `get_my_profile`
- 멱등성: 상태를 바꾸지 않음. 다시 부르면 이전 프리뷰는 `superseded`.

### `POST /v1/sales/subscriptions` — 사람 전용
- 경로·스코프: `POST /v1/sales/subscriptions` `{"preview_id": "pv_2210"}` · `human:order`. **툴이 아니며 모델은 보지 못합니다.** 쇼퍼가 확인 카드의 버튼으로만 보냅니다. 레지스트리 이름 `sales.subscription.start`(`start_subscription`).
- 요약: 쇼퍼가 준비된 정기 주문을 시작합니다. 서버가 다시 계산해 회차 금액이 다르면 `estimate_changed`, 구독한 줄은 장바구니에서 빠지고, 지금은 아무것도 결제하지 않습니다. 이후 결제일마다 예약 작업 `subscription.renew`가 주문을 만들고 저장된 카드로 결제합니다(아래).
- consumes → produces: `preview_id` → `subscription_id`
- 입력: 본문 `preview_id`(`^pv_`, ✓). 헤더 `Idempotency-Key`, `X-CSRF-Token`.
- 출력: `{"ok": true, "id": "sub_01jd…", "next_charge_date": "2026-12-01", "undo": null}` — 해지는 `cancel_subscription`.
- 정책: `payment.subscription_card_only`
- 에러(모두 `next_tool: null`): `preview_expired` → null · `estimate_changed` → null · `payment_declined`(저장된 카드가 없어짐) → null
- 멱등성: 같은 `preview_id`로 다시 보내면 첫 결과.

#### 예약 작업 `subscription.renew`
- **툴이 아니며 모델은 보지 못합니다**([platform.md](platform.md#예약-작업)). `payload {subscription_id, charge_date}`, 결제일의 스토어 시간 0시에 실행.
- 동작: 구독이 `active`이고 `next_charge_date`가 그 날이면 구독한 줄·할인·금액 그대로 주문(`processing`)을 만들고 재고를 잡고(`inventory.reserve_on`) 저장된 카드로 결제합니다(카드사는 이 시스템 밖이라 승인된 것으로 기록하고 주문 이벤트의 `outside_system`에 그렇게 남김). 그 뒤 `next_charge_date`를 `subscription.cycle_length` 한 주기 뒤로 옮기고 그 날의 작업을 예약합니다. 멈춘·해지된 구독, 날짜가 바뀐 구독의 작업은 아무것도 만들지 않습니다. 재고가 없으면 작업이 실패하고 실행기가 나중에 다시 시도합니다(아무것도 남지 않음). outbox에 `subscription.renewed`.
- 예약: `start_subscription`, `resume_subscription`, `update_schedule`(날짜를 바꿀 때), 이전 회차.

### `pause_subscription` — Kind: W
- 논리주소/REST: `sales.subscription.pause` · `POST /v1/sales/subscriptions/{subscription_id}/pause`
- 요약: 정기 주문을 잠시 멈춥니다(`active → paused`). 멈춘 동안 회차 주문이 생기지 않습니다.
- consumes → produces: `subscription_id` → `subscription_id`
- 입력: `subscription_id`(`^sub_`, ✓, 예 `"sub_3301"`)
- 출력(프리뷰): `{"preview_id": "pv_s1p0", "expires_at": "2026-11-07T11:30:00-05:00", "changes": [{"op": "update", "entity": "subscription", "id": "sub_3301", "from": "active", "to": "paused"}], "checks": [], "needs_approval": false, "approval_reason": null}` · 출력(실행): `{"ok": true, "id": "sub_3301", "undo": null}` — 다시 시작은 `resume_subscription`.
- 프리뷰: `update subscription`. `needs_approval: false`.
- 정책: 없음(상태 전이는 [enums.md](enums.md) §1.14)
- 에러: `invalid_state`(이미 `cancelled`) → `list_subscriptions` · `not_found`(`detail.entity: subscription`) → `list_subscriptions` · `forbidden`(남의 구독) → `list_subscriptions`
- 멱등성: 이미 `paused`면 같은 결과(변경 없음). 변경셋 안에서는 되돌리기(다시 `active`)가 됩니다.

### `resume_subscription` — Kind: W
- 논리주소/REST: `sales.subscription.resume` · `POST /v1/sales/subscriptions/{subscription_id}/resume`
- 요약: 멈춘 정기 주문을 다시 시작합니다(`paused → active`). 다음 결제일은 오늘 이후의 가장 가까운 회차.
- consumes → produces: `subscription_id` → `subscription_id`
- 입력: `subscription_id`(`^sub_`, ✓)
- 출력(프리뷰): `changes: [{"op": "update", "entity": "subscription", "id": "sub_3302", "from": "paused", "to": "active", "note": "Next charge 2026-11-13"}]`, checks `payment.subscription_card_only`, `needs_approval: false` · 출력(실행): `{"ok": true, "id": "sub_3302", "undo": null}` — 예: 시드의 Grace(격주, Oct 30 결제 대기 중 멈춤)를 Sat Nov 7에 다시 시작하면 Nov 13.
- 프리뷰: `update subscription`. 다음 결제일은 저장된 결제일에서 `subscription.cycle_length`씩 더해 오늘 이후의 첫 날(스토어 시간대). 그 날의 `subscription.renew`를 예약.
- 정책: `payment.subscription_card_only`(카드가 여전히 유효한지), `subscription.cycle_length`
- 에러: `invalid_state`(이미 `cancelled`) → `list_subscriptions` · `not_card_payment` → `get_payment_methods`
- 멱등성: 이미 `active`면 같은 결과.

### `cancel_subscription` — Kind: W
- 논리주소/REST: `sales.subscription.cancel` · `POST /v1/sales/subscriptions/{subscription_id}/cancel`
- 요약: 정기 주문을 해지합니다(`active | paused → cancelled`, 끝 상태). 이미 만들어진 회차 주문은 그대로입니다.
- consumes → produces: `subscription_id` → `subscription_id`
- 입력: `subscription_id`(`^sub_`, ✓), `reason`(Text, 선택)
- 출력(프리뷰): `changes: [{"op": "update", "entity": "subscription", "id": "sub_3301", "from": "active", "to": "cancelled"}]`, `needs_approval: false` · 출력(실행): `{"ok": true, "id": "sub_3301", "undo": null}` — 해지는 되돌릴 수 없고 다시 하려면 새로 `prepare_subscription`. `reason`은 구독에 남습니다(`cancel_reason_text`).
- 프리뷰: `update subscription`.
- 정책: 없음
- 에러: `invalid_state`(이미 `cancelled`) → `list_subscriptions` · `not_found`(`detail.entity: subscription`) → `list_subscriptions`
- 멱등성: 이미 `cancelled`면 같은 결과.

### `update_schedule` — Kind: W
- 논리주소/REST: `sales.subscription.update_schedule` · `PATCH /v1/sales/subscriptions/{subscription_id}/schedule`
- 요약: 정기 주문의 주기·다음 결제일·수량·배송지를 바꿉니다(다음 회차부터).
- consumes → produces: `subscription_id` → `subscription_id`
- 입력: `subscription_id`(`^sub_`, ✓), `billing_cycle`(enum `every_week` \| `every_2_weeks` \| `every_month`), `next_charge_date`(Date, 오늘 이후), `order_quantity`(Integer 1–`cart.max_quantity_per_line`, 줄이 하나인 구독만; 오늘 가격과 구독 할인으로 회차 금액을 다시 계산), `delivery_address_id`(`^adr_`, 구독한 쇼퍼의 저장 주소). 바꿀 것 하나 이상.
- 출력(프리뷰): `changes: [{"op": "update", "entity": "subscription", "id": "sub_3301", "from": {"billing_cycle": "every_month"}, "to": {"billing_cycle": "every_2_weeks"}}]`, `needs_approval: false` · 출력(실행): `{"ok": true, "id": "sub_3301", "undo": null}`
- 프리뷰: `update subscription`.
- 정책: `subscription.billing_cycles`, `payment.subscription_card_only`
- 에러: `invalid_input`(`detail.field: billing_cycle` 또는 과거 `next_charge_date`, 줄이 여럿인 구독의 `order_quantity` `detail.reason: several_items`) → `update_schedule` · `not_found`(`detail.entity: subscription`) → `list_subscriptions` · `not_found`(`detail.entity: postal_address`) → `list_addresses` · `address_not_deliverable` → `list_addresses` · `invalid_state`(해지됨) → `list_subscriptions` · `not_card_payment` → `get_payment_methods`
- 멱등성: `Idempotency-Key`. 같은 값이면 같은 결과.

### `list_subscriptions` — Kind: R
- 논리주소/REST: `sales.subscription.list` · `GET /v1/sales/subscriptions`
- 요약: 정기 주문 목록. 쇼퍼는 본인 것만(`customer_id` 생략 또는 `me`).
- consumes → produces: → `subscription_id[]`
- 입력: `customer_id`(`^cus_` 또는 `me`), `status`(enum[] `active` \| `paused` \| `cancelled`), `limit`(기본 20), `cursor`
- 출력: `{"items": [{"id": "sub_3301", "customer_id": "cus_jamie", "status": "active", "billing_cycle": "every_month", "next_charge_date": "2026-12-01", "amount": {"value": 64.00, "currency": "USD"}, "lines": [{"variant_id": "var_merino_tee_m_charcoal", "product_id": "prd_merino_tee", "name": "Merino trail tee", "size": "M", "color": "Charcoal", "quantity": 1}], "delivery": {"address_id": "adr_home", "address_label": "Home · 12 Harbor St, Apt 3, Brooklyn, NY 11201"}, "payment": {"payment_method": "credit_card", "label": "Card ending 4821"}, "paused_at": null, "cancelled_at": null}], "total": 1}` — 시드의 Jamie
- 프리뷰: 해당 없음(읽기)
- 정책: 없음
- 에러: `invalid_filter`(운영자가 `me`) → `search_members` · `forbidden`(쇼퍼가 남의 `customer_id`) → `list_subscriptions`
- 멱등성: 읽기.

---

## installment

### `set_installment` — Kind: W
- 논리주소/REST: `sales.installment.set` · `POST /v1/sales/orders/{order_id}/installment`
- 요약: 신용카드 할부 개월 수를 정합니다(결제 확정 전). 가능한 개월 수는 `get_payment_methods`의 `installment_months`. 카드사는 주문이 출고될 때 결제를 확정하므로, "결제 확정 전"은 주문이 출고되기 전(`order.change.allowed_before`)입니다. 카드사는 이 시스템 밖이라 할부 계획은 여기 기록만 하고, 결과의 `outside_system`(`{"system": "card_provider", "connected": false, …}`)이 그렇게 말합니다.
- consumes → produces: `order_id` → `payment_id`
- 입력: `order_id`(`^ord_`, ✓), `installment_months`(Integer, ✓, `payment.installment.months`의 값, 0은 일시불)
- 출력(프리뷰): `{"preview_id": "pv_i3m1", "expires_at": "2026-11-07T11:40:00-05:00", "changes": [{"op": "update", "entity": "payment", "id": "pay_20521", "from": {"installment_months": 0}, "to": {"installment_months": 3}}], "checks": [{"rule": "payment.installment.min_amount", "passed": true, "detail": "Order 213.60 USD is over the 50.00 USD minimum; 3 × 71.20", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "pay_20521", "undo": null}` — 결제 확정 전까지는 다시 `set_installment`로 바꿀 수 있습니다.
- 프리뷰: `update payment`.
- 정책: `payment.installment.methods`, `payment.installment.min_amount`, `payment.installment.months`
- 에러: `months_not_available` → `get_payment_methods` · `below_min_amount` → `get_payment_methods` · `not_card_payment` → `get_payment_methods` · `already_shipped` → `get_payment` · `invalid_state`(취소된 주문, 거절된 결제) → `get_payment`
- 멱등성: `Idempotency-Key`. 같은 개월 수면 같은 결과. 변경셋 안에서는 되돌리기(이전 개월 수)가 됩니다(출고 전까지).
- 예: 시드의 Owen #29002(받는 사람 수락을 기다리는 선물, 카드 $89.00) → 3 × 29.67.

---

## gift_order

### `prepare_gift_order` — Kind: W (프리뷰만)
- 논리주소/REST: `sales.gift_order.prepare` · `POST /v1/sales/gift-orders/prepare`
- 요약: 호출한 쇼퍼의 장바구니로 **선물 주문을 준비만** 합니다(받는 사람, 카드 메시지, 포장, 가격 없는 선물 영수증). `prepare_order`처럼 프리뷰만 만들고 `needs_approval`은 항상 `false`이며, 보내는 사람이 확인 카드에서 같은 사람 전용 `POST /v1/sales/orders {preview_id}`로 주문합니다. 받는 사람의 주소를 알면(`address_known`) 그 주소로 바로 보내고, 모르면(`recipient_accepts`) 받는 사람에게 수락 링크가 가며 **수락하기 전에는 결제가 보류**됩니다. 받는 사람의 정보(주소·사이즈)는 이 주문에만 쓰고 쇼퍼의 선호나 주소록에 저장하지 않습니다.
- consumes → produces: `cart_id` → `preview_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | recipient_mode | enum | ✓ | `address_known`: `delivery_address`로 보냄; `recipient_accepts`: 받는 사람이 링크로 주소·옵션 입력 | `"address_known"` |
  | recipient_name | Text | ✓ | 받는 사람 이름(카드·송장에 씀) | `"Dad"` |
  | recipient_contact | Text | recipient_accepts일 때 ✓ | 수락 링크를 받을 이메일 또는 전화 | `"dad@example.com"` |
  | delivery_address | PostalAddress | address_known일 때 ✓ | 이 주문에만 쓰는 주소(저장하지 않음). `name`(생략하면 `recipient_name`), `street_address`, `address_locality`, `address_country`는 필수, 보내는 사람이 모를 수 있는 `address_region`·`postal_code`는 선택 | `{"street_address": "48 Pine Rd", "address_locality": "Lakeview", "address_country": "US"}` |
  | card_message | Text | | 카드 문구(300자) | `"For the next trail. Love, J."` |
  | gift_wrap | Boolean | | 포장(`gift.wrap_fee`). 기본 false | `true` |
  | gift_receipt | Boolean | | 가격 없는 선물 영수증 동봉. 기본 true | `true` |
  | client_total | MonetaryAmount | ✓ | 보내는 사람이 본 합계. 선물 합계(포장비 포함) 또는 장바구니 합계(포장비 전) 중 하나여야 하고, 서버가 다시 계산해 비교 | `{"value": 76.20, "currency": "USD"}` |
- 출력(프리뷰):
  ```jsonc
  {"preview_id": "pv_g118", "expires_at": "2026-11-07T10:52:00-05:00",
   "changes": [{"op": "create", "entity": "order", "id": null, "from": null,
                "to": {"order_type": "gift",
                       "lines": [{"variant_id": "var_windbreaker_l_slate", "name": "Packable windbreaker", "color": "Slate", "size": "L", "quantity": 1,
                                  "price": {"value": 89.00, "currency": "USD"}}],
                       "subtotal": {"value": 89.00, "currency": "USD"},
                       "discounts": [{"label": "VIP weekend", "amount": {"value": 17.80, "currency": "USD"}, "promotion_id": "promo_1042"}],
                       "gift_wrap": {"value": 5.00, "currency": "USD"},
                       "shipping": {"value": 0.00, "currency": "USD"},
                       "total": {"value": 76.20, "currency": "USD"},
                       "delivery": {"address_label": "Dad · 48 Pine Rd, Lakeview", "arrives": "2026-11-10", "address_saved": false},
                       "gift_options": {"card_message": "For the next trail. Love, J.", "gift_receipt": true, "exchange_until": "2026-12-31"},
                       "payment": {"payment_method": "credit_card", "label": "Card ending 4821"}}}],
   "checks": [{"rule": "gift.recipient_per_order", "passed": true, "detail": "One recipient", "fix_tool": null},
              {"rule": "checkout_validation.stock_available", "passed": true, "detail": "Windbreaker L Slate is in stock", "fix_tool": null},
              {"rule": "checkout_validation.server_total_matches_client", "passed": true, "detail": "89.00 − 17.80 + 5.00 wrap = 76.20, same as shown", "fix_tool": null}],
   "needs_approval": false, "approval_reason": null}
  ```
  - 예: F4(Jamie가 아버지에게, Tue Nov 10 도착, 합계 $76.20). `exchange_until`은 `gift_exchange_until`.
  - `to`에는 `gift_options`(`wrap`, `wrap_fee`, `card_message`, `gift_receipt`, `receipt_hides_prices`, `exchange_until`), `recipient`(`name`, `mode`), 포장비 전 합계 `goods_total`도 실립니다. `recipient_accepts`면 `delivery`는 `{to, address_label: null, arrives: null, address_saved: false, recipient_contact, accept_within_days}`이고 주소는 받는 사람이 수락할 때 정해집니다.
  - checks는 `gift.recipient_per_order`와 `checkout_validation`의 여섯 항목입니다(`server_total_matches_client`의 `detail`에 포장비를 더한 합계가 붙음).
  - 주문하기(`POST /v1/sales/orders {preview_id}`)는 `sales_order.is_gift`·`gift_options`·포장비를 기록하고, 결제는 `address_known`이면 바로, `recipient_accepts`면 받는 사람이 수락할 때까지 보류합니다(`order_status: payment_due`). 이때 응답에 `gift_order_id`와 `accept_url`(`/gifts/{gift_order_id}`)이 더해지고, 수락 대기는 `sales.gift_order`(`action_status: potential`, `expires` = 주문 시각 + `gift.auto_cancel_if_no_address_days`)에 남습니다.
- 프리뷰: `create order`(`order_type: gift`, `id: null`). `needs_approval: false`(프리뷰만).
- 정책: `gift.recipient_per_order`, `gift.recipient_modes`, `gift.wrap_fee`, `gift.gift_receipt_hides_prices`, `gift.auto_cancel_if_no_address_days`, `gift_exchange_until`, `checkout_validation.stock_available`, `checkout_validation.server_total_matches_client`, [approval.yml](../../policies/approval.yml) `preview_only`
- 에러: `cart_empty` → `add_to_cart` · `out_of_stock` → `get_cart` · `address_not_deliverable`(`address_known`일 때) → `prepare_gift_order`
- 멱등성: 상태를 바꾸지 않음. 다시 부르면 이전 프리뷰는 `superseded`.

### `get_gift_order` — Kind: R
- 논리주소/REST: `sales.gift_order.get` · `GET /v1/sales/gift-orders/{gift_order_id}`
- 요약: 받는 사람이 수락 링크로 여는 선물: 무엇인지(이름·사진·사이즈·색·수량), 보낸 사람의 이름과 카드, 같은 가격으로 바꿀 수 있는 옵션, 언제까지 수락할 수 있는지. **가격은 싣지 않습니다**(`gift.gift_receipt_hides_prices`).
- consumes → produces: `gift_order_id` → `gift_order_id`
- 입력: `gift_order_id`(`^gift_`, ✓)
- 출력: `{"id": "gift_01jc9x2m", "action_status": "potential", "expires": "2026-11-11T09:00:00-05:00", "recipient_name": "Dad", "from": "Jamie", "card_message": "For the next trail. Love, J.", "gift_wrap": true, "exchange_until": "2026-12-31", "items": [{"name": "Packable windbreaker", "image": "/media/garment/jacket-navy.svg", "size": "L", "color": "Navy", "quantity": 1, "options": [{"size": "M", "color": "Navy", "available": true}, {"size": "L", "color": "Navy", "available": true}, {"size": "XL", "color": "Navy", "available": false}]}], "can_change": ["acceptance", "address", "options"], "ships_to": ["US"]}` — `action_status`는 `potential`(수락 대기), `completed`(수락함), `failed`(기한 경과로 취소). 기한이 지났지만 아직 정리되지 않은 선물도 `failed`로 보입니다.
  - `options`는 받는 사람이 고를 수 있는 같은 상품의 변형 중 **정가가 같은 것**만이고, 선물에 줄이 하나이고 `gift.recipient_can_change`에 `options`가 있을 때만 실립니다(아니면 `[]`). 지금 고른 변형도 들어 있습니다.
  - 링크의 `gift_order_id`가 곧 볼 권한입니다. 보내는 사람의 연락처·주소·결제, 받는 사람의 연락처는 싣지 않습니다.
- 프리뷰: 해당 없음(읽기)
- 정책: `gift.recipient_can_change`, `gift.gift_receipt_hides_prices`, `shipping.ships_to`
- 에러: `not_found`(`detail.entity: gift_order`) → null
- 멱등성: 읽기.

### `POST /v1/sales/gift-orders/{id}/accept` — 사람 전용
- 경로·스코프: `POST /v1/sales/gift-orders/{gift_order_id}/accept` · `human:order`(수락 링크로 연 세션). **툴이 아니며 모델은 보지 못합니다.** 받는 사람이 수락 링크의 버튼으로만 보냅니다.
- 요약: 받는 사람이 선물을 수락하고 배송지·옵션(사이즈·색)을 입력합니다(`recipient_accepts` 선물만). **가격은 바꿀 수 없습니다.** 수락하면 보류된 결제가 진행됩니다.
- consumes → produces: `gift_order_id` → `order_id`
- 입력: 경로 `gift_order_id`(`^gift_[a-z0-9_]+$`), 본문 `delivery_address`(PostalAddress, ✓, 선택 필드는 `prepare_gift_order`와 같음), `size`(Text), `color`(Text). 헤더 `Idempotency-Key`, `X-CSRF-Token`.
- 동작: 링크의 `gift_order_id`가 곧 수락 권한입니다(받는 사람은 이 스토어의 회원이 아닐 수 있음). `size`·`color`는 같은 상품의 다른 변형이고 **정가가 같아야** 하며(다르면 `invalid_input`, `detail.reason: price_differs`), 바꾸면 새 변형의 재고를 잡고 원래 변형의 재고를 돌려놓습니다. 카드 결제는 이때 완료(`payment_complete`)되고 주문은 `processing`이 됩니다. 수락 이벤트(`AcceptAction`)에 이번 주문에만 쓰는 배송지와 도착 예정이 남습니다.
- 출력: `{"ok": true, "id": "ord_20530", "undo": null}`
- 정책: `gift.recipient_can_change`(수락·주소·옵션만), `gift.auto_cancel_if_no_address_days`
- 에러(모두 `next_tool: null`): `gift_expired`(수락 기한 경과, 보내는 사람에게 알림) → null · `out_of_stock`(고른 옵션 품절, 다른 옵션 선택) → null
- 멱등성: 이미 수락한 선물은 첫 결과.
