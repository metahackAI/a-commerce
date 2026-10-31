# 에러

모든 에러는 같은 모양이고, **분류(class)** 가 HTTP 상태를 정하며, **코드(code)** 는 에이전트가 분기할 만큼만 구체적입니다.

```jsonc
{"error": {
  "code": "below_price_floor",          // 구체적인 코드 (도메인 파일의 '에러:' 줄)
  "class": "policy_violation",          // 아래 §1의 분류 중 하나 → HTTP 상태
  "message": "At 20% the Seersucker overshirt would sell below cost + 15%.",   // 사람이 읽는 문장
  "next_tool": "set_sale_prices",       // 다음에 부를 툴 이름 또는 null
  "detail": {"rule": "price_floor", "items": ["prd_seersucker_overshirt"]}     // 선택
}}
```

규칙:
1. `next_tool`은 **툴 이름 또는 `null`** 입니다. "다시 입력하세요" 같은 안내는 `message`나 `detail.hint`에 씁니다. 사람 전용 엔드포인트의 에러는 항상 `null`입니다(사람이 화면에서 처리).
2. 한 개념에 코드 하나. 무엇이 없는지는 코드가 아니라 `detail.entity`로 말합니다.
3. 에러 코드와 정책 체크 키는 다른 이름 공간입니다. 정책 위반 에러는 `detail.rule`에 체크 키를 담습니다.
4. 같은 `preview_id` 재실행, 같은 `Idempotency-Key` 재요청은 에러가 아니라 첫 결과입니다.

---

## 1. 분류

| class | HTTP | 언제 | 흡수하는 코드 (예전 표기 포함) | `detail` |
|---|---|---|---|---|
| `unauthenticated` | 401 | 로그인 안 됨, 토큰 만료·폐기 | `not_signed_in`, `invalid_credentials`, `refresh_expired`, `refresh_reused`, `revoked`, `invalid_signature` | `reason` |
| `forbidden` | 403 | 스코프·역할·소유자 아님, 다른 스토어 | `forbidden`, `forbidden_scope`, `scope_not_granted`, `unmask_forbidden`, `wrong_approver`, `forbidden_initiator`, `write_not_allowed`, `not_author`, `not_your_order`, `order_not_for_customer`, `tenant_mismatch` | `reason`: `scope` · `role` · `owner` · `initiator` · `tenant`, `required_scope` |
| `not_found` | 404 | 대상 없음 | 모든 `*_not_found`, `unknown_channel`, `channel_not_registered`, `unknown_tier`, `no_tier` | `entity`, `id` |
| `invalid_input` | 422 | 스키마·형식·범위 위반 | `invalid_filter`, `invalid_status`, `invalid_<값>`, `unknown_*`, `unsupported_*`, `*_not_supported`, `missing_required_field`, 입력 누락을 뜻하는 `*_required`, `invalid_period`, `past_date`, `valid_from_in_past`, `as_of_in_future`, `invalid_schedule`, `negative_*`, `empty_text`, `preview_input_conflict` | `field`, `reason`: `missing` · `format` · `enum` · `range` · `past` · `future`, `allowed[]` |
| `already_exists` | 409 | 중복 | `duplicate_*`, `code_taken`, `already_reviewed`, `already_subscribed`, `already_reported`, `already_assigned`, `already_issued` | `entity`, `field`, `existing_id` |
| `invalid_state` | 409 | 지금 상태에서 할 수 없음 | `invalid_state`, `invalid_transition`, `already_in_state`, 끝 상태를 뜻하는 `already_*`, `not_blocked`, `not_failed`, `not_picked_up`, `inquiry_closed`, `event_not_active`, `entries_open`, `change_set_not_run`, `inspection_pending`, `reindex_in_progress`, `evaluation_in_progress` | `entity`, `from`, `allowed_to[]` |
| `limit_exceeded` | 422 | 한도 초과 | `*_limit_exceeded`, `too_many_*`, `over_limit`, `batch_too_large`, `depth_exceeded`, `daily_send_cap_exceeded`, `pool_exhausted`, `max_retries_reached` | `limit`, `actual`, `policy_key` |
| `rate_limited` | 429 | 요청이 너무 잦음 | `rate_limited`, `too_many_attempts`(빈도) | `retry_after_seconds` |
| `locked` | 423 | 잠김 | `account_locked`, `reauthentication_locked`, `post_locked`, `too_many_attempts`(잠금) | `until` |
| `policy_violation` | 422 | 정책 체크 실패로 실행 불가 | `below_price_floor`, `over_max_discount`, `low_stock`, `promotion_overlap`, `stacking_conflict`, `cost_share_sum`, `odds_exceed_one`, `commission_out_of_bounds`, `slot_taken`, `quiet_hours`, `tier_in_use` 등 | `rule`(체크 키), `items[]` |
| `preview_expired` | 410 | 프리뷰가 없거나 만료·대체됨 | `preview_expired`, `preview_not_found`, `preview_superseded` | `preview_id`, `superseded_by` |
| `approval_required` | 409 | 승인이 필요한 프리뷰를 승인 없이 실행 | `approval_required` | `change_set_id`, `state` |
| `version_out_of_date` | 409 | 최신 버전이 아닌 변경셋 승인 | `version_out_of_date` | `latest_version` |
| `undo_window_closed` | 409 | 되돌리기 기한 지남, 되돌릴 수 없는 변경 | `undo_window_closed`, `not_revocable` | `until` |
| `estimate_changed` | 409 | 서버 재계산 결과가 프리뷰와 다름 | `estimate_changed`, `price_changed`, `eligibility_changed`, `policy_changed` | `new_preview` |
| `idempotency_conflict` | 409 | 같은 키, 다른 요청 본문 | `idempotency_conflict` | `original_request_id` |
| `domain` | 422 | 위에 속하지 않는 업무 규칙 | §2의 코드 | 코드별 |
| `upstream_unavailable` | 503 | 외부 시스템·모델·검색 실패 | `index_unavailable`, `swap_failed`, `generation_failed`, `connection_failed`, `external_lookup_failed`, `payment_gateway_error` | `upstream`, `retryable` |
| `internal` | 500 | 우리 버그 | — | `request_id` |

