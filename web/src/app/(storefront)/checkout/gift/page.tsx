import type {Metadata} from 'next';
import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {HStack, Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';

import {ErrorNotice} from '@/components/fo/ErrorNotice';
import {GiftCheckoutFlow} from '@/components/fo/GiftCheckoutFlow';

import {load, shopper} from '../../_lib/server';
import type {Cart} from '../../_lib/types';

export const metadata: Metadata = {title: 'Send as a gift'};

// Gift checkout: the cart sent to someone else (scene F4). The address is for this order only.
export default async function GiftCheckoutPage() {
  const who = await shopper();
  const body = await (async () => {
    if (!who.isShopper) {
      return (
        <Banner
          status="info"
          title="Sign in to send a gift"
          description="A gift is an order: it needs a signed-in shopper so payment and the gift receipt are tied to you."
          endContent={
            <HStack gap={2}>
              <Button label="Sign in" variant="primary" size="sm" href="/login?next=%2Fcheckout%2Fgift" />
              <Button label="Back to cart" size="sm" href="/cart" />
            </HStack>
          }
        />
      );
    }
    const cart = await load<Cart>('/v1/sales/carts/mine');
    if (!cart.ok) return <ErrorNotice error={cart.error} />;
    if (cart.data.lines.length === 0) {
      return (
        <EmptyState
          title="Your cart is empty"
          description="Put the gift in your cart first, or ask the assistant to find one."
          actions={<Button label="Start shopping" variant="primary" href="/search" />}
        />
      );
    }
    return <GiftCheckoutFlow cart={cart.data} />;
  })();

  return (
    <Layout
      height="auto"
      contentWidth={1100}
      content={
        <LayoutContent padding={6}>
          <VStack gap={4}>
            <VStack gap={1}>
              <Heading level={1}>Send as a gift</Heading>
              <Text color="secondary">Your cart, sent to someone else. Only you can place it.</Text>
            </VStack>
            {body}
          </VStack>
        </LayoutContent>
      }
    />
  );
}
