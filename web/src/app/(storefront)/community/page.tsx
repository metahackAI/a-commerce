import {Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';

import {ErrorNotice} from '@/components/fo/ErrorNotice';
import {BoardList, PostList} from '@/components/fo/StoreContent';
import type {Page} from '@/lib/api';

import {load, loadBody} from '../_lib/server';
import type {Board, PostSummary} from '../_lib/types';

export const metadata = {title: 'Community'};

// The store's boards (list_boards) with the newest posts of each (list_posts). Authors come
// masked and private posts only to their writers, as the API answers.
export default async function CommunityPage() {
  const boards = await load<Page<Board>>('/v1/community/boards');
  const latest = boards.ok
    ? await Promise.all(
        boards.data.items.map(async b => ({
          board: b,
          posts: await loadBody<Page<PostSummary>>('/v1/community/posts/search', {board_id: b.id, sort: 'newest', limit: 3}),
        })),
      )
    : [];
  return (
    <Layout
      height="auto"
      contentWidth={960}
      content={
        <LayoutContent padding={6}>
          <VStack gap={6}>
            <Heading level={1}>Community</Heading>
            {!boards.ok ? (
              <ErrorNotice error={boards.error} title="The boards could not be loaded." />
            ) : (
              <>
                <BoardList boards={boards.data.items} />
                {latest.map(({board, posts}) =>
                  posts.ok && posts.data.items.length > 0 ? (
                    <VStack key={board.id} gap={2}>
                      <Heading level={2}>{`New on ${board.name}`}</Heading>
                      <PostList posts={posts.data.items} />
                    </VStack>
                  ) : null,
                )}
                {latest.every(l => !l.posts.ok || l.posts.data.items.length === 0) ? <Text color="secondary">No posts yet.</Text> : null}
              </>
            )}
          </VStack>
        </LayoutContent>
      }
    />
  );
}
