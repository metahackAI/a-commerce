# 값 목록 (enum의 단일 출처)

상태·종류·사유처럼 정해진 단어 목록은 여기 하나에만 정의합니다. 도메인 파일, 정책 YAML, DB 체크 제약, Rust enum이 모두 이 목록을 씁니다. schema.org에 열거 멤버가 있으면 그 이름을 snake_case로 쓰고(오른쪽 열), 없으면 `acom:` 확장입니다.

규칙:
- 값은 소문자 snake_case 영어 단어. 코드값·숫자 금지.
- 목록을 바꾸려면 이 파일을 먼저 바꿉니다. 값을 더하는 것은 호환, 빼거나 이름을 바꾸는 것은 파괴적 변경입니다.
- 스토어가 항목을 더할 수 있는 목록(이벤트 참여 방식, 보상 종류 등)은 **레지스트리**로 표시하고, 기본 항목만 여기 적습니다.

---

## 1. 생애 상태

### 1.1 주문 `order_status` — schema.org `OrderStatus`

| 값 | schema.org | 뜻 |
|---|---|---|
| `payment_due` | OrderPaymentDue | 결제 대기 |
| `processing` | OrderProcessing | 결제 완료, 출고 준비 |
| `in_transit` | OrderInTransit | 배송 중(하나 이상 출고) |
| `delivered` | OrderDelivered | 모든 품목 배송 완료 |
| `returned` | OrderReturned | 모든 품목 반품 완료 |
| `cancelled` | OrderCancelled | 모든 품목 취소 |
| `problem` | OrderProblem | 사람이 봐야 하는 문제 |

주문 품목 `order_item_status`도 같은 목록이고 품목마다 따로 움직입니다. 구매 확정은 상태가 아니라 시각 필드 `acom:confirmed_at`입니다(배송 완료 후 `order.yml`의 `auto_confirm_days`).

### 1.2 출고 단계 `acom:fulfillment_stage` (품목·배송 단위)

| 값 | 뜻 |
|---|---|
| `awaiting_stock` | 재고 부족으로 대기 (예전 표기 `ship_wait`) |
| `ship_instructed` | 출고 지시 |
| `packed` | 포장 완료, 송장 대기 (예전 표기 `ship_confirmed`) |
| `shipped` | 택배사 인계 |

취소 경로 판단(셀프 취소 / 운영자 취소 / 취소 불가)은 이 단계로 합니다(`fulfillment.yml`).

### 1.3 배송 `acom:delivery_state` — `ParcelDelivery`

`awaiting_stock`, `ship_instructed`, `packed`, `label_created`, `picked_up`, `in_transit`, `delivered`, `delivery_failed`, `cancelled`. 앞의 셋은 우리 내부 단계, 나머지는 택배사 이벤트입니다.

### 1.4 결제 `payment_status` — schema.org `PaymentStatusType`

`payment_due`, `payment_complete`, `payment_declined`, `payment_past_due`, `payment_automatically_applied`. 예전 표기 `paid` → `payment_complete`, `payment_pending` → `payment_due`.

### 1.5 클레임 `acom:claim_status` (취소·반품·교환 공통)

| 값 | 뜻 | 액션 로그의 `action_status` |
|---|---|---|
| `charge_due` | 진행 전 추가 비용 결제 필요 | potential |
| `requested` | 접수, 판단 대기 (예전 표기 `waiting`) | potential |
| `photos_requested` | 사진 요청함 | potential |
| `approved` | 승인 | active |
| `pickup_instructed` | 회수 지시 | active |
| `picked_up` | 회수 완료 | active |
| `inspected` | 검수 완료 | active |
| `reship_requested` | 교환품 발송 요청(교환) | active |
| `completed` | 끝남(환불 완료, 교환품 배송 완료, 취소 완료) | completed |
| `declined` | 거절 | failed |
| `withdrawn` | 고객이 철회 | failed |
| `converted` | 교환이 반품으로 전환됨 | completed |

