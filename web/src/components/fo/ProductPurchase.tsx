'use client';

// Choosing a variant and buying it: colors and sizes from get_product's variants with their
// availability, the variant's price from get_price_breakdown, delivery dates for the shopper's
// address from get_delivery_dates, then add_to_cart (preview → run; no approval) and
// add_to_wishlist. Every number is the API's.

import {Badge} from '@astryxdesign/core/Badge';
import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {NumberInput} from '@astryxdesign/core/NumberInput';
import {Spinner} from '@astryxdesign/core/Spinner';
import {Text} from '@astryxdesign/core/Text';
import {ToggleButton} from '@astryxdesign/core/ToggleButton';
import {useRouter} from 'next/navigation';
import {useEffect, useMemo, useState} from 'react';

import {useAiTarget} from '@/ai/useAiTarget';
import {day, money, words} from '@/app/(storefront)/_lib/format';
import type {CartWriteResult, DeliveryDate, PriceBreakdown, Product, Variant} from '@/app/(storefront)/_lib/types';
import {api, type ApiErrorBody, type Page} from '@/lib/api';

import {AskButton} from './Ask';
import {ErrorNotice} from './ErrorNotice';
import {PriceTag} from './PriceTag';
import {COMPARE_MAX, useStorefront} from './StorefrontContext';
import {errorBody, write} from './write';

function unique(values: (string | null | undefined)[]): string[] {
  return [...new Set(values.filter((v): v is string => !!v))];
}

function OptionChip({
  productId,
  kind,
  value,
  isSelected,
  isUnavailable,
  onSelect,
}: {
  productId: string;
  kind: 'color' | 'size';
  value: string;
  isSelected: boolean;
  isUnavailable: boolean;
  onSelect: () => void;
}) {
  const target = useAiTarget({
    kind: kind === 'size' ? 'size_chip' : 'color_chip',
    label: `${words(kind)} ${value}`,
    ids: {[kind]: value, product_id: productId},
  });
  return (
    <ToggleButton
      ref={target.ref}
      {...target.props}
      label={value}
      size="sm"
      isPressed={isSelected}
      tooltip={isUnavailable ? `${value} is out of stock` : undefined}
      onPressedChange={pressed => {
        if (pressed) onSelect();
      }}>
      {isUnavailable ? `${value} · sold out` : value}
    </ToggleButton>
  );
}

