'use client';

// What the store publishes besides products: notices and FAQs (customer.md faq_notice), the
// community boards with their posts and replies (community.md), shoppers' questions on a product
// with the store's answers (catalog.md question), and collection pages (merchandising.md
// collection_page). Operators keep all of it in the back office; every row here is an AI target
// so a shopper can ask about it.

import {Button} from '@astryxdesign/core/Button';
import {Card} from '@astryxdesign/core/Card';
import {Collapsible, CollapsibleGroup} from '@astryxdesign/core/Collapsible';
import {Grid} from '@astryxdesign/core/Grid';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {List, ListItem} from '@astryxdesign/core/List';
import {Heading, Text} from '@astryxdesign/core/Text';
import {Token} from '@astryxdesign/core/Token';

import {useAiTarget} from '@/ai/useAiTarget';
import {dateTime, imageUrl, longDate, words} from '@/app/(storefront)/_lib/format';
import type {
  Board,
  CollectionPageSummary,
  Faq,
  NoticeSummary,
  Post,
  PostSummary,
  ProductQuestion,
  ProductSummary,
  Reply,
} from '@/app/(storefront)/_lib/types';

import {AskButton} from './Ask';
import {ProductImage} from './ProductImage';

function personName(author: string | {name?: string} | null | undefined, fallback: string): string {
  if (!author) return fallback;
  return typeof author === 'string' ? author : (author.name ?? fallback);
}

// ---------------------------------------------------------------- notices

function NoticeRow({notice}: {notice: NoticeSummary}) {
  const target = useAiTarget({kind: 'notice', label: notice.headline, ids: {notice_id: notice.id}, term: 'schema:NewsArticle'});
  const meta = [notice.pinned ? 'Pinned' : null, notice.category ? words(notice.category) : null, notice.date_published ? longDate(notice.date_published) : null]
    .filter(Boolean)
    .join(' · ');
  return <ListItem ref={target.ref} {...target.props} label={notice.headline} description={meta} href={`/help/notices/${encodeURIComponent(notice.id)}`} />;
}

export function NoticeList({notices}: {notices: NoticeSummary[]}) {
  return (
    <List hasDividers>
      {notices.map(n => (
        <NoticeRow key={n.id} notice={n} />
      ))}
    </List>
  );
}

// ---------------------------------------------------------------- FAQs

function FaqItem({faq}: {faq: Faq}) {
  const target = useAiTarget({kind: 'faq', label: faq.name, ids: {faq_id: faq.id}, term: 'schema:Question'});
  return (
    <Collapsible value={faq.id} trigger={<Text weight="semibold">{faq.name}</Text>}>
      <VStack gap={2} ref={target.ref} {...target.props}>
        <Text>{faq.accepted_answer ?? ''}</Text>
        <AskButton target={target} label={faq.name} />
      </VStack>
    </Collapsible>
  );
}

/** FAQs grouped under their section words (`returns`, `shipping`, …), in the store's order. */
export function FaqGroups({faqs}: {faqs: Faq[]}) {
  const groups = new Map<string, Faq[]>();
  for (const f of faqs) {
    const key = f.category ?? 'other';
    groups.set(key, [...(groups.get(key) ?? []), f]);
  }
  return (
    <VStack gap={6}>
      {[...groups.entries()].map(([category, items]) => (
        <VStack key={category} gap={2}>
          <Heading level={3}>{words(category)}</Heading>
          <CollapsibleGroup type="multiple" hasDividers>
            {items.map(f => (
              <FaqItem key={f.id} faq={f} />
            ))}
          </CollapsibleGroup>
        </VStack>
      ))}
    </VStack>
  );
}

// ---------------------------------------------------------------- community

const WHO_WRITES: Record<string, string> = {member: 'Members write here', staff: 'The store writes here', everyone: 'Anyone can write here'};

function BoardRow({board}: {board: Board}) {
  const target = useAiTarget({kind: 'board', label: board.name, ids: {board_id: board.id}, term: 'acom:Board'});
  const permission = board['acom:write_permission'];
  const meta = [board['acom:board_type'] ? words(board['acom:board_type']) : null, permission ? (WHO_WRITES[permission] ?? words(permission)) : null]
    .filter(Boolean)
    .join(' · ');
  return <ListItem ref={target.ref} {...target.props} label={board.name} description={meta} href={`/community/${encodeURIComponent(board.id)}`} />;
}

export function BoardList({boards}: {boards: Board[]}) {
  return (
    <List hasDividers>
      {boards.map(b => (
        <BoardRow key={b.id} board={b} />
      ))}
    </List>
  );
}

function PostRow({post}: {post: PostSummary}) {
  const target = useAiTarget({kind: 'post', label: post.headline, ids: {post_id: post.id}, term: 'schema:DiscussionForumPosting'});
  const replies = post.comment_count ?? 0;
  const likes = post['acom:like_count'] ?? 0;
  const meta = [
    post.author ?? null,
    post.date_published ? longDate(post.date_published) : null,
    replies === 1 ? '1 reply' : `${replies} replies`,
    likes > 0 ? (likes === 1 ? '1 like' : `${likes} likes`) : null,
    post['acom:is_secret'] ? 'Private' : null,
  ]
    .filter(Boolean)
    .join(' · ');
  return <ListItem ref={target.ref} {...target.props} label={post.headline} description={meta} href={`/community/posts/${encodeURIComponent(post.id)}`} />;
}

