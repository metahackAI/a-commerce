import type {ReactNode} from 'react';

import {AiProvider} from '@/ai/AiProvider';
import type {Page} from '@/lib/api';
import {StorefrontShell, type NavCategory} from '@/components/fo/StorefrontShell';

import {load, orElse, shopper} from './_lib/server';
import type {Cart, Category, Profile} from './_lib/types';

// Every storefront page reads the shopper's own session, cart and prices.
export const dynamic = 'force-dynamic';

// Storefront (/): shoppers browse and talk; the assistant prepares, the shopper decides.
export default async function StorefrontLayout({children}: {children: ReactNode}) {
  const who = await shopper();
  const [cart, categories, profile] = await Promise.all([
    load<Cart>('/v1/sales/carts/mine', {fields: ['lines']}),
    load<Page<Category>>('/v1/merchandising/categories', {term_set: 'display', limit: 50}),
    who.isShopper ? load<Profile>('/v1/customer/people/me', {fields: ['id', 'name', 'tier_id']}) : null,
  ]);

  const nav: NavCategory[] = orElse(categories, {items: [], total: 0})
    .items.filter(c => c.active !== false && !c.parent_id)
    .sort((a, b) => (a.display_order ?? 0) - (b.display_order ?? 0))
    .slice(0, 5)
    .map(c => ({id: c.id, name: c.name}));
  const p = profile && profile.ok ? profile.data : null;

  return (
    <AiProvider surface="fo">
      <StorefrontShell
        shopper={{
          session: who.session,
          isShopper: who.isShopper,
          isGuest: who.isGuest,
          name: p?.name ?? who.session.user?.name ?? null,
          tierId: p?.tier_id ?? null,
        }}
        cartLines={cart.ok ? cart.data.lines.length : 0}
        categories={nav}>
        {children}
      </StorefrontShell>
    </AiProvider>
  );
}
