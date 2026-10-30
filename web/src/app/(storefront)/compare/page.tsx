import type {Metadata} from 'next';
import {Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Heading} from '@astryxdesign/core/Text';

import {CompareView} from '@/components/fo/CompareView';

export const metadata: Metadata = {title: 'Compare'};

// `/compare?ids=prd_a,prd_b` compares those; plain `/compare` uses the products picked with "Compare".
export default async function ComparePage({searchParams}: {searchParams: Promise<Record<string, string | string[] | undefined>>}) {
  const {ids} = await searchParams;
  const list = (Array.isArray(ids) ? ids : ids ? [ids] : [])
    .flatMap(s => s.split(','))
    .map(s => s.trim())
    .filter(s => /^prd_[a-z0-9_]+$/.test(s))
    .slice(0, 6);
  return (
    <Layout
      height="auto"
      contentWidth={1200}
      content={
        <LayoutContent padding={6}>
          <VStack gap={4}>
            <Heading level={1}>Compare</Heading>
            <CompareView ids={list} />
          </VStack>
        </LayoutContent>
      }
    />
  );
}
