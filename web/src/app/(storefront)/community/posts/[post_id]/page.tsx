import type {Metadata} from 'next';
import {BreadcrumbItem, Breadcrumbs} from '@astryxdesign/core/Breadcrumbs';
import {Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';

import {ErrorNotice} from '@/components/fo/ErrorNotice';
import {PostView, ReplyList} from '@/components/fo/StoreContent';
import type {Page} from '@/lib/api';

import {load} from '../../../_lib/server';
import type {Board, Post, Reply} from '../../../_lib/types';

type Params = Promise<{post_id: string}>;

export async function generateMetadata({params}: {params: Params}): Promise<Metadata> {
  const id = decodeURIComponent((await params).post_id);
  const post = await load<Post>(`/v1/community/posts/${encodeURIComponent(id)}`, {fields: ['id', 'headline']});
  return {title: post.ok ? post.data.headline : 'Post'};
}

// One post (get_post) and its replies (list_replies). A private post is the writer's and the
// store's only; the API says so and the page shows that sentence.
export default async function PostPage({params}: {params: Params}) {
  const id = decodeURIComponent((await params).post_id);
  const enc = encodeURIComponent(id);
  const [post, replies, boards] = await Promise.all([
    load<Post>(`/v1/community/posts/${enc}`),
    load<Page<Reply>>(`/v1/community/posts/${enc}/replies`, {limit: 50}),
    load<Page<Board>>('/v1/community/boards'),
  ]);
  const board = post.ok && boards.ok ? boards.data.items.find(b => b.id === post.data.board_id) : undefined;
  return (
    <Layout
      height="auto"
      contentWidth={960}
      content={
        <LayoutContent padding={6}>
          {!post.ok ? (
            <ErrorNotice error={post.error} title="This post is not available." />
          ) : (
            <VStack gap={5}>
              <Breadcrumbs variant="supporting">
                <BreadcrumbItem href="/community">Community</BreadcrumbItem>
                {board ? <BreadcrumbItem href={`/community/${encodeURIComponent(board.id)}`}>{board.name}</BreadcrumbItem> : null}
                <BreadcrumbItem isCurrent>{post.data.headline}</BreadcrumbItem>
              </Breadcrumbs>
              <PostView post={post.data} />
              <VStack gap={2}>
                <Heading level={2}>Replies</Heading>
                {!replies.ok ? (
                  <Text color="secondary">{replies.error.message}</Text>
                ) : replies.data.items.length === 0 ? (
                  <Text color="secondary">No replies yet.</Text>
                ) : (
                  <ReplyList replies={replies.data.items} />
                )}
              </VStack>
            </VStack>
          )}
        </LayoutContent>
      }
    />
  );
}
