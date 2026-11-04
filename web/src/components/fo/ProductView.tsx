'use client';

// One product where there is no page around it: the next canvas page, and the agent's
// product_detail block. It reads what the product page reads and shows the same parts, so the
// shopper can pick a size, buy, read reviews and ask about any of it without leaving the canvas.

import {Card} from '@astryxdesign/core/Card';
import {Center} from '@astryxdesign/core/Center';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Spinner} from '@astryxdesign/core/Spinner';
import {Text} from '@astryxdesign/core/Text';
import {useEffect, useState} from 'react';

import {useAiTarget} from '@/ai/useAiTarget';
import {type Loaded, loadInBrowser} from '@/app/(storefront)/_lib/loaded';
import type {DeliveryDate, Measurements, Product, ProductQuestion, Profile, ReturnPolicy, Review, ReviewSummary} from '@/app/(storefront)/_lib/types';
import type {Page} from '@/lib/api';

import {Intents} from './Ask';
import {OpenInNewTab} from './CanvasPages';
import {ErrorNotice} from './ErrorNotice';
import {ProductSections, type ProductReads} from './ProductSections';
import {ProductShowcase} from './ProductShowcase';
import {useStorefront} from './StorefrontContext';

/** What a shopper usually wants to know next, each answered with this product as its context. */
const NEXT_QUESTIONS = ['What size should I get?', 'What do reviewers say about it?', 'Is it right for me?', 'Show me something similar'];

interface Reads extends ProductReads {
  product: Loaded<Product>;
  address: {id: string; label: string} | null;
  delivery: DeliveryDate | null;
}

async function readProduct(id: string, isShopper: boolean): Promise<Reads> {
  const enc = encodeURIComponent(id);
  const [product, measurements, summary, reviews, policy, questions, profile] = await Promise.all([
    loadInBrowser<Product>(`/v1/catalog/products/${enc}`),
    loadInBrowser<Measurements>(`/v1/catalog/products/${enc}/measurements`),
    loadInBrowser<ReviewSummary>(`/v1/catalog/products/${enc}/reviews/summary`, {sample_limit: 3}),
    loadInBrowser<Page<Review>>('/v1/catalog/reviews', {product_id: id, sort: 'newest', limit: 5}),
    loadInBrowser<ReturnPolicy>(`/v1/catalog/products/${enc}/return-policy`),
    loadInBrowser<Page<ProductQuestion>>('/v1/catalog/questions', {product_id: id, limit: 10}),
    isShopper ? loadInBrowser<Profile>('/v1/customer/people/me', {fields: ['addresses']}) : null,
  ]);
  const addresses = profile?.ok ? (profile.data.addresses ?? []) : [];
  const address = addresses.find(a => a.is_primary) ?? addresses[0] ?? null;
  const delivery = address
    ? await loadInBrowser<Page<DeliveryDate>>('/v1/fulfillment/delivery-dates', {product_id: id, address_mode: 'saved', address_id: address.id})
    : null;
  return {
    product,
    measurements,
    summary,
    reviews,
    policy,
    questions,
    address: address ? {id: address.id, label: address.label ?? address.street_address} : null,
    delivery: delivery?.ok ? (delivery.data.items[0] ?? null) : null,
  };
}

function ProductParts({reads, color, blockId}: {reads: Reads & {product: {ok: true; data: Product}}; color?: string | null; blockId?: string}) {
  const p = reads.product.data;
  const target = useAiTarget({kind: 'product_card', label: p.name, ids: {product_id: p.id}, term: 'schema:Product', blockId});
  return (
    <VStack gap={4}>
      <Card ref={target.ref} {...target.props}>
        <VStack gap={5}>
          <ProductShowcase
            compact
            product={p}
            image={p.image}
            description={p.description}
            addressId={reads.address?.id ?? null}
            addressLabel={reads.address?.label ?? null}
            initialDelivery={reads.delivery}
            initialColor={color}
          />
          <VStack gap={2}>
            <HStack gap={2} vAlign="center" justify="between" wrap="wrap">
              <Text type="label" weight="semibold">
                Ask about this product
              </Text>
              <OpenInNewTab href={`/p/${encodeURIComponent(p.id)}`} />
            </HStack>
            <Intents says={NEXT_QUESTIONS} about={target.context} />
          </VStack>
        </VStack>
      </Card>
      <Card>
        <ProductSections product={p} reads={reads} />
      </Card>
    </VStack>
  );
}

export function ProductView({productId, color, blockId}: {productId: string; color?: string | null; blockId?: string}) {
  const {isShopper} = useStorefront();
  const [reads, setReads] = useState<{id: string; reads: Reads} | null>(null);
  useEffect(() => {
    let current = true;
    void readProduct(productId, isShopper).then(r => {
      if (current) setReads({id: productId, reads: r});
    });
    return () => {
      current = false;
    };
  }, [productId, isShopper]);

  const shown = reads?.id === productId ? reads.reads : null;
  if (!shown) {
    return (
      <Card>
        <Center>
          <Spinner aria-label="Loading the product" />
        </Center>
      </Card>
    );
  }
  const {product} = shown;
  if (!product.ok) return <ErrorNotice error={product.error} title="This product is not available." askAbout="Show me something similar." />;
  return <ProductParts reads={{...shown, product}} color={color} blockId={blockId} />;
}
