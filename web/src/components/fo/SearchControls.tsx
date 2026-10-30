'use client';

// Sorting and filters for search and category pages. Filters come from `get_facets` (search.md)
// with the store's counts; choosing one changes the URL, and the page reads search_products again.
// search_products filters by category, availability and list-price range; brand, color and size
// facets narrow the search words instead.

import {Collapsible} from '@astryxdesign/core/Collapsible';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Selector} from '@astryxdesign/core/Selector';
import {Text} from '@astryxdesign/core/Text';
import {Token} from '@astryxdesign/core/Token';
import {usePathname, useRouter} from 'next/navigation';

import {useAiTarget} from '@/ai/useAiTarget';
import {price, words} from '@/app/(storefront)/_lib/format';
import type {Facet} from '@/app/(storefront)/_lib/types';
import {hrefWith, type SearchParamsState} from '@/app/(storefront)/_lib/search-href';

function priceRangeLabel(value: string, currency: string): string {
  const [lo, hi] = value.split('-');
  const a = Number(lo);
  const b = Number(hi);
  if (hi === undefined || hi === '' || Number.isNaN(b)) return `${price(a, currency)} and up`;
  if (lo === '' || Number.isNaN(a)) return `Up to ${price(b, currency)}`;
  return `${price(a, currency)} – ${price(b, currency)}`;
}

function FacetToken({
  facet,
  label,
  isActive,
  href,
  onRemove,
}: {
  facet: Facet;
  label: string;
  isActive: boolean;
  href: string;
  onRemove?: () => void;
}) {
  const target = useAiTarget({kind: 'chip', label: `${words(facet.name)}: ${label}`, ids: {facet: `${facet.name}:${facet.value}`}});
  return (
    <Token
      ref={target.ref}
      {...target.props}
      label={`${label} (${facet.count})`}
      size="sm"
      color={isActive ? 'blue' : 'default'}
      href={isActive ? undefined : href}
      onRemove={isActive ? onRemove : undefined}
    />
  );
}

export function SearchControls({
  state,
  facets,
  categoryNames,
  currency,
  showCategoryFacet,
}: {
  state: SearchParamsState;
  facets: Facet[];
  categoryNames: Record<string, string>;
  currency: string;
  showCategoryFacet: boolean;
}) {
  const router = useRouter();
  const pathname = usePathname();
  const go = (change: Partial<SearchParamsState>) => router.push(hrefWith(pathname, state, change));

  const groups: {name: string; title: string; items: Facet[]}[] = [
    {name: 'category', title: 'Category', items: showCategoryFacet ? facets.filter(f => f.name === 'category') : []},
    {name: 'price_range', title: 'Price', items: facets.filter(f => f.name === 'price_range')},
    {name: 'availability', title: 'Availability', items: facets.filter(f => f.name === 'availability')},
    {name: 'brand', title: 'Brand', items: facets.filter(f => f.name === 'brand')},
    {name: 'color', title: 'Color', items: facets.filter(f => f.name === 'color')},
    {name: 'size', title: 'Size', items: facets.filter(f => f.name === 'size')},
  ].filter(g => g.items.length > 0);

  const sortOptions = [
    ...(state.q ? [{value: 'relevance', label: 'Best match'}] : []),
    {value: 'newest', label: 'Newest'},
    {value: 'price', label: 'Price: low to high'},
    {value: '-price', label: 'Price: high to low'},
  ];

  function labelOf(f: Facet): string {
    if (f.label) return f.label;
    if (f.name === 'category') return categoryNames[f.value] ?? words(f.value.replace(/^cat_/, ''));
    if (f.name === 'price_range') return priceRangeLabel(f.value, currency);
    if (f.name === 'availability') return words(f.value);
    return f.value;
  }

  function active(f: Facet): boolean {
    if (f.name === 'category') return state.category_id === f.value;
    if (f.name === 'availability') return (state.availability ?? []).includes(f.value);
    if (f.name === 'price_range') {
      const [lo, hi] = f.value.split('-');
      return (state.min_price ?? '') === (lo ?? '') && (state.max_price ?? '') === (hi ?? '');
    }
    return (state.q ?? '').toLowerCase().split(/\s+/).includes(f.value.toLowerCase());
  }

  function change(f: Facet, on: boolean): Partial<SearchParamsState> {
    if (f.name === 'category') return {category_id: on ? f.value : undefined};
    if (f.name === 'availability') {
      const cur = state.availability ?? [];
      return {availability: on ? [...cur, f.value] : cur.filter(a => a !== f.value)};
    }
    if (f.name === 'price_range') {
      const [lo, hi] = f.value.split('-');
      return on ? {min_price: lo || undefined, max_price: hi || undefined, currency} : {min_price: undefined, max_price: undefined};
    }
    const q = (state.q ?? '').split(/\s+/).filter(Boolean);
    return {q: on ? [...q, f.value].join(' ') : q.filter(w => w.toLowerCase() !== f.value.toLowerCase()).join(' ')};
  }

  return (
    <VStack gap={3}>
      <HStack gap={3} vAlign="end" wrap="wrap" justify="between">
        <Selector
          label="Sort by"
          size="sm"
          variant="secondary"
          options={sortOptions}
          value={state.sort ?? (state.q ? 'relevance' : 'newest')}
          onChange={v => go({sort: v === 'relevance' ? undefined : v})}
        />
      </HStack>
      {groups.length > 0 ? (
        <Collapsible trigger={<Text type="label">Filters</Text>} defaultIsOpen>
          <VStack gap={3} paddingBlockStart={2}>
            {groups.map(g => (
              <VStack key={g.name} gap={1}>
                <Text type="supporting" color="secondary">
                  {g.title}
                </Text>
                <HStack gap={1} wrap="wrap">
                  {g.items.map(f => {
                    const on = active(f);
                    return (
                      <FacetToken
                        key={`${f.name}:${f.value}`}
                        facet={f}
                        label={labelOf(f)}
                        isActive={on}
                        href={hrefWith(pathname, state, change(f, true))}
                        onRemove={() => go(change(f, false))}
                      />
                    );
                  })}
                </HStack>
              </VStack>
            ))}
          </VStack>
        </Collapsible>
      ) : null}
    </VStack>
  );
}
