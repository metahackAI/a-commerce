# API 명세 — `analytics` (집계 읽기: 매출·고객·상담)

> 계층: `analytics.{resource}.{operation}`. 공통 규약(응답 형식·헤더)은 [conventions.md](conventions.md)와 [README.md](README.md) §3, 에러는 [errors.md](errors.md), 데이터 타입은 [data-model.md](data-model.md).
> **범위**: 이 도메인은 읽기 전용 집계만 둡니다. 리포트 화면, 엑셀 내보내기, 정산 같은 리포팅 기능은 두지 않습니다. 데모 시나리오에서 쓰는 것은 `sales_explainer` 블록([chat.md](chat.md) §3.3)의 데이터를 주는 `explain_sales_change`와 일별 차트용 `get_sales_trend`입니다(장면 B3, [../scenarios.md](../scenarios.md) §6). 자연어 통계 질문은 전용 엔드포인트 없이 에이전트가 아래 읽기 툴을 조합해 답합니다.
> **설계 축**: ① 모두 R(부작용 없음, 승인 불필요). 집계 입력이 커서 POST를 쓰지만 부작용 없는 읽기입니다. ② 결과는 schema.org `Dataset`과 `Observation`, 지표 이름은 `acom:metric`(의미 있는 영단어, 코드값 금지), ③ 기간·차원을 파라미터로 받고 지표마다 엔드포인트를 따로 두지 않음, ④ 개인정보 없이 집계값만. ⑤ 금액은 `{value, currency}`, 감소는 음수.
> **구현**: `crates/domains/analytics`. 읽는 곳은 이 도메인의 읽기 모델인 `analytics` 뷰뿐입니다(`migrations/0011`, `0016`: `daily_sales`, `order_summary`, `order_line_summary`, `variant_stock_out`, `campaign_send`, `member_tier_count`, `daily_new_members`). 뷰에는 이름·연락처·주소·고객 id가 없습니다. 사람을 세지 않고는 뷰로 낼 수 없는 수(기간에 산 회원 수)는 `acom_sales::service::for_analytics`가 셉니다. 기간은 스토어 날짜이고 오늘까지만 받습니다(366일까지). 비율(`*_rate`)은 0–1, 소수 넷째 자리까지입니다.

