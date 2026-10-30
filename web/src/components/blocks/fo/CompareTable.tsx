'use client';

// compare_table: products side by side on the same attributes (chat.md §3.3, compare_products).
// Products are columns, attributes are rows; the table scrolls sideways on a phone. Every column
// and row is an AI target, so "why does weight matter?" or "sort by price" point at something.

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {Table, TableBody, TableCell, TableHeader, TableHeaderCell, TableRow} from '@astryxdesign/core/Table';
import {Text} from '@astryxdesign/core/Text';

import type {BlockAction, UiBlock} from '@/ai/types';
import {useAiTarget} from '@/ai/useAiTarget';
import {imageUrl, words} from '@/app/(storefront)/_lib/format';
import type {ImageRef} from '@/app/(storefront)/_lib/types';
import {Intents} from '@/components/fo/Ask';
import {ProductImage} from '@/components/fo/ProductImage';

import type {BlockProps} from '../types';
import {ActionButton, BlockActions, BlockFrame, dataOf, intentsFor, list} from './common';
import type {CompareTableData} from './data';
import {moneyText, textOf} from './values';
import {StatusTag} from '@/components/fo/StatusTag';

type Product = CompareTableData['products'][number];

function ColumnHeader({block, product}: {block: UiBlock; product: Product}) {
  const target = useAiTarget({
    kind: 'compare_column',
    label: product.name,
    ids: {product_id: product.product_id},
    term: 'schema:Product',
    blockId: block.block_id,
  });
  const src = imageUrl(product.image as ImageRef);
  const sub = [product.color, product.material].filter(Boolean).join(' · ');
  return (
    <TableHeaderCell scope="col" ref={target.ref} {...target.props}>
      <VStack gap={1.5}>
        {product.badge ? (
          <HStack>
            <StatusTag label={product.badge} tone="info" />
          </HStack>
        ) : null}
        {src ? <ProductImage src={src} alt={product.name} ratio={1} /> : null}
        <Link href={`/p/${encodeURIComponent(product.product_id)}`}>
          <Text type="body" weight="semibold">
            {product.name}
          </Text>
        </Link>
        {sub ? (
          <Text type="supporting" color="secondary">
            {sub}
          </Text>
        ) : null}
        <Intents says={intentsFor(block, 'compare_column', product.product_id)} />
      </VStack>
    </TableHeaderCell>
  );
}

function AttributeRow({block, row, count}: {block: UiBlock; row: CompareTableData['rows'][number]; count: number}) {
  const target = useAiTarget({kind: 'compare_row', label: row.label, ids: {row: row.label}, blockId: block.block_id});
  const values = list(row.values);
  const says = intentsFor(block, 'compare_row', row.label);
  return (
    <TableRow ref={target.ref} {...target.props}>
      <TableCell scope="row">
        <VStack gap={1}>
          <Text type="supporting" color="secondary">
            {row.label}
          </Text>
          <Intents says={says} />
        </VStack>
      </TableCell>
      {Array.from({length: count}, (_, i) => (
        <TableCell key={i}>
          <Text type="body">{textOf(values[i]) || 'Not listed'}</Text>
        </TableCell>
      ))}
    </TableRow>
  );
}

function PriceCell({block, product, price}: {block: UiBlock; product: Product; price: CompareTableData['prices'][number] | undefined}) {
  const shown = moneyText(price?.price);
  const list_ = moneyText(price?.list_price);
  const target = useAiTarget({
    kind: 'price',
    label: `${product.name} · ${shown}${price?.label ? ` (${price.label})` : ''}`,
    ids: {product_id: product.product_id},
    term: 'schema:Offer',
    blockId: block.block_id,
  });
  return (
    <TableCell>
      <VStack gap={0.5} ref={target.ref} {...target.props}>
        <HStack gap={2} vAlign="center" wrap="wrap">
          <Text type="body" weight="semibold" hasTabularNumbers>
            {shown || '—'}
          </Text>
          {list_ && list_ !== shown ? (
            <Text type="supporting" color="secondary" hasStrikethrough hasTabularNumbers>
              {list_}
            </Text>
          ) : null}
        </HStack>
        {price?.label ? (
          <Text type="supporting" color="secondary">
            {price.label}
          </Text>
        ) : null}
      </VStack>
    </TableCell>
  );
}

/** The product a block button is about, from its args. */
function productOf(action: BlockAction): string | undefined {
  if (action.kind !== 'say') return undefined;
  const id = action.args?.product_id;
  return typeof id === 'string' ? id : undefined;
}

export function CompareTableBlock({block}: BlockProps) {
  const data = dataOf<CompareTableData>(block);
  const products = list(data.products);
  const rows = list(data.rows);
  const prices = list(data.prices);
  const perProduct = block.actions.filter(a => productOf(a));
  const general = {...block, actions: block.actions.filter(a => !productOf(a))};
  const aside = [
    products.length > 0 ? `${products.length} compared` : null,
    data.sorted_by ? `sorted by ${words(data.sorted_by).toLowerCase()}` : null,
  ]
    .filter(Boolean)
    .join(' · ');

  return (
    <BlockFrame block={block} aside={aside} footer={<BlockActions block={general} />}>
      <Table density="balanced" verticalAlign="top">
        <TableHeader>
          <TableRow>
            <TableHeaderCell scope="col">
              <Text type="supporting" color="secondary">
                Compared
              </Text>
            </TableHeaderCell>
            {products.map(p => (
              <ColumnHeader key={p.product_id} block={block} product={p} />
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map(r => (
            <AttributeRow key={r.label} block={block} row={r} count={products.length} />
          ))}
          {prices.length > 0 ? (
            <TableRow>
              <TableCell scope="row">
                <Text type="supporting" color="secondary">
                  Your price
                </Text>
              </TableCell>
              {products.map((p, i) => (
                <PriceCell key={p.product_id} block={block} product={p} price={prices[i]} />
              ))}
            </TableRow>
          ) : null}
          {perProduct.length > 0 ? (
            <TableRow>
              <TableCell scope="row" />
              {products.map(p => (
                <TableCell key={p.product_id}>
                  <HStack gap={1} wrap="wrap">
                    {perProduct
                      .filter(a => productOf(a) === p.product_id)
                      .map(a => (
                        <ActionButton blockId={block.block_id} key={a.action_id} action={a} />
                      ))}
                  </HStack>
                </TableCell>
              ))}
            </TableRow>
          ) : null}
        </TableBody>
      </Table>
    </BlockFrame>
  );
}
