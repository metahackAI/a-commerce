import {FilterBar} from '@/components/bo/FilterBar';
import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';
import {Pager} from '@/components/bo/Pager';
import {ProductList} from '@/components/bo/lists/catalog';

import {param, read, readOptional, type SearchParams} from '../../_lib/server';
import type {Category, Page, ProductGroupSummary} from '../../_lib/types';

export const metadata = {title: 'Products'};

const opts = (xs: string[]) => xs.map(v => ({value: v, label: v.charAt(0).toUpperCase() + v.slice(1).replace(/_/g, ' ')}));

// Products (search_product_groups): the catalog manager's search, drafts and products in review
// included. Categories for the filter come from list_categories.
export default async function ProductsPage({searchParams}: {searchParams: SearchParams}) {
  const sp = await searchParams;
  const values = {
    query: param(sp, 'query'),
    approval_status: param(sp, 'approval_status'),
    availability: param(sp, 'availability'),
    category_id: param(sp, 'category_id'),
    brand_id: param(sp, 'brand_id'),
    sort: param(sp, 'sort'),
  };
  const cursor = param(sp, 'cursor');
  const [list, categories] = await Promise.all([
    read<Page<ProductGroupSummary>>('/v1/catalog/product-groups', {...values, limit: 50, cursor}),
    readOptional<Page<Category>>('/v1/merchandising/categories', {term_set: 'standard', limit: 100}),
  ]);

  return (
    <PageFrame
      title="Products"
      description="Search the catalog, open a product to see its prices, stock, reviews and history, and change a variant’s price or stock."
      toolbar={
        <FilterBar
          values={values}
          filters={[
            {kind: 'text', key: 'query', label: 'Search', placeholder: 'Name or code'},
            {kind: 'select', key: 'approval_status', label: 'Approval', options: opts(['draft', 'pending_review', 'active', 'rejected', 'discontinued'])},
            {kind: 'select', key: 'availability', label: 'Availability', options: opts(['in_stock', 'out_of_stock', 'pre_order', 'back_order', 'discontinued'])},
            ...(categories && categories.items.length > 0
              ? [{kind: 'select' as const, key: 'category_id', label: 'Category', options: categories.items.map(c => ({value: c.id, label: c.name}))}]
              : []),
            {kind: 'text', key: 'brand_id', label: 'Brand id', placeholder: 'brn_…', width: 160},
            {
              kind: 'select',
              key: 'sort',
              label: 'Sort',
              placeholder: 'Relevance',
              options: [
                {value: 'relevance', label: 'Relevance'},
                {value: 'newest', label: 'Newest'},
                {value: 'price', label: 'Price, low to high'},
                {value: '-price', label: 'Price, high to low'},
              ],
            },
          ]}
        />
      }>
      {list.ok ? (
        <>
          <ProductList items={list.data.items} />
          <Pager base="/bo/products" params={{...values, cursor}} shown={list.data.items.length} total={list.data.total} nextCursor={list.data.next_cursor} noun="products" />
        </>
      ) : (
        <ErrorState error={list.error} title="The products could not be loaded." />
      )}
    </PageFrame>
  );
}