클레임 종류 `claim_type`: `cancel`, `return`, `exchange`. 처리 경로 `acom:claim_path`: `self_service`, `store_review`, `not_allowed`. 교환품의 배송 진행은 클레임이 아니라 교환 배송(1.3)에 있습니다.

### 1.6 환불 `acom:refund_status`

`requested`, `completed`, `failed`. 환불 수단 `acom:refund_method`: `original_payment`, `bank_account`, `store_credit`. 환불 종류는 schema.org `RefundTypeEnumeration`: `full_refund`, `exchange_refund`, `store_credit_refund`.

### 1.7 액션 `action_status` — schema.org `ActionStatusType`

`potential`(프리뷰, 승인 대기), `active`(실행 중), `completed`, `failed`. 액션 로그와 프리뷰 항목에 씁니다.

### 1.8 변경셋 `acom:change_set_state`

`drafting`, `waiting_for_approval`, `running`, `verifying`, `live`, `failed`, `rejected`, `undone`. 끝 상태는 `rejected`, `undone`, 그리고 되돌림이 끝난 `failed`. `live`만 되돌릴 수 있습니다.

### 1.9 프리뷰 `acom:preview_state`

`open`, `committed`, `expired`, `superseded`(같은 변경셋 항목이 새 프리뷰로 바뀜).

### 1.10 상품

| 목록 | 값 |
|---|---|
| 승인 `acom:approval_status` | `draft`, `pending_review`, `active`, `rejected`, `discontinued` |
| 재고 상태 `availability` (schema.org `ItemAvailability`) | `in_stock`, `out_of_stock`, `pre_order`, `back_order`, `discontinued` |
| 가격 종류 `price_type` (schema.org `PriceTypeEnumeration`) | `list_price`, `sale_price` |
| 판매 방식 `acom:sale_type` | `always`, `pre_order`, `subscription`, `gift_card` |
| 초안 진행 `acom:draft_status` (공급사 시트에서 만든 초안) | `ready`(시트의 필수 필드가 다 있음), `needs_you`(필수 필드가 없거나 불분명해 사람이 채워야 함), `working`(가져오기가 아직 채우는 중) |
| 가져오기 방식 `acom:import_mode` (`batch_import`) | `create`(새 초안만), `upsert`(앞선 가져오기가 만든 초안도 시트 값으로 고침) |
| 생성 문구 종류 `acom:copy_type` (`generate_copy`) | `title`(상품명), `summary`(한 줄 요약), `description`(상세 설명: 스토어 어조의 본문 + 관리 한 줄), `seo`(검색 결과에 보일 설명) |
| 생성 이미지 연출 `acom:image_style` (`generate_image`) | `studio`(무지 배경), `lifestyle`(입은 모습·장면), `flat_lay`(바닥에 펼쳐 놓음) |

### 1.11 프로모션·쿠폰·이벤트

| 목록 | 값 |
|---|---|
| 프로모션 `acom:promotion_status` | `draft`, `scheduled`, `running`, `paused`, `ended` |
| 쿠폰 `acom:coupon_status` | `issued`, `available`, `used`, `restored`, `expired`, `revoked` |
| 쿠폰 적용 `acom:coupon_application` | `automatic_at_checkout`, `code_entry`, `download` |
| 쿠폰 중복 `acom:coupon_stacking` | `no_tier_coupons`, `with_tier_coupons`, `no_other_coupons` |
| 할인 층 `acom:discount_layer` (예전 정수 `stacking_tier` 1·2·3) | `item_price`, `order_discount`, `add_on` |
| 프로모션 종류 `acom:promotion_type` (`create_promotion`, 회원가·기간가 프로모션은 없음) | `bogo`, `bundle`, `threshold`, `amount_off` |
| 적립금 거래 `acom:loyalty_transaction_type` | `accrual`(적립), `spend`(사용), `expire`(만료), `grant`(지급), `restore`(취소·반품 복원), `reversal`(지급 되돌리기의 차감) |
| 이벤트 `acom:event_status` | `draft`, `scheduled`, `running`, `paused`, `closed`, `drawing`, `announced`, `fulfilling`, `completed`, `cancelled` |
| 응모 `acom:entry_status` | `entered`, `invalid`, `won`, `lost`, `waitlist`, `cancelled` |
| 보상 `acom:reward_status` | `pending`, `compliance_pending`, `ready`, `dispatched`, `delivered`, `failed`, `forfeited` |
| 이벤트 레지스트리 | 참여 방식·당첨 방식·보상 종류: `policies/promotion.yml`의 `event.*.builtin` |

