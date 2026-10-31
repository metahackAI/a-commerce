# 데이터베이스 (PostgreSQL)

A-Commerce의 DB는 **AI가 읽는 DB**입니다. 에이전트(운영 에이전트, 툴 구현을 돕는 코딩 에이전트, 외부 분석 도구)와 사람이 테이블 이름, 컬럼 이름, 주석만 보고 무엇이 무엇인지 알 수 있어야 합니다. 그래서 이름은 schema.org에서 가져오고, 모든 테이블과 컬럼에 schema.org 용어가 적힌 주석을 달고, 규칙은 `cargo xtask check-schema`가 강제합니다.

- 개념 모델(엔티티 ↔ schema.org 타입, API 필드): [../api/data-model.md](../api/data-model.md)
- 값 목록: [../api/enums.md](../api/enums.md), 식별자 접두: [../api/conventions.md](../api/conventions.md) §9
- 실제 DDL: `migrations/*.sql` (이 문서의 규칙을 따르는 단일 원천)
- 생성되는 컬럼 사전: [schema.generated.md](schema.generated.md), `schema-map.json` (`cargo xtask check-schema --write`)

---

## 1. 원칙

1. **schema.org 이름 그대로.** 테이블은 schema.org 타입, 컬럼은 schema.org 프로퍼티의 snake_case. 모델이 이미 아는 단어이므로 추측이 줄어듭니다. schema.org 타입의 한 종류는 그 타입 + `additionalType`(`acom:`)으로 적고(쿠폰 = `Offer` + `acom:Coupon`), 새 용어는 마지막 수단입니다([../api/vocabulary.md](../api/vocabulary.md)).
2. **스스로 설명하는 DB.** 모든 테이블과 컬럼에 `COMMENT`가 있고, 첫 토큰이 schema.org 용어(`schema:orderStatus`) 또는 우리 용어(`acom:percentOff`)입니다. 에이전트는 `describe_data` 툴(§7)로 이 주석을 읽습니다.
3. **id는 하나.** 기본 키는 접두가 붙은 텍스트 id(`prd_windbreaker`, `ord_20402`)이고, API·로그·이벤트·DB가 같은 값을 씁니다. 숨은 숫자 키나 UUID가 따로 없습니다.
4. **값은 단어.** 상태·종류는 [enums.md](../api/enums.md)의 단어를 그대로 저장하고 `CHECK`로 목록을 고정합니다. 코드값·숫자 코드·Y/N 금지.
5. **단위가 이름에 보임.** 금액은 `price` + `price_currency`, 수량 값은 `*_value` + `*_unit_text`. 맨숫자만 있는 컬럼에 단위를 숨기지 않습니다.
6. **줄임말 금지.** `qty`, `amt`, `cnt`, `dt`, `cd`, `nm`, `no`, `yn`, `flg`, `seq`, `desc`, `addr`, `tel` 같은 줄임은 쓰지 않습니다(검사기가 막음).
7. **이력은 지우지 않음.** 상태 변화는 schema.org `Action` 모양의 이벤트 행으로 추가만 합니다.

## 2. 스키마

| 스키마 | 내용 |
|---|---|
| `platform` | 스토어(테넌트), 운영자, 역할, 세션, 조직, 백오피스 메뉴, API 키, 정책 덮어쓰기, 프리뷰, 변경셋, 액션 로그, 할 일, 작업 큐, 아웃박스, 멱등 키, 데모 시계 |
| `catalog` | 브랜드, 상품(ProductGroup), 변형(Product), 가격 제안(Offer), 예약 정가, 묶음 구성, 실측, 반품 정책, 리뷰·도움돼요·신고, 상품 문의·답, 재입고 알림, 공급사 재주문, 공급사 시트와 가져오기, 생성한 문구·이미지 초안 |
| `merchandising` | 카테고리(DefinedTerm)와 매핑, 진열 슬롯, 배너, 진열 예약, 기획전, 팝업, 캐시 비우기 기록 |
| `search` | 검색 문서(전문 검색, 임베딩 열은 비어 있음), 검색 사전 버전, 재색인 기록 |
| `promotion` | 프로모션, 기간·회원가(UnitPriceSpecification), 쿠폰, 회원 쿠폰, 적립금 원장, 사은품, 세그먼트, 이벤트·응모·추첨·보상 |
| `customer` | 사람(Person), 주소, 회원 프로그램(버전)·등급·멤버십과 등급 변경 이력·정기 평가, 기억(선호), 위시리스트, 동의 기록, 문의와 그 대화, FAQ·공지, 직원, 보상, 본인 확인, 계정 찾기, 탈퇴 |
| `sales` | 장바구니, 주문(Order), 주문 품목(OrderItem), 주문 이벤트, 결제, 선물 주문, 정기 주문, 영수증, 비회원 주문 조회 |
| `fulfillment` | 배송(ParcelDelivery)과 그 이벤트, 클레임, 검수, 환불과 그 이벤트, 배송 구역 |
| `messaging` | 메시지, 메시지 템플릿, 캠페인 |
| `community` | 게시판(`acom:Board`), 글(DiscussionForumPosting), 댓글(Comment), 글 신고 |
| `analytics` | 읽기 전용 뷰 8개(일별 매출, 주문·주문 줄 요약, 품절, 캠페인 발송, 등급별 회원 수, 일별 가입, 문의 요약) |
| `partner` | 입점 판매자(Organization), 판매자가 파는 상품, 상태 이력, 계약과 계약 버전, 등급 |
| `integration` | 외부 채널(자격 증명·서명 키는 해시만), 채널이 가진 주문·클레임 사본, 외부 피드 가져오기와 행별 결과, 받은 택배사·결제사 이벤트 |
| `agent` | 대화, 턴, 단계, 모델 호출 (agent 서비스 소유) |

