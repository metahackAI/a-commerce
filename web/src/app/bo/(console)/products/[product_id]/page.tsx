import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';
import {ProductView} from '@/components/bo/ProductView';

import {read} from '../../../_lib/server';
import type {HistoryEntry, Inventory, Page, Product, ProductGroup, ReviewSummary} from '../../../_lib/types';

export const metadata = {title: 'Product'};

// One product: get_product (sales view, cost for operators), get_product_group (catalog view),
// get_inventory, get_review_summary and get_change_history, read together.
export default async function ProductPage({params}: {params: Promise<{product_id: string}>}) {
  const {product_id: id} = await params;
  const enc = encodeURIComponent(id);
  const [product, group, inventory, reviews, history] = await Promise.all([
    read<Product>(`/v1/catalog/products/${enc}`),
    read<ProductGroup>(`/v1/catalog/product-groups/${enc}`),
    read<Inventory>(`/v1/catalog/products/${enc}/inventory`),
    read<ReviewSummary>(`/v1/catalog/products/${enc}/reviews/summary`),
    read<Page<HistoryEntry>>(`/v1/catalog/products/${enc}/history`, {limit: 50}),
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
    />
  );
}