export function ProductPurchase({
  product,
  addressId,
  addressLabel,
  initialDelivery,
}: {
  product: Product;
  addressId: string | null;
  addressLabel: string | null;
  initialDelivery: DeliveryDate | null;
}) {
  const router = useRouter();
  const {isShopper, compare, toggleCompare} = useStorefront();
  const variants: Variant[] = useMemo(() => product.has_variant ?? [], [product.has_variant]);
  const colors = unique(variants.map(v => v.color));
  const sizes = unique(variants.map(v => v.size));
  const firstAvailable = variants.find(v => v.availability === 'in_stock') ?? variants[0];

  const [color, setColor] = useState<string | null>(firstAvailable?.color ?? null);
  const [size, setSize] = useState<string | null>(null);
  const [quantity, setQuantity] = useState(1);
  const [breakdown, setBreakdown] = useState<PriceBreakdown | null>(null);
  const [delivery, setDelivery] = useState<DeliveryDate | null>(initialDelivery);
  const [deliveryError, setDeliveryError] = useState<ApiErrorBody | null>(null);
  const [isLoadingVariant, setLoadingVariant] = useState(false);
  const [added, setAdded] = useState<CartWriteResult | null>(null);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<ApiErrorBody | null>(null);

  const variant = variants.find(v => (colors.length === 0 || v.color === color) && (sizes.length === 0 || v.size === size)) ?? null;
  const needsSize = sizes.length > 0 && !size;
  const target = useAiTarget({kind: 'product', label: product.name, ids: {product_id: product.id}, term: 'schema:Product'});

  // The selected variant's own price and delivery.
  useEffect(() => {
    if (!variant) return;
    const ctl = new AbortController();
    setLoadingVariant(true);
    const priceRead = api<PriceBreakdown>(`/v1/catalog/variants/${encodeURIComponent(variant.variant_id)}/price-breakdown`, {
      query: {order_quantity: quantity},
      signal: ctl.signal,
    })
      .then(setBreakdown)
      .catch(() => setBreakdown(null));
    const deliveryRead = addressId
      ? api<Page<DeliveryDate>>('/v1/fulfillment/delivery-dates', {
          query: {product_id: product.id, variant_id: variant.variant_id, address_mode: 'saved', address_id: addressId},
          signal: ctl.signal,
        })
          .then(r => {
            setDelivery(r.items[0] ?? null);
            setDeliveryError(null);
          })
          .catch(e => {
            if (!ctl.signal.aborted) setDeliveryError(errorBody(e));
          })
      : Promise.resolve();
    void Promise.all([priceRead, deliveryRead]).finally(() => {
      if (!ctl.signal.aborted) setLoadingVariant(false);
    });
    return () => ctl.abort();
  }, [variant, addressId, product.id, quantity]);

  const inStock = variant ? variant.availability === 'in_stock' || variant.availability === 'pre_order' || variant.availability === 'back_order' : false;

  async function addToCart() {
    if (!variant) return;
    setError(null);
    setAdded(null);
    try {
      const result = await write<CartWriteResult>('POST', '/v1/sales/carts/mine/lines', {
        mode: 'variant',
        variant_id: variant.variant_id,
        order_quantity: quantity,
      });
      setAdded(result);
      router.refresh();
    } catch (e) {
      setError(errorBody(e));
    }
  }

  async function saveToWishlist() {
    setError(null);
    try {
      await write('POST', '/v1/customer/people/me/wishlist', {product_id: product.id});
      setSaved(true);
    } catch (e) {
      setError(errorBody(e));
    }
  }

  const comparing = compare.some(c => c.productId === product.id);
  const memberLabel = breakdown?.applied[0]?.name ?? null;

  return (
    <VStack gap={5} ref={target.ref} {...target.props}>
      <HStack gap={2} vAlign="center" wrap="wrap">
        {breakdown ? (
          <PriceTag
            productId={product.id}
            name={product.name}
            price={breakdown.list_price}
            currency={breakdown.price_currency}
            member={breakdown.member_price}
            memberLabel={memberLabel}
            size="lg"
          />
        ) : (
          <PriceTag productId={product.id} name={product.name} price={product.price} currency={product.price_currency} member={product.member_price} size="lg" />
        )}
        {isLoadingVariant ? <Spinner size="sm" aria-label="Updating price and delivery" /> : null}
        <AskButton target={target} label={product.name} />
      </HStack>
      {product.member_price?.valid_through ? (
        <Text type="supporting" color="secondary">
          {product.member_price.name} price until {day(product.member_price.valid_through)}
        </Text>
      ) : null}

      {colors.length > 0 ? (
        <VStack gap={2}>
          <Text type="label">Color{color ? `: ${color}` : ''}</Text>
          <HStack gap={1} wrap="wrap">
            {colors.map(c => (
              <OptionChip
                key={c}
                productId={product.id}
                kind="color"
                value={c}
                isSelected={c === color}
                isUnavailable={!variants.some(v => v.color === c && v.availability === 'in_stock')}
                onSelect={() => setColor(c)}
              />
            ))}
          </HStack>
        </VStack>
      ) : null}

      {sizes.length > 0 ? (
        <VStack gap={2}>
          <Text type="label">Size{size ? `: ${size}` : ''}</Text>
          <HStack gap={1} wrap="wrap">
            {sizes.map(s => {
              const v = variants.find(x => x.size === s && (colors.length === 0 || x.color === color));
              return (
                <OptionChip
                  key={s}
                  productId={product.id}
                  kind="size"
                  value={s}
                  isSelected={s === size}
                  isUnavailable={!v || v.availability !== 'in_stock'}
                  onSelect={() => setSize(s)}
                />
              );
            })}
          </HStack>
        </VStack>
      ) : null}

      {variant ? (
        <HStack gap={2} vAlign="center" wrap="wrap">
          <Badge label={words(variant.availability)} variant={variant.availability === 'in_stock' ? 'success' : 'warning'} />
          {typeof variant.inventory_level === 'number' && variant.availability === 'in_stock' ? (
            <Text type="supporting" color="secondary">
              {variant.inventory_level} available in {[variant.color, variant.size].filter(Boolean).join(', ')}
            </Text>
          ) : null}
        </HStack>
      ) : needsSize ? (
        <Text type="supporting" color="secondary">
          Choose a size to see stock and delivery.
        </Text>
      ) : (
        <Text type="supporting" color="secondary">
          This combination is not made.
        </Text>
      )}

      <VStack gap={1}>
        <Text type="label">Delivery</Text>
        {!addressId ? (
          <Text type="supporting" color="secondary">
            {isShopper ? 'Add a delivery address in your account to see dates.' : 'Sign in to see when it reaches your address.'}
          </Text>
        ) : deliveryError ? (
          <Text type="supporting" color="secondary">
            {deliveryError.message}
          </Text>
        ) : delivery ? (
          <VStack gap={0.5}>
            <Text>
              Arrives {day(delivery.earliest_arrival)} to {addressLabel ?? 'your address'}
            </Text>
            <Text type="supporting" color="secondary">
              Ships {day(delivery.ships_on)} · {delivery.shipping.value === 0 ? 'Free shipping' : `${money(delivery.shipping)} shipping`}
              {delivery.shipping_note ? ` · ${delivery.shipping_note}` : ''}
            </Text>
          </VStack>
        ) : (
          <Text type="supporting" color="secondary">
            Choose a size to see the delivery date.
          </Text>
        )}
      </VStack>

      <HStack gap={2} vAlign="end" wrap="wrap">
        <NumberInput label="Quantity" value={quantity} onChange={v => setQuantity(Math.max(1, Math.round(v || 1)))} min={1} isIntegerOnly width={120} hasNumberSteppers />
        <Button
          label={needsSize ? 'Choose a size' : inStock ? 'Add to cart' : 'Out of stock'}
          variant="primary"
          size="lg"
          isDisabled={!variant || !inStock}
          clickAction={addToCart}
        />
      </HStack>
      <HStack gap={2} wrap="wrap">
        {isShopper ? (
          <Button label={saved ? 'Saved to wishlist' : 'Save to wishlist'} isDisabled={saved} clickAction={saveToWishlist} />
        ) : (
          <Button label="Sign in to save" variant="ghost" href={`/login?next=${encodeURIComponent(`/p/${product.id}`)}`} />
        )}
        <Button
          label={comparing ? 'In comparison' : 'Compare'}
          variant={comparing ? 'secondary' : 'ghost'}
          isDisabled={!comparing && compare.length >= COMPARE_MAX}
          onClick={() => toggleCompare({productId: product.id, name: product.name})}
        />
        {compare.length >= 2 ? <Button label={`Compare ${compare.length}`} variant="ghost" href="/compare" /> : null}
      </HStack>

      {added ? (
        <Banner
          status="success"
          title="Added to your cart"
          description={added.cart ? `Cart total ${money(added.cart.total)}` : undefined}
          endContent={<Button label="View cart" size="sm" variant="primary" href="/cart" />}
        />
      ) : null}
      {error ? <ErrorNotice error={error} askAbout={`I couldn't add the ${product.name}. What can I do?`} /> : null}
    </VStack>
  );
}