## 3. 이름 규칙

| 대상 | 규칙 | 예 |
|---|---|---|
| 테이블 | schema.org 타입의 snake_case, 단수. 우리 확장은 `acom:` 용어의 snake_case | `product_group`, `offer`, `unit_price_specification`, `order_item`, `parcel_delivery`, `change_set` |
| 예약어 | 예약어와 겹치면 스키마 이름을 앞에 붙임 | schema.org `Order` → `sales.sales_order` |
| 컬럼 | schema.org 프로퍼티의 snake_case | `price_currency`, `valid_from`, `valid_for_member_tier`, `order_status`, `inventory_level` |
| 참조 컬럼 | 가리키는 테이블 이름 + `_id`. 같은 테이블을 두 번 가리키거나 역할이 중요하면 schema.org 프로퍼티 + `_id`. 예외: 회원 등급은 id가 스토어가 정한 단어라 schema.org 프로퍼티 이름 그대로 둠 | `product_group_id`(schema:isVariantOf), `customer_id`(schema:customer), `broader_id`, `valid_for_member_tier`(→ `member_program_tier`) |
| 불리언 | schema.org에 있으면 그 이름(`is_gift`, `is_accessory_or_spare_part_for`는 참조), 없으면 `is_`/`has_` | `is_gift`, `has_merchant_return_policy` |
| 금액 | 가격 컬럼은 schema.org처럼 `price` + `price_currency`. 그 밖의 금액은 `<프로퍼티>_value numeric(12,2)` + `<프로퍼티>_currency char(3)` | `total_payment_due_value`, `total_payment_due_currency` |
| 수량 값 | schema.org `QuantitativeValue`를 `<프로퍼티>_value` + `<프로퍼티>_unit_text`로 펼침. 이름이 원래 `…Value`인 schema.org 프로퍼티는 그대로(`rating_value` = `schema:ratingValue`) | `weight_value 190`, `weight_unit_text 'g'` |
| 비율 | `<이름>_percent`(0–100) 또는 `<이름>_rate`(0–1). [../api/conventions.md](../api/conventions.md) §4 | `percent_off 20.00` |
| 시각 | `timestamptz`, schema.org 이름(`order_date`, `valid_from`, `start_time`, `end_time`). 우리 시각은 `_at` | `created_at`, `confirmed_at` |
| 날짜 | `date` | `birth_date` |
| 가변 구조 | `jsonb`, 주석에 모양을 적음. 자주 걸러 보는 값은 일반 컬럼 | `additional_property jsonb` (`[{name, value, unit_text}]`) |
| 인덱스·제약 | `<테이블>_<컬럼>_idx`, `_key`(unique), `_fkey`, `_check` | `offer_product_id_idx` |

## 4. 공통 컬럼

스토어 범위의 모든 테이블이 가집니다.

| 컬럼 | 타입 | 뜻 |
|---|---|---|
| `tenant_id` | `text not null references platform.online_store(id)` | 이 행을 가진 스토어. RLS 키 |
| `id` | `text not null`, `PRIMARY KEY (tenant_id, id)` | 접두 id. `CHECK (id ~ '^<접두>_[a-z0-9_]+$')`. 스토어가 정하는 단어 id(등급 `vip`)는 접두 없이 `^[a-z][a-z0-9_]*$` |
| `created_at` | `timestamptz not null default now()` | 만든 시각 |
| `updated_at` | `timestamptz not null default now()` | 고친 시각 |
| `version` | `integer not null default 1` | 고칠 때마다 +1. 프리뷰의 `basis`가 이 값을 기록하고 실행 때 비교 |

사람이 보는 번호는 따로 둡니다: `sales_order.order_number`, `change_set.number`. 둘 다 스토어 안에서 unique입니다.

