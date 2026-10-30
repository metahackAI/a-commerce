'use client';

// The cart (get_cart) with its lines, discounts, shipping and total as returned. Quantity changes,
// removing a line and emptying the cart are shopper writes with no approval: preview, then run
// (update_cart_item, remove_cart_item, clear_cart). Each run returns the recalculated cart.

import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {Card} from '@astryxdesign/core/Card';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {Grid} from '@astryxdesign/core/Grid';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {List, ListItem} from '@astryxdesign/core/List';
import {NumberInput} from '@astryxdesign/core/NumberInput';
import {Text} from '@astryxdesign/core/Text';
import {useRouter} from 'next/navigation';
import {useState} from 'react';

import {useAiTarget} from '@/ai/useAiTarget';
import {money} from '@/app/(storefront)/_lib/format';
import type {Cart, CartLine, CartWriteResult} from '@/app/(storefront)/_lib/types';
import {api, type ApiErrorBody} from '@/lib/api';

import {AskButton} from './Ask';
import {CartSummary} from './CartSummary';
import {ErrorNotice} from './ErrorNotice';
import {useStorefront} from './StorefrontContext';
import {errorBody, write} from './write';

function LineRow({
  line,
  isBusy,
  onQuantity,
  onRemove,
}: {
  line: CartLine;
  isBusy: boolean;
  onQuantity: (q: number) => void;
  onRemove: () => void;
}) {
  const options = [line.color, line.size].filter(Boolean).join(' · ');
  const target = useAiTarget({
    kind: 'cart_line',
    label: `${line.name}${options ? `, ${options}` : ''} × ${line.quantity}`,
    ids: {cart_line_id: line.cart_line_id, product_id: line.product_id, variant_id: line.variant_id},
    term: 'schema:OrderItem',
  });
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      align="start"
      label={
        <Link href={`/p/${encodeURIComponent(line.product_id)}`}>
          <Text weight="semibold">{line.name}</Text>
        </Link>
      }
      description={
        <VStack gap={2}>
          <Text type="supporting" color="secondary">
            {[options, `${money(line.price)} each`, line.note].filter(Boolean).join(' · ')}
          </Text>
          <HStack gap={2} vAlign="end" wrap="wrap">
            <NumberInput
              label="Quantity"
              isLabelHidden
              value={line.quantity}
              min={1}
              isIntegerOnly
              hasNumberSteppers
              width={120}
              size="sm"
              isDisabled={isBusy}
              onChange={v => {
                const q = Math.round(v);
                if (q >= 1 && q !== line.quantity) onQuantity(q);
              }}
            />
            <Button label="Remove" size="sm" variant="ghost" isDisabled={isBusy} onClick={onRemove} />
            <AskButton target={target} label={line.name} />
          </HStack>
        </VStack>
      }
    />
  );
}

export function CartView({initial}: {initial: Cart}) {
  const router = useRouter();
  const {isShopper} = useStorefront();
  const [cart, setCart] = useState<Cart>(initial);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<ApiErrorBody | null>(null);

  async function run(key: string, action: () => Promise<CartWriteResult>) {
    setBusy(key);
    setError(null);
    try {
      const result = await action();
      setCart(result.cart ?? (await api<Cart>('/v1/sales/carts/mine')));
      router.refresh();
    } catch (e) {
      setError(errorBody(e));
    } finally {
      setBusy(null);
    }
  }

  const line = (id: string) => `/v1/sales/carts/mine/lines/${encodeURIComponent(id)}`;

  if (cart.lines.length === 0) {
    return (
      <EmptyState
        title="Your cart is empty"
        description="Find something you like, or ask Northwind to help you look."
        actions={<Button label="Start shopping" variant="primary" href="/search" />}
      />
    );
  }

  return (
    <VStack gap={4}>
      {error ? <ErrorNotice error={error} askAbout="Something went wrong with my cart. What should I do?" /> : null}
      <Grid columns={{minWidth: 320, max: 2}} gap={6} align="start">
        <VStack gap={2}>
          <List hasDividers density="spacious">
            {cart.lines.map(l => (
              <LineRow
                key={l.cart_line_id}
                line={l}
                isBusy={busy !== null}
                onQuantity={q => void run(l.cart_line_id, () => write('PATCH', line(l.cart_line_id), {order_quantity: q}))}
                onRemove={() => void run(l.cart_line_id, () => write('DELETE', line(l.cart_line_id), {}))}
              />
            ))}
          </List>
          <HStack>
            <Button
              label="Empty cart"
              variant="ghost"
              size="sm"
              isLoading={busy === 'clear'}
              isDisabled={busy !== null && busy !== 'clear'}
              onClick={() => void run('clear', () => write('DELETE', '/v1/sales/carts/mine/lines', {}))}
            />
          </HStack>
        </VStack>
        <Card>
          <VStack gap={4}>
            <CartSummary
              subtotal={cart.subtotal}
              discounts={cart.discounts}
              shipping={cart.shipping}
              total={cart.total}
              delivery={cart.delivery}
              payment={cart.payment}
              coupons={cart.coupons_recorded}
            />
            {isShopper ? (
              <Button label={`Check out · ${money(cart.total)}`} variant="primary" size="lg" width="100%" href="/checkout" />
            ) : (
              <VStack gap={2}>
                <Banner status="info" title="Sign in to order" description="Guests can keep a cart. Placing an order needs a signed-in shopper so the order, delivery and returns are yours." />
                <Button label="Sign in to check out" variant="primary" size="lg" width="100%" href="/login?next=%2Fcheckout" />
              </VStack>
            )}
          </VStack>
        </Card>
      </Grid>
    </VStack>
  );
}