### 1.12 전시

| 목록 | 값 |
|---|---|
| 예약·기획전 `acom:display_status` | `draft`, `scheduled`, `active`, `ended` |
| 슬롯 종류 `acom:slot_type` | `hero`, `banner_board`, `product_grid`, `brand_strip`, `collection_feed` |
| 카테고리 묶음 `acom:term_set` | `standard`, `display` |
| 팝업 종류 `acom:popup_type` | `image`, `notice`, `event`, `consent` |
| 캐시 무효화 범위 `acom:cache_resource_type` | `category`, `display_slot`, `banner`, `popup`, `collection_page`, `all` |

### 1.13 회원·고객 서비스

| 목록 | 값 |
|---|---|
| 회원 `acom:member_state` | `active`, `dormant`, `blocked`, `pending_withdrawal`, `withdrawn` |
| 회원 유형 | `individual`, `business` |
| 사업자 확인 `acom:business_verification` | `unverified`, `verified` |
| 등급 혜택 종류 | `member_price`, `points_multiplier`, `free_shipping`, `free_returns`, `coupon_on_entry`, `recurring_coupon`, `custom` |
| 등급 조건 지표 | `spend`, `order_count`, `points_earned`, `manual`, `invite`, `custom` |
| 문의 `acom:inquiry_status` | `received`, `in_progress`, `resolved` |
| 문의 종류 `acom:inquiry_type` | `order_status`, `return`, `product_question`, `account`, `callback`, `other` |
| 보상 처리 `acom:remedy_status` | `requested`, `approved`, `rejected`, `payout_pending`, `paid`, `failed` |
| 보상 종류 `acom:remedy_type` | `loyalty_points`, `coupon`, `cash` |
| 동의 종류 `acom:consent_type` | `terms`, `privacy`, `marketing_email`, `marketing_sms`, `marketing_push`, `marketing_in_app`, `entry_terms`, `winner_contact` |
| 주소 용도 | `shipping`, `billing`, `return` |
| 등급 변경 원인 `acom:tier_change_cause` (`list_tier_history`의 `cause`) | `entry`, `evaluation`, `event`, `manual_override`, `override_expired`, `grace_started`, `grace_ended`, `program_change`, `withdrawal` |
| 문의 답글 작성자 `acom:author_type` (`get_inquiry`의 `thread[].author_type`) | `customer`, `staff`, `agent` |
| 임직원 할인 한도 기간 `acom:discount_limit_period` | `month`, `year` |
| 계정 복구 요청 `acom:recovery_kind` | `password_reset`(비밀번호 재설정), `login_id`(로그인 id 찾기) |

### 1.14 그 밖의 상태

