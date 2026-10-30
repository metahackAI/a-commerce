# 에이전트 평가 과업 세트

[../docs/backend/testing.md](../docs/backend/testing.md) §3, [../docs/design.md](../docs/design.md) §7의 실험에 쓰는 과업입니다. MD·마케터·운영 담당·스토어 주인이 실제로 에이전트에게 시키는 말을 공개 자료에서 모으고, 데모 스토어의 데이터로 기대 결과를 정했습니다.

## 1. 무엇을 재는가

- 과업 하나 = 직무 + 페르소나 + 면(bo/fo) + 데모 시계 + 사람의 말(여러 턴 가능) + 기대 결과(사실, 변경셋) + 허용 툴 + 부르면 안 되는 툴.
- 같은 과업을 두 툴 레이어(A-Commerce 매니페스트, 화면형 파사드)와 두 모델에서 돌려 완료율·툴 선택 정확도·단계·호출·비용·pass^k를 잽니다(testing.md §3).
- 정답이 "없음"이거나 "할 수 없음"인 과업을 일부러 넣었습니다. 실제 운영자는 없는 것을 묻고, 정책이 막는 것을 시킵니다. 에이전트가 지어내지 않고 그 사실을 말하는지가 신뢰의 핵심입니다(R20, R22).

## 2. 출처

말투는 아래 자료의 실제 프롬프트를 따르고, 업무 규칙(판매율, 할인 폭, 승인, 반품·주소 변경 조건)은 실무 자료를 따릅니다. 과업의 `phrasing.from`과 `sources`가 이 표의 ID를 가리킵니다.

