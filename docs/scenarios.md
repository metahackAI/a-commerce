# 데모 시나리오 (시드와 테스트의 기준)

데모 스토어 Northwind Apparel의 사람·상품·정책·장면을 한곳에 정리합니다. 근거는 [design/](../design) 시안과 [design.md](design.md) §9이고, 시안끼리 숫자가 다른 곳은 §8에서 하나로 정했습니다. 시안에 없는 값은 우리가 정한 값이며 **[정함]** 으로 표시합니다.

이 문서가 쓰이는 곳:
- `seed/demo/`의 시드 데이터(이 문서와 숫자가 같아야 함)
- 시나리오 재생 테스트와 평가 과업의 기대값([backend/testing.md](backend/testing.md) §2–3)

---

## 1. 스토어

| 항목 | 값 |
|---|---|
| 이름 | Northwind Apparel (스토어프론트 표기 NORTHWIND), `northwind.example` |
| 테넌트 id | `store_northwind` |
| 통화 | USD, 센트 단위 반올림(반올림은 0.5에서 올림) |
| 시간대 | **America/New_York [정함]**. 11월 기준 UTC−05:00 |
| 단위 | g, cm. 의류 사이즈 XS–XL, 바지 28–36, 신발 US 7–12 |
| 세금 | 범위 밖, 0 **[정함]** |
| 배송비 | 기본 $6.00, 할인 전 상품 소계 $80 이상이면 무료(진행 중 프로모션 "Free shipping over $80") **[정함]** |
| 모델 표기 | 운영 에이전트 "Muse Glimmer 30B, hands planning to Muse Spark when needed" |

### 1.1 데모 시계

| 장면 묶음 | 시계 |
|---|---|
| 백오피스 전부 | **Fri Nov 6, 2026**. 장면 시각 07:55–11:20, 기본값 11:00 |
| Alex 반품 요청 | Fri Nov 6, 09:50 (반품 검토 10:40보다 앞) |
| Jamie 구매·핏·선물, Mina | **Sat Nov 7, 2026** 10:00 |

시나리오 날짜가 해커톤 이후이므로 시계는 항상 데모 시계(`platform.demo_clock`)로 맞춥니다.

---

## 2. 회원 등급과 사람

### 2.1 등급 프로그램 [정함]

| 등급 id | 이름 | 순서 | 회원 수 | 상시 혜택 |
|---|---|---|---|---|
| `member` | Member | 1 | 9,816 | 생일 쿠폰 |
| `gold` | Gold | 2 | 2,900 | 등급 쿠폰 "Gold tier 10%" |
| `vip` | VIP | 3 | **1,284** | 없음(주말 세일 같은 한정 혜택 대상) |

- 등급은 배타적이며 회원은 하나의 등급에만 속합니다. 등급과 승급 규칙은 `policies/customer.yml`의 구조를 따르고, 이 표는 데모 스토어의 데이터입니다.
- 할인 상한(정책 덮어쓰기): 전체 20%, `vip` 30%.

### 2.2 운영자

| id | 이름 | 역할 |
|---|---|---|
| `adm_sj` | Son SeongJun ("SJ") | merchandiser. 승인 권한: 프로모션·전시·환불·메시지 |

### 2.3 쇼퍼

| id | 이름 | 등급 | 사실 |
|---|---|---|---|
| `cus_jamie` | Jamie | vip | 주문 6건(아래), 프로필: Size M · Natural fabrics · Compares specs · Travels light. 배송지 "Home · 12 Harbor St, Apt 3, Brooklyn, NY 11201"(도시·우편번호 **[정함]**). 카드 끝자리 4821 |
| `cus_alex` | Alex | member **[정함]** | 주문 #20402(부츠). 카드 끝자리 0937 |
| (게스트) | Mina | — | 첫 방문, 로그인 안 함, 휴대폰. 기억 없음 |
| — | Jamie의 아버지 | — | 선물 수령인. "48 Pine Rd, Lakeview". 쇼퍼 프로필에 저장하지 않음 |

