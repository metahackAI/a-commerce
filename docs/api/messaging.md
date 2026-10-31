# API 명세 — `messaging` (고객 메시지·템플릿·캠페인·알림)

> 계층: `messaging.{resource}.{operation}`. 공통 규약(응답 형식·프리뷰/실행·멱등·헤더)은 [conventions.md](conventions.md)와 [README.md](README.md) §3, 값 목록은 [enums.md](enums.md), 데이터 타입은 [data-model.md](data-model.md), 정책은 [approval.yml](../../policies/approval.yml) · [customer.yml](../../policies/customer.yml)(동의 게이트). 예시 값은 데모 시나리오([../scenarios.md](../scenarios.md) B2)와 같습니다.
> **설계 축**: ① 고객에게 닿는 발송은 모두 사람 승인 — 프리뷰가 고객이 읽을 그대로의 메시지를 보여주고 사람이 승인(정책 `customer_messages: always_ask`), ② 마케팅 발송은 동의(opt-in) 게이트 — 동의 없는 대상은 서버가 발송 집합에서 뺌, ③ 채널(`email`·`sms`·`push`·`in_app`, [enums.md](enums.md) §3)은 한 계약, ④ 템플릿 변수는 의미 있는 이름, ⑤ 같은 메시지·캠페인은 두 번 나가지 않음(멱등), ⑥ 보낸 메시지는 되돌릴 수 없음(`change_set.irreversible`).

