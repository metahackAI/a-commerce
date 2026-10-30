import {DataDictionary} from '@/components/bo/DataDictionary';
import {ErrorState} from '@/components/bo/ErrorState';
import {FilterBar} from '@/components/bo/FilterBar';
import {PageFrame} from '@/components/bo/PageFrame';

import {param, read, type SearchParams} from '../../_lib/server';
import type {DataTable, Page} from '../../_lib/types';

export const metadata = {title: 'Data dictionary'};

const SCHEMAS = ['platform', 'catalog', 'merchandising', 'search', 'promotion', 'customer', 'sales', 'fulfillment', 'messaging', 'analytics'];

// The data dictionary (describe_data): what every table and column means, by schema.org term.
export default async function DataPage({searchParams}: {searchParams: SearchParams}) {
  const sp = await searchParams;
  const values = {
    term: param(sp, 'term'),
    schema: param(sp, 'schema'),
    table: param(sp, 'table'),
    api_name: param(sp, 'api_name'),
  };
  const filtered = Object.values(values).some(Boolean);
  const list = await read<Page<DataTable>>('/v1/platform/schema', values);

  return (
    <PageFrame
      title="Data dictionary"
      description="Every table and column in the store’s database: its schema.org term, what it means, the values it allows, and its name in the API."
      toolbar={
        <FilterBar
          values={values}
          filters={[
            {kind: 'text', key: 'term', label: 'Term', placeholder: 'orderStatus or schema:Order', width: 220},
            {kind: 'select', key: 'schema', label: 'Schema', options: SCHEMAS.map(s => ({value: s, label: s}))},
            {kind: 'text', key: 'table', label: 'Table', placeholder: 'sales.sales_order', width: 200},
            {kind: 'text', key: 'api_name', label: 'API name', placeholder: 'variant_id', width: 160},
          ]}
        />
      }>
      {list.ok ? <DataDictionary tables={list.data.items} open={filtered && list.data.items.length <= 3} /> : <ErrorState error={list.error} title="The data dictionary could not be loaded." />}
    </PageFrame>
  );
}