Jamie의 과거 주문 6건 **[정함]**: 가장 최근 2건이 M 사이즈이고, 그중 하나가 **Packable windbreaker M(Black), 9월 배송, 반품 없음**입니다(핏 장면의 "Windbreaker M, kept"). 나머지 4건은 시드 생성 규칙(§7)으로 만듭니다.

---

## 3. 상품

### 3.1 이름이 나오는 상품

원가는 모두 **[정함]** 입니다(시안에 없음). 세일 대상 상품의 원가는 `정가 × 0.8 ≥ 원가 × 1.15`를 지켜야 20%에서 하한을 넘지 않습니다. 단, Seersucker overshirt는 20%에서 하한에 걸려야 하므로 원가를 그 구간((54.96, 58.39])에서 골랐습니다.

| id | 상품 | 카테고리 | 정가 | 원가 | 재고(합) | 사양 |
|---|---|---|---|---|---|---|
| `prd_field_jacket` | Linen blend field jacket | summer_outerwear | $128.00 | $56.00 | 86 | Sand 외 3색, linen·cotton, M 340 g, folds flat, warmth light, rain no |
| `prd_windbreaker` | Packable windbreaker | summer_outerwear | $89.00 | $36.00 | 214 | Slate 외 4색, recycled nylon, M 190 g, 자기 주머니에 접힘 14 × 18 cm, wind layer only, rain yes. M 실측: 가슴 57 / 어깨 47 / 소매 64 / 기장 71 cm |
| `prd_seersucker_overshirt` | Seersucker overshirt | summer_outerwear | $79.00 | **$58.00** | 53 | Clay 외 2색, cotton, M 360 g, folds flat |
| `prd_utility_jacket` | Cotton utility jacket | summer_outerwear | $118.00 | $50.00 | 31 | Navy, M 480 g, folds flat |
| `prd_chore_coat` | Cotton chore coat | summer_outerwear | **$142.00** | $60.00 | **7** (S 3, XL 4, M·L 0) | Olive, 620 g. M·L은 Wed Nov 4 15:00 품절, "back Nov 12" |
| `prd_rain_parka` | Light rain parka | summer_outerwear | **$118.00** | $50.00 | **4** (S 2, XL 2, M·L 0) | Stone, 420 g, rolls into hood. M·L Wed 15:00 품절 |
| `prd_camp_overshirt` | Linen camp overshirt | summer_outerwear | $88.00 | $36.00 | 40 | Sand, 300 g, folds flat |
| `prd_wool_overcoat` | Wool overcoat | outerwear | $289.00 | $120.00 | 24 | Charcoal, 70% wool. M: 56/46/64/98, L: 59/48/65.5/100 cm. 30일 내 사이즈 교환 무료. 리뷰 41건(175–182 cm): true to size 29, a bit snug 12, "size up for a hoodie" 9회 |
| `prd_chelsea_boots` | Leather chelsea boots | shoes | $312.00 | $130.00 | Tan 9: 6 | Tan, US 7–12 |
| `prd_cashmere_sweater` | Cashmere sweater | knitwear | $248.00 | $100.00 | 18 | |
| `prd_fleece_half_zip` | Fleece half-zip | outerwear | $84.00 | $34.00 | 30 | "Warm layer for cold starts, fits under a shell" |
| `prd_merino_tee` | Merino trail tee | shirts | $58.00 | $22.00 | 45 | "Doesn't smell after a long day" |
| `prd_denim_jacket` | Denim jacket | outerwear | $118.00 **[정함]** | $48.00 | M·L 0 | bestseller, Wed 15:00 M·L 품절, "back Nov 12" |
| `prd_linen_shirt` | Linen shirt | shirts | $68.00 **[정함]** | $26.00 | 38 | 최근 7일 61개 판매 → 약 4일 뒤 품절 |

**Summer outerwear는 42개 상품**입니다. 위의 7개 + 생성 상품 35개 **[정함]**(§7 규칙). 세일 대상 40개 = 42 − 재고 10개 미만 2개(chore coat, rain parka).