리소스: [message](#message) · [template](#template) · [campaign](#campaign) · [notification](#notification)

- schema.org: 메시지 `Message`(`recipient`, `text`, `about`), 발송은 `SendAction`. 캠페인은 `acom:Campaign`.
- id 예: 메시지 `msg_5512`, 템플릿 `tpl_back_in_stock`, 캠페인 `cmp_77`, 알림 `ntf_901`.

---

## message

### `send_customer_message` — Kind: W$
- 논리주소/REST: `messaging.message.send` · `POST /v1/messaging/messages`
- 요약: 고객 한 명에게 주문·클레임에 관한 거래 메시지(사진 요청, 반품 거절 안내, 환불 완료 안내, 그 밖의 안내)를 보냅니다. 프리뷰는 **고객이 읽을 그대로**의 메시지를 `changes[].to`에 담고, 정책 `customer_messages: always_ask`에 따라 **항상** 변경셋에서 사람이 승인한 뒤 실행기가 보냅니다. 반품 검토에서는 `refund_return` 프리뷰와 한 변경셋으로 묶입니다(B2 `cs_1043`: "Approve 9 refunds and send 3 messages"). 판촉 문구는 이 툴이 아니라 `create_campaign`으로 보냅니다. MVP 툴.
- consumes → produces: `customer_id`?, `order_id`? → `message_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 | 예 |
  |------|------|:--:|------|----|
  | customer_id | Text (`^cus_[a-z0-9_]+$`) | `order_id`가 없을 때 | 받는 고객. `order_id`가 있으면 생략할 수 있고, 그러면 그 주문을 한 고객에게 갑니다(주문 번호만 들은 에이전트가 고객 id를 지어내지 않도록). 둘 다 있으면 그 고객의 주문이어야 함 | `"cus_riley"` |
  | order_id | Text (`^ord_[a-z0-9_]+$`) | | 메시지가 다루는 주문(`about`). `kind`가 `custom`이 아니면 필수 | `"ord_20366"` |
  | kind | Text (enum) | ✓ | `ask_for_photos`: 파손 사진 요청; `decline`: 반품·교환 거절 안내; `refund_done`: 환불 완료 안내; `custom`: 그 밖의 거래 안내 | `"ask_for_photos"` |
  | channel | Text (enum `acom:message_channel`) | | `email` \| `sms` \| `push` \| `in_app`. 기본 `email`. 고객에게 그 채널의 확인된 연락처가 있어야 함 | `"email"` |
  | subject | Text | | 제목(`email`·`push`). 생략하면 `kind`와 주문 번호로 만듦 | `"About your return for order 20366"` |
  | text | Text | ✓ | 고객에게 보낼 본문(평이한 문장, 2,000자 이하) | `"Sorry your rain parkas arrived damaged. Could you reply with a photo or two of the damage? We'll take it from there."` |
- 출력(프리뷰): 변경셋 `cs_1043`에 붙은 프리뷰. 메시지 내용은 `changes[].to`에 있고 수신자 이름은 가립니다.
  ```jsonc
  {"preview_id": "pv_m01a",
   "expires_at": "2026-11-06T11:24:00-05:00",
   "change_set_id": "cs_1043",
   "changes": [{"op": "send", "entity": "message", "id": null, "from": null,
                "to": {"recipient": "R*** P***", "customer_id": "cus_riley", "about": "ord_20366",
                       "kind": "ask_for_photos", "channel": "email",
                       "subject": "About your return for order 20366",
                       "text": "Sorry your rain parkas arrived damaged. Could you reply with a photo or two of the damage? We'll take it from there."}}],
   "checks": [{"rule": "customer_messages", "passed": true, "detail": "Every customer message needs a person's approval", "fix_tool": null},
              {"rule": "damage_claim_needs_photo", "passed": true, "detail": "Order 20366 has no photo of the damage", "fix_tool": null},
              {"rule": "change_set.irreversible", "passed": true, "detail": "A sent message can't be taken back", "fix_tool": null}],
   "needs_approval": true,
   "approval_reason": "Sends a message to a customer"}
  ```
- 출력(실행): `{"ok": true, "id": "msg_5512", "undo": null}` — 보낸 메시지는 되돌릴 수 없습니다.
- 프리뷰: `checks` `customer_messages`, `change_set.irreversible`, 그리고 `kind`에 맞는 정책 근거 — `ask_for_photos` → `damage_claim_needs_photo`, `decline` → `return_window_days`(또는 거절 사유의 정책 키), `refund_done` → `refund.to`(해당 환불이 실행됐거나 같은 변경셋에 있는지). `needs_approval`은 항상 `true`.
- 정책: approval.yml `customer_messages`(always_ask), `change_set.irreversible`. 주문에 대한 거래 메시지이므로 마케팅 동의 게이트(`consent.marketing_gate`)는 적용하지 않습니다.
- 에러: `customer_not_found`(없는 고객, 또는 `customer_id` 없이 보낸 비회원 주문) → `search_members` · `customer_required`(`customer_id`도 `order_id`도 없음, class `invalid_input`) → `send_customer_message` · `order_not_found` → `get_order` · `order_not_for_customer`(그 고객의 주문이 아님, class `forbidden`) → `get_order` · `order_required`(`custom`이 아닌데 `order_id` 없음) → `send_customer_message` · `refund_not_found`(`refund_done`인데 환불 없음) → `refund_return` · `no_verified_contact`(그 채널의 확인된 연락처 없음, 다른 `channel`로) → `send_customer_message` · `empty_text` → `send_customer_message`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 서버는 `(customer_id, order_id, kind)`로도 같은 안내가 두 번 나가지 않게 막습니다(`custom`은 본문 해시 포함).
- 구현: 주문의 고객은 sales가 내놓은 `acom_sales::service::for_messaging::order_owner`로 찾습니다(messaging은 반품 검토 때부터 sales·fulfillment의 `service::for_messaging`을 씁니다). 이메일·SMS·푸시는 바깥 시스템이라, 실행은 메시지를 보낸 것으로 기록할 뿐 실제 제공자에게 넘기지 않습니다.

### `send_bulk` — Kind: W$
- 논리주소/REST: `messaging.message.send_bulk` · `POST /v1/messaging/campaigns/{campaign_id}/send`
- 요약: 캠페인(초안·예약·발송 실패한 것)을 지금 대상 전체에 발송. 그 채널의 마케팅 동의(opt-in)가 없거나, 그 채널로 닿을 수 없거나(연락처 없음, 차단·탈퇴 중), 오늘 이미 상한만큼 마케팅 메시지를 받은 대상은 서버가 자동으로 뺍니다. 많은 고객에게 닿으므로 항상 승인.
- consumes → produces: `campaign_id` → `message_id[]`
- 입력: `campaign_id`(✓, 경로)
- 출력(프리뷰): `{"preview_id": "pv_b77a", "expires_at": "2026-11-07T09:30:00-05:00", "changes": [{"op": "send", "entity": "message", "id": null, "from": null, "to": {"campaign_id": "cmp_77", "name": "VIP weekend reminder", "recipients": 1198, "left_out": 86, "channel": "email", "sample": {"recipient": "J*** C***", "subject": "VIP weekend: the wool overcoat", "text": "Hi J*** C***, ..."}}, "note": "No email, SMS or push provider is connected: the messages are recorded as sent and nothing was delivered outside the store."}], "checks": [{"rule": "consent.marketing_gate", "passed": true, "detail": "1,198 of 1,284 opted in to email; 86 left out", "fix_tool": "get_consent"}, {"rule": "messaging.quiet_hours", "passed": true, "detail": "Sends at 10:00 store time", "fix_tool": "create_campaign"}, {"rule": "messaging.daily_send_cap", "passed": true, "detail": "Everyone is under 2 marketing messages today", "fix_tool": null}, {"rule": "change_set.irreversible", "passed": true, "detail": "Sent messages can't be taken back", "fix_tool": null}], "needs_approval": true, "approval_reason": "Sends email to 1,198 customers"}` — `sample`은 첫 수신자가 읽을 메시지(이름은 가림). `change_set.irreversible`은 변경셋에 붙을 때 엔진이 더합니다.
- 출력(실행): `{"ok": true, "id": "cmp_77", "undo": null}` — 고객마다 메시지 하나(`message_kind: campaign`, `campaign_id`)를 보낸 것으로 기록하고 캠페인은 `sent`. 이메일·SMS·푸시 제공자는 바깥 시스템이라 연결되어 있지 않고, 실제로 전달되지는 않습니다(`get_campaign`의 `delivery_note`).
- 프리뷰: `checks` `consent.marketing_gate`, `messaging.quiet_hours`(지금이 조용한 시간이면 실패. 실행 때 다시 보고 `quiet_hours`), `messaging.daily_send_cap`([messaging.yml](../../policies/messaging.yml) `daily_send_cap`: 회원 한 명이 하루(스토어 날짜)에 받는 마케팅 메시지 수. 상한에 닿은 회원은 빠짐), `change_set.irreversible`.
- 정책: `consent.marketing_gate` ([customer.yml](../../policies/customer.yml)), approval.yml `customer_messages`, `change_set.irreversible`
- 에러: `campaign_not_found` → `search_campaigns` · `campaign_not_ready`(취소됨, 템플릿 없음·채널 다름, 템플릿 변수 값 없음) → `get_campaign` · `no_consented_recipients` → `preview_segment` · `daily_send_cap_exceeded`(남은 대상이 모두 오늘 상한, 다른 날로) → `create_campaign` · `already_sent` → `get_campaign` · `quiet_hours`(실행 때 조용한 시간) → `create_campaign`
- 멱등성: `campaign_id`당 한 번 — 다시 보내면 첫 결과 반환(이중 발송 없음). 고객별 메시지 id는 `msg_{campaign_id}_{customer_id}`.

---

## template

### `create_message_template` — Kind: W
- 논리주소/REST: `messaging.template.create` · `POST /v1/messaging/templates`
- 요약: 채널별 메시지 템플릿(제목·본문·변수·언어). 만들기만 하고 보내지 않으므로 승인 없이 실행.
- consumes → produces: → `template_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | name | Text | ✓ | `"Back in stock"` |
  | channel | Text(enum) | ✓ | `email` \| `sms` \| `push` \| `in_app` |
  | purpose | Text(enum) | ✓ | `marketing` \| `transactional` |
  | headline | Text | | 제목(email·push) |
  | text | Text | ✓ | 본문, 변수는 `{{customer_name}}`처럼 |
  | variables | Text[] | | `["customer_name", "product_name"]` |
  | in_language | Text | | `"en-US"` |
- 출력(프리뷰): `{"preview_id": "pv_tpl_3", "expires_at": "2026-11-06T14:30:00-05:00", "changes": [{"op": "create", "entity": "message_template", "id": null, "from": null, "to": {"name": "Back in stock", "channel": "email", "status": "draft"}}], "checks": [{"rule": "messaging.template.variables_declared", "passed": true, "detail": "2 variables, all declared", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "tpl_back_in_stock", "undo": {"by": "person", "target": "cs_1052", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: [messaging.yml](../../policies/messaging.yml) `messaging.template.variables_declared`(템플릿 변수는 선언된 것만)
- 에러: `unknown_variable` → `create_message_template` · `channel_not_supported` → `list_enums`
- 멱등성: 같은 `preview_id` 재실행은 첫 결과.

### `list_message_templates` — Kind: R
- 논리주소/REST: `messaging.template.list` · `GET /v1/messaging/templates?channel=email`
- 요약: 템플릿 목록(채널·용도·언어로 거르기).
- consumes → produces: → `template_id[]`
- 입력: `channel?`, `purpose?`, `in_language?`, `fields?[]`, `limit`(기본 20), `cursor`
- 출력: `{"items": [{"id": "tpl_back_in_stock", "name": "Back in stock", "channel": "email", "purpose": "marketing", "status": "draft"}], "total": 6}`
- 정책: 없음(읽기).
- 에러: `invalid_filter` → `list_message_templates`
- 멱등성: 읽기.

---

## campaign

### `create_campaign` — Kind: W$
- 논리주소/REST: `messaging.campaign.create` · `POST /v1/messaging/campaigns`
- 요약: 캠페인(대상·채널·템플릿·발송 시각) 생성. 대상은 세그먼트 또는 회원 목록이고, 동의 없는 대상은 프리뷰에서 빠진 수로 보입니다. 고객에게 닿을 발송을 예약하므로 W$.
- consumes → produces: `segment_id` \| `customer_id[]`, `template_id` → `campaign_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명·예 |
  |------|------|:--:|--------|
  | name | Text | ✓ | `"VIP weekend reminder"` |
  | channel | Text(enum) | ✓ | `email` \| `sms` \| `push` \| `in_app` |
  | template_id | Text | ✓ | 채널이 맞아야 함 (`list_message_templates`) |
  | audience_mode | Text(enum) | ✓ | `segment`: `segment_id`로; `customers`: `customer_ids`로; `tier`: `valid_for_member_tier`로 |
  | segment_id | Text | `segment`일 때 | `"seg_lapsed_90"` |
  | customer_ids | Text[] | `customers`일 때 | `["cus_jamie"]` |
  | valid_for_member_tier | Text | `tier`일 때 | 등급 id, 예 `"vip"` |
  | scheduled_time | DateTime | | 스토어 시각. 있으면 `scheduled`로 예약되고 그 시각에 예약 작업(`messaging.send_campaign`)이 보냄. 비우면 `draft`로 만들어지고 `send_bulk`가 보냄 |
  | variables | Object[] | | 모든 고객에게 같은 템플릿 변수 값 `[{name, value}]`. `customer_name`은 고객마다 채움. 템플릿의 다른 변수는 모두 값이 있어야 함 |
- 출력(프리뷰): `{"preview_id": "pv_cmp_77", "expires_at": "2026-11-06T14:40:00-05:00", "changes": [{"op": "schedule", "entity": "campaign", "id": null, "from": null, "to": {"name": "VIP weekend reminder", "status": "scheduled", "scheduled_time": "2026-11-07T10:00:00-05:00", "recipients": 1198}}], "checks": [{"rule": "consent.marketing_gate", "passed": true, "detail": "1,198 of 1,284 VIP members opted in", "fix_tool": "get_consent"}, {"rule": "messaging.quiet_hours", "passed": true, "detail": "Sends at 10:00 store time", "fix_tool": "create_campaign"}, {"rule": "messaging.template.channel_match", "passed": true, "detail": "Email template on email", "fix_tool": "list_message_templates"}], "needs_approval": true, "approval_reason": "Schedules email to 1,198 customers"}`
- 출력(실행): `{"ok": true, "id": "cmp_77", "undo": {"by": "person", "target": "cs_1052", "until": "2026-11-07T09:59:00-05:00"}}` — 발송 전까지만 되돌릴 수 있습니다(되돌리면 `cancelled`, 예약 발송은 아무것도 하지 않음. 보낸 뒤에는 `not_revocable`).
- 예약 발송: `messaging.send_campaign` 작업이 `scheduled_time`에 그때의 동의·하루 상한으로 대상을 다시 세어 보냅니다(승인은 캠페인을 만들 때 받음). 보낼 사람이 없으면 캠페인은 `failed`(`error.code: no_consented_recipients`)가 됩니다.
- 대상: `tier`는 그 등급 회원, `customers`는 목록, `segment`는 promotion의 세그먼트(지금 조건에 맞는 회원, registry가 넘겨주는 reader로 읽음). 동의는 채널별 `marketing_<channel>`의 가장 최근 기록입니다.
- 정책: `consent.marketing_gate` ([customer.yml](../../policies/customer.yml)), `messaging.quiet_hours`, approval.yml `customer_messages`
- 에러: `no_consented_recipients` → `preview_segment` · `template_channel_mismatch` → `list_message_templates` · `quiet_hours`(발송 시각 조정) → `create_campaign` · `unknown_variable`(템플릿에 없는 변수) → `list_message_templates` · `customer_not_found`(목록의 없는 고객) → `search_members` · `not_found`(`detail.entity`: `template` → `list_message_templates`, `tier` → `list_tiers`, `segment` → `list_segments`)
- 멱등성: 같은 `preview_id` 재실행은 첫 결과. 서버는 `(name, channel, 대상, scheduled_time)`으로도 중복 예약을 막습니다.

### `search_campaigns` — Kind: R
- 논리주소/REST: `messaging.campaign.search` · `GET /v1/messaging/campaigns`
- 요약: 캠페인 검색(상태·채널·기간·템플릿).
- consumes → produces: → `campaign_id[]`
- 입력: `status`(`draft` \| `scheduled` \| `sending` \| `sent` \| `failed` \| `cancelled`), `channel`, `scheduled_from`, `scheduled_through`, `template_id`, `limit`(기본 20), `cursor`
- 출력: `{"items": [{"id": "cmp_77", "name": "VIP weekend reminder", "channel": "email", "template_id": "tpl_vip_weekend", "status": "scheduled", "scheduled_time": "2026-11-07T10:00:00-05:00", "date_sent": null, "recipient_count": 1198, "error_message": null}], "total": 14}` — 발송 시각이 늦은 것부터(시각 없는 초안이 맨 앞)
- 정책: 없음(읽기).
- 에러: `invalid_filter` → `search_campaigns`
- 멱등성: 읽기.

### `get_campaign` — Kind: R
- 논리주소/REST: `messaging.campaign.get` · `GET /v1/messaging/campaigns/{campaign_id}`
- 요약: 캠페인 한 건과 발송 집계(대상·발송·도착·열람·실패).
- consumes → produces: `campaign_id` → campaign
- 입력: `campaign_id`(✓), `fields?[]`
- 출력: `{"id": "cmp_77", "name": "VIP weekend reminder", "channel": "email", "template_id": "tpl_vip_weekend", "valid_for_member_tier": "vip", "audience": {"audience_mode": "tier", "valid_for_member_tier": "vip"}, "variables": [{"name": "headline_item", "value": "the wool overcoat"}], "status": "sent", "scheduled_time": "2026-11-07T10:00:00-05:00", "date_sent": "2026-11-07T10:00:00-05:00", "recipient_count": 1198, "error": null, "stats": {"sent": 1198, "delivered": null, "opened": 0, "failed": 0}, "delivery_note": "No email, SMS or push provider is connected: ..."}` — 전달 제공자가 없으므로 `delivered`는 `null`(알 수 없음). 메시지가 기록되기 전에 보낸 캠페인은 `sent`가 `recipient_count`이고 `opened`도 `null`. 발송이 실패한 캠페인은 `error`에 `{code, message}`
- 정책: 없음(읽기).
- 에러: `campaign_not_found` → `search_campaigns`
- 멱등성: 읽기.

---

## notification

### `list_notifications` — Kind: R
- 논리주소/REST: `messaging.notification.list` · `GET /v1/messaging/notifications?customer_id=cus_alex`
- 요약: 한 회원이 받은 알림(앱 안 알림·푸시·이메일 발송 기록). 쇼퍼 본인은 `customer_id=me`.
- consumes → produces: `customer_id` → `notification_id[]`
- 입력: `customer_id`(✓), `channel?`, `read?`(Boolean), `sent_from`, `sent_through`, `limit`(기본 20), `cursor`
- 출력: `{"items": [{"id": "ntf_901", "channel": "email", "headline": "Your refund is on its way", "text": "$312.00 back to card ending 0937.", "about": "ord_20402", "date_sent": "2026-11-06T11:25:00-05:00", "read": false}], "total": 7}`
- 정책: 없음(읽기).
- 에러: `customer_not_found` → `search_members`
- 멱등성: 읽기.
