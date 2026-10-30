import type {Metadata} from 'next';
import {Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Heading} from '@astryxdesign/core/Text';

import {CartView} from '@/components/fo/CartView';
import {ErrorNotice} from '@/components/fo/ErrorNotice';

import {load} from '../_lib/server';
import type {Cart} from '../_lib/types';

export const metadata: Metadata = {title: 'Cart'};

export default async function CartPage() {
  const cart = await load<Cart>('/v1/sales/carts/mine');
  return (
    <Layout
      height="auto"
      contentWidth={1100}
      content={
        <LayoutContent padding={6}>
          <VStack gap={4}>
            <Heading level={1}>Your cart</Heading>
            {cart.ok ? <CartView initial={cart.data} /> : <ErrorNotice error={cart.error} />}
          </VStack>
        </LayoutContent>
      }
    />
  );
}