### 3.2 원가 이하 상품 4개 (금요일 07:55에 감지) [정함]

`prd_breton_tee`($19.00, 원가 $20.00), `prd_canvas_tote`($24.00, 원가 $25.00), `prd_ribbed_beanie`($14.00, 원가 $15.00), `prd_hiking_socks`($12.00, 원가 $13.00). 자동화가 08:31에 하한(원가 × 1.15)으로 올렸고 되돌리기 기한은 Nov 7 23:59입니다.

### 3.3 공급사 시트 초안 (FW26, 미게시)

`fw26_supplier.xlsx`, 30행. 가격 규칙 "원가 × 2.5, 가장 가까운 x9, 동률이면 위로" **[정함]**.

| 초안 | 원가 → 판매가 | 상태 |
|---|---|---|
| Wool blend overcoat, charcoal | $96 → $239 | ready |
| Merino crew-neck sweater, navy | $38 → $99 | ready |
| Flannel overshirt, check | $30 → $79 | ready |
| Quilted liner jacket, olive | $52 → **$129** | needs_you: 충전재 비율 없음 |
| Corduroy trousers, sand | $34 → $89 | needs_you: 사진에 두 상품 |
| Lambswool cardigan, oatmeal | $44 → $109 | 설명 작성 중 |

전체 30행: ready 14, needs_you 4, 작업 중 12.

---

## 4. 진행 중인 프로모션 (금요일 기준)

| id | 이름 | 상태 | 대상 | 기간 |
|---|---|---|---|---|
| `promo_autumn_knit` | Autumn knitwear 15% | running | everyone | Nov 1 – Nov 15 |
| `promo_free_ship_80` | Free shipping over $80 | running | everyone | 상시 |
| `promo_birthday` | Birthday coupon | running | member | 상시 |
| `promo_gold_10` | Gold tier 10% | running | gold | 상시 |
| `promo_denim_bundle` | Denim bundle | scheduled | everyone | Nov 13 – Nov 20 |
| `promo_linen_clearance` | Linen clearance | ended | everyone | Oct 1 – Oct 20 |

메인 페이지 hero 슬롯(`slot_home_hero`)은 대상별 배치를 가집니다. 지금은 모두에게 "Fall layers"(Fri 10:00부터)가 나오고, VIP 대상 배치는 Nov 7–8에 비어 있습니다.

---

## 5. 데모 정책 (스토어 덮어쓰기)

`policies/*.yml` 기본값 위에 데모 스토어가 덮어쓰는 값입니다(`seed/demo/policy-overrides/*.yml`).

| 정책 키 | 값 |
|---|---|
| `promotion.max_discount.everyone` | 0.20 |
| `promotion.max_discount.by_tier.vip` | 0.30 |
| `catalog.pricing.price_floor` | 원가 × 1.15 (같아도 통과) |
| `promotion.promo_min_stock` | 10 (미만은 세일에서 제외) |
| `promotion.member_benefits_per_order` | 1 (쿠폰은 등급 쿠폰과 중복 불가) |
| `approval.thresholds.refund` | $200.00 초과 환불은 사람 승인 |
| `approval.customer_messages` | always_ask |
| `fulfillment.return_window_days` | 30 |
| `fulfillment` 불량 | 전액 환불, 반품 배송비 무료 |
| `fulfillment` 단순 변심 | 30일 안, 반품 배송비 고객 부담 |
| `fulfillment` 파손 주장 | 사진 필수 |
| `fulfillment` 반품 이력 플래그 | 90일 안 4회 |
| `approval.change_set.undo_window_after_end_days` | 1 |

---

## 6. 장면

장면마다 기대 결과를 적었습니다. 시나리오 재생 테스트는 이 결과를 DB에서 확인합니다.

### B1. VIP 주말 세일 (변경셋 #1042)