export function PostList({posts}: {posts: PostSummary[]}) {
  return (
    <List hasDividers>
      {posts.map(p => (
        <PostRow key={p.id} post={p} />
      ))}
    </List>
  );
}

export function PostView({post}: {post: Post}) {
  const target = useAiTarget({kind: 'post', label: post.headline, ids: {post_id: post.id, board_id: post.board_id}, term: 'schema:DiscussionForumPosting'});
  const status = post['acom:moderation_status'];
  return (
    <VStack gap={3} ref={target.ref} {...target.props}>
      <Heading level={1}>{post.headline}</Heading>
      <Text type="supporting" color="secondary">
        {[post.author ?? null, post.date_published ? dateTime(post.date_published) : null].filter(Boolean).join(' · ')}
      </Text>
      {status && status !== 'published' ? <Token label={words(status)} /> : null}
      {post.article_body ? <Text>{post.article_body}</Text> : null}
      <AskButton target={target} label={post.headline} />
    </VStack>
  );
}

function ReplyRow({reply}: {reply: Reply}) {
  const author = reply.author ?? 'A shopper';
  const target = useAiTarget({kind: 'reply', label: `Reply by ${author}`, ids: {reply_id: reply.id}, term: 'schema:Comment'});
  const when = reply.date_created ?? reply.date_published;
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      label={reply.text ?? ''}
      description={[author, when ? dateTime(when) : null].filter(Boolean).join(' · ')}
    />
  );
}

export function ReplyList({replies}: {replies: Reply[]}) {
  return (
    <List hasDividers>
      {replies.map(r => (
        <ReplyRow key={r.id} reply={r} />
      ))}
    </List>
  );
}

// ---------------------------------------------------------------- product questions

function QuestionRow({question}: {question: ProductQuestion}) {
  const asker = personName(question.author, 'A shopper');
  const target = useAiTarget({kind: 'question', label: question.text, ids: {question_id: question.id, product_id: question.product_id}, term: 'schema:Question'});
  const answer = question.accepted_answer;
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      label={question.text}
      description={
        <VStack gap={1}>
          <Text type="supporting" color="secondary">
            {[asker, question.date_created ? longDate(question.date_created) : null].filter(Boolean).join(' · ')}
          </Text>
          {answer ? (
            <Text>{`${personName(answer.author, 'The store')}: ${answer.text}`}</Text>
          ) : (
            <Text color="secondary">Not answered yet.</Text>
          )}
        </VStack>
      }
    />
  );
}

export function QuestionList({questions}: {questions: ProductQuestion[]}) {
  return (
    <List hasDividers density="spacious">
      {questions.map(q => (
        <QuestionRow key={q.id} question={q} />
      ))}
    </List>
  );
}

// ---------------------------------------------------------------- collection pages

function CollectionCard({page, products}: {page: CollectionPageSummary; products: ProductSummary[]}) {
  const target = useAiTarget({kind: 'collection_page', label: page.name, ids: {collection_page_id: page.id}, term: 'schema:CollectionPage'});
  const count = page.product_count ?? products.length;
  const href = `/collections/${encodeURIComponent(page.id)}`;
  return (
    <Card padding={4} ref={target.ref} {...target.props}>
      <VStack gap={3}>
        {products.length > 0 ? (
          // The collection's first products, small; as many as it has, up to four.
          <Grid columns={4} gap={2}>
            {products.map(p => (
              <ProductImage key={p.id} src={imageUrl(p.image)} alt={p.name} ratio={1} href={`/p/${encodeURIComponent(p.id)}`} />
            ))}
          </Grid>
        ) : null}
        <VStack gap={1} hAlign="start">
          <Heading level={3}>{page.name}</Heading>
          <Text color="secondary">
            {[count === 1 ? '1 product' : `${count} products`, page.valid_through ? `until ${longDate(page.valid_through)}` : null].filter(Boolean).join(' · ')}
          </Text>
        </VStack>
        {page.keywords && page.keywords.length > 0 ? (
          <HStack gap={1} wrap="wrap">
            {page.keywords.map(k => (
              <Token key={k} label={k} />
            ))}
          </HStack>
        ) : null}
        <HStack gap={2} vAlign="center">
          <Button label="See the collection" variant="secondary" size="sm" href={href} />
          <AskButton target={target} label={page.name} />
        </HStack>
      </VStack>
    </Card>
  );
}

export function CollectionCards({collections}: {collections: {page: CollectionPageSummary; products: ProductSummary[]}[]}) {
  return (
    <Grid columns={{minWidth: 280}} gap={4}>
      {collections.map(c => (
        <CollectionCard key={c.page.id} page={c.page} products={c.products} />
      ))}
    </Grid>
  );
}
