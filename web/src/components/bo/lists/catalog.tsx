'use client';

// Lists of the catalog domain.

import {count, price} from '@/app/bo/_lib/format';
import type {ProductGroupSummary} from '@/app/bo/_lib/types';
import {Text} from '@astryxdesign/core/Text';

import {StateToken} from '../StateToken';
import {DataTable, Figure, RecordCell} from '../tables';

/** Category and brand names by id, so rows read as words rather than ids. */
export interface ProductNames {
  categories?: Record<string, string>;
  brands?: Record<string, string>;
}

export function ProductList({items, names = {}}: {items: ProductGroupSummary[]; names?: ProductNames}) {
  return (
    <DataTable
      rows={items}
      rowKey={r => r.id}
      empty={{title: 'No products match', description: 'Try another search or clear the filters.'}}
      target={r => ({
        kind: 'product_row',
        label: `${r.name}${typeof r.price === 'number' ? ` · ${price(r.price, r.price_currency)}` : ''}${typeof r.inventory_level === 'number' ? ` · stock ${r.inventory_level}` : ''}`,
        ids: {product_id: r.id},
        term: 'schema:ProductGroup',
      })}
      columns={[
        {header: 'Product', width: '30%', cell: r => <RecordCell href={`/bo/products/${encodeURIComponent(r.id)}`} name={r.name} id={r.id} image={r.image ?? null} />},
        {header: 'Approval', cell: r => (r.approval_status ? <StateToken state={r.approval_status} /> : <Text color="secondary">—</Text>)},
        {header: 'Availability', cell: r => (r.availability ? <StateToken state={r.availability} /> : <Text color="secondary">—</Text>)},
        {header: 'List price', cell: r => <Figure>{price(r.price, r.price_currency)}</Figure>},
        {header: 'Stock', cell: r => <Figure>{count(r.inventory_level)}</Figure>},
        {header: 'Category', cell: r => <Text color="secondary">{(r.category_id && names.categories?.[r.category_id]) ?? r.category_id ?? '—'}</Text>},
        {header: 'Brand', cell: r => <Text color="secondary">{(r.brand_id && names.brands?.[r.brand_id]) ?? r.brand_id ?? '—'}</Text>},
      ]}
    />
  );
}
