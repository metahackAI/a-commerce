# API 명세 — `sales` (장바구니·주문 준비·주문·결제·영수증·구독·할부·선물 주문)

> 계층: `sales.{resource}.{operation}`. 공통 규약(응답 형식·에러·프리뷰/실행·멱등·헤더)은 [README.md](README.md). 데이터 타입은 [data-model.md](data-model.md). 정책은 [order.yml](policies/order.yml) · [approval.yml](policies/approval.yml). MVP 툴 시그니처는 [plan.md §3.4](../plan.md), 위치는 [architecture.md §3.4](../architecture.md).
> **설계 축**: ① 에이전트는 `prepare_order`로 주문을 **준비만** 하고, 주문하기는 쇼퍼가 누르는 사람 전용 `POST /orders`(모델은 보지 못함), ② 가격·할인은 서버가 `calculate_benefits`로 다시 계산한 값만 신뢰, ③ 주문·결제 쓰기는 멱등, ④ 주문 상태는 이벤트로 쌓음(덮어쓰기 금지).

리소스: [cart](#cart) · [checkout](#checkout) · [order](#order) · [payment](#payment) · [receipt](#receipt) · [subscription](#subscription) · [installment](#installment) · [gift_order](#gift_order)

---

## cart

### `get_cart` — Kind: R
- 논리주소/REST: `sales.cart.get` · `GET /v1/sales/carts/mine`
- 요약: 로그인한 쇼퍼 **본인의** 장바구니 조회. 라인, 소계, 회원 혜택, 배송비, 합계를 한 번에 반환. 혜택은 서버가 `calculate_benefits`로 계산.
- consumes → produces: `cart_id` → cart
- 입력: 없음(쇼퍼 본인 장바구니). 선택 `fields`(Text[])로 응답 축소.
- 출력: `acom:Cart`
```jsonc
{"cart_id": "cart_jamie",
 "item_list_element": [{"line_id": "ln_1", "product_id": "prd_windbreaker", "name": "Packable windbreaker",
                        "size": "M", "color": "slate", "order_quantity": 1, "price": 89.00}],
 "subtotal": 89.00,
 "benefit": {"label": "VIP weekend", "amount": -17.80, "promotion_id": "promo_1042"},
 "shipping": 0.00,
 "total_payment_due": 71.20,
 "price_currency": "USD"}
```
- 정책: 혜택 계산은 [promotion.yml](policies/promotion.yml) `stacking`.
- 에러: `cart_not_found` → add_to_cart
- 멱등성: 읽기(부작용 없음).

### `add_to_cart` — Kind: W (승인 없음)
- 논리주소/REST: `sales.cart.add` · `POST /v1/sales/carts/mine/items`
- 요약: 상품을 사이즈·색상·수량과 함께 쇼퍼 본인 장바구니에 담기. 장바구니가 없으면 만든다. 프리뷰는 변경셋에 들어가지만 정책상 승인이 필요 없어 바로 실행된다.
- consumes → produces: `product_id` → `cart_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | product_id | Text | ✓ | `prd_windbreaker` |
  | size | Text | ✓ | `"M"` |
  | color | Text | ✓ | `"slate"` |
  | order_quantity | Integer | | 기본 1, 구매 한도 이내 |
  | preview | Boolean | | 기본 true |
- 출력: 갱신된 `acom:Cart`(`cart_id` 포함, 형식은 `get_cart`와 같음)
- 프리뷰: `changes: [{op: "create", entity: "cart_line", id: "ln_1", from: null, to: {product_id, size, color, order_quantity}}]`, `needs_approval: false`
- 정책: `order.cart.max_lines`, `order.cart.no_mix`(국내/해외 혼합 금지)
- 에러: `out_of_stock` → search_products · `invalid_variant` → get_product · `purchase_limit_exceeded` → update_cart_item · `mixed_order_type` → remove_cart_item
- 멱등성: `(cart_id, product_id, size, color)` 기준 upsert. 같은 라인을 다시 담으면 수량을 `order_quantity`로 맞춘다.

### `update_cart_item` — Kind: W
- 논리주소/REST: `sales.cart.update` · `PATCH /v1/sales/carts/mine/items/{line_id}`
- 요약: 라인의 수량·사이즈·색상 변경(판매 중인 변형으로만).
- consumes → produces: `cart_id` → `cart_id`
- 입력: line_id(Text,✓), order_quantity(Integer), size(Text), color(Text), preview(Boolean)
- 출력: `acom:Cart`
- 에러: `invalid_variant` → get_product · `line_not_found` → get_cart
- 멱등성: 같은 값으로 재요청 시 같은 결과.

### `remove_cart_item` — Kind: W
- 논리주소/REST: `sales.cart.remove` · `DELETE /v1/sales/carts/mine/items/{line_id}`
- 요약: 라인 삭제. 입력: line_id(✓). 출력: `acom:Cart`. 에러: `line_not_found` → get_cart. 멱등성: 이미 없는 라인 삭제는 성공으로 처리.

### `clear_cart` — Kind: W
- 논리주소/REST: `sales.cart.clear` · `DELETE /v1/sales/carts/mine/items`
- 요약: 장바구니 비우기. 입력: 없음. 출력: 빈 `acom:Cart`. 멱등성: 반복 호출해도 같은 결과.

---

## checkout

### `get_payment_methods` — Kind: R
- 논리주소/REST: `sales.checkout.list_payment_methods` · `GET /v1/sales/checkout/payment-methods?cart_id=`
- 요약: 장바구니의 모든 상품에 **공통으로 쓸 수 있는 결제수단**과 할부·적립금 사용 가능 여부. `prepare_order`의 `common_payment_method_exists` 검사가 실패했을 때 다음 툴.
- consumes → produces: `cart_id` → payment methods
- 입력: cart_id(Text,✓)
- 출력: `{items: PaymentMethod[], total}`. 각 항목은 `{payment_method: "credit_card", label: "Card ending 4821", acom:installment_options: [0, 3, 6], acom:loyalty_usable: true}`
- 정책: `order.payment.methods`, `order.payment.primary_methods_per_order`
- 에러: `cart_not_found` → get_cart
- 멱등성: 읽기.

---

## order

### `prepare_order` — Kind: W (프리뷰만)
- 논리주소/REST: `sales.order.prepare` · `POST /v1/sales/orders/prepare`
- 요약: 장바구니로 **쇼퍼가 주문할 수 있는 상태의 주문을 준비**한다. 이 툴은 프리뷰만 만든다(`preview_id`로 실행하는 단계가 없음). 주문하기는 쇼퍼가 버튼으로 보내는 사람 전용 `POST /orders`뿐이다. schema.org로는 `PotentialActionStatus`의 `OrderAction`이고, 쇼퍼의 탭이 이를 완료한다.
- consumes → produces: `cart_id` → `order_preview_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | cart_id | Text | ✓ | `cart_jamie` |
  | delivery_address | Text \| PostalAddress | ✓ | `"home"`(저장된 집 주소) 또는 이번 한 번만 쓸 주소 |
  | gift | Boolean | | 선물 주문으로 준비(기본 false). 받는 사람 정보는 `prepare_gift_order` |
- 출력(프리뷰): `preview_id`가 곧 `order_preview_id`
```jsonc
{"preview_id": "op_7731",
 "changes": [{"op": "create", "entity": "order", "id": "op_7731", "from": null,
              "to": {"lines": 1, "delivery_address": "Home · 12 Harbor St, Apt 3", "arrives": "2026-11-10",
                     "pay_with": "Card ending 4821", "total_payment_due": 71.20, "price_currency": "USD"}}],
 "checks": [{"rule": "common_payment_method_exists", "passed": true, "detail": "Card ending 4821 works for every line", "fix_tool": null},
            {"rule": "stock_available", "passed": true, "detail": "Windbreaker M slate: 18 in stock", "fix_tool": null},
            {"rule": "server_price_ge_client_price", "passed": true, "detail": "Recalculated with calculate_benefits: 89.00 − 17.80 = 71.20", "fix_tool": null}],
 "needs_approval": true,
 "approval_reason": "Only the shopper can place the order"}
```
- 프리뷰: 검사 3가지 — ① 모든 라인에 공통 결제수단이 있는가, ② 재고가 있는가, ③ 가격을 서버에서 promotion `calculate_benefits`로 다시 계산한 값이 화면 가격과 맞는가(서버 계산가 ≥ 클라이언트가). 실패한 검사는 `fix_tool`에 다음 툴을 담는다.
- 정책: `order.checkout_validation`(`common_payment_method_exists`, `stock_available`, `product_sellable`, `no_domestic_overseas_mix`, `server_price_ge_client_price`, `margin_not_reversed`), `order.confirm_to_buy`
- 에러: `cart_empty` → add_to_cart · `no_common_payment_method` → get_payment_methods · `out_of_stock` → search_products · `address_not_found` → get_my_profile · `mixed_order_type` → remove_cart_item
- 멱등성: 부작용 없음(재고 예약·결제 없음). 같은 장바구니로 다시 부르면 새 `order_preview_id`를 만들고 이전 것은 만료된다.

### `POST /orders` — Kind: 사람 전용
- 논리주소/REST: `POST /orders` `{order_preview_id}` — **툴이 아님. 모델은 보지 못하고 부를 수 없다.** 쇼퍼가 UI의 주문하기 버튼으로만 보낸다.
- 요약: 쇼퍼가 준비된 주문을 **주문한다**. 재고를 예약하고 결제를 시작한다.
- consumes → produces: `order_preview_id` → `order_id`
- 입력: order_preview_id(Text,✓), `Idempotency-Key` 헤더
- 출력: `{"ok": true, "id": "ord_20518", "undo": null}` — 주문 취소는 사람 전용 `POST /orders/{id}/cancel`([fulfillment.md](fulfillment.md)).
- 정책: `order.inventory.reserve_on`(주문 시점 예약), `order.inventory.unpaid_ttl_minutes`(미결제 자동 취소), `order.confirm_to_buy`
- 에러: `preview_expired` → prepare_order · `price_changed` → prepare_order · `out_of_stock` → prepare_order · `payment_declined` → get_payment_methods
- 멱등성: 같은 `order_preview_id`(또는 같은 `Idempotency-Key`)로 다시 보내면 첫 주문을 그대로 반환. 주문은 하나만 생긴다.

### `get_my_orders` — Kind: R
- 논리주소/REST: `sales.order.list_mine` · `GET /v1/sales/orders/mine?limit=&offset=`
- 요약: 쇼퍼 본인의 주문 목록(라인과 배송일 포함). 반품 흐름의 시작점(`get_return_options`).
- consumes → produces: → `order_id[]`
- 입력: limit(Integer, 기본 10), offset(Integer, 기본 0)
- 출력: `{items: [{order_id: "ord_20402", order_number: "#20402", order_status: "delivered", ordered_item: [...], delivered_at: "2026-10-31", total_payment_due: 312.00, price_currency: "USD"}], total}`
- 에러: 없음(주문이 없으면 `items: []`, `total: 0`).
- 멱등성: 읽기.

### `get_order` — Kind: R
- 논리주소/REST: `sales.order.get` · `GET /v1/sales/orders/{order_id}`
- 요약: 주문 한 건(라인, 결제, 배송일, 사용한 쿠폰 `discount_code`). 운영자는 모든 주문, 쇼퍼는 본인 주문만.
- consumes → produces: `order_id` → order
- 입력: order_id(Text,✓), fields(Text[])
- 출력: `Order`
- 에러: `order_not_found` → search_orders
- 멱등성: 읽기.

### `search_orders` — Kind: R
- 논리주소/REST: `sales.order.search` · `GET /v1/sales/orders?order_status=&customer_id=&from=&through=&limit=&offset=`
- 요약: 운영자용 주문 검색.
- consumes → produces: → `order_id[]`
- 입력: order_status(Text), customer_id(Text), from / through(Date), seller_id(Text), sort(Text), limit(Integer, 기본 20), offset(Integer, 기본 0), fields(Text[])
- 출력: `{items: Order[], total}`
- 에러: `invalid_filter` → list_enums
- 멱등성: 읽기.

### `verify_guest` — Kind: W
- 논리주소/REST: `sales.order.verify_guest` · `POST /v1/sales/orders/verify-guest`
- 요약: 비회원 주문 조회 인증(주문번호 + 연락처, 설정 시 주문 비밀번호). 성공하면 그 주문만 볼 수 있는 단기 토큰 발급. 쓰기지만 승인 없이 바로 실행.
- consumes → produces: `order_number, telephone` → `acom:guest_token`
- 입력: order_number(Text,✓), telephone(Text,✓), acom:order_password(Text, 설정 시)
- 출력: `{ok: true, id: "ord_20518", acom:guest_token: "...", expires_in: 1800}`
- 정책: 비회원 조회 횟수 제한, 개인정보 마스킹.
- 에러: `guest_match_failed` → (다시 입력) · `too_many_attempts` → (잠시 후 다시 시도)
- 멱등성: 토큰 발급은 상태가 없어 멱등 키가 필요 없음.

### `change_shipping_address` — Kind: W
- 논리주소/REST: `sales.order.change_shipping_address` · `POST /v1/sales/orders/{order_id}/delivery-address`
- 요약: 주문의 배송지 변경(**출고 전만**). 지역 추가 배송비를 다시 계산.
- consumes → produces: `order_id` → `order_id`
- 입력: order_id(Text,✓), delivery_address(PostalAddress,✓), preview(Boolean,✓)
- 출력(프리뷰): `{preview_id, changes: [{op: "update", entity: "order", id: "ord_20518", from: "12 Harbor St, Apt 3", to: "40 Pier Rd"}], checks: [{rule: "allowed_before_shipped", passed: true, detail: "Not shipped yet", fix_tool: null}, {rule: "regional_surcharge", passed: true, detail: "Surcharge +0.00", fix_tool: null}], needs_approval: false, approval_reason: null}`
- 출력(실행): `{ok: true, id: "ord_20518", undo: {tool: "change_shipping_address", until: "<출고 전까지>"}}`
- 정책: `order.change.allowed_before`(shipped), [fulfillment.yml](policies/fulfillment.yml) 지역 추가 배송비
- 에러: `already_shipped` → get_order · `invalid_postal_code` → (다시 입력) · `invalid_state` → get_order
- 멱등성: `Idempotency-Key`

---

## payment

### `get_payment` — Kind: R
- 논리주소/REST: `sales.payment.get` · `GET /v1/sales/orders/{order_id}/payment`
- 요약: 주문의 결제 상태·수단·분할 내역.
- consumes → produces: `order_id` → payment
- 입력: order_id(Text,✓)
- 출력: `{payment_status: "paid", payment_method: "credit_card", label: "Card ending 4821", total_payment_due: 71.20, price_currency: "USD", acom:installment_months: 0}`
- 에러: `order_not_found` → search_orders
- 멱등성: 읽기.

### `confirm_virtual_account_deposit` — Kind: W$
- 논리주소/REST: `sales.payment.confirm_deposit` · `POST /v1/sales/orders/{order_id}/payment/confirm-deposit`
- 요약: 가상계좌 입금을 운영자가 수동 확인. 상태 `payment_pending → paid`. 돈이 움직이므로 항상 사람 승인.
- consumes → produces: `order_id` → `order_id`
- 입력: order_id(Text,✓), amount(Number,✓, 예 71.20), preview(Boolean,✓)
- 출력(프리뷰): `{preview_id, changes: [{op: "update", entity: "payment", id: "ord_20518", from: "payment_pending", to: "paid"}], checks: [{rule: "virtual_account_hold_days", passed: true, detail: "Deposit within 3 days", fix_tool: null}], needs_approval: true, approval_reason: "Confirms a payment"}`
- 정책: `order.payment.virtual_account_hold_days`
- 에러: `deadline_passed` → get_order · `already_paid` → get_order · `amount_mismatch` → get_payment
- 멱등성: `Idempotency-Key`. 이미 `paid`면 첫 결과 반환.

---

## receipt

### `issue_receipt` — Kind: W
- 논리주소/REST: `sales.receipt.issue` · `POST /v1/sales/orders/{order_id}/receipt`
- 요약: 영수증·세금계산서 발급.
- consumes → produces: `order_id` → `receipt_id`
- 입력: order_id(Text,✓), acom:receipt_type(`deduction` \| `proof` \| `tax_invoice`,✓), tax_id(Text, 사업자일 때), preview(Boolean)
- 출력: `acom:Receipt` 또는 `Invoice`
- 정책: 같은 달 거래만, 클레임 시 취소 후 다시 발급.
- 에러: `business_id_required` → (입력 요청) · `already_issued` → get_receipt
- 멱등성: `(order_id, receipt_type)` 한 건.

### `get_receipt` — Kind: R
- 논리주소/REST: `sales.receipt.get` · `GET /v1/sales/orders/{order_id}/receipt`
- 요약: 발급된 영수증·세금계산서 조회. 입력: order_id(✓). 출력: `acom:Receipt` / `Invoice`. 에러: `receipt_not_found` → issue_receipt.

---

## subscription

### `prepare_subscription` — Kind: W (프리뷰만)
- 논리주소/REST: `sales.subscription.prepare` · `POST /v1/sales/subscriptions/prepare`
- 요약: 장바구니로 정기 주문을 **준비만** 한다(주기, 첫 결제일, 결제 카드). 구독 시작은 쇼퍼의 사람 전용 `POST /subscriptions`뿐이다.
- consumes → produces: `cart_id` → `subscription_preview_id`
- 입력: cart_id(Text,✓), acom:billing_cycle(`every_week` \| `every_2_weeks` \| `every_month`,✓), next_charge_date(Date,✓), payment_method_id(Text, 카드,✓), delivery_address(`"home"` 또는 PostalAddress,✓)
- 출력(프리뷰): `{preview_id: "sp_2210", changes: [{op: "create", entity: "subscription", id: "sp_2210", from: null, to: {billing_cycle: "every_month", next_charge_date: "2026-12-01", amount: 58.00, price_currency: "USD"}}], checks: [{rule: "subscription_card_only", passed: true, detail: "Card ending 4821", fix_tool: null}, {rule: "stock_available", passed: true, detail: "In stock", fix_tool: null}], needs_approval: true, approval_reason: "Only the shopper can start a subscription"}`
- 정책: `order.payment.subscription_card_only`, 할인은 프로모션 판매가(1계층)만, 적립금 사용 불가.
- 에러: `card_required` → get_payment_methods · `past_date` → (미래 날짜 지정)
- 멱등성: 부작용 없음.

### `POST /subscriptions` — Kind: 사람 전용
- 논리주소/REST: `POST /subscriptions` `{subscription_preview_id}` — **툴이 아님. 모델은 보지 못한다.** 쇼퍼가 UI 버튼으로만 보낸다.
- 요약: 쇼퍼가 준비된 정기 주문을 시작한다. 이후 주기마다 주문이 자동 생성된다.
- consumes → produces: `subscription_preview_id` → `subscription_id`
- 입력: subscription_preview_id(Text,✓), `Idempotency-Key` 헤더
- 출력: `{"ok": true, "id": "sub_3301", "undo": null}` — 해지는 `cancel_subscription`.
- 에러: `preview_expired` → prepare_subscription · `card_declined` → get_payment_methods
- 멱등성: 같은 `subscription_preview_id`로 다시 보내면 첫 결과 반환.

### `pause_subscription` / `resume_subscription` / `cancel_subscription` — Kind: W
- 논리주소/REST: `sales.subscription.{pause|resume|cancel}` · `POST /v1/sales/subscriptions/{subscription_id}/{pause|resume|cancel}`
- 요약: 상태 전이 `active ↔ paused → cancelled`.
- consumes → produces: `subscription_id` → `subscription_id`
- 입력: subscription_id(Text,✓), preview(Boolean)
- 출력(실행): `{ok: true, id: "sub_3301", undo: {tool: "resume_subscription", until: null}}`
- 에러: `invalid_state` → list_subscriptions · `subscription_not_found` → list_subscriptions
- 멱등성: 같은 상태로의 전이는 성공으로 처리.

### `update_schedule` — Kind: W
- 논리주소/REST: `sales.subscription.update_schedule` · `PATCH /v1/sales/subscriptions/{subscription_id}/schedule`
- 요약: 정기 주문의 주기·다음 결제일·수량·배송지 변경(다음 회차부터).
- consumes → produces: `subscription_id` → `subscription_id`
- 입력: subscription_id(Text,✓), acom:billing_cycle(enum), next_charge_date(Date), order_quantity(Integer), delivery_address(PostalAddress), preview(Boolean,✓)
- 출력: `acom:SubscriptionOrder`
- 정책: `order.payment.subscription_card_only`
- 에러: `invalid_cycle` → list_enums · `past_date` → (미래 날짜 지정) · `subscription_not_found` → list_subscriptions
- 멱등성: `Idempotency-Key`

### `list_subscriptions` — Kind: R
- 논리주소/REST: `sales.subscription.list` · `GET /v1/sales/subscriptions?customer_id=&status=&limit=&offset=`
- 요약: 정기 주문 목록. 쇼퍼는 본인 것만. 출력: `{items: acom:SubscriptionOrder[], total}`.

---

## installment

### `set_installment` — Kind: W
- 논리주소/REST: `sales.installment.set` · `POST /v1/sales/orders/{order_id}/installment`
- 요약: 신용카드 할부 개월 수 설정(결제 확정 전). 카드사·금액별 가능 개월과 무이자 조건은 `get_payment_methods`의 `acom:installment_options`를 따른다.
- consumes → produces: `order_id` → `order_id`
- 입력: order_id(Text,✓), acom:installment_months(Integer,✓, 0은 일시불), preview(Boolean,✓)
- 출력(프리뷰): `{preview_id, changes: [{op: "update", entity: "payment", id: "ord_20518", from: 0, to: 3}], checks: [{rule: "installment_min_amount", passed: true, detail: "Monthly 23.73, interest free", fix_tool: null}], needs_approval: false, approval_reason: null}`
- 출력(실행): `{ok: true, id: "ord_20518", undo: {tool: "set_installment", until: "<결제 확정 전까지>"}}`
- 정책: `order.payment.installment`(카드 전용, 최소 금액)
- 에러: `months_not_available` → get_payment_methods · `below_min_amount` → (일시불) · `not_card_payment` → get_payment_methods
- 멱등성: `Idempotency-Key`

---

## gift_order

### `prepare_gift_order` — Kind: W
- 논리주소/REST: `sales.gift_order.prepare` · `POST /v1/sales/gift-orders`
- 요약: `prepare_order(gift=true)`로 준비한 주문에 받는 사람과 메시지를 붙인다. 보내는 사람이 `POST /orders`로 주문하면 받는 사람에게 수락 링크가 가고, **받는 사람이 수락하기 전에는 결제가 보류**된다. 에이전트는 준비만 한다. 선물 요청은 쇼퍼 본인 선호로 저장하지 않는다.
- consumes → produces: `order_preview_id` → `gift_order_id`
- 입력: order_preview_id(Text,✓), recipient(ContactPoint,✓), message(Text), acom:gift_card_template(Text), preview(Boolean)
- 출력: `{ok: true, id: "gift_118", acom:accept_url: "...", acom:expires_in_days: 3}`
- 정책: `order.gift.recipient_per_order`(1), `order.gift.auto_cancel_if_no_address_days`(3)
- 에러: `not_gift_order` → prepare_order · `preview_expired` → prepare_order
- 멱등성: `(order_preview_id)` 한 건.

### `POST /gift-orders/{id}/accept` — Kind: 사람 전용
- 논리주소/REST: `POST /gift-orders/{id}/accept` — **툴이 아님. 모델은 보지 못한다.** 받는 사람이 수락 링크의 버튼으로만 보낸다.
- 요약: 받는 사람이 선물을 수락하고 배송지·옵션(사이즈·색상)을 입력한다. **가격은 바꿀 수 없다.** 수락하면 보류된 결제가 진행된다.
- consumes → produces: `gift_order_id` → `order_id`
- 입력: delivery_address(PostalAddress,✓), size(Text), color(Text)
- 출력: `{"ok": true, "id": "ord_20530", "undo": null}`
- 정책: `order.gift.recipient_can_change`(수락·주소·옵션만), [approval.yml](policies/approval.yml) `customer_confirmation_required`
- 에러: `gift_expired` → (보내는 사람에게 알림) · `option_out_of_stock` → (다른 옵션 선택)
- 멱등성: 이미 수락한 선물은 첫 결과 반환.
