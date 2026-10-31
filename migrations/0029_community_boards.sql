-- 0029_community_boards.sql
-- The community domain (docs/api/community.md): boards, the posts shoppers write on them, replies,
-- and reports that put a post in the moderation queue. Rules: policies/community.yml.
-- Posts and replies are never removed: deleting or blinding one is a moderation state, so its place
-- (and the replies of other people under it) stays.

CREATE SCHEMA IF NOT EXISTS community;

COMMENT ON SCHEMA community IS
  'Community boards, the posts and replies shoppers write on them, and reports for moderation.';

CREATE TABLE community.board (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  name text NOT NULL,
  board_type text NOT NULL,
  write_permission text NOT NULL,
  allow_secret boolean NOT NULL,
  allow_attachment boolean NOT NULL,
  moderation_mode text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT board_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT board_id_check CHECK (id ~ '^brd_[a-z0-9_]+$'),
  CONSTRAINT board_name_check CHECK (length(btrim(name)) > 0),
  CONSTRAINT board_board_type_check CHECK (board_type IN ('free', 'review', 'question', 'event')),
  CONSTRAINT board_write_permission_check CHECK (write_permission IN ('everyone', 'member', 'staff')),
  CONSTRAINT board_moderation_mode_check CHECK (moderation_mode IN ('pre_review', 'post_review', 'none'))
);

CREATE TABLE community.discussion_forum_posting (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  board_id text NOT NULL,
  author_id text NOT NULL,
  author_name text,
  headline text NOT NULL,
  article_body text NOT NULL,
  associated_media text[] NOT NULL DEFAULT '{}',
  is_secret boolean NOT NULL DEFAULT false,
  in_language text,
  moderation_status text NOT NULL DEFAULT 'published',
  moderation_reason text,
  like_count integer NOT NULL DEFAULT 0,
  date_published timestamptz NOT NULL,
  date_modified timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT discussion_forum_posting_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT discussion_forum_posting_id_check CHECK (id ~ '^post_[a-z0-9_]+$'),
  CONSTRAINT discussion_forum_posting_board_id_fkey FOREIGN KEY (tenant_id, board_id) REFERENCES community.board (tenant_id, id),
  CONSTRAINT discussion_forum_posting_author_id_check CHECK (author_id ~ '^(cus|adm|ses)_[a-z0-9_]+$'),
  CONSTRAINT discussion_forum_posting_moderation_status_check CHECK (moderation_status IN ('published', 'held_for_review', 'blinded', 'deleted')),
  CONSTRAINT discussion_forum_posting_like_count_check CHECK (like_count >= 0)
);

CREATE TABLE community.comment (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  discussion_forum_posting_id text NOT NULL,
  parent_item_id text,
  reply_depth integer NOT NULL,
  author_id text NOT NULL,
  author_name text,
  text text NOT NULL,
  is_secret boolean NOT NULL DEFAULT false,
  moderation_status text NOT NULL DEFAULT 'published',
  moderation_reason text,
  date_created timestamptz NOT NULL,
  date_modified timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT comment_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT comment_id_check CHECK (id ~ '^rpl_[a-z0-9_]+$'),
  CONSTRAINT comment_discussion_forum_posting_id_fkey FOREIGN KEY (tenant_id, discussion_forum_posting_id) REFERENCES community.discussion_forum_posting (tenant_id, id),
  CONSTRAINT comment_parent_item_id_fkey FOREIGN KEY (tenant_id, parent_item_id) REFERENCES community.comment (tenant_id, id),
  CONSTRAINT comment_reply_depth_check CHECK (reply_depth >= 1),
  CONSTRAINT comment_author_id_check CHECK (author_id ~ '^(cus|adm|ses)_[a-z0-9_]+$'),
  CONSTRAINT comment_moderation_status_check CHECK (moderation_status IN ('published', 'held_for_review', 'blinded', 'deleted'))
);

CREATE TABLE community.content_report (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  discussion_forum_posting_id text NOT NULL,
  customer_id text NOT NULL,
  report_reason text NOT NULL,
  description text,
  start_time timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version integer NOT NULL DEFAULT 1,
  CONSTRAINT content_report_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT content_report_id_check CHECK (id ~ '^rpt_[a-z0-9_]+$'),
  CONSTRAINT content_report_discussion_forum_posting_id_fkey FOREIGN KEY (tenant_id, discussion_forum_posting_id) REFERENCES community.discussion_forum_posting (tenant_id, id),
  CONSTRAINT content_report_customer_id_fkey FOREIGN KEY (tenant_id, customer_id) REFERENCES customer.person (tenant_id, id),
  CONSTRAINT content_report_discussion_forum_posting_id_customer_id_key UNIQUE (tenant_id, discussion_forum_posting_id, customer_id),
  CONSTRAINT content_report_report_reason_check CHECK (report_reason IN ('spam', 'abuse', 'obscene', 'advertisement', 'fake', 'irrelevant', 'other'))
);

