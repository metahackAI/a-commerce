'use client';

// `product_table` block (docs/api/chat.md §3.3): products the agent read, in the columns it chose.
// Each row is a product people can ask about or open.

import {Text} from '@astryxdesign/core/Text';

import type {ProductTableBlockData} from '@/app/bo/_lib/blocks';
import {count, words} from '@/app/bo/_lib/format';

import type {BlockProps} from '../types';
import {BlockFrame, intentsLookup} from './parts';
import {RowsTable, type RowColumn} from './rows';

const NAME = new Set(['name', 'title', 'label', 'item', 'product_id', 'id']);

export function ProductTableBlock({block}: BlockProps) {
  const d = block.data as ProductTableBlockData;
  const intentsFor = intentsLookup(block);
  const rows = d.rows ?? [];
  const columns: RowColumn[] | undefined = d.columns
    ?.map(c => (typeof c === 'string' ? {key: c, label: words(c)} : {key: c.key, label: c.label ?? c.header ?? words(c.key)}))
    .filter(c => !NAME.has(c.key));

  return (
    <BlockFrame block={block} kind="product_table" id={block.block_id} label="Products">
      <RowsTable rows={rows} columns={columns} kind="product_row" blockId={block.block_id} intentsFor={intentsFor} empty="No products" />
      {typeof d.total === 'number' ? (
        <Text type="supporting" color="secondary">
          {`${count(rows.length)} shown of ${count(d.total)}`}
        </Text>
      ) : null}
    </BlockFrame>
  );
}
