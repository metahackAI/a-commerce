import type {Metadata} from 'next';
import {BreadcrumbItem, Breadcrumbs} from '@astryxdesign/core/Breadcrumbs';
import {Button} from '@astryxdesign/core/Button';
import {HStack, Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';

import {ErrorNotice} from '@/components/fo/ErrorNotice';
import {PostList} from '@/components/fo/StoreContent';
import type {Page} from '@/lib/api';

import {load, loadBody} from '../../_lib/server';
import type {Board, PostSummary} from '../../_lib/types';

type Params = Promise<{board_id: string}>;
type SearchParams = Promise<{sort?: string; cursor?: string}>;

async function boardOf(id: string): Promise<Board | null> {
  const boards = await load<Page<Board>>('/v1/community/boards');
  return boards.ok ? (boards.data.items.find(b => b.id === id) ?? null) : null;
}

export async function generateMetadata({params}: {params: Params}): Promise<Metadata> {
  const board = await boardOf(decodeURIComponent((await params).board_id));
  return {title: board?.name ?? 'Community'};
}

// One board's posts (list_posts), newest or most liked first, a page at a time.
export default async function BoardPage({params, searchParams}: {params: Params; searchParams: SearchParams}) {
  const id = decodeURIComponent((await params).board_id);
  const sp = await searchParams;
  const sort = sp.sort === 'popular' ? 'popular' : 'newest';
  const [board, posts] = await Promise.all([
    boardOf(id),
    loadBody<Page<PostSummary>>('/v1/community/posts/search', {board_id: id, sort, limit: 20, ...(sp.cursor ? {cursor: sp.cursor} : {})}),
  ]);
  const base = `/community/${encodeURIComponent(id)}`;
  return (
    <Layout
      height="auto"
      contentWidth={960}
      content={
        <LayoutContent padding={6}>
          <VStack gap={4}>
            <Breadcrumbs variant="supporting">
              <BreadcrumbItem href="/community">Community</BreadcrumbItem>
              <BreadcrumbItem isCurrent>{board?.name ?? id}</BreadcrumbItem>
            </Breadcrumbs>
            <HStack gap={2} justify="between" vAlign="center" wrap="wrap">
              <Heading level={1}>{board?.name ?? 'Board'}</Heading>
              <HStack gap={1}>
                <Button label="Newest" size="sm" variant={sort === 'newest' ? 'primary' : 'ghost'} href={base} />
                <Button label="Most liked" size="sm" variant={sort === 'popular' ? 'primary' : 'ghost'} href={`${base}?sort=popular`} />
              </HStack>
            </HStack>
            {!posts.ok ? (
              <ErrorNotice error={posts.error} title="The posts could not be loaded." />
            ) : posts.data.items.length === 0 ? (
              <Text color="secondary">No posts on this board yet.</Text>
            ) : (
              <>
                <Text type="supporting" color="secondary">
                  {posts.data.total === 1 ? '1 post' : `${posts.data.total} posts`}
                </Text>
                <PostList posts={posts.data.items} />
                {posts.data.next_cursor ? (
                  <Button label="More posts" variant="secondary" href={`${base}?sort=${sort}&cursor=${encodeURIComponent(posts.data.next_cursor)}`} />
                ) : null}
              </>
            )}
          </VStack>
        </LayoutContent>
      }
    />
  );
}
