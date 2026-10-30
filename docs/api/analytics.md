# API 명세 — `analytics` (집계 읽기: 매출·고객·상담)

> 계층: `analytics.{resource}.{operation}`. 공통 규약(응답 형식·에러·헤더)은 [README.md](README.md) §3, 데이터 타입은 [data-model.md](data-model.md).
> **범위**: 이 도메인은 읽기 전용 집계만 둡니다. 리포트 화면, 엑셀 내보내기, 정산 같은 리포팅·정산 기능 대부분은 해커톤 MVP 범위 밖입니다. 해커톤에서 쓰는 것은 P2 `sales_explainer` 블록([../plan.md](../plan.md) §4.10)의 데이터를 주는 `explain_sales_change` 하나입니다. 자연어 통계 질문은 전용 엔드포인트 없이 에이전트가 아래 읽기 툴을 조합해 답합니다.
> **설계 축**: ① 모두 R(부작용 없음, 승인 불필요), ② 결과는 schema.org `Dataset`과 `Observation`, 지표 이름은 `acom:metric`(의미 있는 영단어, 코드값 금지), ③ 기간·차원을 파라미터로 받고 지표마다 엔드포인트를 따로 두지 않음, ④ 개인정보 없이 집계값만.

리소스: [sales](#sales) · [customer](#customer) · [service](#service)

---

## sales

### `explain_sales_change` — Kind: R
- 논리 주소 · REST: `analytics.sales.explain_sales_change` · `POST /v1/analytics/sales/explain`
- 요약: 한 기간의 매출을 비교 기간과 견주고, 차이를 원인별 금액으로 나눕니다. 원인 금액과 설명되지 않은 부분을 더하면 전체 차이와 정확히 같습니다. 원인은 서버가 데이터로 확인한 것만(품절 시각, 발송 실패 로그 등) 근거와 함께 반환합니다. P2 `sales_explainer` 블록의 데이터입니다.
- consumes → produces: → explanation
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `period` | object | ✓ | `{"valid_from": "2026-11-05", "valid_through": "2026-11-05"}` |
  | `compare_to` | enum \| object | ✓ | `same_weekday_last_4_weeks` \| `previous_period` \| 직접 지정 `{valid_from, valid_through}` |
  | `acom:metric` | enum | | `revenue`(기본) \| `order_count` |
  | `max_causes` | Integer | | 기본 5 |
- 출력:
  ```jsonc
  {
    "items": [
      {"cause": "out_of_stock", "detail": "3 bestsellers out of M and L since Wed Nov 4 15:00",
       "amount": {"value": -2700.00, "currency": "USD"}, "evidence": ["prd_denim_jacket"]},
      {"cause": "campaign_not_sent", "detail": "Weekly email not sent: list import failed Thu 08:00",
       "amount": {"value": -1300.00, "currency": "USD"}, "evidence": ["cmp_weekly_1105"]}
    ],
    "total": 2,
    "period":     {"revenue": {"value": 18240.00, "currency": "USD"}, "order_count": 201},
    "compare_to": {"revenue": {"value": 22900.00, "currency": "USD"}, "order_count": 247, "basis": "same_weekday_last_4_weeks"},
    "gap":         {"value": -4660.00, "currency": "USD"},
    "unexplained": {"value": -660.00, "currency": "USD"}
  }
  ```
  `items[].amount`의 합 + `unexplained` = `gap`. 일별 시계열은 `get_sales_trend`로 따로 읽습니다.
- 에러: `invalid_period` → `explain_sales_change`(기간 조정) · `no_data_for_period` → `get_sales_trend`

### `get_sales_overview` — Kind: R
- 논리 주소 · REST: `analytics.sales.get_sales_overview` · `POST /v1/analytics/sales/overview`
- 요약: 기간의 판매 핵심 지표(매출, 주문 수, 객단가, 취소·반품률) 한 번에.
- consumes → produces: → metrics
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `period` | object | ✓ | `{valid_from, valid_through}` |
  | `acom:metrics` | Text[] | | `["revenue", "order_count", "average_order_value", "cancellation_rate", "return_rate"]` |
  | `acom:dimensions` | Text[] | | `["category", "member_tier"]` |
  | `compare_to` | object | | 비교 기간(증감 계산) |
- 출력: `{"items": [Observation {acom:metric, value, acom:dimension, acom:change_ratio}], "total"}` — 금액 지표의 `value`는 `{value, currency}`
- 에러: `invalid_period` → `get_sales_overview` · `dimension_not_supported` → `get_sales_overview`(허용 차원으로)

### `get_sales_trend` — Kind: R
- 논리 주소 · REST: `analytics.sales.get_sales_trend` · `POST /v1/analytics/sales/trend`
- 요약: 일·주·월 단위 판매 추이. `sales_explainer`의 일별 매출 차트(Oct 23 – Nov 5)도 이것으로 읽습니다.
- consumes → produces: → series
- 입력: `period`(✓), `acom:granularity`(✓, `day` \| `week` \| `month`), `acom:metrics?[]`, `acom:dimension?`
- 출력: `{"items": [Observation {observation_date, acom:metric, value}], "total"}`
- 에러: `invalid_granularity` → `get_sales_trend`

---

## customer

### `get_customer_overview` — Kind: R
- 논리 주소 · REST: `analytics.customer.get_customer_overview` · `POST /v1/analytics/customers/overview`
- 요약: 회원 현황 집계(신규·활성 회원, 등급 분포, 재구매율). 개인정보는 반환하지 않습니다.
- consumes → produces: → metrics
- 입력: `period`(✓), `acom:metrics?[]`(`new_members`, `active_members`, `tier_distribution`, `repurchase_rate`), `acom:dimensions?[]`(`member_tier`)
- 출력: `{"items": [Observation {acom:metric, value, acom:dimension}], "total"}`
- 에러: `invalid_period` → `get_customer_overview`

---

## service

### `get_cs_overview` — Kind: R
- 논리 주소 · REST: `analytics.service.get_cs_overview` · `POST /v1/analytics/service/overview`
- 요약: 고객 상담 운영 지표(문의 수, 유형 분포, 첫 응답·처리 시간, 미처리 건수, 해결률).
- consumes → produces: → metrics
- 입력: `period`(✓), `acom:metrics?[]`(`inquiry_count`, `unresolved_count`, `average_response_time`, `resolution_rate`), `acom:dimensions?[]`(`inquiry_type`)
- 출력: `{"items": [Observation {acom:metric, value, acom:dimension}], "total"}`
- 에러: `invalid_period` → `get_cs_overview`