CREATE INDEX discussion_forum_posting_board_id_idx ON community.discussion_forum_posting (tenant_id, board_id, date_published DESC);
CREATE INDEX discussion_forum_posting_author_id_idx ON community.discussion_forum_posting (tenant_id, author_id, date_published DESC);
CREATE INDEX comment_discussion_forum_posting_id_idx ON community.comment (tenant_id, discussion_forum_posting_id, date_created);
CREATE INDEX comment_author_id_idx ON community.comment (tenant_id, author_id, date_created DESC);
CREATE INDEX comment_parent_item_id_idx ON community.comment (tenant_id, parent_item_id);

COMMENT ON TABLE community.board IS
  'schema:CreativeWork/acom:Board — a community board shoppers write posts on, with who may write and how it is moderated. id prefix brd_.';
COMMENT ON COLUMN community.board.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN community.board.id IS
  '- id with prefix brd_; the same value the API, logs and events use (API board_id).';
COMMENT ON COLUMN community.board.name IS
  'schema:name — the board''s name shown to shoppers; one board per name in a store (community.yml board.name_unique).';
COMMENT ON COLUMN community.board.board_type IS
  'acom:boardType — what the board is for. values: board_type';
COMMENT ON COLUMN community.board.write_permission IS
  'acom:writePermission — who may write posts: everyone (guests too), member (signed-in members) or staff (the store). values: write_permission';
COMMENT ON COLUMN community.board.allow_secret IS
  'acom:allowSecret — true when a post may be secret (only its writer and the store read it).';
COMMENT ON COLUMN community.board.allow_attachment IS
  'acom:allowAttachment — true when posts may carry uploaded images (community.yml attachment).';
COMMENT ON COLUMN community.board.moderation_mode IS
  'acom:moderationMode — pre_review holds new posts until the store looks; post_review shows them at once and reports hold them; none keeps no queue. values: moderation_mode';
COMMENT ON COLUMN community.board.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN community.board.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN community.board.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE community.discussion_forum_posting IS
  'schema:DiscussionForumPosting — a post on a community board. id prefix post_. API: post.';
COMMENT ON COLUMN community.discussion_forum_posting.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN community.discussion_forum_posting.id IS
  '- id with prefix post_; the same value the API, logs and events use (API post_id).';
COMMENT ON COLUMN community.discussion_forum_posting.board_id IS
  'schema:isPartOf — the board the post is on (brd_ id).';
COMMENT ON COLUMN community.discussion_forum_posting.author_id IS
  'schema:author — who wrote it: a shopper (cus_), a guest''s browser session (ses_) or an operator of the store (adm_); only they may edit it.';
COMMENT ON COLUMN community.discussion_forum_posting.author_name IS
  'schema:author — the name shown with the post, masked as community.yml moderation.author_masking said when it was written (Ja***); NULL for the store and for guests.';
COMMENT ON COLUMN community.discussion_forum_posting.headline IS
  'schema:headline — the post''s title.';
COMMENT ON COLUMN community.discussion_forum_posting.article_body IS
  'schema:articleBody — the post''s text.';
COMMENT ON COLUMN community.discussion_forum_posting.associated_media IS
  'schema:associatedMedia — uploaded images attached to the post (upl_ ids); empty when none.';
COMMENT ON COLUMN community.discussion_forum_posting.is_secret IS
  'acom:isSecret — true when only the writer and the store may read it (board allow_secret).';
COMMENT ON COLUMN community.discussion_forum_posting.in_language IS
  'schema:inLanguage — the language it is written in (BCP 47, for example en); NULL when not given.';
COMMENT ON COLUMN community.discussion_forum_posting.moderation_status IS
  'acom:moderationStatus — published; held_for_review (board pre_review, a filter hold, or enough reports); blinded by the store; deleted by its writer or the store. values: moderation_status';
COMMENT ON COLUMN community.discussion_forum_posting.moderation_reason IS
  'acom:moderationReason — why the store blinded or deleted it, or why it is held (report_threshold, profanity_filter, spam, pre_review); NULL for a published post.';
COMMENT ON COLUMN community.discussion_forum_posting.like_count IS
  'acom:likeCount — how many shoppers liked the post; 0 or more.';
COMMENT ON COLUMN community.discussion_forum_posting.date_published IS
  'schema:datePublished — when it was written (store clock).';
COMMENT ON COLUMN community.discussion_forum_posting.date_modified IS
  'schema:dateModified — when its writer last changed it; NULL when never changed.';
