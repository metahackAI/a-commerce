'use client';

// What a canvas page shows: the product, order or collection a person opened from something on the
// canvas. Each reads what its full page reads, in the browser, and is made of the same parts.

import {Card} from '@astryxdesign/core/Card';
import {Center} from '@astryxdesign/core/Center';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Spinner} from '@astryxdesign/core/Spinner';
import {Heading, Text} from '@astryxdesign/core/Text';
import {useEffect, useState} from 'react';

import {longDate, slotItemId} from '@/app/(storefront)/_lib/format';
import {type Loaded, loadInBrowser} from '@/app/(storefront)/_lib/loaded';
import type {CollectionPage, Delivery, Order, Product, TrackingEvent} from '@/app/(storefront)/_lib/types';
import type {Page} from '@/lib/api';

import type {CanvasPage} from './CanvasPages';
import {ErrorNotice} from './ErrorNotice';
import {OrderBody} from './OrderBody';
import {StatusBadge} from './OrderViews';
import {ProductGrid} from './ProductGrid';
import {ProductImage} from './ProductImage';
import {ProductView} from './ProductView';

function Waiting() {
  return (
    <Card>
      <Center>
        <Spinner aria-label="Loading" />
      </Center>
    </Card>
  );
}

/** Runs `read` for `id`, and shows nothing stale when `id` changes. */
function useRead<T>(id: string, read: (id: string) => Promise<T>): T | null {
  const [loaded, setLoaded] = useState<{id: string; value: T} | null>(null);
  useEffect(() => {
    let current = true;
    void read(id).then(value => {
      if (current) setLoaded({id, value});
    });
    return () => {
      current = false;
    };
  }, [id, read]);
  return loaded?.id === id ? loaded.value : null;
}

async function readOrder(id: string) {
  const order = await loadInBrowser<Order>(`/v1/sales/orders/${encodeURIComponent(id)}`);
  const deliveryId = order.ok ? (order.data.delivery?.id ?? null) : null;
  const [delivery, tracking] = await Promise.all([
    deliveryId ? loadInBrowser<Delivery>(`/v1/fulfillment/deliveries/${encodeURIComponent(deliveryId)}`) : null,
    deliveryId
      ? loadInBrowser<Page<TrackingEvent>>(`/v1/fulfillment/deliveries/${encodeURIComponent(deliveryId)}/tracking`, {mode: 'delivery', delivery_id: deliveryId, limit: 20})
      : null,
  ]);
  return {order, delivery: delivery?.ok ? delivery.data : null, tracking: tracking?.ok ? tracking.data.items : []};
}

function OrderPage({id}: {id: string}) {
  const read = useRead(id, readOrder);
  if (!read) return <Waiting />;
  if (!read.order.ok) return <ErrorNotice error={read.order.error} title="This order is not available." />;
  const o = read.order.data;
  return (
    <VStack gap={4}>
      <VStack gap={1}>
        <HStack gap={2} vAlign="center" wrap="wrap">
          <Heading level={2}>Order #{o.order_number}</Heading>
          <StatusBadge status={o.order_status} />
        </HStack>
        <Text color="secondary">Placed {longDate(o.order_date)}</Text>
      </VStack>
      <OrderBody order={o} delivery={read.delivery} tracking={read.tracking} />
    </VStack>
  );
}

async function readCollection(id: string): Promise<{page: Loaded<CollectionPage>; sections: {name: string; products: Product[]}[]}> {
  const page = await loadInBrowser<CollectionPage>(`/v1/merchandising/collection-pages/${encodeURIComponent(id)}`);
  if (!page.ok) return {page, sections: []};
  const hidden = new Set((page.data.hidden_items ?? []).map(h => h.product_id));
  const sections = await Promise.all(
    page.data.has_part.map(async part => {
      const ids = part.item_list_element.map(slotItemId).filter((p): p is string => !!p && p.startsWith('prd_') && !hidden.has(p));
      const loaded = await Promise.all(ids.map(p => loadInBrowser<Product>(`/v1/catalog/products/${encodeURIComponent(p)}`)));
      return {name: part.name, products: loaded.flatMap(l => (l.ok ? [l.data] : []))};
    }),
  );
  return {page, sections};
}

function CollectionView({id}: {id: string}) {
  const read = useRead(id, readCollection);
  if (!read) return <Waiting />;
  if (!read.page.ok) return <ErrorNotice error={read.page.error} title="This collection is not available." askAbout="What collections are on now?" />;
  const c = read.page.data;
  return (
    <VStack gap={5}>
      {c.image_url ? <ProductImage src={c.image_url} alt={c.name} ratio={3} /> : null}
      <VStack gap={1}>
        <Heading level={2}>{c.name}</Heading>
        {c.valid_through ? <Text color="secondary">{`On until ${longDate(c.valid_through)}`}</Text> : null}
      </VStack>
      {read.sections.map(s =>
        s.products.length > 0 ? (
          <VStack key={s.name} gap={3}>
            <Heading level={3}>{s.name}</Heading>
            <ProductGrid products={s.products} minWidth={200} />
          </VStack>
        ) : null,
      )}
    </VStack>
  );
}

export function CanvasPageView({page}: {page: CanvasPage}) {
  if (page.kind === 'order') return <OrderPage id={page.id} />;
  if (page.kind === 'collection') return <CollectionView id={page.id} />;
  return <ProductView productId={page.id} color={page.color} />;
}
