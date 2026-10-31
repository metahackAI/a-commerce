# API 명세 — `community` (게시판·글·댓글)

> 계층: `community.{resource}.{operation}`. 공통 규약(응답 형식·프리뷰/실행·멱등성·헤더)은 [README.md](README.md) §3과 [conventions.md](conventions.md), 에러는 [errors.md](errors.md), 값 목록은 [enums.md](enums.md), 데이터 타입은 [data-model.md](data-model.md), 정책은 [community.yml](../../policies/community.yml).
> **설계 축**: ① 고객이 쓰는 콘텐츠(운영 공지는 `customer.faq_notice`가 담당), ② 모더레이션(신고 → 블라인드 → 삭제, 금칙어·스팸), ③ 비밀글은 작성자와 운영자만 열람, ④ 공개 목록의 작성자 이름은 마스킹(`ho****`, `moderation.author_masking`), ⑤ 글은 schema.org `DiscussionForumPosting`, 댓글은 `Comment`, 게시판은 `acom:Board`(subtype of `CreativeWork`).
> **쓰기**: 쇼퍼가 자기 글·댓글을 쓰고 고치는 것은 승인 없이 프리뷰 후 바로 실행되고 변경셋 밖이므로 `undo: null`입니다(지우기는 `delete_post`·`delete_reply`). 운영자의 블라인드·삭제·게시판 설정은 변경셋에 들어가 `undo`가 변경셋을 가리킵니다.

