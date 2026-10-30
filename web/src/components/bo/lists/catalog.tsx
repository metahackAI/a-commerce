'use client';

// Lists of the catalog domain.

import {count, price} from '@/app/bo/_lib/format';
import type {ProductGroupSummary} from '@/app/bo/_lib/types';
import {Text} from '@astryxdesign/core/Text';

import {StateToken} from '../StateToken';
import {DataTable, Figure, RecordCell} from '../tables';

export function ProductList({items}: {items: ProductGroupSummary[]}) {
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
        {header: 'Product', cell: r => <RecordCell href={`/bo/products/${encodeURIComponent(r.id)}`} name={r.name} id={r.id} />},
        {header: 'Approval', cell: r => (r.approval_status ? <StateToken state={r.approval_status} /> : <Text color="secondary">—</Text>)},
        {header: 'Availability', cell: r => (r.availability ? <StateToken state={r.availability} /> : <Text color="secondary">—</Text>)},
        {header: 'List price', cell: r => <Figure>{price(r.price, r.price_currency)}</Figure>},
        {header: 'Stock', cell: r => <Figure>{count(r.inventory_level)}</Figure>},
        {header: 'Category', cell: r => <Text color="secondary">{r.category_id ?? '—'}</Text>},
        {header: 'Brand', cell: r => <Text color="secondary">{r.brand_id ?? '—'}</Text>},
      ]}
    />
  );
}