COMMENT ON COLUMN community.discussion_forum_posting.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN community.discussion_forum_posting.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN community.discussion_forum_posting.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE community.comment IS
  'schema:Comment — a reply to a post, or to another reply up to community.yml reply.max_depth. id prefix rpl_. API: reply.';
COMMENT ON COLUMN community.comment.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN community.comment.id IS
  '- id with prefix rpl_; the same value the API, logs and events use (API reply_id).';
COMMENT ON COLUMN community.comment.discussion_forum_posting_id IS
  'schema:about — the post the reply belongs to (post_ id, API post_id).';
COMMENT ON COLUMN community.comment.parent_item_id IS
  'schema:parentItem — the reply this one answers (rpl_ id); NULL for a reply to the post itself.';
COMMENT ON COLUMN community.comment.reply_depth IS
  'acom:replyDepth — 1 for a reply to the post, 2 for a reply to a reply, and so on.';
COMMENT ON COLUMN community.comment.author_id IS
  'schema:author — who wrote it: a shopper (cus_), a guest''s browser session (ses_) or an operator of the store (adm_); only they may edit it.';
COMMENT ON COLUMN community.comment.author_name IS
  'schema:author — the name shown with the reply, masked as community.yml moderation.author_masking said when it was written; NULL for the store and for guests.';
COMMENT ON COLUMN community.comment.text IS
  'schema:text — the reply as its writer wrote it.';
COMMENT ON COLUMN community.comment.is_secret IS
  'acom:isSecret — true when only its writer, the post''s writer and the store may read it.';
COMMENT ON COLUMN community.comment.moderation_status IS
  'acom:moderationStatus — published, held_for_review, blinded by the store, or deleted by its writer. values: moderation_status';
COMMENT ON COLUMN community.comment.moderation_reason IS
  'acom:moderationReason — why the store blinded it, or why it is held; NULL for a published reply.';
COMMENT ON COLUMN community.comment.date_created IS
  'schema:dateCreated — when it was written (store clock).';
COMMENT ON COLUMN community.comment.date_modified IS
  'schema:dateModified — when its writer last changed it; NULL when never changed.';
COMMENT ON COLUMN community.comment.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN community.comment.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN community.comment.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

COMMENT ON TABLE community.content_report IS
  'schema:Action/acom:ContentReport — a shopper''s report that a post is inappropriate; enough reports from different shoppers hold or blind it (community.yml moderation.report_threshold). id prefix rpt_.';
COMMENT ON COLUMN community.content_report.tenant_id IS
  '- store (tenant) that owns this row; row-level security key.';
COMMENT ON COLUMN community.content_report.id IS
  '- id with prefix rpt_; the same value the API, logs and events use (API report_id).';
COMMENT ON COLUMN community.content_report.discussion_forum_posting_id IS
  'schema:object — the post reported (post_ id, API post_id).';
COMMENT ON COLUMN community.content_report.customer_id IS
  'schema:agent — the shopper who reported it (cus_ id); one report per shopper and post.';
COMMENT ON COLUMN community.content_report.report_reason IS
  'acom:reportReason — why it was reported (community.yml moderation.report_reasons). values: report_reason';
COMMENT ON COLUMN community.content_report.description IS
  'schema:description — the reporter''s own words; NULL when they gave none.';
COMMENT ON COLUMN community.content_report.start_time IS
  'schema:startTime — when the report was made (store clock).';
COMMENT ON COLUMN community.content_report.created_at IS
  '- when the row was created.';
COMMENT ON COLUMN community.content_report.updated_at IS
  '- when the row was last changed.';
COMMENT ON COLUMN community.content_report.version IS
  '- row version, +1 on every change; previews record it in basis and execution compares it.';

ALTER TABLE community.board ENABLE ROW LEVEL SECURITY;
ALTER TABLE community.board FORCE ROW LEVEL SECURITY;
CREATE POLICY board_tenant_isolation ON community.board
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE community.discussion_forum_posting ENABLE ROW LEVEL SECURITY;
ALTER TABLE community.discussion_forum_posting FORCE ROW LEVEL SECURITY;
CREATE POLICY discussion_forum_posting_tenant_isolation ON community.discussion_forum_posting
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE community.comment ENABLE ROW LEVEL SECURITY;
ALTER TABLE community.comment FORCE ROW LEVEL SECURITY;
CREATE POLICY comment_tenant_isolation ON community.comment
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

ALTER TABLE community.content_report ENABLE ROW LEVEL SECURITY;
ALTER TABLE community.content_report FORCE ROW LEVEL SECURITY;
CREATE POLICY content_report_tenant_isolation ON community.content_report
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- DELETE where removing a row is part of the design (see 0014_app_delete_grants.sql):
--   community.board                undoing configure_board's creation of a board that has no posts
-- Posts, replies and reports are kept: deleting one is a moderation state.
GRANT DELETE ON community.board TO acom_app;