도메인 파일의 '에러:' 줄에 적힌 예전 표기(위 표의 "흡수하는 코드")는 그대로 두어도 됩니다. 구현은 이 표로 class를 정하고, 같은 개념의 여러 표기는 코드 하나로 모읍니다:

| 이 코드로 통일 | 예전 표기 |
|---|---|
| `not_found` + `detail.entity` | `product_not_found`, `order_not_found`, `tier_not_found`, `unknown_tier`, `no_tier` 등 54종 |
| `forbidden` + `detail.reason` | `not_your_order`, `not_author`, `order_not_for_customer`, `tenant_mismatch` 등 |
| `estimate_changed` | `price_changed`, `eligibility_changed` |
| `payment_declined` | `card_declined` |
| `profanity_blocked` | `banned_words` |
| `cycle_detected` | `circular_hierarchy` |

에이전트가 분기해야 하는 `*_not_found`(예: `change_set_not_found`, `preview_not_found`)는 코드로 남겨도 되고, 이때도 class는 `not_found` 또는 `preview_expired`입니다.

## 2. 도메인 코드 (class `domain`, 또는 위 분류의 구체 코드)

| 도메인 | 코드 |
|---|---|
| catalog | `below_price_floor`(policy_violation), `min_age_not_allowed`, `mixed_seller`, `mixed_age_restriction`, `incomplete_combination`, `has_open_subscriptions`, `brand_guideline_violation`, `no_reviews` |
| sales | `out_of_stock`, `cart_empty`, `no_common_payment_method`, `mixed_order_type`, `payment_declined`, `months_not_available`, `below_min_amount`, `not_card_payment`, `gift_expired`, `guest_match_failed`, `already_shipped`(주문 뒤 변경: 배송지·할부), `deadline_passed`(가상계좌 입금 기한), `amount_mismatch`(입금액이 결제액과 다름), `too_many_attempts`(class `locked`, `verify_guest`), `already_issued`(class `already_exists`, 영수증), `business_id_required`(class `invalid_input`, 세금계산서) |
| fulfillment | `outside_return_window`, `outside_exchange_window`, `not_returnable`, `not_exchangeable`, `not_cancellable`, `not_shipped_yet`, `already_shipped`, `already_handed_over`, `whole_quantity_only`, `quantity_over_cancellable`, `reason_not_selectable`, `reason_not_store_fault`, `claim_in_progress`, `refund_destination_required`, `evidence_required`, `charge_unpaid`, `replacement_unavailable`, `amount_over_payment`, `coupon_not_deducted`, `customer_not_notified`, `address_not_deliverable`, `not_an_exchange`, `price_differs`, `quantity_mismatch`, `no_ship_item`, `payment_declined`(데모 결제사가 추가 비용 결제를 거절) |
| promotion | `over_max_discount`·`low_stock`·`promotion_overlap`·`stacking_conflict`(policy_violation), `coupon_tier_mismatch`, `recipient_tier_mismatch`, `insufficient_balance`, `min_order_not_met`, `compliance_pending`, `changes_after_entry`, `combination_not_allowed`, `type_disabled`, `reward_type_disabled`, `waitlist_empty`, `sold_out`, `not_eligible` |
| customer | `identity_not_verified`, `reverify_required`, `code_mismatch`, `verification_expired`, `mandatory_consent_missing`, `cannot_revoke_mandatory`, `rejoin_blocked`, `id_reuse_blocked`, `withdrawal_blocked`, `password_policy`, `password_reused`, `gift_recipient_data`, `sensitive_data`, `migration_required`, `entry_tier_missing`, `tier_rank_conflict`, `payout_not_approved`, `no_verified_contact` |
| messaging | `no_consented_recipients`, `template_channel_mismatch`, `campaign_not_ready` |
| search | `circular_synonym`, `reindex_pending` |
| merchandising | `slot_taken`·`item_discontinued`·`product_discontinued`·`empty_curation`·`priority_conflict`(policy_violation), `too_many_popups`(limit_exceeded), `cross_set_violation`, `primary_needs_reassign`, `cycle_detected`, `slot_type_mismatch`, `empty_slot` |
| integration | `status_unmapped`, `category_unmapped` |
| platform | `change_set_not_found`, `preview_not_found`, `irreversible_change`(되돌릴 수 없는 항목이 있는 변경셋 되돌리기), `demo_mode_off`(invalid_state, 데모 모드가 아닌 스토어에서 데모 시계 옮기기, 또는 평가용 화면형 파사드 쓰기: `GET /v1/tools?set=facade`, 턴의 `tool_set: facade`), `demo_reset_unavailable`(invalid_state, 데모 스택이 아닌 곳에서 데모 스토어 다시 적재) |
| analytics | `no_data_for_period`(비교할 날에 매출이 없음), `invalid_period`·`invalid_granularity`·`dimension_not_supported`(invalid_input) |

## 3. 구현

- `acom-contract`의 `ErrorCode`는 코드와 class를 함께 가진 상수이고, 도메인 크레이트는 자기 코드를 상수로 선언합니다. HTTP 상태는 class에서만 나옵니다.
- 오퍼레이션 메타의 `errors`에 선언하지 않은 코드를 돌려주면 계약 테스트가 실패합니다.
- 공통으로 모든 오퍼레이션이 낼 수 있는 코드(선언 불필요): `unauthenticated`, `forbidden`, `invalid_input`, `rate_limited`, `upstream_unavailable`, `internal`, 그리고 쓰기의 `preview_expired`, `approval_required`, `estimate_changed`.