- **요청(11:02):** "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page."
- **해석:** Who VIP tier(1,284명) · Products summer outerwear 40 of 42 · Benefit 20%, overshirt 15% · When Sat Nov 7 00:00 – Sun Nov 8 23:59 · Where 메인 hero, VIP에게만.
- **v1(11:02):** 42개 상품 모두 20% → 체크 6개 중 2개 실패: `price_floor`(overshirt $63.20 < 하한 $66.70), `promo_min_stock`(chore coat 7, rain parka 4).
- **대화 수정(11:05):** "Drop the low-stock ones. Give the overshirt 15% instead." → **v2**.
- **v2 체크 6개 모두 통과:**

| 체크 키 | 화면 문구 | 세부 |
|---|---|---|
| `max_discount` | Discount within the VIP limit | Highest is 20%; policy allows 30% |
| `price_floor` | No price below the floor | Overshirt moved to 15% to stay at cost + 15% |
| `promo_min_stock` | Enough stock | 2 products under 10 units excluded |
| `promotion_overlap` | No overlap with running promotions | Checked 4 running promotions |
| `member_benefits_per_order` | Coupon doesn't stack with tier coupons | One member benefit per order |
| `slot_free` | Main page slot is free | Hero slot open Nov 7–8 for VIP |

- **변경(승인 후 실행 순서):**
  1. `set_sale_prices` — 40개 상품의 VIP 회원가(`UnitPriceSpecification`, `price_type: sale_price`, `valid_for_member_tier: vip`). 예: field jacket $128.00 → $102.40, windbreaker $89.00 → $71.20, overshirt $79.00 → $67.15, utility jacket $118.00 → $94.40.
  2. `create_coupon` — `VIPWEEKEND20`, VIP 1,284명, 결제 시 자동 적용, 등급 쿠폰과 중복 불가. **두 번째 할인이 아니라** 회원가 혜택을 주문에 기록하는 쿠폰입니다(`promotion.coupon.records_benefit_only`).
  3. `schedule_banner` — `slot_home_hero`, VIP 대상, Nov 7 00:00 – Nov 8 23:59. 문구 "VIP WEEKEND / 20% off summer outerwear / Shop the sale", 이미지 field jacket(Sand).
  4. `verify_change_set` — 셋을 다시 읽어 모두 일치 → `live`.
  - 제외 2개(chore coat, rain parka)는 실행이 아니라 프리뷰에 남는 기록입니다.
- **되돌리기 기한:** Nov 9, 23:59 (종료 + 1일).
- **승인 전 툴 호출:** 9회 [정함]: `get_policy`, `search_products`, `list_tiers`, `list_active_promotions`, `list_display_slots`, 프리뷰 3회(`set_sale_prices`, `create_coupon`, `schedule_banner`), 대화 수정 후 `set_sale_prices` 재프리뷰.

### B2. 반품 12건 검토 (10:40, 에이전트 시작)

- 감시 작업이 "$200 초과 반품 요청 12건 대기"를 할 일로 만들고, 운영 에이전트가 정책 기준으로 분류합니다(읽기만, 14회 툴 호출).
- **분류:** 환불 9건 $2,463.00 · 사진 요청 2건 · 거절 1건.

| 주문 | 상품 | 금액 | 사유 · 경과 | 제안 |
|---|---|---|---|---|
| #20402 | Leather chelsea boots | $312.00 | 불량(밑창), 사진 2, 6일 | 환불 |
| #20431 | Wool overcoat | $289.00 | 색상 차이, 사진 1, 4일 | 환불 (고객 90일 4회 반품 플래그) |
| #20418 | Linen blend field jacket ×2 | $256.00 | 작음, 7일 | 환불 |
| #20377 | Field jacket + windbreaker | $217.00 | 단순 변심, 9일 | 환불(반품 배송비 고객) |
| #20366 | Light rain parka ×2 | $236.00 | 파손 주장, 사진 없음, 2일 | 사진 요청 |
| #20290 | Cashmere sweater | $248.00 | 단순 변심, 39일 | 거절(30일 초과) |
| 숨김 5건 [정함] | #20333 Wool overcoat $289.00 · #20341 Leather chelsea boots $312.00 · #20352 Field jacket ×2 $256.00 · #20388 Cashmere sweater $248.00 · #20395 Chore coat ×2 $284.00 | 합 $1,389.00 | 30일 안 | 환불 |
| 숨김 1건 [정함] | #20409 Wool overcoat $289.00 | | 파손 주장, 사진 없음 | 사진 요청 |

