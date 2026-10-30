import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';
import {ProductView} from '@/components/bo/ProductView';

import {param, read, readOptional, type SearchParams} from '../../../_lib/server';
import type {Brand, Category, HistoryEntry, Inventory, Page, Product, ProductGroup, ReviewSummary} from '../../../_lib/types';

export const metadata = {title: 'Product'};

// One product: get_product (sales view, cost for operators), get_product_group (catalog view),
// get_inventory, get_review_summary and get_change_history, read together; categories and brands
// for the editor (update_product_group, set_variants). `?edit=variants` opens the editor.
export default async function ProductPage({params, searchParams}: {params: Promise<{product_id: string}>; searchParams: SearchParams}) {
  const {product_id: id} = await params;
  const sp = await searchParams;
  const edit = param(sp, 'edit');
  const enc = encodeURIComponent(id);
  const [product, group, inventory, reviews, history, categories, brands] = await Promise.all([
    read<Product>(`/v1/catalog/products/${enc}`),
    read<ProductGroup>(`/v1/catalog/product-groups/${enc}`),
    read<Inventory>(`/v1/catalog/products/${enc}/inventory`),
    read<ReviewSummary>(`/v1/catalog/products/${enc}/reviews/summary`),
    read<Page<HistoryEntry>>(`/v1/catalog/products/${enc}/history`, {limit: 50}),
    readOptional<Page<Category>>('/v1/merchandising/categories', {term_set: 'standard', limit: 100}),
    readOptional<Page<Brand>>('/v1/catalog/brands', {limit: 100}),
  ]);

  if (!product.ok && !group.ok) {
    return (
      <PageFrame back={{href: '/bo/products', label: 'Products'}} title={id}>
        <ErrorState error={product.error} title="This product could not be loaded." />
      </PageFrame>
    );
  }

  return (
    <ProductView
      productId={id}
      product={product.ok ? product.data : null}
      productError={product.ok ? null : product.error}
      group={group.ok ? group.data : null}
      inventory={inventory.ok ? inventory.data : null}
      inventoryError={inventory.ok ? null : inventory.error}
      reviews={reviews.ok ? reviews.data : null}
      reviewsError={reviews.ok ? null : reviews.error}
      history={history.ok ? history.data.items : null}
      historyError={history.ok ? null : history.error}
      categories={categories?.items ?? []}
      brands={brands?.items ?? []}
      initialEdit={edit === 'variants' || edit === 'details' ? edit : null}
    />
  );
}
