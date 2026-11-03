'use client';

// Sorting and filters for search and category pages. Filters come from `get_facets` (search.md)
// with the store's counts; choosing one changes the URL, and the page reads search_products again.
// search_products filters by category, availability and list-price range; brand, color and size
// facets narrow the search words instead.
//
// The filters sit in a panel on the left that stays in view while the results scroll (on a phone,
// a collapsible section above them). Each group shows its first few values and the rest behind
// "More"; a chosen value always shows.

import {Button} from '@astryxdesign/core/Button';
import {CheckboxInput} from '@astryxdesign/core/CheckboxInput';
import {Collapsible} from '@astryxdesign/core/Collapsible';
import {Divider} from '@astryxdesign/core/Divider';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {useMediaQuery} from '@astryxdesign/core/hooks';
import {usePathname, useRouter} from 'next/navigation';
import {useState, type ReactNode} from 'react';

import {useAiTarget} from '@/ai/useAiTarget';
import {price, words} from '@/app/(storefront)/_lib/format';
import type {Facet} from '@/app/(storefront)/_lib/types';
import {hrefWith, type SearchParamsState} from '@/app/(storefront)/_lib/search-href';

/** Values a group shows before "More". */
const VISIBLE = 5;
const PANEL_WIDTH = 240;
/** Where the filter panel stops fitting beside the results. */
const PHONE = '(max-width: 767px)';

function priceRangeLabel(value: string, currency: string): string {
  const [lo, hi] = value.split('-');
  const a = Number(lo);
  const b = Number(hi);
  if (hi === undefined || hi === '' || Number.isNaN(b)) return `${price(a, currency)} and up`;
  if (lo === '' || Number.isNaN(a)) return `Up to ${price(b, currency)}`;
  return `${price(a, currency)} – ${price(b, currency)}`;
}

// ---------------------------------------------------------------- sort

export function SortBar({state}: {state: SearchParamsState}) {
  const router = useRouter();
  const pathname = usePathname();
  const current = state.sort ?? (state.q ? 'relevance' : 'newest');
  const options = [
    ...(state.q ? [{value: 'relevance', label: 'Best match'}] : []),
    {value: 'newest', label: 'Newest'},
    {value: 'price', label: 'Price: low to high'},
    {value: '-price', label: 'Price: high to low'},
  ];
  return (
    <HStack gap={1} wrap="wrap" vAlign="center" role="group" aria-label="Sort by">
      {options.map(o => (
        <Button
          key={o.value}
          label={o.label}
          size="sm"
          variant={o.value === current ? 'secondary' : 'ghost'}
          aria-pressed={o.value === current}
          onClick={() => router.push(hrefWith(pathname, state, {sort: o.value === 'relevance' ? undefined : o.value}))}
        />
      ))}
    </HStack>
  );
}

// ---------------------------------------------------------------- filters

function FacetCheckbox({facet, label, isActive, onChange}: {facet: Facet; label: string; isActive: boolean; onChange: (on: boolean) => void}) {
  const target = useAiTarget({kind: 'chip', label: `${words(facet.name)}: ${label}`, ids: {facet: `${facet.name}:${facet.value}`}});
  return (
    <div ref={target.ref} {...target.props}>
      <CheckboxInput size="sm" label={`${label} (${facet.count})`} value={isActive} onChange={on => onChange(on)} />
    </div>
  );
}

function FacetGroup({
  title,
  items,
  render,
  onChange,
}: {
  title: string;
  items: Facet[];
  render: (f: Facet) => {label: string; active: boolean};
  onChange: (f: Facet, on: boolean) => void;
}) {
  const [open, setOpen] = useState(false);
  const shown = open ? items : items.filter((f, i) => i < VISIBLE || render(f).active);
  const hiddenCount = items.length - shown.length;
  return (
    <VStack gap={1}>
      <Text type="label" weight="semibold">
        {title}
      </Text>
      {shown.map(f => {
        const {label, active} = render(f);
        return <FacetCheckbox key={`${f.name}:${f.value}`} facet={f} label={label} isActive={active} onChange={on => onChange(f, on)} />;
      })}
      {items.length > VISIBLE ? (
        <HStack>
          <Button
            label={open ? 'Show less' : `More (${hiddenCount})`}
            size="sm"
            variant="ghost"
            aria-expanded={open}
            onClick={() => setOpen(o => !o)}
          />
        </HStack>
      ) : null}
    </VStack>
  );
}

