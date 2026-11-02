'use client';

// Search and category results that keep loading as the shopper scrolls: the page renders the
// first page of search_products, and each time the end of the grid comes into view the next page
// (`next_cursor`) is read with the same query and added below. A new query (filters, sort,
// words) starts over from the server's first page.

import {Button} from '@astryxdesign/core/Button';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Spinner} from '@astryxdesign/core/Spinner';
import {Text} from '@astryxdesign/core/Text';
import {useCallback, useEffect, useRef, useState} from 'react';

import type {ProductSummary} from '@/app/(storefront)/_lib/types';
import {api, ApiError, type Page, type Query} from '@/lib/api';

import {ProductGrid} from './ProductGrid';

export function InfiniteProducts({
  query,
  initial,
  initialCursor,
}: {
  /** The search_products query of the first page, without `cursor`. */
  query: Query;
  initial: ProductSummary[];
  initialCursor: string | null;
}) {
  const [items, setItems] = useState(initial);
  const [cursor, setCursor] = useState(initialCursor);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sentinel = useRef<HTMLDivElement | null>(null);
  const busy = useRef(false);

  const loadMore = useCallback(async () => {
    if (!cursor || busy.current) return;
    busy.current = true;
    setLoading(true);
    setError(null);
    try {
      const page = await api<Page<ProductSummary>>('/v1/search/products', {query: {...query, cursor}});
      setItems(prev => {
        const seen = new Set(prev.map(p => p.id));
        return [...prev, ...page.items.filter(p => !seen.has(p.id))];
      });
      setCursor(page.next_cursor ?? null);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : 'The next products could not be loaded.');
    } finally {
      busy.current = false;
      setLoading(false);
    }
  }, [cursor, query]);

  useEffect(() => {
    const el = sentinel.current;
    if (!el || !cursor || error) return;
    const observer = new IntersectionObserver(entries => {
      if (entries.some(e => e.isIntersecting)) void loadMore();
    }, {rootMargin: '600px 0px'});
    observer.observe(el);
    return () => observer.disconnect();
  }, [cursor, error, loadMore]);

  return (
    <VStack gap={5}>
      <ProductGrid products={items} />
      <div ref={sentinel} aria-hidden />
      <HStack justify="center" vAlign="center" gap={2}>
        {loading ? <Spinner size="md" label="Loading more products" /> : null}
        {error ? (
          <>
            <Text color="secondary">{error}</Text>
            <Button label="Try again" size="sm" onClick={() => void loadMore()} />
          </>
        ) : null}
        {!cursor && !loading && items.length > 0 ? (
          <Text type="supporting" color="secondary">
            {items.length === 1 ? 'That is the one product.' : `That is all ${items.length} products.`}
          </Text>
        ) : null}
      </HStack>
    </VStack>
  );
}