리소스: [post](#post) · [reply](#reply) · [board](#board)

---

## post

### `list_posts` — Kind: R
- 논리 주소 · REST: `community.post.list_posts` · `POST /v1/community/posts/search` (조건이 많아 POST지만 부작용 없는 읽기)
- 요약: 게시판의 글 목록. 기본은 공개 글만, 비밀글은 작성자 본인이나 운영자 토큰일 때만 포함. 작성자 본인에게는 검수 대기 글도 보이고, 다른 사람의 댓글이 달린 삭제 글은 본문 없이 자리만 남음. 작성자 마스킹 적용. 목록과 개수를 한 번에 반환. 항목에는 `acom:moderation_status`가 있고, 운영자에게는 `acom:report_count`도 있음.
- consumes → produces: `board_id` → `post_id[]`
- 입력: `board_id`(✓, 예 `"brd_reviews"`), `author_id?`(`cus_…`), `keyword?`, `has_reply?`(Boolean), `reported?`(Boolean, 운영자만), `date_published_from?`, `date_published_to?`(DateTime), `scope`(`all` \| `mine`, 기본 `all`), `sort`(`newest` \| `popular`, 기본 `newest`), `limit`(기본 20, 최대 100), `cursor?`
- 출력: `{"items": [{"id": "post_8812", "headline": "Runs a bit small", "author": "ho****", "date_published": "2026-11-05T19:12:00-05:00", "comment_count": 3, "acom:like_count": 12, "acom:is_secret": false}], "total": 41, "next_cursor": "c_2"}`
- 정책: `moderation.author_masking`
- 에러: `board_not_found` → `list_boards` · `invalid_filter` → `list_posts`
- 멱등성: 읽기(부작용 없음).

### `get_post` — Kind: R
- 논리 주소 · REST: `community.post.get_post` · `GET /v1/community/posts/{post_id}`
- 요약: 글 하나의 본문·첨부·댓글 수. 비밀글은 작성자 본인과 운영자만 볼 수 있음. 블라인드·삭제된 글은 상태와 자리만 보이고 본문은 운영자만. 읽기는 부작용이 없으므로 조회수는 세지 않음.
- consumes → produces: `post_id` → post
- 입력: `post_id`(✓, 예 `"post_8812"`), `fields?[]`
- 출력(목록 봉투 없이 엔티티): `{"id": "post_8812", "board_id": "brd_reviews", "headline": "Runs a bit small", "article_body": "…", "associated_media": [], "author": "ho****", "date_published": "2026-11-05T19:12:00-05:00", "comment_count": 3, "acom:is_secret": false, "acom:moderation_status": "published", "acom:report_count": 0}` — `acom:moderation_status`: `published` \| `held_for_review` \| `blinded` \| `deleted`
- 정책: `moderation.author_masking`
- 에러: `secret_post`(비밀글, 작성자·운영자만) → `list_posts` · `post_not_found` → `list_posts`
- 멱등성: 읽기.

### `create_post` — Kind: W
- 논리 주소 · REST: `community.post.create_post` · `POST /v1/community/boards/{board_id}/posts`
- 요약: 고객이 글을 씀. 금칙어·스팸·첨부 규칙을 검사. 비밀글 지정 가능. 게시판이 사전 검수(`pre_review`)면 실행 후 `held_for_review` 상태가 됨.
- consumes → produces: `board_id` → `post_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `board_id` | Text | ✓ | 게시판. 예 `"brd_reviews"` |
  | `headline` | Text | ✓ | 제목. 예 `"Runs a bit small"` |
  | `article_body` | Text | ✓ | 본문 |
  | `associated_media` | Text[] | | 첨부 업로드 id(`upl_…`). 게시판이 허용할 때만(`attachment`) |
  | `acom:is_secret` | Boolean | | 비밀글 여부(기본 `false`, 게시판의 `allow_secret`일 때만) |
  | `in_language` | Text | | 로케일. 예 `"en"` |
- 출력(프리뷰): `{"preview_id": "pv_k01a", "expires_at": "2026-11-07T10:30:00-05:00", "changes": [{"op": "create", "entity": "post", "id": null, "from": null, "to": "published"}], "checks": [{"rule": "moderation.profanity_filter", "passed": true, "detail": "No blocked words", "fix_tool": null}, {"rule": "spam", "passed": true, "detail": "1 link, 2 posts this hour", "fix_tool": null}, {"rule": "attachment", "passed": true, "detail": "No attachments", "fix_tool": null}, {"rule": "board.write_permission", "passed": true, "detail": "Members can write here", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "post_8812", "undo": null}`
- 정책: `moderation`, `moderation.profanity_filter`, `spam`, `board.write_permission`, `attachment`
- 에러: `profanity_blocked`(금칙어, 본문을 고쳐 다시) → `create_post` · `write_not_allowed` → `list_boards` · `attachment_not_allowed`(첨부를 빼고 다시) → `create_post` · `board_not_found` → `list_boards`. 스팸 차단(`spam.on_match: block`)은 공통 에러로 돌려줍니다: 링크가 많으면 `invalid_input`(`detail.rule: spam`) → `create_post`, 시간당 글·댓글이 많으면 `rate_limited`(`retry_after_seconds`)
- 멱등성: `Idempotency-Key`. 같은 작성자·게시판·제목이 `spam.duplicate_window_minutes`(5분) 안에 다시 오면 첫 결과 반환.

### `update_post` — Kind: W
- 논리 주소 · REST: `community.post.update_post` · `PATCH /v1/community/posts/{post_id}`
- 요약: 작성자 본인이 제목·본문·비밀 여부를 수정. 모더레이션을 다시 검사.
- consumes → produces: `post_id` → `post_id`
- 입력: `post_id`(✓), `headline?`, `article_body?`, `acom:is_secret?`
- 출력(프리뷰): `{"preview_id": "pv_k02b", "expires_at": "2026-11-07T10:31:00-05:00", "changes": [{"op": "update", "entity": "post", "id": "post_8812", "from": "Runs small", "to": "Runs a bit small"}], "checks": [{"rule": "moderation.profanity_filter", "passed": true, "detail": "No blocked words", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "post_8812", "undo": null}`
- 정책: `moderation.profanity_filter`, `spam`
- 에러: `not_author` → `get_post` · `profanity_blocked` → `update_post` · `post_not_found` → `list_posts` · `post_locked`(블라인드·삭제된 글) → `get_post`
- 멱등성: `Idempotency-Key`. 같은 값으로 다시 보내면 같은 결과.

### `delete_post` — Kind: W
- 논리 주소 · REST: `community.post.delete_post` · `DELETE /v1/community/posts/{post_id}`
- 요약: 작성자는 소프트 삭제(`deleted`), 운영자는 블라인드(`blinded`) 또는 삭제. 다른 사람의 댓글이 있으면 내용만 가리고 글 자리는 남깁니다(`moderation.blind.keep_replies_on_delete`).
- consumes → produces: `post_id` → `post_id`
- 입력: `post_id`(✓), `action`(`delete` \| `blind`, 기본 `delete`. `blind`는 운영자만), `reason?`(운영자일 때 필수)
- 출력(프리뷰): `{"preview_id": "pv_k03c", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "update", "entity": "post", "id": "post_8812", "from": "published", "to": "blinded"}], "checks": [{"rule": "moderation.blind", "passed": true, "detail": "Reason recorded: reported 5 times for spam", "fix_tool": null}], "needs_approval": false, "approval_reason": null, "change_set_id": "cs_1045"}`
- 출력(실행): 작성자 `{"ok": true, "id": "post_8812", "undo": null}` · 운영자 `{"ok": true, "id": "post_8812", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}`. 결과 상태는 `get_post`의 `acom:moderation_status`.
- 정책: `moderation.blind`
- 에러: `not_author` → `get_post` · `reason_required`(운영자 블라인드·삭제) → `delete_post` · `post_not_found` → `list_posts`
- 멱등성: 이미 같은 상태면 성공으로 수렴.

### `report_post` — Kind: W
- 논리 주소 · REST: `community.post.report_post` · `POST /v1/community/posts/{post_id}/report`
- 요약: 글 신고. 서로 다른 신고자 수가 임계치(`moderation.report_threshold`)에 이르면 `moderation.blind.on_threshold`대로 모더레이션 대기열에 들어가거나 바로 블라인드됩니다.
- consumes → produces: `post_id` → `report_id`
- 입력: `post_id`(✓), `acom:report_reason`(✓, `spam` \| `abuse` \| `obscene` \| `advertisement` \| `fake` \| `irrelevant` \| `other`), `text?`
- 출력(프리뷰): `{"preview_id": "pv_k04d", "expires_at": "2026-11-07T10:32:00-05:00", "changes": [{"op": "create", "entity": "report", "id": null, "from": null, "to": "spam"}], "checks": [{"rule": "moderation.report_threshold", "passed": true, "detail": "4 of 5 reports; one more holds it for review", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "rpt_301", "undo": null}`. 누적 신고 수와 블라인드 여부는 `get_post`로 확인.
- 정책: `moderation.report_reasons`, `moderation.report_threshold`, `moderation.blind`
- 에러: `post_not_found` → `list_posts` · `invalid_input`(사유가 목록 밖) → `report_post`
- 멱등성: `(post_id, 신고자)` 유일. 같은 신고자의 재신고는 첫 결과를 돌려줍니다(`already_reported`는 에러가 아님).

---

## reply

### `list_replies` — Kind: R
- 논리 주소 · REST: `community.reply.list_replies` · `GET /v1/community/posts/{post_id}/replies`
- 요약: 글의 댓글 목록(계층 구조, 작성자 마스킹, 블라인드 제외). 비밀 댓글은 당사자와 운영자만.
- consumes → produces: `post_id` → `reply_id[]`
- 입력: `post_id`(✓), `limit`(기본 20, 최대 100), `cursor?`
- 출력: `{"items": [{"id": "rpl_5521", "text": "Same here, size up.", "author": "ja****", "date_created": "2026-11-05T20:01:00-05:00", "parent_item": null, "acom:is_secret": false}], "total": 3}`
- 정책: `moderation.author_masking`
- 에러: `post_not_found` → `list_posts`
- 멱등성: 읽기.

### `create_reply` — Kind: W
- 논리 주소 · REST: `community.reply.create_reply` · `POST /v1/community/posts/{post_id}/replies`
- 요약: 댓글 작성. `parent_item`을 주면 대댓글(깊이는 `reply.max_depth`까지). 금칙어·스팸 검사. 회원과 운영자는 어느 게시판에나, 게스트는 `everyone` 게시판에만 댓글을 씀(아니면 `forbidden`).
- consumes → produces: `post_id` → `reply_id`
- 입력: `post_id`(✓), `text`(✓), `parent_item?`(상위 댓글 id `rpl_…`), `acom:is_secret?`
- 출력(프리뷰): `{"preview_id": "pv_k05e", "expires_at": "2026-11-07T10:33:00-05:00", "changes": [{"op": "create", "entity": "reply", "id": null, "from": null, "to": "published"}], "checks": [{"rule": "moderation.profanity_filter", "passed": true, "detail": "No blocked words", "fix_tool": null}, {"rule": "spam", "passed": true, "detail": "No links", "fix_tool": null}, {"rule": "reply.max_depth", "passed": true, "detail": "Depth 1 of 2", "fix_tool": null}], "needs_approval": false, "approval_reason": null}`
- 출력(실행): `{"ok": true, "id": "rpl_5521", "undo": null}`
- 정책: `moderation.profanity_filter`, `spam`, `reply.max_depth`
- 에러: `profanity_blocked` → `create_reply` · `post_locked`(블라인드·삭제된 글이나 댓글) → `get_post` · `depth_exceeded` → `create_reply` · `post_not_found` → `list_posts` · `not_found`(상위 댓글, `detail.entity: reply`) → `list_replies`
- 멱등성: `Idempotency-Key`. 같은 작성자·글·본문이 `spam.duplicate_window_minutes` 안에 다시 오면 첫 결과 반환.

### `update_reply` — Kind: W
- 논리 주소 · REST: `community.reply.update_reply` · `PATCH /v1/community/replies/{reply_id}`
- 요약: 작성자 본인이 댓글 수정. 모더레이션을 다시 검사.
- consumes → produces: `reply_id` → `reply_id`
- 입력: `reply_id`(✓), `text?`, `acom:is_secret?`
- 출력(프리뷰): `{"preview_id": "pv_k06f", "expires_at": "2026-11-07T10:34:00-05:00", "changes": [{"op": "update", "entity": "reply", "id": "rpl_5521", "from": "Same here.", "to": "Same here, size up."}], "checks": [{"rule": "moderation.profanity_filter", "passed": true, "detail": "No blocked words", "fix_tool": null}], "needs_approval": false, "approval_reason": null}` · 실행 `{"ok": true, "id": "rpl_5521", "undo": null}`
- 정책: `moderation.profanity_filter`
- 에러: `not_author` → `list_replies` · `profanity_blocked` → `update_reply` · `post_locked`(블라인드·삭제된 댓글이나 글) → `get_post` · `not_found`(`detail.entity: reply`) → `list_replies`
- 멱등성: `Idempotency-Key`.

### `delete_reply` — Kind: W
- 논리 주소 · REST: `community.reply.delete_reply` · `DELETE /v1/community/replies/{reply_id}`
- 요약: 작성자는 소프트 삭제, 운영자는 블라인드(사유 필수).
- consumes → produces: `reply_id` → `reply_id`
- 입력: `reply_id`(✓), `reason?`(운영자일 때 필수)
- 출력: 프리뷰 `changes` `[{"op": "update", "entity": "reply", "id": "rpl_5521", "from": "published", "to": "deleted"}]` · 실행 `{"ok": true, "id": "rpl_5521", "undo": null}`(운영자 블라인드면 `{"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}`)
- 정책: `moderation.blind`
- 에러: `not_author` → `list_replies` · `reason_required` → `delete_reply` · `not_found`(`detail.entity: reply`) → `list_replies`
- 멱등성: 이미 지운 댓글은 성공으로 수렴.

---

## board

### `list_boards` — Kind: R
- 논리 주소 · REST: `community.board.list_boards` · `GET /v1/community/boards`
- 요약: 게시판 목록(유형·쓰기 권한·허용 옵션).
- consumes → produces: → `board_id[]`
- 입력: `acom:board_type?`(`free` \| `review` \| `question` \| `event`), `limit`(기본 20, 최대 100), `cursor?`
- 출력: `{"items": [{"id": "brd_reviews", "name": "Product reviews", "acom:board_type": "review", "acom:write_permission": "member", "acom:allow_secret": false, "acom:allow_attachment": true, "acom:moderation_mode": "post_review"}], "total": 4}`
- 정책: `board.types`
- 에러: `invalid_filter` → `list_boards`
- 멱등성: 읽기.

### `configure_board` — Kind: W
- 논리 주소 · REST: `community.board.configure_board` · `POST /v1/community/boards`(생성), `PATCH /v1/community/boards/{board_id}`(수정)
- 요약: 게시판 생성과 설정(유형·쓰기 권한·비밀글·첨부·모더레이션 방식). 운영자 전용. 오퍼레이션 하나는 경로 하나로 등록되므로 수정도 `POST /v1/community/boards`에 `board_id`를 담아 보냅니다(`PATCH /v1/community/boards/{board_id}`는 따로 열리지 않음).
- consumes → produces: → `board_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `board_id` | Text | | 수정할 때. 예 `"brd_reviews"` |
  | `name` | Text | ✓ | `"Product reviews"` |
  | `acom:board_type` | enum | ✓ | `free` \| `review` \| `question` \| `event` |
  | `acom:write_permission` | enum | | `everyone` \| `member` \| `staff`. 기본 `board.write_permission.default` |
  | `acom:allow_secret` | Boolean | | 비밀글 허용. 기본 `board.allow_secret` |
  | `acom:allow_attachment` | Boolean | | 첨부 허용. 기본 `attachment.allowed_by_default` |
  | `acom:moderation_mode` | enum | | `pre_review` \| `post_review` \| `none`. 기본 `moderation.default_mode` |
- 출력(프리뷰): `{"preview_id": "pv_k07a", "expires_at": "2026-11-06T11:30:00-05:00", "changes": [{"op": "create", "entity": "board", "id": null, "from": null, "to": "Product reviews"}], "checks": [{"rule": "board.name_unique", "passed": true, "detail": "No other board uses this name", "fix_tool": "list_boards"}], "needs_approval": false, "approval_reason": null, "change_set_id": "cs_1045"}`
- 출력(실행): `{"ok": true, "id": "brd_reviews", "undo": {"by": "person", "target": "cs_1045", "until": "2026-11-07T23:59:00-05:00"}}`
- 정책: `board`, `moderation.default_mode`, `attachment`
- 에러: `duplicate_board_name` → `list_boards` · `board_not_found` → `list_boards`
- 멱등성: `Idempotency-Key`
