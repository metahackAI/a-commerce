import {Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';

import {ErrorNotice} from '@/components/fo/ErrorNotice';
import {CollectionCards} from '@/components/fo/StoreContent';
import type {Page} from '@/lib/api';

import {load} from '../_lib/server';
import type {CollectionPageSummary} from '../_lib/types';

export const metadata = {title: 'Collections'};

// The collection pages open to this shopper now (list_curations answers a shopper with only those).
export default async function CollectionsPage() {
  const list = await load<Page<CollectionPageSummary>>('/v1/merchandising/collection-pages', {limit: 50});
  return (
    <Layout
      height="auto"
      contentWidth={1200}
      content={
        <LayoutContent padding={6}>
          <VStack gap={5}>
            <Heading level={1}>Collections</Heading>
            {!list.ok ? (
              <ErrorNotice error={list.error} title="Collections could not be loaded." askAbout="What collections are on now?" />
            ) : list.data.items.length === 0 ? (
              <Text color="secondary">No collections are on right now.</Text>
            ) : (
              <CollectionCards pages={list.data.items} />
            )}
          </VStack>
        </LayoutContent>
      }
    />
  );
}