**참조도 스토어를 포함합니다.** 다른 테이블을 가리키는 외래 키는 `FOREIGN KEY (tenant_id, product_group_id) REFERENCES catalog.product_group (tenant_id, id)`처럼 `tenant_id`를 함께 씁니다. 그래서 DB 수준에서 다른 스토어의 행을 가리킬 수 없고, 시드처럼 읽기 쉬운 id(`prd_windbreaker`)가 여러 스토어에 있어도 충돌하지 않습니다. `platform.online_store`만 `id` 하나가 키입니다.

## 5. 값 목록과 제약

- 상태·종류 컬럼은 `text`와 `CHECK (<컬럼> IN (...))`이고 값은 [enums.md](../api/enums.md)와 같습니다. Rust의 `acom-core` enum과 마이그레이션의 목록이 같은지 `cargo xtask check-schema`가 확인합니다.
- Postgres `ENUM` 타입은 쓰지 않습니다. 값을 더할 때 마이그레이션이 쉽고, `\d+`와 주석으로 목록이 바로 보입니다.
- 금액은 `numeric(12,2)`, 통화는 `char(3)`이며 `CHECK (price_currency ~ '^[A-Z]{3}$')`를 둡니다. 음수가 안 되는 값은 `CHECK (>= 0)`을 둡니다.
- 기간은 `CHECK (valid_through > valid_from)`.

## 6. 주석 형식

```sql
COMMENT ON TABLE catalog.offer IS
  'schema:Offer — the price and availability of one variant. id prefix off_.';
COMMENT ON TABLE promotion.coupon IS
  'schema:Offer/acom:Coupon — a discount a shopper applies to an order. id prefix cpn_.';
COMMENT ON COLUMN catalog.offer.inventory_level IS
  'schema:inventoryLevel — units that can be sold now, across all locations.';
COMMENT ON COLUMN promotion.unit_price_specification.percent_off IS
  'acom:percentOff — the requested discount behind this price, 0-100.';
COMMENT ON COLUMN catalog.offer.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
```

- 테이블 주석의 첫 토큰: `schema:<Type>`, 특수화면 `schema:<Type>/acom:<Term>`(앞은 `@type`, 뒤는 `additionalType`, [../api/vocabulary.md](../api/vocabulary.md)), 순수 기술 테이블(작업 큐, 아웃박스 등)은 `-`.
- 컬럼 주석의 첫 토큰: `schema:<property>`, `acom:<property>`, 또는 기술 컬럼이면 `-`(이때는 `- 문장` 형식).
- ` — ` 뒤에 영어 한 문장. 단위, 값의 범위, 비어 있을 때의 뜻을 씁니다.
- 테이블 주석에는 id 접두와, API 이름이 다르면 `API: <이름>`을 씁니다.
- 열거 컬럼은 `values: enums.md#<목록>`을 덧붙입니다.

## 7. 에이전트에게 보이는 데이터 사전

- `cargo xtask check-schema --write`가 마이그레이션을 읽어 세 파일을 만듭니다. CI는 생성물이 최신인지도 확인합니다.
  - [schema.generated.md](schema.generated.md): 사람용 컬럼 사전
  - `docs/api/generated/schema-map.json`: 기계용
  - [`vocab/acom.jsonld`](../../vocab/acom.jsonld): 주석에 쓴 모든 `acom:` 용어의 정의. 손으로 쓴 `vocab/terms.json` + 주석 + `acom-core` enum으로 만들기 때문에, DB에 나오는 우리 용어는 모두 정의를 가집니다.
- `schema-map.json` 항목: `{schema, table, term, id_prefix, api_name, columns: [{name, type, nullable, term, description, values?, references?}]}`.
- 커머스 서비스는 이 지도를 `platform.schema.describe`(툴 `describe_data`, 읽기)로 내놓습니다. "order_status가 무슨 뜻인가", "일별 매출은 어디에 있나"를 물을 수 있습니다. 툴 범위에 `platform`이 있는 운영 에이전트(promotion, orders, catalog)가 부를 수 있고, 인사이트 에이전트의 범위는 `analytics`뿐입니다. 외부 분석 도구는 `acom_readonly` 역할로 `analytics` 뷰만 읽습니다(§8).

## 8. 스토어 격리 (RLS)

```sql
ALTER TABLE catalog.offer ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.offer FORCE ROW LEVEL SECURITY;
CREATE POLICY offer_tenant_isolation ON catalog.offer
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
```

- 스토어 범위의 모든 테이블에 위 정책이 있습니다(검사기가 확인).
- 역할: `acom_owner`(마이그레이션, 테이블 소유), `acom_app`(서비스 접속, RLS 적용, `DELETE`는 허용된 테이블만), `acom_readonly`(분석 뷰 읽기). 역할과 권한은 `infra/postgres/init.sql`이 만들고, `acom_owner`는 도메인 스키마를 만들 권한과 `public` 스키마에 마이그레이션 이력 테이블(`_sqlx_migrations`)을 만들 권한을 가집니다.
- 트랜잭션마다 `SELECT set_config('app.tenant_id', $1, true)`(`acom-store`가 자동으로).

