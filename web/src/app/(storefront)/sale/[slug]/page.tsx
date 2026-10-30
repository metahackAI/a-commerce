import type {Metadata} from 'next';
import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {Layout, LayoutContent} from '@astryxdesign/core/Layout';

import {ErrorNotice} from '@/components/fo/ErrorNotice';
import type {Page} from '@/lib/api';

import {dateTime, words} from '../../_lib/format';
import {ResultsView, parseSearchParams, type RawParams} from '../../_lib/ResultsView';
import {load} from '../../_lib/server';

// A sale's page, where its banner links (`/sale/vip-weekend`). The promotion comes from the sales
// running now for this shopper (list_active_promotions answers a shopper with only those); the
// products are its category with the shopper's own prices, as search_products computes them.

type Params = Promise<{slug: string}>;

interface ActivePromotion {
  id: string;
  name: string;
  valid_for_member_tier: string;
  valid_through: string | null;
  applies_to?: {scope: string; category_ids?: string[]; product_count?: number | null};
}

function slug(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '');
}

async function saleFor(s: string) {
  // The store's own clock (the demo clock in demo mode), never the browser's or the server's.
  const store = await load<{now: string}>('/v1/platform/store');
  const now = store.ok ? store.data.now.slice(0, 16) : new Date().toISOString().slice(0, 16);
  const active = await load<Page<ActivePromotion>>('/v1/promotion/promotions/active', {
    valid_from: now,
    valid_through: now,
    limit: 50,
  });
  if (!active.ok) return {error: active.error, promotion: null};
  return {error: null, promotion: active.data.items.find(p => slug(p.name) === s || p.id === s) ?? null};
}

export async function generateMetadata({params}: {params: Params}): Promise<Metadata> {
  const {promotion} = await saleFor((await params).slug);
  return {title: promotion?.name ?? 'Sale'};
}

export default async function SalePage({params, searchParams}: {params: Params; searchParams: Promise<RawParams>}) {
  const [{slug: s}, raw] = await Promise.all([params, searchParams]);
  const {error, promotion} = await saleFor(s);
  // A sale over a category (or chosen products of one, as when low stock is left out) shows that
  // category; each card carries the shopper's own price.
  const categoryId = promotion?.applies_to?.category_ids?.[0];
  if (!promotion || !categoryId) {
    return (
      <Layout
        height="auto"
        contentWidth={1200}
        content={
          <LayoutContent padding={6}>
            {error ? (
              <ErrorNotice error={error} title="This sale could not be loaded." />
            ) : (
              <Banner
                status="info"
                title="This sale is not on for you right now"
                description="It may be for members of another tier, or not started yet. Sign in to see the prices for your tier."
                endContent={<Button label="Shop all" size="sm" href="/search" />}
              />
            )}
          </LayoutContent>
        }
      />
    );
  }
  const who = promotion.valid_for_member_tier === 'everyone' ? 'Everyone' : `${words(promotion.valid_for_member_tier)} members`;
  const until = promotion.valid_through ? ` · until ${dateTime(promotion.valid_through)}` : '';
  return (
    <ResultsView
      pathname={`/sale/${encodeURIComponent(s)}`}
      state={{...parseSearchParams(raw), category_id: undefined}}
      title={promotion.name}
      subtitle={`${who}${until} · prices below are yours`}
      fixedCategoryId={categoryId}
    />
  );
}
