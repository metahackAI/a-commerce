'use client';

// Side-by-side comparison of the products the shopper picked (compare_products, search.md): one
// column per product, one row per attribute, values and prices exactly as the API wrote them.

import {Button} from '@astryxdesign/core/Button';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {Skeleton} from '@astryxdesign/core/Skeleton';
import {Table, TableBody, TableCell, TableHeader, TableHeaderCell, TableRow} from '@astryxdesign/core/Table';
import {Text} from '@astryxdesign/core/Text';
import {useEffect, useState} from 'react';

import {useAiTarget} from '@/ai/useAiTarget';
import {words} from '@/app/(storefront)/_lib/format';
import type {CompareItem} from '@/app/(storefront)/_lib/types';
import {api, type ApiErrorBody, type Page} from '@/lib/api';

import {AskButton, useSay} from './Ask';
import {ErrorNotice} from './ErrorNotice';
import {PriceTag} from './PriceTag';
import {useStorefront} from './StorefrontContext';
import {errorBody} from './write';

function ColumnHead({item, onRemove}: {item: CompareItem; onRemove: () => void}) {
  const target = useAiTarget({kind: 'compare_column', label: item.name, ids: {product_id: item.product_id}, term: 'schema:Product'});
  return (
    <TableHeaderCell>
      <VStack gap={1} ref={target.ref} {...target.props}>
        <Link href={`/p/${encodeURIComponent(item.product_id)}`}>
          <Text weight="semibold">{item.name}</Text>
        </Link>
        <Text type="supporting" color="secondary">
          {[item.color, item.material].filter(Boolean).join(' · ')}
        </Text>
        <HStack gap={1}>
          <AskButton target={target} label={item.name} />
          <Button label="Remove" size="sm" variant="ghost" onClick={onRemove} />
        </HStack>
      </VStack>
    </TableHeaderCell>
  );
}

function AttributeRow({attribute, items}: {attribute: string; items: CompareItem[]}) {
  const target = useAiTarget({kind: 'compare_row', label: words(attribute), ids: {attribute}});
  return (
    <TableRow ref={target.ref} {...target.props}>
      <TableCell>
        <Text color="secondary">{words(attribute)}</Text>
      </TableCell>
      {items.map(i => (
        <TableCell key={i.product_id}>{i.values.find(v => v.attribute === attribute)?.display ?? 'Not listed'}</TableCell>
      ))}
    </TableRow>
  );
}

export function CompareView({ids: fromUrl}: {ids: string[]}) {
  const {compare, toggleCompare, clearCompare} = useStorefront();
  const say = useSay();
  const ids = fromUrl.length > 0 ? fromUrl : compare.map(c => c.productId);
  const key = ids.join(',');
  const [items, setItems] = useState<CompareItem[] | null>(null);
  const [error, setError] = useState<ApiErrorBody | null>(null);

  useEffect(() => {
    if (ids.length < 2) {
      setItems([]);
      return;
    }
    const ctl = new AbortController();
    setItems(null);
    setError(null);
    api<Page<CompareItem>>('/v1/search/products/compare', {query: {product_ids: key}, signal: ctl.signal})
      .then(r => setItems(r.items))
      .catch(e => {
        if (!ctl.signal.aborted) {
          setError(errorBody(e));
          setItems([]);
        }
      });
    return () => ctl.abort();
    // `key` is `ids` joined, so the read runs once per set of products.
  }, [key, ids.length]);

  if (ids.length < 2) {
    return (
      <EmptyState
        title="Pick two or more products to compare"
        description="Press Compare on any product. Up to six side by side."
        actions={<Button label="Browse products" variant="primary" href="/search" />}
      />
    );
  }
  if (error) return <ErrorNotice error={error} askAbout={`Compare ${compare.map(c => c.name).join(', ')} for me.`} />;
  if (items === null) {
    return (
      <VStack gap={2}>
        <Skeleton height={64} />
        <Skeleton height={240} index={1} />
      </VStack>
    );
  }

  const attributes = [...new Set(items.flatMap(i => i.values.map(v => v.attribute)))].filter(a => a !== 'price');
  return (
    <VStack gap={4}>
      <HStack gap={2} wrap="wrap">
        <Button label="Ask which one fits me" variant="secondary" onClick={() => say(`Which of these fits me best: ${items.map(i => i.name).join(', ')}?`)} />
        <Button label="Clear comparison" variant="ghost" onClick={clearCompare} />
      </HStack>
      <Table density="balanced" verticalAlign="top">
        <TableHeader>
          <TableRow>
            <TableHeaderCell>
              <Text type="supporting" color="secondary">
                {items.length} compared
              </Text>
            </TableHeaderCell>
            {items.map(i => (
              <ColumnHead key={i.product_id} item={i} onRemove={() => toggleCompare({productId: i.product_id, name: i.name})} />
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {attributes.map(a => (
            <AttributeRow key={a} attribute={a} items={items} />
          ))}
          <TableRow>
            <TableCell>
              <Text color="secondary">Your price</Text>
            </TableCell>
            {items.map(i => (
              <TableCell key={i.product_id}>
                <VStack gap={1}>
                  <PriceTag productId={i.product_id} name={i.name} price={i.price} currency={i.price_currency} member={i.member_price} />
                  {i.values.find(v => v.attribute === 'price')?.display ? (
                    <Text type="supporting" color="secondary">
                      {i.values.find(v => v.attribute === 'price')?.display}
                    </Text>
                  ) : null}
                  <Button label="Choose size" size="sm" href={`/p/${encodeURIComponent(i.product_id)}`} />
                </VStack>
              </TableCell>
            ))}
          </TableRow>
        </TableBody>
      </Table>
    </VStack>
  );
}