| ID | 자료 | 쓴 곳 |
|---|---|---|
| R1 | [Shopify Sidekick help and guidance](https://help.shopify.com/en/manual/ai-powered-tools/sidekick/help-and-guidance) | 원문 프롬프트: best-selling this month, how many remain |
| R2 | [Shopify Sidekick](https://www.shopify.com/sidekick) | 원문: weekly performance summary, never run out of inventory |
| R3 | [Shopify Sidekick skills](https://help.shopify.com/en/manual/ai-powered-tools/sidekick/skills) | 할 수 있는 일의 범위 |
| R4 | [Digital Commerce 360, Shopify Sidekick update (2025-05-08)](https://www.digitalcommerce360.com/2025/05/08/shopify-sidekick-ai-tool-ecommerce-platform-update/) | 원문: why are my sales down, launch a product promotion |
| R5 | [Amazon Project Amelia](https://www.aboutamazon.com/news/innovation-at-amazon/amazon-project-amelia) | 원문: how is my business doing, top things to prepare |
| R6 | [Salesforce Agentforce Merchant](https://trailhead.salesforce.com/content/learn/modules/agentforce-merchant-quick-look/get-to-know-your-merchant-agent) | 원문: create promotion for a category |
| R7 | [Klaviyo AI segments](https://help.klaviyo.com/hc/en-us/articles/18986425586715) | 원문: viewed and not purchased |
| R8 | [Craftshift, Sidekick prompts (제3자)](https://craftshift.com/shopify-sidekick-prompts-that-save-time/) | 원문: out of stock 30 days, win-back, sales drop |
| R9 | [BigCommerce Merchant AI (제3자 앱)](https://www.bigcommerce.com/apps/merchant-ai/) | 참고만 |
| R10 | [Shoplworks, apparel MD job skills](https://www.shoplworks.com/en/blog-insight/apparel-md-job-skills-guide-jun) | MD 업무 범위(리뷰·핏 확인) |
| R11 | [Drapers jobs, merchandiser roles](https://www.drapersjobs.com/job/senior-merchandiser-ecommerce-trading) | MD 업무 범위(검색 요약만) |
| R12 | [Uphance, sell-through rate](https://uphance.com/blog/what-is-a-sell-through-rate-and-why-apparel-brands-misread-it/) | 판매율, 결품 사이즈가 평균에 숨는 문제 |
| R13 | [EightX, apparel markdown strategy](https://eightx.co/blog/apparel-markdown-strategy) | 첫 마크다운 10–25%, 시즌 2–3단계 |
| R14 | [Manifest.ly, promotion checklist](https://app.manifest.ly/public/checklists/promotion-checklist) | 할인·원가 이하 승인(검색 요약만) |
| R15 | [Oracle Retail MFP, merch plan task](https://docs.oracle.com/en/industries/retail/merchandise-financial-planning-cloud/25.1.101.0/rmfcf/create-merch-plan-task.htm) | 계획 업무(검색 요약만) |
| R16 | [Promo OS, promotional calendar](https://promo-os.pages.dev/blog/shopify-promotional-calendar) | 프로모션 겹침·캘린더 |
| R17 | [Ringly, Gorgias ticket examples](https://www.ringly.io/blog/gorgias-ticket-example) | 배송 문의(WISMO)가 티켓의 18–40% |
| R18 | [Shopify + Gorgias setup](https://shopify-fee-calc.pages.dev/blog/shopify-gorgias-customer-service-setup/) | 상담 유형(검색 요약만) |
| R19 | [Gorgias AI Agent actions](https://helpcenter.gorgias.com/en-US/ai-agent-actions-make-changes-to-shopify-orders-757792) | 취소·주소 변경은 출고 전만, 쇼퍼 확인 |
| R20 | [Shopify Community, AI for store performance](https://community.shopify.com/t/how-are-you-using-ai-to-understand-your-store-s-performance-beyond-basic-shopify-stats/586606) | 비교 기간이 있는 수치를 원함 |
| R21 | [Shopify Sidekick manual](https://help.shopify.com/en/manual/shopify-admin/productivity-tools/sidekick) | 범위 |
| R22 | [Shopify Community, Sidekick inaccurate](https://community.shopify.com/t/sidekick-provided-inaccurate-information/666606) | 자신 있게 틀린 답이 신뢰를 깸 |
| R23 | [Tante-e, Sidekick test](https://tante-e.com/en/blogs/tante-e-blog/shopify-sidekick-test) | 실사용 평가(검색 요약만) |
| R24 | [AgencyAnalytics, Shopify sales and refunds](https://help.agencyanalytics.com/en/articles/5608569-shopify-sales-and-refunds-don-t-match-what-i-see-in-my-shopify-account) | 날짜 기준·환불 귀속으로 숫자가 어긋남 |
| R25 | [Shopify Community, home stats date range](https://community.shopify.com/t/home-page-stats-no-longer-show-time-period-or-allow-date-range-changes/637497) | 기간 표시 요구(검색 요약만) |

"검색 요약만"은 본문을 열지 못하고 검색 결과 요약으로 확인한 자료입니다. 이 자료만으로 정한 기대값은 없습니다. 배너 교체 빈도, 오픈 투 바이(OTB) 같은 숫자는 공개 자료를 찾지 못해 과업에 넣지 않았습니다.

## 3. 과업 형식

파일은 `tasks/{design,held_out}/{merchandiser,marketer,operations,owner}.yml`이고, 파일 하나가 과업 목록입니다.

```yaml
- id: md-end-of-season-markdown       # 두 세트를 통틀어 유일
  job: merchandiser                    # merchandiser | marketer | operations | owner
  persona: adm_sj                      # 데모 계정. 권한은 이 계정의 역할을 따름
  surface: bo                          # bo(운영자) | fo(쇼퍼). 툴이 이 면에서 보여야 함
  frequency: seasonal                  # daily | weekly | monthly | seasonal | occasional
  kind: write_approval                 # 과업에서 가장 무거운 동작: read | write | write_approval
  scene: B1                            # 선택. docs/scenarios.md §6의 장면
  phrasing: {how: ours}                # verbatim | adapted | ours. ours가 아니면 from(R ID)·original(원문) 필수
  sources: [R13, R14]                  # 업무 규칙의 근거
  turns: ["사람의 말 1", "사람의 말 2"]
  tools:
    required: [search_products, set_sale_prices]   # 반드시 불러야 하는 툴
    allowed: [get_policy]                           # 불러도 되는 툴. 나머지는 감점
  expect:
    change_set: {state: drafting, items: [{tool: set_sale_prices, args: {...}}], checks_failed: [max_discount]}
    facts: ["답에 있어야 하는 사실"]
    must_not_call: []                  # 부르면 실패인 툴
  gaps: ["data: ...", "tool: ..."]     # 선택. 지금 데이터나 툴이 없어 못 하는 부분
  notes: 과업을 넣은 이유
```

- `verbatim`은 원문 그대로(자리표시자 `@PRODUCT`·`[Category Name]`만 채움), `adapted`는 같은 요청을 우리 스토어의 대상·숫자로 바꾼 것, `ours`는 실무 규칙에서 우리가 쓴 말입니다.
- 사람의 말은 영어입니다. 데모 스토어가 미국 스토어(USD, America/New_York)이기 때문입니다.
- 변경셋을 만드는 과업도 기대 상태는 `drafting`입니다. 승인·게시는 사람의 버튼이라 과업 밖입니다.

## 4. 커버리지와 검사

```bash
cargo xtask check-eval
```

- 과업이 이름 붙인 툴은 모두 레지스트리에 있거나(구현됨) `docs/api/`에 명세돼 있어야 합니다. 어디에도 없는 이름은 오타나 지어낸 툴이라 검사가 실패합니다. 구현된 툴은 과업의 면에서 보여야 합니다.
- 커버리지는 손으로 적지 않고 검사가 계산합니다.
  - `ready`: 필요한 툴이 다 구현돼 있고 데이터 빈 곳이 없음. 지금 바로 잴 수 있음.
  - `data`: 툴은 있지만 `gaps`에 `data:` 항목이 있음. 기대 결과는 "데이터가 없다고 말하기"입니다.
  - `spec`: 필요한 툴 중 명세만 있고 구현되지 않은 것이 있음. 구현되면 `ready`가 되고, 그때 기대 사실을 채웁니다.
- 숫자를 문서에 옮겨 적지 않습니다. 현황은 검사 출력이 원본입니다.

## 4.1 실행

```bash
ACOM_EVAL_LOGIN=... ACOM_EVAL_PASSWORD=... \
  cargo xtask eval-run --base https://<store> --tasks eval/tasks/design --out eval/results/<date>-<model>-acom-design.json
```

- 웹과 같은 채팅 API(`POST /api/chat/bo/turns`)로 과업마다 새 대화를 열고 턴을 차례로 보냅니다. 에이전트, 계획 단계 수, 부른 툴과 상태, 답, UI 블록 종류, 사용량을 기록합니다.
- 모델 없이 매기는 점수: 필요한 툴을 다 불렀는지(`required_called`), 부르면 안 되는 툴(`forbidden_called`), 단계 0개 계획 수(`empty_plans`), 기대 사실의 숫자가 답·블록에 나온 비율(`fact_numbers`). 마지막은 사람이 대화를 읽을 때 쓰는 대략의 신호이고 채점이 아닙니다.
- 쓰기 과업은 스토어에 초안을 남기므로 `--writes`를 줄 때만 돌립니다. 공유 데모에서는 돌리지 않습니다.
- 같은 과업을 여러 번 돌려 일관성(pass^k)을 봅니다. 한 번 통과는 통과가 아닙니다.

## 5. 설계 세트와 보류 세트

- `design/`: 에이전트(프롬프트, 툴 검색, 계획기)를 고치면서 보고 돌리는 과업.
- `held_out/`: 고치는 동안 열어 보지도 돌리지도 않습니다. 마지막에 한 번 돌린 숫자만 README에 씁니다. 설계 세트에서만 오른 개선은 개선으로 보지 않습니다.
- 보류 세트도 직무 네 개를 모두 덮고, 설계 세트와 같은 유형(없음 답, 정책 거절, 승인 변경셋)을 섞었습니다.

## 6. 기대값을 정한 방법

- 시계는 모든 과업에서 Fri Nov 6 2026 11:00 America/New_York입니다([../docs/scenarios.md](../docs/scenarios.md) §1.1).
- 기대 사실은 2026-10-30에 데모 스토어에서 같은 툴(운영자 로그인)로 읽어 정했습니다. 장면이 있는 과업(B1·B2·B3)은 scenarios.md §6과 맞춰 봤습니다.
- 날짜는 스토어 시간으로 셉니다. UTC 날짜로 세면 "오늘 주문"이 0건이 아니라 42건으로 나와 틀립니다(직접 겪은 실수, R24와 같은 함정).
- 데모 DB가 바뀌면(시드 재생성, 테스트로 만든 변경셋) 기대 사실도 다시 읽어야 합니다.

## 7. 데이터에서 찾은 빈 곳

기대값을 정하면서 확인한 것입니다. 과업의 `gaps`에도 적혀 있습니다.

- 데모 당일(Nov 6) 주문이 0건입니다. 마지막 주문은 Nov 5 23:40입니다.
- 출고 전 문제 주문, 사흘 넘게 출고 안 된 주문, 초안 상품이 없습니다. 처리 중 412건 중 가장 오래된 것이 Nov 4 00:09입니다.
- 배송 중 주문의 추적 이벤트는 picked_up 하나와, 시드를 넣은 시각(Oct 30 06:52)으로 찍힌 in_transit 하나뿐입니다. in_transit이 픽업보다 앞서고, 픽업 뒤 스캔이 없습니다.
- `cs_1041`(Linen shirt 재주문)은 요약에 "3 sizes, 240 units"라고 쓰여 있지만 항목이 없고, 발주(PO) 툴도 없습니다.
- 작업 로그의 `change_set_id`가 비어 있어 로그와 변경셋을 시각으로만 이을 수 있습니다.
- 상품 조회 이력, 재고 이력(언제부터 품절인지)이 없습니다.
- 상품 400개 중 396개가 한 줄 설명이고 키워드가 있는 상품은 9개, 이미지 주소는 실제로 열리지 않는 예시 주소입니다.
- 공유 데모에서 테스트하며 만든 `cs_1042` 초안이 있어, 장면 B1이 기대하는 변경셋 번호와 겹칩니다. 데모 전에 DB를 다시 시드해야 합니다.
