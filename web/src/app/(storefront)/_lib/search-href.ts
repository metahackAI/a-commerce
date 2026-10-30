// Search and category page links from the current filters. Shared by the server page (paging links)
// and the client controls, so it lives outside any 'use client' module.

export interface SearchParamsState {
  q?: string;
  category_id?: string;
  sort?: string;
  availability?: string[];
  min_price?: string;
  max_price?: string;
  currency?: string;
}

export function hrefWith(pathname: string, state: SearchParamsState, change: Partial<SearchParamsState>): string {
  const next: SearchParamsState = {...state, ...change};
  const p = new URLSearchParams();
  if (next.q) p.set('q', next.q);
  if (next.category_id) p.set('category_id', next.category_id);
  if (next.sort) p.set('sort', next.sort);
  for (const a of next.availability ?? []) p.append('availability', a);
  if (next.min_price) p.set('min_price', next.min_price);
  if (next.max_price) p.set('max_price', next.max_price);
  if ((next.min_price || next.max_price) && next.currency) p.set('currency', next.currency);
  const s = p.toString();
  return s ? `${pathname}?${s}` : pathname;
}
