// Search results with sorting, filters and endless scrolling: shared by /search and /c/[category_id].
// Reads search_products (results, the shopper's own prices) and get_facets (filter counts).

import {Button} from '@astryxdesign/core/Button';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {HStack, Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';

import {ErrorNotice} from '@/components/fo/ErrorNotice';
import {InfiniteProducts} from '@/components/fo/InfiniteProducts';
import {SearchBox} from '@/components/fo/SearchBox';
import {BrowseLayout, FilterPanel, SortBar} from '@/components/fo/SearchControls';

import type {SearchParamsState} from './search-href';
import type {Page, Query} from '@/lib/api';

import {load, orElse} from './server';
import type {Category, Facet, ProductSummary} from './types';

export type RawParams = Record<string, string | string[] | undefined>;

const PAGE_SIZE = 24;
const SORTS = new Set(['relevance', 'price', '-price', 'newest']);
const AVAILABILITY = new Set(['in_stock', 'out_of_stock', 'pre_order', 'back_order', 'discontinued']);

function one(v: string | string[] | undefined): string | undefined {
  const s = Array.isArray(v) ? v[0] : v;
  return s && s.trim() ? s.trim() : undefined;
}

function many(v: string | string[] | undefined): string[] {
  return (Array.isArray(v) ? v : v ? [v] : []).flatMap(s => s.split(',')).filter(Boolean);
}

/** The URL's search state, keeping only values the API accepts. */
export function parseSearchParams(raw: RawParams): SearchParamsState & {cursor?: string} {
  const sort = one(raw.sort);
  const money = (s?: string) => (s && /^\d+(\.\d+)?$/.test(s) ? s : undefined);
  return {
    q: one(raw.q),
    category_id: one(raw.category_id),
    sort: sort && SORTS.has(sort) ? sort : undefined,
    availability: many(raw.availability).filter(a => AVAILABILITY.has(a)),
    min_price: money(one(raw.min_price)),
    max_price: money(one(raw.max_price)),
    currency: one(raw.currency)?.toUpperCase(),
    cursor: one(raw.cursor),
  };
}

export async function ResultsView({
  pathname,
  state,
  title,
  subtitle,
  fixedCategoryId,
}: {
  pathname: string;
  state: SearchParamsState & {cursor?: string};
  title: string;
  subtitle?: string;
  /** On a category page the category is the page itself, not a filter. */
  fixedCategoryId?: string;
}) {
  const categoryId = fixedCategoryId ?? state.category_id;
  // search_products takes list-price bounds as MonetaryAmount; in a query string an object field is JSON.
  const currency = state.currency ?? 'USD';
  // The query without a cursor: the results keep loading its next pages as the shopper scrolls,
  // and a new query (its key) starts over.
  const baseQuery: Query = {
    query: state.q,
    category_id: categoryId,
    sort: state.sort,
    availability: state.availability && state.availability.length > 0 ? state.availability : undefined,
    min_price: state.min_price ? JSON.stringify({value: Number(state.min_price), currency}) : undefined,
    max_price: state.max_price ? JSON.stringify({value: Number(state.max_price), currency}) : undefined,
    limit: PAGE_SIZE,
  };
  const query: Query = {...baseQuery, cursor: state.cursor};
  const [results, facets, categories] = await Promise.all([
    load<Page<ProductSummary>>('/v1/search/products', query),
    load<Page<Facet>>('/v1/search/facets', {
      query: state.q,
      category_id: categoryId,
      facet_fields: [...(fixedCategoryId ? [] : ['category']), 'price_range', 'availability', 'brand', 'color', 'size'],
    }),
    load<Page<Category>>('/v1/merchandising/categories', {term_set: 'standard', limit: 100}),
  ]);
  const categoryNames = Object.fromEntries(orElse(categories, {items: [], total: 0}).items.map(c => [c.id, c.name]));
  const items = results.ok ? results.data.items : [];
  const shownCurrency = items[0]?.price_currency ?? currency;
  const clearHref = state.q ? `${pathname}?q=${encodeURIComponent(state.q)}` : pathname;
  const queryKey = JSON.stringify(baseQuery);

  return (
    <Layout
      height="auto"
      contentWidth={1280}
      content={
        // Visible overflow: the filter panel sticks to the shell's scrolling area, not to this box.
        <LayoutContent padding={6} style={{overflow: 'visible'}}>
          <BrowseLayout
            filters={
              <FilterPanel
                state={state}
                facets={orElse(facets, {items: [], total: 0}).items}
                categoryNames={categoryNames}
                currency={shownCurrency}
                showCategoryFacet={!fixedCategoryId}
                clearHref={clearHref}
              />
            }>
            <VStack gap={4}>
              <VStack gap={1}>
                <Heading level={1}>{title}</Heading>
                {subtitle ? <Text color="secondary">{subtitle}</Text> : null}
              </VStack>
              <SearchBox initial={state.q ?? ''} />
              <HStack gap={3} justify="between" vAlign="center" wrap="wrap">
                {results.ok ? (
                  <Text type="supporting" color="secondary">
                    {results.data.total === 1 ? '1 product' : `${results.data.total} products`}
                  </Text>
                ) : (
                  <span />
                )}
                <SortBar state={state} />
              </HStack>
              {!results.ok ? (
                <ErrorNotice error={results.error} askAbout={state.q ? `Help me find ${state.q}` : undefined} />
              ) : items.length === 0 ? (
                <EmptyState
                  title="Nothing matches yet"
                  description={state.q ? `No products for “${state.q}” with these filters.` : 'No products with these filters.'}
                  actions={
                    <HStack gap={2} wrap="wrap">
                      <Button label="Clear filters" href={clearHref} />
                      <Button label="Shop all" variant="ghost" href="/search" />
                    </HStack>
                  }
                />
              ) : (
                <InfiniteProducts key={queryKey} query={baseQuery} initial={items} initialCursor={results.data.next_cursor ?? null} />
              )}
            </VStack>
          </BrowseLayout>
        </LayoutContent>
      }
    />
  );
}