## 9. 상태와 이벤트

상태가 있는 집계는 현재 상태 컬럼과 이벤트 테이블을 같은 트랜잭션에서 씁니다. 이벤트 테이블의 컬럼은 schema.org `Action`의 프로퍼티입니다. 이벤트 행은 따로 id를 갖지 않고 `(tenant_id, <집계>_id, position)`이 키입니다(집계마다 1부터 증가). 액션 로그는 `act_` id를 가집니다.

| 컬럼 | schema.org | 뜻 |
|---|---|---|
| `id` | identifier | `act_…` (액션 로그) 또는 `<집계>_event` 접두 |
| `<집계>_id` | object | 바뀐 집계 |
| `action_type` | `@type` | `UpdateAction`, `AuthorizeAction` 등 |
| `action_status` | actionStatus | `potential`, `active`, `completed`, `failed` |
| `agent_id` | agent | 사람(`adm_`, `cus_`), 시스템(`sys_`), 모델 이름은 `agent_name` |
| `instrument` | instrument | 툴 이름 또는 사람 전용 엔드포인트 |
| `from_state`, `to_state` | — (acom) | 상태 전이 |
| `result` | result | `jsonb` |
| `error` | error | `jsonb` (`{code, class, message}`) |
| `start_time`, `end_time` | startTime, endTime | |
| `request_id` | — | 요청 추적 id (`req_…`) |

`platform.action_log`는 모든 툴 호출·프리뷰·승인·실행·검증·되돌리기를 같은 컬럼으로 기록합니다.

## 10. 검색

`search.search_document`는 상품마다 한 행: `product_group_id`, `search_vector tsvector`(이름 A, 브랜드·카테고리·키워드 B, 소재 C, 설명 D), `embedding vector(1024)`와 `embedding_model`, 거르기용 컬럼(`brand_id`, `category_ids text[]`, `keywords`, `available_sizes`, `available_colors`, `price_min`, `price_max`, `price_currency`, `approval_status`). `reindex`가 catalog에서 다시 만듭니다. 재고·판매 상태·가격 조건은 catalog의 상품 질의(`service::for_search::find_products`) 안에서 거르고(결과를 받은 뒤 거르지 않음), 회원가는 상품 카드를 만들 때 promotion에서 읽습니다. 임베딩 모델은 연결돼 있지 않아 `embedding`은 NULL이고 순위는 전문 검색으로만 정합니다. 모델을 붙여 차원이 바뀌면 마이그레이션으로 바꿉니다.

## 11. API 이름과 DB 이름

대부분 같고, 다른 곳은 아래뿐입니다. 테이블 주석의 `API:`와 `schema-map.json`의 `api_name`에도 같은 대응이 있습니다.

| API (사람·모델이 말하는 이름) | DB | 이유 |
|---|---|---|
| 상품 `product_id` (`prd_`) | `catalog.product_group.id` | schema.org에서 변형을 묶은 "상품"은 `ProductGroup` |
| 변형 `variant_id` (`var_`) | `catalog.product.id` | schema.org `Product`는 사고파는 한 단위(SKU) |
| 주문 `order_id` (`ord_`) | `sales.sales_order.id` | `order`는 SQL 예약어 |
| 고객 `customer_id` (`cus_`) | `customer.person.id` | schema.org `Person` |
| 운영자 `admin_id` (`adm_`) | `platform.admin_user.id` | `user`는 SQL 예약어 |
| 스토어 `tenant` (`store_`) | `platform.online_store.id` | schema.org `OnlineStore` |
| 조직 `org_id` (`org_`), `acom:org_type` | `platform.organization.id`, `organization_type` | schema.org `Organization`. 줄임말을 컬럼에 쓰지 않음 |
| 메뉴 `menu_id` (`mnu_`), `path` | `platform.site_navigation_element.id`, `url` | schema.org `SiteNavigationElement`(schema.org `Menu`는 음식 메뉴) |
| 카테고리 `category_id` (`cat_`) | `merchandising.defined_term.id` | schema.org `DefinedTerm`(묶음 `in_defined_term_set`) |
| 기억 `memory_id` (`mem_`) | `customer.preference.id` | `acom:Preference` ⊂ `PropertyValue` |

변형을 가리키는 DB 컬럼은 `product_id`, 상품을 가리키는 DB 컬럼은 `product_group_id`입니다. 값의 접두(`var_`, `prd_`)가 항상 어느 쪽인지 알려 줍니다.

## 12. 테이블 목록

마이그레이션 31개가 만드는 테이블과 뷰는 135개, 컬럼은 1,816개이고, 모두 schema.org나 `acom:` 용어로 시작하는 주석이 있습니다(`cargo xtask check-schema`). 컬럼까지는 [schema.generated.md](schema.generated.md)에 있습니다.

