# API 명세 — `community` (게시판·글·댓글)

> 계층: `community.{resource}.{operation}`. 공통 규약(응답 형식·에러·프리뷰/커밋·멱등성·헤더)은 [README.md](README.md) §3, 데이터 타입은 [data-model.md](data-model.md).
> **설계 축**: ① 고객이 쓰는 콘텐츠(운영 공지는 `customer.faq_notice`가 담당), ② 모더레이션(신고 → 블라인드 → 삭제, 금칙어·스팸), ③ 비밀글은 작성자와 운영자만 열람, ④ 공개 목록의 작성자 이름은 마스킹(`ho****`), ⑤ 글은 schema.org `DiscussionForumPosting`, 댓글은 `Comment`, 게시판은 `acom:Board`(subtype of `CreativeWork`).

리소스: [post](#post) · [reply](#reply) · [board](#board)

---

## post

### `list_posts` — Kind: R
- 논리 주소 · REST: `community.post.list_posts` · `POST /v1/community/posts/search`
- 요약: 게시판의 글 목록. 기본은 공개 글만, 비밀글은 작성자 본인이나 운영자 토큰일 때만 포함. 작성자 마스킹 적용. 목록과 개수를 한 번에 반환.
- consumes → produces: `board_id` → `post_id[]`
- 입력: `board_id`(✓), `author?`, `keyword?`, `has_reply?`, `reported?`, `date_published_from?`, `date_published_to?`, `scope`(`all` \| `mine`), `sort`(`newest` \| `popular`), `limit=20`, `offset=0`
- 출력: `{"items": [{"identifier", "headline", "author"(마스킹), "date_published", "comment_count", "acom:like_count", "acom:is_secret"}], "total"}`
- 에러: `board_not_found` → `list_boards`

### `get_post` — Kind: R
- 논리 주소 · REST: `community.post.get_post` · `GET /v1/community/posts/{post_id}`
- 요약: 글 하나의 본문·첨부·댓글 수. 비밀글은 작성자 본인과 운영자만 볼 수 있음. 조회수 증가는 서버의 부수 효과이며 별도 툴이 아님.
- consumes → produces: `post_id` → post
- 입력: `post_id`(✓), `fields?[]`
- 출력: `DiscussionForumPosting {identifier, headline, article_body, associated_media[], author, date_published, comment_count, acom:is_secret, acom:moderation_status}` — `acom:moderation_status`: `published` \| `held_for_review` \| `blinded` \| `deleted`
- 에러: `secret_post` → `list_posts` · `post_not_found` → `list_posts`

### `create_post` — Kind: W
- 논리 주소 · REST: `community.post.create_post` · `POST /v1/community/boards/{board_id}/posts`
- 요약: 고객이 글을 씀. 금칙어·스팸·첨부 규칙을 검사. 비밀글 지정 가능. 게시판이 사전 검수(`pre_review`)면 커밋 후 `held_for_review` 상태가 됨.
- consumes → produces: `board_id` → `post_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `board_id` | Text | ✓ | 게시판 |
  | `headline` | Text | ✓ | 제목 |
  | `article_body` | Text | ✓ | 본문 |
  | `associated_media` | MediaObject[] | | 첨부(게시판이 허용할 때) |
  | `acom:is_secret` | Boolean | | 비밀글 여부(기본 `false`) |
  | `in_language` | Text | | 로케일 |
  | `preview` | Boolean | ✓ | `true` 먼저 |
- 출력(프리뷰): `{"preview_id", "changes": [{"op": "create", "entity": "post"}], "checks": [{"rule": "banned_words", ...}, {"rule": "spam", ...}, {"rule": "attachment_allowed", ...}], "needs_approval": false, "approval_reason": null}`
- 출력(커밋): `{"ok": true, "id": "post_8812", "undo": null}`
- 정책 키: `community.moderation`, `community.spam`, `community.board.write_permission`, `community.attachment`
- 에러: `banned_words` → `update_post` · `write_not_allowed` → `list_boards` · `attachment_not_allowed` → `create_post`(첨부 제거)
- 멱등성: `Idempotency-Key`. 같은 작성자·게시판·제목이 5분 안에 다시 오면 첫 결과 반환.

### `update_post` — Kind: W
- 논리 주소 · REST: `community.post.update_post` · `PATCH /v1/community/posts/{post_id}`
- 요약: 작성자 본인이 제목·본문·비밀 여부를 수정. 모더레이션을 다시 검사.
- consumes → produces: `post_id` → `post_id`
- 입력: `post_id`(✓), `headline?`, `article_body?`, `acom:is_secret?`, `preview`(✓)
- 출력: 프리뷰 → 커밋 `{"ok": true, "id": "post_8812", "undo": null}`
- 에러: `not_author` → `get_post` · `banned_words` → `update_post`

### `delete_post` — Kind: W
- 논리 주소 · REST: `community.post.delete_post` · `DELETE /v1/community/posts/{post_id}`
- 요약: 작성자는 소프트 삭제, 운영자는 블라인드 또는 삭제. 댓글도 함께 처리.
- consumes → produces: `post_id` → `post_id`
- 입력: `post_id`(✓), `reason?`(운영자일 때 필수), `preview`(✓)
- 출력: 커밋 `{"ok": true, "id": "post_8812", "undo": null}`. 결과 상태(`deleted` \| `blinded`)는 `get_post`의 `acom:moderation_status`.
- 정책 키: `community.moderation.blind`
- 에러: `not_author` → `get_post` · `has_protected_replies` → `list_replies`

### `report_post` — Kind: W
- 논리 주소 · REST: `community.post.report_post` · `POST /v1/community/posts/{post_id}/report`
- 요약: 글 신고. 신고가 임계치에 이르면 자동 블라인드 후보로 모더레이션 대기열에 들어감.
- consumes → produces: `post_id` → `report_id`
- 입력: `post_id`(✓), `acom:report_reason`(✓, `spam` \| `abuse` \| `obscene` \| `advertisement` \| `other`), `text?`
- 출력: 커밋 `{"ok": true, "id": "rpt_301", "undo": null}`. 누적 신고 수와 자동 블라인드 여부는 `get_post`로 확인.
- 정책 키: `community.moderation.report_threshold`
- 에러: `already_reported` → 없음(같은 신고자의 재신고는 멱등 성공)
- 멱등성: `(post_id, 신고자)` 유일.

---

## reply

### `list_replies` — Kind: R
- 논리 주소 · REST: `community.reply.list_replies` · `GET /v1/community/posts/{post_id}/replies`
- 요약: 글의 댓글 목록(계층 구조, 작성자 마스킹, 블라인드 제외). 비밀 댓글은 당사자와 운영자만.
- consumes → produces: `post_id` → `reply_id[]`
- 입력: `post_id`(✓), `limit=50`, `offset=0`
- 출력: `{"items": [Comment {identifier, text, author, date_created, parent_item, acom:is_secret}], "total"}`
- 에러: `post_not_found` → `list_posts`

### `create_reply` — Kind: W
- 논리 주소 · REST: `community.reply.create_reply` · `POST /v1/community/posts/{post_id}/replies`
- 요약: 댓글 작성. `parent_item`을 주면 대댓글. 금칙어·스팸 검사.
- consumes → produces: `post_id` → `reply_id`
- 입력: `post_id`(✓), `text`(✓), `parent_item?`(상위 댓글 id), `acom:is_secret?`, `preview`(✓)
- 출력(커밋): `{"ok": true, "id": "rpl_5521", "undo": null}`
- 정책 키: `community.moderation`, `community.spam`
- 에러: `banned_words` → `update_reply` · `post_locked` → `get_post`
- 멱등성: `Idempotency-Key`. 같은 작성자·글·본문이 5분 안에 다시 오면 첫 결과 반환.

### `update_reply` — Kind: W
- 논리 주소 · REST: `community.reply.update_reply` · `PATCH /v1/community/replies/{reply_id}`
- 요약: 작성자 본인이 댓글 수정. 입력: `reply_id`(✓), `text?`, `acom:is_secret?`, `preview`(✓).
- 에러: `not_author` → `list_replies`

### `delete_reply` — Kind: W
- 논리 주소 · REST: `community.reply.delete_reply` · `DELETE /v1/community/replies/{reply_id}`
- 요약: 작성자는 소프트 삭제, 운영자는 블라인드. 커밋 `{"ok": true, "id": "rpl_5521", "undo": null}`.
- 에러: `not_author` → `list_replies`

---

## board

### `list_boards` — Kind: R
- 논리 주소 · REST: `community.board.list_boards` · `GET /v1/community/boards`
- 요약: 게시판 목록(유형·쓰기 권한·허용 옵션).
- consumes → produces: → `board_id[]`
- 출력: `{"items": [acom:Board {identifier, name, acom:board_type, acom:write_permission, acom:allow_secret, acom:allow_attachment, acom:moderation_mode}], "total"}`
- `acom:board_type`: `free` \| `review` \| `question` \| `event`

### `configure_board` — Kind: W
- 논리 주소 · REST: `community.board.configure_board` · `POST /v1/community/boards`(생성), `PATCH /v1/community/boards/{board_id}`(수정)
- 요약: 게시판 생성과 설정(유형·쓰기 권한·비밀글·첨부·모더레이션 방식). 운영자 전용.
- consumes → produces: → `board_id`
- 입력:
  | 필드 | 타입 | 필수 | 설명 · 예 |
  |---|---|:--:|---|
  | `board_id` | Text | | 수정할 때 |
  | `name` | Text | ✓ | "Product reviews" |
  | `acom:board_type` | enum | ✓ | `free` \| `review` \| `question` \| `event` |
  | `acom:write_permission` | enum | ✓ | `everyone` \| `member` \| `staff` |
  | `acom:allow_secret` | Boolean | | 비밀글 허용 |
  | `acom:allow_attachment` | Boolean | | 첨부 허용 |
  | `acom:moderation_mode` | enum | | `pre_review` \| `post_review` \| `none` |
  | `preview` | Boolean | ✓ | `true` 먼저 |
- 출력(커밋): `{"ok": true, "id": "brd_reviews", "undo": null}`
- 정책 키: `community.board`, `community.moderation`
- 에러: `duplicate_board_name` → `list_boards`
- 멱등성: `Idempotency-Key`