| 목록 | 값 |
|---|---|
| 구독 `acom:subscription_status` | `active`, `paused`, `cancelled` |
| 캠페인 `acom:campaign_status` | `draft`, `scheduled`, `sending`, `sent`, `failed`(발송이 메시지 하나 나가기 전에 멈춤, 예: 수신자 목록 가져오기 실패), `cancelled` |
| 공지 `acom:notice_status` | `draft`, `published` |
| 게시글 | `published`, `held_for_review`, `blinded`, `deleted` |
| 리뷰 노출 `review_status` (schema:creativeWorkStatus) | `visible`, `hidden`, `reported` |
| 상품 문의 노출 `question_status` (schema:creativeWorkStatus) | `visible`, `hidden`(운영자가 숨김, 또는 `question.profanity_filter: hold_for_review`로 보류. 문의와 답이 함께 숨음) |
| 리뷰의 사이즈 평가 `acom:fit_vote` | `true_to_size`, `runs_small`, `runs_large` |
| 실측 항목 `acom:measurement_name` | `chest`, `shoulder`, `sleeve`, `length`, `waist`, `hip`, `inseam`, `rise`, `foot_length` |
| 판매자 | `applied`, `under_review`, `approved`, `active`, `suspended`, `terminated` |
| API 키 `acom:api_key_status` | `active`, `revoked` |
| 채널 연결 | `connected`, `degraded`, `disconnected` |
| 할 일 `acom:task_state` | `needs_you`, `working`, `done`, `dismissed` |
| 할 일 출처 `acom:task_origin` | `you_asked`, `agent_noticed`, `automation` |
| 예약 작업 `acom:job_state` (`job.updated` 이벤트, 열에서 계산) | `scheduled`, `retrying`, `done` |
| 운영자 계정 `acom:admin_status` | `active`, `suspended`, `locked` |
| 스토어 `acom:store_status` (API `acom:status`) | `active`, `suspended` |
| 조직 종류 `acom:organization_type` (API `acom:org_type`) | `department`, `vendor`, `branch` |

### 1.15 에이전트

| 목록 | 값 |
|---|---|
| 대화 면 `acom:chat_surface` | `bo`, `fo` |
| 단계 종류 `acom:step_kind` | `route`, `search_tools`, `plan`, `call_tool`, `check` |
| 주체 종류 `acom:actor_kind` | `operator`, `shopper`, `agent_on_behalf`, `external_agent`, `system` |
| 주소 용도 `acom:address_usage` | `shipping`, `billing`, `return` |
| 기억 키 `acom:preference_key` | `size`, `fit_note`, `fabric_preference`, `compares_specs`, `layout_preference`, `travel_priority`, `other` |
| 메시지 종류 `acom:message_kind` | `ask_for_photos`, `decline`, `refund_done`, `custom` |

---

## 2. 사유와 귀책

### 2.1 귀책 `acom:liability`

`store_fault`, `seller_fault`, `customer_fault`, `carrier_fault`, `none`. 예전 표기 `customer_claim`은 귀책이 아니므로 쓰지 않습니다(고객 보상 판단에는 `none`이나 실제 귀책을 씁니다).

### 2.2 반품·교환 사유 `acom:claim_reason`

| 값 | 귀책 기본값 | 예전 표기 |
|---|---|---|
| `defect` | store_fault | |
| `damaged_in_transit` | store_fault | `damaged`, `arrived_damaged` |
| `not_as_described` | store_fault | `misdescription` |
| `wrong_item_sent` | store_fault | |
| `wrong_option_sent` | store_fault | |
| `missing_parts` | store_fault | |
| `wrong_size` | customer_fault | `wrong_size_ordered` |
| `different_option_wanted` | customer_fault | |
| `changed_mind` | customer_fault | |

### 2.3 취소 사유 `acom:cancel_reason`

고객: `changed_mind`, `wrong_option_ordered`, `found_lower_price`, `discount_changed`, `payment_method_change`, `duplicate_order`, `payment_not_completed`. 스토어: `shipping_delay`, `out_of_stock`, `free_gift_unavailable`, `product_info_inaccurate`, `seller_cancelled`, `store_cancelled`.

### 2.4 그 밖의 사유

