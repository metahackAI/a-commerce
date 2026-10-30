import type {Metadata} from 'next';
import {Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';

import {ErrorNotice} from '@/components/fo/ErrorNotice';
import {GiftPage} from '@/components/fo/GiftAcceptFlow';

import {load, shopper} from '../../_lib/server';
import type {GiftView} from '../../_lib/types';

type Params = Promise<{gift_id: string}>;

export const metadata: Metadata = {title: 'A gift for you'};

// The page a gift's accept link opens (docs/api/sales.md §gift_order). The link is the permission
// to see the gift; accepting it is the recipient's own button.
export default async function GiftLinkPage({params}: {params: Params}) {
  const id = decodeURIComponent((await params).gift_id);
  const [gift, who] = await Promise.all([load<GiftView>(`/v1/sales/gift-orders/${encodeURIComponent(id)}`), shopper()]);
  const canAccept = who.session.scopes.includes('human:order');
  const signInHref = `/login?next=${encodeURIComponent(`/gifts/${id}`)}`;

  return (
    <Layout
      height="auto"
      contentWidth={1100}
      content={
        <LayoutContent padding={6}>
          <VStack gap={5}>
            {gift.ok ? (
              <>
                <VStack gap={1}>
                  <Heading level={1}>{gift.data.from ? `A gift from ${gift.data.from}` : 'A gift for you'}</Heading>
                  <Text color="secondary">{`For ${gift.data.recipient_name}`}</Text>
                </VStack>
                <GiftPage gift={gift.data} canAccept={canAccept} signInHref={signInHref} />
              </>
            ) : (
              <ErrorNotice error={gift.error} title="This gift link does not open." />
            )}
          </VStack>
        </LayoutContent>
      }
    />
  );
}
