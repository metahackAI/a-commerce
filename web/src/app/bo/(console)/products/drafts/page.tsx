import {Button} from '@astryxdesign/core/Button';

import {DraftsReview} from '@/components/bo/DraftsReview';
import {ErrorState} from '@/components/bo/ErrorState';
import {FilterBar} from '@/components/bo/FilterBar';
import {PageFrame} from '@/components/bo/PageFrame';
import {Pager} from '@/components/bo/Pager';

import {param, read, type SearchParams} from '../../../_lib/server';
import type {Page, ProductGroupSummary} from '../../../_lib/types';

export const metadata = {title: 'Product drafts'};

// Product drafts and products in review (search_product_groups by approval state, by the import
// a supplier sheet made, and by what is still open). Drafts are invisible to shoppers.
export default async function DraftsPage({searchParams}: {searchParams: SearchParams}) {
  const sp = await searchParams;
  const values = {
    approval_status: param(sp, 'approval_status') ?? 'draft',
    draft_status: param(sp, 'draft_status'),
    import_id: param(sp, 'import_id'),
    query: param(sp, 'query'),
  };
  const cursor = param(sp, 'cursor');
  const list = await read<Page<ProductGroupSummary>>('/v1/catalog/product-groups', {...values, sort: 'newest', limit: 100, cursor});

  return (
    <PageFrame
      back={{href: '/bo/products', label: 'Products'}}
      title="Product drafts"
      description="Drafts are not visible to shoppers. Fill what a supplier sheet left open, write the description, send each one to review, and publish it in a change set a person approves."
      actions={<Button label="New product" href="/bo/products/new" />}
      toolbar={
        <FilterBar
          values={values}
          filters={[
            {
              kind: 'select',
              key: 'approval_status',
              label: 'State',
              options: [
                {value: 'draft', label: 'Drafts'},
                {value: 'pending_review', label: 'In review'},
                {value: 'rejected', label: 'Sent back'},
              ],
            },
            {
              kind: 'select',
              key: 'draft_status',
              label: 'From a sheet',
              placeholder: 'Any',
              options: [
                {value: 'needs_you', label: 'Needs you'},
                {value: 'ready', label: 'Ready'},
                {value: 'working', label: 'Still working'},
              ],
            },
            {kind: 'text', key: 'import_id', label: 'Import', placeholder: 'imp_…', width: 200},
            {kind: 'text', key: 'query', label: 'Search', placeholder: 'Name or SKU'},
          ]}
        />
      }>
      {list.ok ? (
        <>
          <DraftsReview rows={list.data.items} />
          <Pager base="/bo/products/drafts" params={{...values, cursor}} shown={list.data.items.length} total={list.data.total} nextCursor={list.data.next_cursor} noun="products" />
        </>
      ) : (
        <ErrorState error={list.error} title="The drafts could not be loaded." />
      )}
    </PageFrame>
  );
}