리소스: [sales](#sales) · [customer](#customer) · [service](#service)

---

## sales

### `explain_sales_change` — Kind: R
- 논리 주소 · REST: `analytics.sales.explain_sales_change` · `POST /v1/analytics/sales/explain`
- 요약: 한 기간의 매출을 비교 기간과 견주고, 차이를 원인별 금액으로 나눕니다. 원인 금액과 설명되지 않은 부분을 더하면 전체 차이와 정확히 같습니다. 원인은 서버가 데이터로 확인한 것만(품절 시각, 발송 실패 기록 등) 근거와 함께 반환합니다. `sales_explainer` 블록의 데이터입니다.
- consumes → produces: → explanation
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `valid_from` / `valid_through` | Date | ✓ | 볼 기간(스토어 날짜, 오늘까지). 예 `"2026-11-05"` / `"2026-11-05"` |
  | `compare_mode` | enum | ✓ | `same_weekday_last_4_weeks`(같은 요일 지난 4주 평균, 7일 이하 기간) \| `previous_period`(바로 앞 같은 길이) \| `custom`(아래 기간, 기간 길이에 맞춰 환산) |
  | `compare_from` / `compare_through` | Date | | `compare_mode: custom`일 때 ✓. 기간이 시작하기 전에 끝나야 함 |
  | `acom:metric` | enum | | `revenue`(기본) \| `order_count` |
  | `max_causes` | Integer | | 기본 5, 최대 10 |
- 출력(장면 B3):
  ```jsonc
  {
    "items": [
      {"cause": "out_of_stock", "detail": "3 bestsellers out of M and L since Wed Nov 4 15:00",
       "amount": {"value": -2700.00, "currency": "USD"}, "evidence": ["prd_chore_coat", "prd_rain_parka", "prd_denim_jacket"],
       "expected": {"value": 2700.00, "currency": "USD"}, "actual": {"value": 0.00, "currency": "USD"},
       "since": "2026-11-04T15:00:00-05:00",
       "products": [{"product_id": "prd_chore_coat", "name": "Cotton chore coat", "sizes": ["M", "L"], "is_bestseller": true,
                     "sold_out_at": "2026-11-04T15:00:00-05:00", "availability_starts": "2026-11-12T00:00:00-05:00",
                     "expected": {"value": 1313.50, "currency": "USD"}, "actual": {"value": 0.00, "currency": "USD"}}]},
      {"cause": "campaign_not_sent", "detail": "Weekly email not sent: list import failed Thu 08:00",
       "amount": {"value": -1300.00, "currency": "USD"}, "evidence": ["cmp_weekly_1105"],
       "expected": {"value": 1300.00, "currency": "USD"}, "actual": {"value": 0.00, "currency": "USD"},
       "campaign": {"campaign_id": "cmp_weekly_1105", "name": "Weekly email", "channel": "email", "campaign_status": "failed",
                    "scheduled_time": "2026-11-05T08:00:00-05:00", "error_message": "List import failed",
                    "compared_with": ["cmp_weekly_1008", "cmp_weekly_1015", "cmp_weekly_1022", "cmp_weekly_1029"]}}
    ],
    "total": 2,
    "acom:metric": "revenue",
    "period":     {"valid_from": "2026-11-05", "valid_through": "2026-11-05", "revenue": {"value": 18240.00, "currency": "USD"}, "order_count": 201},
    "compare_to": {"basis": "same_weekday_last_4_weeks", "observation_dates": ["2026-10-08", "2026-10-15", "2026-10-22", "2026-10-29"],
                   "revenue": {"value": 22900.00, "currency": "USD"}, "order_count": 247},
    "gap":         {"value": -4660.00, "currency": "USD"},
    "change_rate": -0.2035,
    "unexplained": {"value": -660.00, "currency": "USD"}
  }
  ```
  `items[].amount`의 합 + `unexplained` = `gap`. `acom:metric: order_count`이면 금액 자리(`amount`, `expected`, `actual`, `gap`, `unexplained`)가 주문 수(소수 둘째 자리까지)입니다. 일별 시계열은 `get_sales_trend`로 따로 읽습니다. `cause`는 열린 목록입니다(지금 데이터로 찾는 것: `out_of_stock`, `campaign_not_sent`. 데이터가 생기면 `price_change`, `promotion_ended`, …).
- 계산(코드: `decompose.rs`):
  - 비교 기준 = 비교 날들의 합 × (기간 일수 / 비교 일수). 같은 요일 4주면 네 목요일 합의 1/4. 비교 날에 매출이 하나도 없으면 `no_data_for_period`.
  - 원인 금액 = 기간에 그 원인의 몫이 번 것 − 비교 기준에서 그 몫이 번 것(같은 비율로 환산). 금액마다 한 번 반올림(통화의 보조 단위)하고, 나머지는 모두 `unexplained`입니다. 0으로 반올림되는 원인은 빠지고, 차이의 방향으로 큰 것부터(하락이면 가장 큰 감소부터) `max_causes`개까지 보입니다.
  - `out_of_stock`: 지금 품절이고 기간이 끝나기 전에 품절된 변형(`variant_stock_out`: `sold_out_at`, `availability = out_of_stock`)의 순매출(가격 × 수량 − 줄 할인, 배송비 제외). 상품별로 묶고 기간이나 비교 날에 판 상품만 `evidence`에 넣으며, 모두 `bestseller` 태그면 "bestsellers"라고 씁니다.
  - `campaign_not_sent`: 기간 안에 예정됐지만 아무에게도 나가지 않은(`failed`·`cancelled`, `date_sent` 없음) 캠페인. 같은 템플릿의 발송이 한 반복 캠페인이고, 그 발송에서 온 주문(`sales_order.campaign_id`)의 매출이 그 캠페인이 보통 가져오는 몫입니다. 이 주문은 품절 원인에서 빼서 같은 매출을 두 번 세지 않습니다.
- 프리뷰: 해당 없음(읽기)
- 정책: 없음(읽기)
- 에러: `invalid_period`(기간 조정: 끝이 시작보다 앞, 오늘 이후, `same_weekday_last_4_weeks`에 7일 넘는 기간, `custom`에 비교 기간 없음·겹침) → `explain_sales_change` · `no_data_for_period`(비교 날에 매출 없음) → `get_sales_trend`
- 멱등성: 읽기(부작용 없음)

### `get_sales_overview` — Kind: R
- 논리 주소 · REST: `analytics.sales.get_sales_overview` · `POST /v1/analytics/sales/overview`
- 요약: 기간의 판매 핵심 지표(매출, 주문 수, 객단가, 취소·반품률) 한 번에.
- consumes → produces: → metrics
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `valid_from` / `valid_through` | Date | ✓ | 기간 |
  | `acom:metrics` | enum[] | | `revenue` \| `order_count` \| `average_order_value` \| `cancellation_rate` \| `return_rate`. 기본 전부 |
  | `acom:dimensions` | enum[] | | `category`(상품의 대표 표준 카테고리, 줄 금액) \| `member_tier`(주문한 회원의 지금 등급). 차원마다 항목 목록 하나 |
  | `compare_from` / `compare_through` | Date | | 비교 기간(증감 계산). 둘 다 있어야 함 |
- 출력: `{"items": [{"acom:metric": "revenue", "value": {"value": 18240.00, "currency": "USD"}, "acom:dimension": null, "acom:change_rate": -0.1929}], "total": 5, "valid_from": "2026-11-05", "valid_through": "2026-11-05"}` — 금액 지표의 `value`는 `{value, currency}`, 비율 지표는 0–1. `cancellation_rate` = 취소된 주문 / 결제된 주문, `return_rate` = 반품 요청(거절·철회 제외)이 있는 주문 / 취소되지 않은 주문. `acom:change_rate`는 비교 기간 대비 상대 변화(비교 값이 0이면 null)
- 프리뷰: 해당 없음(읽기)
- 정책: 없음(읽기)
- 에러: `invalid_period` → `get_sales_overview` · `dimension_not_supported`(취소·반품률은 주문 단위라 `category`로 나눌 수 없음. 허용 차원으로) → `get_sales_overview`
- 멱등성: 읽기(부작용 없음)

### `get_sales_trend` — Kind: R
- 논리 주소 · REST: `analytics.sales.get_sales_trend` · `POST /v1/analytics/sales/trend`
- 요약: 일·주·월 단위 판매 추이. `sales_explainer`의 일별 매출 차트(Oct 23 – Nov 5)도 이것으로 읽습니다.
- consumes → produces: → series
- 입력: `valid_from` / `valid_through`(✓, Date), `acom:granularity`(✓, `day` \| `week`(월요일 시작) \| `month`), `acom:metrics?[]`(`revenue`(기본) \| `order_count` \| `average_order_value`), `acom:dimension?`(`category` \| `member_tier`)
- 출력: `{"items": [{"observation_date": "2026-11-05", "acom:metric": "revenue", "value": {"value": 18240.00, "currency": "USD"}}], "total": 14}` — 점마다 지표마다 항목 하나. 차원이 없으면 판매가 없는 점도 0으로 채우고, 차원이 있으면 항목에 `"acom:dimension": {"member_tier": "vip"}`가 붙습니다
- 프리뷰: 해당 없음(읽기)
- 정책: 없음(읽기)
- 에러: `invalid_period` → `get_sales_trend` · `invalid_granularity`(`day`는 92일까지. 더 길면 `week`·`month`) → `get_sales_trend`
- 멱등성: 읽기(부작용 없음)

---

## customer

### `get_customer_overview` — Kind: R
- 논리 주소 · REST: `analytics.customer.get_customer_overview` · `POST /v1/analytics/customers/overview`
- 요약: 회원 현황 집계(신규·활성 회원, 등급 분포, 재구매율). 개인정보는 반환하지 않습니다.
- consumes → produces: → metrics
- 입력: `valid_from` / `valid_through`(✓), `acom:metrics?[]`(`new_members` \| `active_members` \| `tier_distribution` \| `repurchase_rate`), `acom:dimensions?[]`(`member_tier`)
- 출력: `{"items": [{"acom:metric": "tier_distribution", "value": 1284, "acom:dimension": {"member_tier": "vip"}}], "total": 3}` — `tier_distribution`은 기간과 상관없이 지금 등급별 회원 수(차단·탈퇴 제외, `list_tiers`와 같음). `active_members`는 기간에 결제되고 취소되지 않은 주문을 한 회원 수, `repurchase_rate`는 그중 두 번 이상 주문한 비율이며 등급으로 나누지 않습니다. `member_tier`는 `new_members`를 지금 등급별로 나눕니다
- 프리뷰: 해당 없음(읽기)
- 정책: 없음(읽기)
- 에러: `invalid_period` → `get_customer_overview`
- 멱등성: 읽기(부작용 없음)

---

## service

### `get_cs_overview` — Kind: R
- 구현: 읽기 모델 `analytics.inquiry_summary`(migration 0028, 문의 하나에 한 행: 들어온 스토어 날짜, 종류, 상태, 직원·에이전트의 첫 답 시각, 해결 시각. 누가 무엇을 물었는지는 없음)만 읽습니다.
- 논리 주소 · REST: `analytics.service.get_cs_overview` · `POST /v1/analytics/service/overview`
- 요약: 고객 상담 운영 지표(문의 수, 유형 분포, 첫 응답·처리 시간, 미처리 건수, 해결률).
- consumes → produces: → metrics
- 입력: `valid_from` / `valid_through`(✓), `acom:metrics?[]`(`inquiry_count` \| `unresolved_count` \| `average_response_time` \| `resolution_rate`), `acom:dimensions?[]`(`inquiry_type`, 값은 enums.md §1.13 문의 종류)
- 출력: `{"items": [{"acom:metric": "inquiry_count", "value": 46, "acom:dimension": {"inquiry_type": "order_status"}}], "total": 4, "valid_from": "2026-10-07", "valid_through": "2026-11-06", "unit": {"average_response_time": "minutes"}}` — 기간에 들어온 문의를 셉니다. `unresolved_count`는 그중 아직 해결되지 않은 것, `average_response_time`은 들어와서 직원·에이전트가 처음 답하기까지의 평균 분(소수 첫째 자리, 답한 문의만, 없으면 null), `resolution_rate`는 해결된 것 / 들어온 것(0–1). 차원이 있으면 문의가 있는 종류마다 항목
- 프리뷰: 해당 없음(읽기)
- 정책: 없음(읽기)
- 에러: `invalid_period` → `get_cs_overview`
- 멱등성: 읽기(부작용 없음)