export function FilterPanel({
  state,
  facets,
  categoryNames,
  currency,
  showCategoryFacet,
  clearHref,
}: {
  state: SearchParamsState;
  facets: Facet[];
  categoryNames: Record<string, string>;
  currency: string;
  showCategoryFacet: boolean;
  /** The page with the search words kept and every filter cleared. */
  clearHref: string;
}) {
  const router = useRouter();
  const pathname = usePathname();

  const groups: {name: string; title: string; items: Facet[]}[] = [
    {name: 'category', title: 'Category', items: showCategoryFacet ? facets.filter(f => f.name === 'category') : []},
    {name: 'price_range', title: 'Price', items: facets.filter(f => f.name === 'price_range')},
    {name: 'availability', title: 'Availability', items: facets.filter(f => f.name === 'availability')},
    {name: 'brand', title: 'Brand', items: facets.filter(f => f.name === 'brand')},
    {name: 'color', title: 'Color', items: facets.filter(f => f.name === 'color')},
    {name: 'size', title: 'Size', items: facets.filter(f => f.name === 'size')},
  ].filter(g => g.items.length > 0);

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

  const onChange = (f: Facet, on: boolean) => router.push(hrefWith(pathname, state, change(f, on)), {scroll: false});
  const render = (f: Facet) => ({label: labelOf(f), active: active(f)});
  const anyActive = groups.some(g => g.items.some(active));

  if (groups.length === 0) return null;
  return (
    <VStack gap={4}>
      <HStack gap={2} justify="between" vAlign="center">
        {/* Folded on a phone, the fold's own trigger says "Filters". */}
        <span className="acom-filters-title">
          <Text type="label" weight="semibold">
            Filters
          </Text>
        </span>
        {anyActive ? <Button label="Clear all" size="sm" variant="ghost" href={clearHref} /> : null}
      </HStack>
      {groups.map((g, i) => (
        <VStack key={g.name} gap={4}>
          {i > 0 ? <Divider /> : null}
          <FacetGroup title={g.title} items={g.items} render={render} onChange={onChange} />
        </VStack>
      ))}
    </VStack>
  );
}

// ---------------------------------------------------------------- layout

/**
 * The filter panel on the left, kept in view while the results scroll; on a phone the filters
 * fold, closed, into a section above the results, so the results come first.
 */
export function BrowseLayout({filters, children}: {filters: ReactNode; children: ReactNode}) {
  const isPhone = useMediaQuery(PHONE);
  if (isPhone) {
    return (
      <VStack gap={4}>
        {/* One "Filters" title (the fold's), and a fold a thumb can hit, not just its line of text. */}
        <style>{'.acom-filters-folded .acom-filters-title { display: none; } .acom-filters-folded .astryx-collapsible-trigger { min-height: 44px; }'}</style>
        {filters ? (
          <div className="acom-filters-folded">
            <Collapsible trigger={<Text type="label">Filters</Text>} defaultIsOpen={false}>
              {filters}
            </Collapsible>
          </div>
        ) : null}
        {children}
      </VStack>
    );
  }
  return (
    <HStack gap={6} vAlign="start">
      {filters ? (
        <aside
          aria-label="Filters"
          style={{
            width: PANEL_WIDTH,
            flexShrink: 0,
            position: 'sticky',
            top: 'var(--spacing-4)',
            // The window less the store's top bar and a margin; a long panel scrolls on its own.
            maxHeight: 'calc(100vh - 120px)',
            overflowY: 'auto',
          }}>
          {filters}
        </aside>
      ) : null}
      <div style={{flex: 1, minWidth: 0}}>{children}</div>
    </HStack>
  );
}