| 목록 | 값 |
|---|---|
| 클레임 거절 | `outside_return_window`, `not_returnable`, `inspection_failed`, `already_handed_over`, `outside_exchange_window` |
| 교환 → 반품 전환 | `replacement_out_of_stock`, `replacement_discontinued`, `customer_declines` |
| 신고(리뷰·게시글 공통) `acom:report_reason` | `spam`, `abuse`, `obscene`, `advertisement`, `fake`, `irrelevant`, `other` |
| 탈퇴 `acom:withdrawal_reason` | `not_using`, `too_many_messages`, `privacy_concern`, `poor_service`, `found_alternative`, `other` |
| 반품·교환 불가 | `used`, `final_sale`, `gift_card`, `digital`, `product_not_exchangeable` |
| 증빙 종류 `acom:evidence_type` (`upload_evidence`) | `damage_photo`, `delivery_photo`, `product_defect`, `receipt`, `other` |

### 2.5 검수 결과 `acom:inspection_condition`

`as_new`(schema.org `NewCondition`과 같은 상태), `damaged`(`DamagedCondition`), `missing_parts`. 예전 표기 `normal` → `as_new`. 검수 뒤 조치: `approve`, `hold`, `decline`.

---

## 3. 채널과 결제

| 목록 | 값 | 메모 |
|---|---|---|
| 메시지 채널 `acom:message_channel` | `email`, `sms`, `push`, `in_app` | 예전 표기 `on_site` → `in_app` |
| 판매 채널 `acom:sales_channel` | `web`, `mobile_web`, `app`, `agent` | `agent` = 외부 에이전트·대리 주문 |
| 결제 수단 `acom:payment_method` | `credit_card`, `bank_transfer`, `virtual_account`, `loyalty_points`, `store_credit` | |
| 정기 주문 주기 `acom:billing_cycle` | `every_week`, `every_2_weeks`, `every_month` | 회차 사이 간격은 `order.yml` `subscription.cycle_length` |
| 영수증 종류 `acom:receipt_type` | `deduction`, `proof`, `tax_invoice` | `tax_invoice`는 사업자 번호(`tax_id`)가 필요 |
| 반품 배송비 (schema.org `ReturnFeesEnumeration`) | `free_return`, `return_fees_customer_responsibility`, `return_shipping_fees` | |
| 배송비 종류 | `paid`, `free`, `free_over_min_subtotal` | 예전 표기 `conditional_free` |
| 배송비 종류 목록 이름 | `shipping_fee_type` (`acom:feeType`, `fulfillment.shipping_zone.fee_type`) | 위 줄의 값 |
| 배송 지역 종류 `acom:zone_type` | `standard`, `remote` | `remote`는 보통 지역 추가 배송비가 있음 |
| 배송 방법 | `standard` (schema.org `ParcelService`) | |

---

## 4. 프리뷰의 변경 표기

### 4.1 `changes[].op`

`create`, `update`, `delete`, `schedule`, `send`, `grant`, `revoke`, `exclude`(대상에서 빠진 것: 실행하지 않음, 기록만).

### 4.2 `changes[].entity`

데이터 모델의 엔티티 이름(snake_case)만 씁니다: `product`, `variant`, `offer`, `unit_price_specification`, `coupon`, `display_booking`, `order`, `order_item`, `claim`, `refund`, `message`, `cart`, `cart_line`, `preference`, `member_tier`, `policy` 등. 가격 변경은 `price`가 아니라 `unit_price_specification`(기간·회원가) 또는 `offer`(정가)입니다. `id`는 바뀌는 엔티티의 id이고, 아직 만들어지지 않았으면 `null`입니다.

---

## 5. 등급 참조

- 등급 id는 스토어가 정한 단어(`vip`, `gold`, `member`)입니다. 접두가 없습니다.
- `"everyone"`은 예약어로 "등급 제한 없음"을 뜻하고, DB에는 NULL로 저장합니다. 등급 프로그램은 `everyone`을 등급 id로 쓸 수 없습니다.

## 6. 비율

[conventions.md](conventions.md) §4: 툴 입력의 `*_percent`·`percent_off`는 0–100, 정책 파일과 `*_rate`는 0–1. `get_policy`는 저장된 값(0–1)과 사람이 읽는 문장을 함께 돌려줍니다.
