// A collection page's first products, for the small pictures on its card: get_curation for the
// sections, then get_product for each of the first few products (the shopper's own prices).
import 'server-only';

import {slotItemId} from './format';
import {load} from './server';
import type {CollectionPage, CollectionPageSummary, Product, ProductSummary} from './types';

/** Products a collection card shows. */
const PREVIEW = 4;

export interface CollectionPreview {
  page: CollectionPageSummary;
  products: ProductSummary[];
}

export {slotItemId};

/** The products of a collection page in section order, without the ones the rules hide. */
export function collectionProductIds(page: CollectionPage): string[] {
  const hidden = new Set((page.hidden_items ?? []).map(h => h.product_id));
  const ids = page.has_part.flatMap(part => part.item_list_element.map(slotItemId));
  return [...new Set(ids.filter((id): id is string => !!id && id.startsWith('prd_') && !hidden.has(id)))];
}

export async function collectionPreviews(pages: CollectionPageSummary[]): Promise<CollectionPreview[]> {
  return Promise.all(
    pages.map(async page => {
      const full = await load<CollectionPage>(`/v1/merchandising/collection-pages/${encodeURIComponent(page.id)}`);
      const ids = full.ok ? collectionProductIds(full.data).slice(0, PREVIEW) : [];
      const loaded = await Promise.all(
        ids.map(id =>
          load<Product>(`/v1/catalog/products/${encodeURIComponent(id)}`, {fields: ['id', 'name', 'image', 'price', 'price_currency', 'member_price']}),
        ),
      );
      return {page, products: loaded.flatMap(l => (l.ok ? [l.data] : []))};
    }),
  );
}
