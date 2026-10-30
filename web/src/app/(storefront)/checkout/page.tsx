import type {Metadata} from 'next';
import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {HStack, Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Heading} from '@astryxdesign/core/Text';

import {CheckoutFlow} from '@/components/fo/CheckoutFlow';
import {ErrorNotice} from '@/components/fo/ErrorNotice';
import type {Page} from '@/lib/api';

import {load, orElse, shopper} from '../_lib/server';
import type {Address, Cart, PaymentMethod} from '../_lib/types';

export const metadata: Metadata = {title: 'Checkout'};

export default async function CheckoutPage() {
  const who = await shopper();
  const body = await (async () => {
    if (!who.isShopper) {
      return (
        <Banner
          status="info"
          title="Sign in to place your order"
          description={
            who.isGuest
              ? 'Your cart is kept for this session. An order needs a signed-in shopper so delivery, payment and returns are tied to you.'
              : 'Ordering needs a signed-in shopper. Sign in, and your cart comes with you.'
          }
          endContent={
            <HStack gap={2}>
              <Button label="Sign in" variant="primary" size="sm" href="/login?next=%2Fcheckout" />
              <Button label="Back to cart" size="sm" href="/cart" />
            </HStack>
          }
        />
      );
    }
    const [cart, addresses, methods] = await Promise.all([
      load<Cart>('/v1/sales/carts/mine'),
      load<Page<Address>>('/v1/customer/people/me/addresses', {limit: 20}),
      load<Page<PaymentMethod>>('/v1/sales/checkout/payment-methods'),
    ]);
    if (!cart.ok) return <ErrorNotice error={cart.error} />;
    if (cart.data.lines.length === 0) {
      return <EmptyState title="Your cart is empty" description="Add something first, then check out." actions={<Button label="Start shopping" variant="primary" href="/search" />} />;
    }
    return (
      <VStack gap={4}>
        {!addresses.ok ? <ErrorNotice error={addresses.error} /> : null}
        {!methods.ok ? <ErrorNotice error={methods.error} /> : null}
        <CheckoutFlow
          cart={cart.data}
          addresses={orElse(addresses, {items: [], total: 0}).items}
          methods={orElse(methods, {items: [], total: 0}).items}
        />
        <HStack>
          <Button label="Send it as a gift instead" size="sm" variant="ghost" href="/checkout/gift" />
        </HStack>
      </VStack>
    );
  })();

  return (
    <Layout
      height="auto"
      contentWidth={1100}
      content={
        <LayoutContent padding={6}>
          <VStack gap={4}>
            <Heading level={1}>Checkout</Heading>
            {body}
          </VStack>
        </LayoutContent>
      }
    />
  );
}