| 스키마.테이블 | 용어 (`@type`/`additionalType`, `-`는 기술 테이블) | 접두 | 비고 |
|---|---|---|---|
| `platform.online_store` | schema:OnlineStore | `store` | 스토어 = 테넌트. 통화, 시간대 |
| `platform.admin_user` | schema:Person | `adm` | 운영자 |
| `platform.role`, `platform.role_scope`, `platform.admin_user_role` | schema:Role, -, - | `rol` | 역할 → 스코프 |
| `platform.session` | - | `ses` | 사람 세션 (게이트웨이)과 API 클라이언트 세션(`login`, `actor_kind: external_agent`, 리프레시 토큰 해시·교체) |
| `platform.organization` | schema:Organization | `org` | 부서·입점사·지점. 운영자는 `admin_user.organization_id`(schema:worksFor) |
| `platform.site_navigation_element`, `platform.menu_favorite` | schema:SiteNavigationElement, - | `mnu` | 백오피스 메뉴(API: menu)와 운영자별 즐겨찾기 |
| `platform.api_key` | schema:Intangible/acom:ApiKey | `key` | 외부 에이전트·연동 클라이언트의 키. 비밀 값은 SHA-256 해시만 |
| `platform.policy_override` | - | — | 스토어별 정책 덮어쓰기(파일 이름, 부분 YAML, 버전) |
| `platform.preview` | schema:Action (PotentialActionStatus) | `pv` | 프리뷰 저장소 |
| `platform.change_set` | schema:ItemList/acom:ChangeSet | `cs` | |
| `platform.change_set_item` | schema:ListItem | — | 순서, 의존 |
| `platform.change_set_event` | schema:Action | — | 상태 전이 |
| `platform.action_log` | schema:Action | `act` | 모든 호출·결정 |
| `platform.task` | schema:Action/acom:Task | `task` | 할 일 |
| `platform.job`, `platform.outbox` | - | — | 예약 작업, 도메인 이벤트 |
| `platform.idempotency_key` | - | — | 사람 전용 엔드포인트 재시도 |
| `platform.demo_clock` | - | — | 스토어별 데모 시각 |
| `catalog.brand` | schema:Brand | `brn` | |
| `catalog.product_group` | schema:ProductGroup | `prd` | API: product |
| `catalog.product` | schema:Product | `var` | API: variant. `color`, `size`, `sku`, `weight_*` |
| `catalog.offer` | schema:Offer | `off` | `price`, `price_currency`, `availability`, `inventory_level`, `cost_value`·`cost_currency`(acom, 운영자만) |
| `catalog.measurement` | schema:QuantitativeValue (hasMeasurement) | — | 실측(가슴, 어깨, 소매, 기장) |
| `catalog.merchant_return_policy` | schema:MerchantReturnPolicy | — | `merchant_return_days`, `return_fees` |
| `catalog.review` | schema:Review | `rev` | `rating_value`, `review_body`, `fit_vote` |
| `catalog.data_feed` | schema:DataFeed | `upl` | 올린 공급사 시트(API: upload_id). 행은 `data_feed_element` |
| `catalog.product_import` | schema:CreateAction/acom:ProductImport | `imp` | `batch_import` 실행 한 번 |
| `catalog.data_feed_item` | schema:DataFeedItem | — | 시트 행 ↔ 초안, `draft_status`, `open_fields` |
| `catalog.unit_price_specification` | schema:UnitPriceSpecification | `ups` | 예약 정가(`schedule_price_change`), 그때그때 계산 |
| `catalog.type_and_quantity_node` | schema:TypeAndQuantityNode | — | 묶음(`create_bundle`)의 구성 변형과 수량. 묶음의 가격·재고는 구성에서 계산 |
| `catalog.review_vote`, `catalog.review_report` | schema:VoteAction/acom:ReviewVote, schema:Action/acom:ContentReport | —, `rpt` | 리뷰 도움돼요(쇼퍼마다 하나), 리뷰 신고(모더레이션 대기열로) |
| `catalog.question`, `catalog.answer` | schema:Question, schema:Answer | `qst`, `ans` | 상품 문의와 채택된 답(문의마다 하나) |
| `catalog.restock_alert` | schema:SubscribeAction/acom:RestockAlert | `rsa` | 품절 변형의 재입고 알림 신청. 보낼 알림은 `platform.outbox`에 남음 |
| `catalog.reorder`, `catalog.reorder_item` | schema:Order/acom:Reorder, schema:OrderItem | `ro` | 공급사에 내는 재주문과 변형별 수량 |
| `catalog.copy_draft`, `catalog.image_object` | schema:CreativeWork/acom:CopyDraft, schema:ImageObject | `cpd`, `img` | `generate_copy`·`generate_image`의 초안(자리 표시 생성). `update_product_group`이 쓸 때까지 상품 페이지 밖 |
| `merchandising.defined_term` | schema:DefinedTerm | `cat` | 카테고리, `in_defined_term_set`(standard/display) |
| `merchandising.product_group_category` | - | — | 상품 ↔ 카테고리 |
| `merchandising.category_mapping` | schema:Intangible/acom:CategoryMapping | `map` | 표준 → 전시 카테고리 |
| `merchandising.display_slot` | schema:WebPageElement | `slot` | `slot_type`, 페이지 |
| `merchandising.banner` | schema:CreativeWork/acom:Banner | `bnr` | `headline`, `alternative_headline`, `image`, `url` |
| `merchandising.display_booking` | schema:Schedule/acom:DisplayBooking | `dsp` | 슬롯 × 대상 × 기간. 슬롯 구성 예약은 `item_list_element` 사본과 `sales_channel` |
| `merchandising.collection_page` | schema:CollectionPage | `col` | 기획전. 섹션은 `has_part`(jsonb), 상태 `display_status` |
| `merchandising.popup` | schema:WebPageElement/acom:Popup | `pop` | `popup_type`, 페이지, 순위, "그만 보기" 일수, 기간 |
| `merchandising.cache_invalidation` | schema:Action/acom:CacheInvalidation | `cin` | 전시 캐시 비우기 요청 기록(캐시가 아직 없어 기록만) |
| `search.search_document` | - | — | §10 |
| `search.lexicon` | schema:DefinedTermSet/acom:Lexicon | `lex` | 언어별 검색 사전의 버전(동의어·신조어·부스트/제외). `v1`은 정책 기본값, 재색인이 쓰면 적용 |
| `search.reindex_job` | schema:UpdateAction/acom:ReindexJob | `rij` | 검색 문서 재작성 한 번(`full`/`incremental`), 쓴 사전 버전, 문서 수 |
| `promotion.promotion` | schema:OfferCatalog/acom:Promotion | `promo` | 상태, 기간, 대상 등급 |
| `promotion.unit_price_specification` | schema:UnitPriceSpecification | `ups` | 회원가·기간가 |
| `promotion.coupon` | schema:Offer/acom:Coupon | `cpn` | `code`, 적용 방식, 중복 규칙 |
| `promotion.member_coupon` | schema:OwnershipInfo/acom:MemberCoupon | `mcp` | 발급된 쿠폰 |
| `promotion.loyalty_transaction` | schema:MoneyTransfer/acom:LoyaltyTransaction | `lyt` | 적립금 원장(추가만), 잔액은 합계 |
| `promotion.conditional_gift` | schema:Offer/acom:ConditionalGift | `cgf` | 금액 조건 사은품 |
| `promotion.segment` | schema:Audience/acom:Segment | `seg` | 조건으로 정의한 고객 묶음 |
| `promotion.event`, `promotion.event_entry` | schema:Event, schema:JoinAction/acom:EventEntry | `evt`, `ent` | 이벤트와 응모 |
| `promotion.draw_record`, `promotion.event_reward` | schema:ChooseAction/acom:DrawRecord, schema:Offer/acom:EventReward | `drw`, `rwd` | 추첨 기록(추가만), 보상 |
| `customer.person` | schema:Person | `cus` | `given_name`, `family_name`, `email`, `telephone`, `member_state` |
| `customer.postal_address` | schema:PostalAddress | `adr` | `street_address`, `address_locality`, `address_region`, `postal_code`, `address_country` |
| `customer.member_program` | schema:MemberProgram | `prg` | |
| `customer.member_program_tier` | schema:MemberProgramTier | — | id는 스토어가 정한 단어(`vip`), `rank` |
| `customer.program_membership` | schema:ProgramMembership | — | 사람 ↔ 등급 |
| `customer.preference` | schema:PropertyValue/acom:Preference | `mem` | 보이는 기억 |
| `customer.member_program_version` | schema:MemberProgram/acom:ProgramVersion | — | 등급 프로그램 정의의 버전(`define_tier_program`). 되돌리기용으로 남김 |
| `customer.program_membership_event` | schema:UpdateAction/acom:TierChange | — | 회원 등급 변경 이력(추가만) |
| `customer.tier_evaluation` | schema:AssessAction/acom:TierEvaluation | `tev` | 등급 평가 한 번과 그 이동 |
| `customer.wishlist_item` | schema:ListItem/acom:WishlistItem | — | 위시리스트. 상품 id가 키 |
| `customer.consent_record` | schema:Action/acom:ConsentRecord | `cns` | 동의·철회 기록(추가만). API: consent |
| `customer.customer_inquiry`, `customer.comment` | schema:Message/acom:CustomerInquiry, schema:Comment | `inq`, `cmt` | 고객 문의와 그 대화. API: inquiry |
| `customer.question`, `customer.news_article` | schema:Question, schema:NewsArticle | `faq`, `ntc` | FAQ, 공지. API: faq, notice |
| `customer.employee_role` | schema:EmployeeRole | `emp` | 직원인 회원과 직원 할인 한도. API: employee |
| `customer.remedy` | schema:Action/acom:Remedy | `rmd` | 고객 보상(적립금·쿠폰·현금), 귀책과 단계별 승인 |
| `customer.identity_verification`, `customer.recovery_request` | schema:CheckAction/acom:IdentityVerification, schema:Action/acom:RecoveryRequest | `ivf`, `rcv` | 본인 확인 한 번, 비밀번호 재설정·아이디 찾기 요청 |
| `customer.withdrawal` | schema:Action/acom:Withdrawal | — | 탈퇴: 사유, 정산, 보관하는 것과 기한, 재가입을 잠시 막는 단방향 지문 |
| `sales.cart` | schema:Order/acom:Cart | `cart` | |
| `sales.cart_line` | schema:OrderItem/acom:CartLine | `cln` | |
| `sales.sales_order` | schema:Order | `ord` | `order_number`, `order_status`, `order_date`, `is_gift` |
| `sales.order_item` | schema:OrderItem | `oln` | `order_item_status`, `order_quantity`, `acom:fulfillment_stage` |
| `sales.payment` | schema:PayAction | `pay` | `payment_method`, `payment_status` |
| `sales.order_event` | schema:Action | — | |
| `sales.gift_order` | schema:AcceptAction/acom:GiftOrder | `gift` | 받는 사람이 수락하고 주소를 줄 때까지 기다리는 선물 |
| `sales.subscription` | schema:Order/acom:Subscription | `sub` | 정기 주문: `subscription_status`, `billing_cycle`, `next_charge_date`, 줄(`ordered_item`)과 배송지(`order_delivery`)는 jsonb |
| `sales.receipt` | schema:Invoice/acom:Receipt | `rct` | `receipt_type`, `tax_id`, 주문·종류마다 취소되지 않은 것 하나 |
| `sales.guest_lookup` | - | — | `verify_guest`의 주문 번호별 틀린 시도·잠금·토큰 해시 |
| `fulfillment.parcel_delivery` | schema:ParcelDelivery | `dlv` | `delivery_state`, `expected_arrival_until`, `tracking_number` |
| `fulfillment.claim` | schema:ReturnAction/acom:Claim (취소는 CancelAction) | `clm` | `claim_type`, `claim_status`, `claim_reason`, `liability` |
| `fulfillment.claim_item` | schema:OrderItem | — | 클레임 하나가 여러 주문 품목을 담을 때(#20377) 품목별 수량·사유·환불 예상액 |
| `fulfillment.claim_event` | schema:Action | — | |
| `fulfillment.inspection` | schema:CheckAction | `ins` | `inspection_condition` |
| `fulfillment.refund` | schema:MoneyTransfer/acom:Refund | `rfd` | `refund_status`, `refund_method`, `amount_value`·`amount_currency` |
| `fulfillment.parcel_delivery_event`, `fulfillment.refund_event` | schema:Action | — | 배송·환불 상태 변화(추가만) |
| `fulfillment.shipping_zone` | schema:ShippingRateSettings/acom:ShippingZone | `zone` | 배송 구역: 배송비, 무료 배송 기준, 추가 요금, 배송 일수 |
| `messaging.message` | schema:Message | `msg` | `recipient_id`, `text`, `message_channel`, `about_id` |
| `messaging.message_template` | schema:CreativeWork/acom:MessageTemplate | `tpl` | 채널별 재사용 메시지와 변수. 만들어도 보내지 않음 |
| `messaging.campaign` | schema:CreativeWork/acom:Campaign | `cmp` | `campaign_status`(`failed` 포함), `scheduled_time`, `date_sent`, `error`, 대상(`valid_for_member_tier`·`segment_id`·`recipient_ids`), `variable_values`. 고객별 메시지는 `messaging.message.campaign_id`(`message_kind: campaign`). 주문은 `sales_order.campaign_id`로 자기를 데려온 캠페인을 가리킴 |
| `community.board` | schema:CreativeWork/acom:Board | `brd` | `board_type`, `write_permission`, `allow_secret`, `allow_attachment`, `moderation_mode` |
| `community.discussion_forum_posting` | schema:DiscussionForumPosting | `post` | API: post. `headline`, `article_body`, `is_secret`, `moderation_status`, 마스킹된 `author_name` |
| `community.comment` | schema:Comment | `rpl` | API: reply. `parent_item_id`, `reply_depth`, `text`, `moderation_status` |
| `community.content_report` | schema:Action/acom:ContentReport | `rpt` | 글 신고. 글·신고자마다 하나 |
| `analytics.daily_sales` | - (뷰) | — | 일별 매출·주문 수 |
| `analytics.order_summary`, `analytics.order_line_summary` | - (뷰) | — | 주문·주문 줄 하나에 한 행: 날짜, 상태, 등급, 캠페인, 금액. 누가 샀는지는 없음 |
| `analytics.variant_stock_out`, `analytics.campaign_send` | - (뷰) | — | 지금 품절인 변형과 품절 시각, 캠페인 발송 여부·실패 이유 |
| `analytics.member_tier_count`, `analytics.daily_new_members` | - (뷰) | — | 등급별 회원 수, 일별 신규 가입 수 |
| `analytics.inquiry_summary` | - (뷰) | — | 고객 문의 하나에 한 행: 들어온 날, 종류, 상태, 첫 답 시각, 해결 시각. 누가 무엇을 물었는지는 없음 |
| `partner.seller` | schema:Organization/acom:Seller | `slr` | `legal_name`, `tax_id`(운영자에게 가림), `seller_status`, `seller_type`, 판매사 조직 `organization_id` |
| `partner.seller_event` | schema:Action | — | 상태 전이와 사유. 나중으로 예약한 전이는 `potential`로 기다림 |
| `partner.seller_product_group` | acom:SellerProduct | — | 판매자 ↔ 상품, 정지로 숨긴 시각 `hidden_at` |
| `partner.seller_contract`, `partner.seller_contract_version` | schema:Intangible/acom:SellerContract, acom:SellerContractTerms | `ctr` | 계약 id는 그대로, 버전은 추가만. 수수료율·정산 주기·방식·정산 계좌 참조·기간 |
| `partner.seller_grade` | schema:Intangible/acom:SellerGrade | — | 확정·조정한 등급, 계산값과 근거 지표, 적용 기간 |
| `integration.channel` | schema:WebAPI/acom:Channel | `chn` | 외부 채널: 종류, 주소, 연결 상태, 상태 매핑(jsonb), 자격 증명·서명 키는 SHA-256 해시만 |
| `integration.external_order`, `integration.external_claim` | schema:Order/acom:ExternalOrder, schema:ReturnAction/acom:ExternalClaim | — | 채널이 마지막으로 보낸 주문·클레임 사본. 키는 채널 + 채널의 참조, 매핑된 `order_id`·`claim_id` |
| `integration.webhook_event` | schema:ReceiveAction/acom:WebhookEvent | `whk` | 받은 이벤트 한 건(중복 키 `event_key`), 반영 대상과 결과. 대상이 없으면 `potential`로 기다림 |
| `integration.product_import`, `integration.product_import_item` | schema:CreateAction/acom:ProductImport, schema:DataFeedItem | `imp` | `import_products` 한 번과 행별 결과·변형 매핑. 새 상품 초안은 같은 id로 `catalog.product_import`에도 |
| `agent.conversation`, `agent.turn`, `agent.step`, `agent.model_call` | schema:Conversation, schema:Message, schema:Action | `conv`, `turn` | agent 서비스 |

## 13. 마이그레이션 규칙과 검사

- 파일: `migrations/NNNN_<내용>.sql`. 지금 `0001`–`0032`의 31개이고 `0015`는 쓰지 않은 번호입니다. 모든 테이블은 우리 문서에서 새로 설계해 작성합니다(다른 시스템의 정의를 옮기지 않음).
- **추가만 합니다.** 데모 서버의 DB가 마이그레이션을 적용하고 있고, `sqlx`는 적용된 파일의 체크섬이 바뀌면 `migrate`를 거부합니다. 그래서 적용된 파일은 고치지 않고 새 번호의 파일을 더하며, 파괴적 변경은 확장 → 이전 → 축소 세 단계로 합니다.
- **번호는 main의 마지막 번호 다음입니다.** 두 브랜치가 같은 번호를 잡았으면 나중에 머지하는 쪽이 번호를 올립니다(예: integration은 `0031`에서 `0032`로).
- 한 파일 안에서: `CREATE TABLE` → 인덱스 → `COMMENT` → RLS 순서.
- `cargo xtask check-schema`가 확인하는 것:
  - 모든 테이블·컬럼에 주석이 있고 첫 토큰이 `schema:`/`acom:`/`-`.
  - 이름이 snake_case이고 줄임말 목록에 걸리지 않음.
  - 참조 컬럼이 `_id`로 끝남.
  - 금액 `_value`에 짝 `_currency`가 있음.
  - 스토어 범위 테이블에 `tenant_id`와 RLS 정책이 있음.
  - 시각이 `timestamptz`, 부동소수 컬럼 없음.
  - `id`에 접두 검사가 있음.
  - `CHECK` 단어 목록이 `acom-core` enum과 같고, 주석에 `values: <목록>`이 있음.
  - 테이블 용어의 우리 타입은 PascalCase, 컬럼 용어의 우리 프로퍼티는 lowerCamelCase.
- 마이그레이션을 실제 DB에 적용하는 테스트는 통합 테스트로 분류하고, 요청이 있을 때만 돌립니다(`cargo test -p acom-store -- --ignored`).
