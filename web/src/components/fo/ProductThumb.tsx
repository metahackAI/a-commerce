'use client';

// A product as a shopper's own records show it (an order line, a return, a subscription, a
// restock alert, a wishlist row): the picture of the color they have, opening the product on
// that color, and "Buy again" for a variant the store still sells.

import {Button} from '@astryxdesign/core/Button';
import {Text} from '@astryxdesign/core/Text';
import {VStack} from '@astryxdesign/core/Layout';
import {useRouter} from 'next/navigation';
import {useState} from 'react';

import {imageUrl} from '@/app/(storefront)/_lib/format';
import type {ApiErrorBody} from '@/lib/api';

import {useCanvasLink} from './CanvasPages';
import {ProductImage} from './ProductImage';
import {errorBody, write} from './write';

/** `/p/prd_x?color=Black`: the product page opens on the color the shopper has. */
export function productHref(productId: string, color?: string | null): string {
  const base = `/p/${encodeURIComponent(productId)}`;
  return color ? `${base}?color=${encodeURIComponent(color)}` : base;
}

/** A small square picture that opens the product. */
export function ProductThumb({
  productId,
  name,
  image,
  color,
  size = 64,
}: {
  productId: string;
  name: string;
  image: string | null | undefined;
  color?: string | null;
  size?: number;
}) {
  const link = useCanvasLink(productHref(productId, color), name);
  return (
    <div style={{width: size, flexShrink: 0}}>
      <ProductImage src={imageUrl(image ?? null)} alt={color ? `${name}, ${color}` : name} ratio={1} {...link} />
    </div>
  );
}

/** Puts the same variant in the cart again (add_to_cart: preview, then run; no approval). */
export function BuyAgainButton({variantId, name, availability}: {variantId: string; name: string; availability?: string | null}) {
  const router = useRouter();
  const [state, setState] = useState<'idle' | 'added'>('idle');
  const [error, setError] = useState<ApiErrorBody | null>(null);
  const sellable = availability === 'in_stock' || availability === 'pre_order' || availability === 'back_order';
  if (!sellable) {
    return (
      <Text type="supporting" color="secondary">
        {availability === 'discontinued' ? 'No longer sold' : 'Out of stock now'}
      </Text>
    );
  }
  return (
    <VStack gap={1} hAlign="end">
      <Button
        label={state === 'added' ? 'In your cart' : 'Buy again'}
        size="sm"
        variant={state === 'added' ? 'ghost' : 'secondary'}
        href={state === 'added' ? '/cart' : undefined}
        clickAction={
          state === 'added'
            ? undefined
            : async () => {
                setError(null);
                try {
                  await write('POST', '/v1/sales/carts/mine/lines', {mode: 'variant', variant_id: variantId, order_quantity: 1});
                  setState('added');
                  router.refresh();
                } catch (e) {
                  setError(errorBody(e));
                }
              }
        }
        aria-label={state === 'added' ? `${name} is in your cart` : `Buy ${name} again`}
      />
      {error ? (
        <Text type="supporting" color="secondary">
          {error.message}
        </Text>
      ) : null}
    </VStack>
  );
}