- **체크 4개:** 원결제수단 환불 가능(12/12) 통과 · 쿠폰 할인 차감(3건 쿠폰 사용: #20352, #20388, #20395 [정함]) 통과 · 반품 기간(11/12) 통과 · 반품 이력 **확인 필요**(#20431 고객 90일 4회).
- **대화:** "Why decline the cashmere sweater?" → 39일 전 배송, 30일 초과, 스토어 크레딧은 정책 밖이라 운영자 OK 필요 → "Keep the decline, but be kind in the message."
- **변경셋:** 환불 9건 + 메시지 3건(사진 요청 2, 거절 1) → "Approve 9 refunds and send 3 messages". 메시지는 되돌릴 수 없음(`irreversible` 표시).

### B3. 매출 질문 (11:20, 읽기만)

"Why did sales drop yesterday?" → Thu Nov 5 매출 $18,240(목요일 평균 $22,900, −$4,660, −20%), 주문 201(평균 247). 원인: 베스트셀러 3개(chore coat, rain parka, denim jacket) M·L 품절 −$2,700 · 주간 이메일 미발송(Thu 08:00 목록 가져오기 실패) −$1,300 · 설명 안 됨 −$660. 다음 행동 3개는 각각 변경셋 후보입니다. 일별 매출 14일(Oct 23 – Nov 5)은 시드에 넣습니다.

### B4. 공급사 시트 등록 (10:58 요청)

"List the 30 products in this supplier sheet. Use our usual pricing and write the descriptions in our tone." → 초안 30개(§3.3), 체크: 중복 SKU 없음 · 모든 가격 하한 이상 · 필수 필드 4건 미해결. 초안은 쇼퍼에게 보이지 않고, 게시는 별도 변경셋입니다.

### B5. 그 밖의 할 일 (운영자 홈)

| 시각 | 할 일 | 상태 |
|---|---|---|
| 07:55 → 08:31 | 원가 이하 상품 4개를 하한으로 | 자동화, 검증 완료, Nov 7까지 되돌리기 |
| 08:40 → 09:12 | Linen shirt 재주문 초안(3 사이즈, 240개) | 검토 대기 |
| 09:30 → 10:00 | 메인 배너 "Fall layers"로 교체 | 운영자 요청, 검증 완료 |
| Nov 5 → 11:00 | 사이즈표 누락 확인 18개 중 15개 | 진행 중, 변경 없음 |
| 08:02 ~ | 주문 상태 문의 46건 응답, 사람 이관 0 | 쇼퍼 에이전트, 읽기만 |

### F1. 같은 질문, 두 쇼퍼

"I need a light jacket for spring trips." 같은 툴·상품·가격 규칙, 다른 화면:
- **Jamie(VIP, 스펙 비교형):** `compare_table` — windbreaker / field jacket / overshirt, 가격은 VIP 주말가($71.20 / $102.40 / $67.15).
- **Mina(첫 방문):** `look_gallery` — windbreaker $89, field jacket $128, overshirt $79, chore coat $142(정가).

### F2. Jamie, 찾고 사기 (Sat Nov 7)

1. 요청 → 자연 소재 위주 + 포켓형 나일론 1개.
2. "Which one packs smallest?" → `compare_table`(접은 크기 순), 기억 "weight and packed size matter to you when traveling"(되돌리기 가능).
3. "The windbreaker. Same size as last time." → `add_to_cart`(windbreaker, Slate, M).
4. 장바구니: 소계 $89.00 · VIP weekend −$17.80 · 배송 무료(할인 전 소계 ≥ $80) · **합계 $71.20** · 쿠폰 `VIPWEEKEND20` 기록 · Tue Nov 10 도착.
5. `prepare_order` → 확인 카드 → Jamie가 **Place order · $71.20**(사람 전용).

### F3. 핏 확인 (Jamie)

"Will M fit in the wool overcoat? I want to wear it over a hoodie." → L 추천(가슴 59 vs 56, 후드티 위로는 약 3 cm 더 필요), 기억 "you layer hoodies under coats". "What if I just wear a sweater?" → M. `fit_check`: 코트 M·L과 Jamie의 windbreaker M 실측 비교, 리뷰 요약, 쓴 데이터(주문 "Windbreaker M, kept", 프로필) / 쓰지 않은 데이터(키·몸무게).

### F4. 선물 (Jamie)

"A gift for my dad. He hikes a lot. Under $100, and it has to arrive by Nov 14." → 3개(windbreaker $71.20 VIP가, fleece half-zip $84.00, merino tee $58.00). **기억에 저장하지 않음.** "He's about my size, maybe a bit bigger." → L. 포장 +$5.00, 카드 "For the next trail. Love, J.", 가격 없는 선물 영수증, Dec 31까지 사이즈 교환, 배송지 "Dad · 48 Pine Rd, Lakeview"(이 주문에만), Tue Nov 10 도착, **합계 $76.20**. 주문은 Jamie가 보냄.

### F5. 불량 반품 (Alex, Fri Nov 6 09:50)

"The sole on my chelsea boots is coming apart after a week." + 사진 2장 → 불량: 새 상품 교환(오늘 출고, 재고 6, Tue Nov 10 도착, 비용 없음) 또는 전액 환불($312.00, 카드 0937, $200 초과라 사람이 확인, 확인 후 3–5일). "Refund, please." → `return_options` → Alex가 **Send refund request**(사람 전용). 수거 Mon Nov 9 9:00–12:00 또는 우체국 QR. 이 요청이 B2의 #20402가 됩니다.

### F6. 요청으로 화면 바꾸기 (Mina, 휴대폰)

"Light jacket for spring trips" → `product_list` 3개 → "Less text. Just show me bigger photos." → 같은 결과를 `look_gallery`로, 기억 "photos over details"(되돌리기 가능). 가격·재고·정책은 그대로.

---

## 7. 시드 생성 규칙 [정함]

이름이 나오지 않는 데이터는 시드 생성기가 고정 난수 시드로 만듭니다. 테스트가 확인하는 값은 아래 제약으로 고정됩니다.

| 대상 | 규칙 |
|---|---|
| summer outerwear 생성 상품 35개 | 정가 $59–$149, 원가 = 정가 × 0.45, 재고 12–120, 사이즈 XS–XL |
| 나머지 카테고리 | 전체 24개 카테고리, 상품 합계 약 400개 |
| 회원 | 등급별 수는 §2.1. VIP 1,284명에 Jamie 포함 |
| 주문 번호 | 이름이 나오는 주문은 위의 번호 그대로. 생성 주문은 주문일 순서로 번호를 받되 이름 있는 번호를 건너뜀(그래서 이름 있는 주문 주변에서는 번호와 날짜 순서가 다를 수 있음) |
| Jamie의 과거 주문 | 6건, 최근 2건 M, 그중 windbreaker M(Black) 9월 배송 |
| 일별 매출 | Oct 23 – Nov 5는 B3의 값, 그 전 목요일 3번(Oct 8·15·22)의 합은 $69,000 |
| 변경셋 번호 | `cs_1042` VIP 세일, `cs_1043` 반품, `cs_1040` 원가 이하 4개 수정(live), `cs_1041` Linen 재주문 초안. 공급사 발주 오퍼레이션이 범위 밖이라 `cs_1041`은 해석만 있고 항목이 없는 `drafting` 상태이며 제출할 수 없음 |

---

### 7.1 시드와 장면 재생

- 시드는 **금요일 10:40 이후의 상태**입니다. F5(Alex가 반품 요청을 보냄, 09:50)의 결과인 `clm_20402`가 이미 들어 있어 B2가 12건을 봅니다. F5 재생 테스트는 이 클레임을 지운 상태에서 시작합니다(`acom-testkit`의 장면별 준비 단계).
- B3의 원인은 데이터에서 나옵니다. 품절은 `catalog.offer`의 `sold_out_at`(Wed Nov 4 15:00)과 품절 사이즈가 비교 목요일에 판 줄, 주간 이메일은 `messaging.campaign`(`cmp_weekly_1105`, `failed`, "List import failed", Thu 08:00)과 지난 네 목요일 발송에서 온 주문(`sales_order.campaign_id`, 합 $5,200 = 4 × $1,300)입니다. 설명 안 됨 −$660은 나머지입니다.

## 8. 시안끼리 다른 곳과 결정

| # | 무엇 | 시안 A | 시안 B | 결정 |
|---|---|---|---|---|
| 1 | Chore coat 정가 | $149.00 (`mockups/store.html`) | $142.00 (`options/c-canvas.html`, `storefront/fo-two-shoppers.html`) | **$142.00** |
| 2 | Rain parka 정가 | $169.00 (`mockups/store.html`) | 반품 #20366 2개 $236.00 → $118.00 (`options/intent-returns.html`) | **$118.00**(산술이 맞음) |
| 3 | 반품 12건 분류 | 승인 10, 보류 2 (`mockups/inbox.html`) | 환불 9, 사진 2, 거절 1 (`options/intent-returns.html`) | **9 / 2 / 1** |
| 4 | 반품 체크 | 4/4 (`options/e-tasks.html`) | 3 통과 + 1 확인 필요 | **3 + 1** |
| 5 | 반품 할 일 시각 | 09:00 자동화 | 10:40 에이전트 | **10:40** |
| 6 | 공급사 시트 요청 시각 | 10:15 | 10:58 | **10:58** |
| 7 | "Fall layers" 배너 | 09:30 요청, 10:00 게시 | 08:10 | **09:30 / 10:00** |
| 8 | VIP 세일 툴 이름 | `create_promotion` → `issue_coupon` → `schedule_display` (design.md §4·§9) | `set_sale_prices` → `create_coupon` → `schedule_banner` → `verify_change_set` (`options/d-command.html`) | **뒤쪽(시안 D)**. 장바구니가 할인 한 줄(−$17.80)로 보여 주는 것과 맞음. design.md의 이름은 같은 계층의 다른 오퍼레이션으로 남음 |
| 9 | 상품 수 | 42 (design.md §6 예) | 40 (변경셋 v2) | 카테고리 42, 세일 **40** |
| 10 | Jamie의 windbreaker | 이미 가지고 있음 (`fo-fit.html`) | Nov 7에 삼 (`store.html`) | 과거 주문(Black, 9월)을 시드에 넣고 Nov 7은 같은 사이즈 재구매 |
| 11 | Alex 반품 시점 | 일주일 뒤 | 금 10:40에 이미 대기열 | Fri Nov 6 **09:50** |
| 12 | hero 슬롯 | Nov 7–8 비어 있음 | "Fall layers" 게시 중 | 슬롯은 대상별 배치. VIP 배치만 비어 있음 |
| 13 | 가격 규칙 반올림 | $96 → $239 | $52 → $139 | "가장 가까운 x9, 동률이면 위로" → $52 → **$129** |
| 14 | 무료 배송 기준 | over $80 | 할인 후 $71.20인데 무료 | **할인 전 상품 소계** 기준 |
| 15 | 품절인데 "7 left" | chore coat M·L 품절 | M 쇼퍼에게 "7 left" | 재고 7은 S·XL에만. 쇼퍼에게는 자기 사이즈 재고를 보여 줌 |
| 16 | 모바일 쇼퍼 | `mockups/mobile.html` | `storefront/fo-mobile-adapt.html` | 앞은 Jamie, 뒤는 Mina |
| 17 | 승인 대기 표기 | Waiting for your approval / Needs your approval / Ready to review | | 상태는 `waiting_for_approval` 하나, 화면 문구는 자유 |
