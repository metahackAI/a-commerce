'use client';

// Price and stock of one variant, edited the way every write is: preview → changes and checks →
// apply when no approval is needed. A price change usually needs approval (catalog.yml makes it
// a structural change); the page then says so and the agent can put it in a change set. The
// current price comes from get_price_breakdown, read when the dialog opens.

import {useEffect, useState} from 'react';
import {useRouter} from 'next/navigation';
import {VStack} from '@astryxdesign/core/Layout';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {TabList, Tab} from '@astryxdesign/core/TabList';
import {NumberInput} from '@astryxdesign/core/NumberInput';
import {TextInput} from '@astryxdesign/core/TextInput';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Spinner} from '@astryxdesign/core/Spinner';
import {Text} from '@astryxdesign/core/Text';

import {api, ApiError, type ApiErrorBody} from '@/lib/api';
import {count, price} from '@/app/bo/_lib/format';
import type {PriceBreakdown, Variant} from '@/app/bo/_lib/types';

import {ErrorState} from './ErrorState';
import {TargetField} from './targets';
import {WriteFlow} from './WriteFlow';

export type EditMode = 'price' | 'stock';

function toError(e: unknown): ApiErrorBody {
  if (e instanceof ApiError) return e.body;
  return {code: 'upstream_unavailable', class: 'upstream_unavailable', message: 'The store could not be reached.', next_tool: null};
}

/** get_price_breakdown for one variant, read in the browser. */
export function usePriceBreakdown(variantId: string | null) {
  const [data, setData] = useState<PriceBreakdown | null>(null);
  const [error, setError] = useState<ApiErrorBody | null>(null);
  const [loading, setLoading] = useState(false);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!variantId) return;
    let alive = true;
    setLoading(true);
    setError(null);
    api<PriceBreakdown>(`/v1/catalog/variants/${encodeURIComponent(variantId)}/price-breakdown`)
      .then(b => alive && setData(b))
      .catch(e => alive && setError(toError(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [variantId, attempt]);
  return {data, error, loading, retry: () => setAttempt(a => a + 1)};
}

export function BreakdownFacts({b}: {b: PriceBreakdown}) {
  return (
    <MetadataList columns="multi">
      <MetadataListItem label="List price">{price(b.list_price, b.price_currency)}</MetadataListItem>
      <MetadataListItem label="Member price now">{b.member_price !== null ? price(b.member_price, b.price_currency) : 'None'}</MetadataListItem>
      {b.cost !== undefined ? <MetadataListItem label="Cost">{price(b.cost, b.price_currency)}</MetadataListItem> : null}
      {b.price_floor !== undefined ? <MetadataListItem label="Price floor">{price(b.price_floor, b.price_currency)}</MetadataListItem> : null}
      <MetadataListItem label="From promotions">{b.applied.length ? b.applied.map(a => a.name).join(', ') : 'None'}</MetadataListItem>
    </MetadataList>
  );
}

export function VariantEditor({
  productName,
  variant,
  mode,
  onModeChange,
  onClose,
  canWrite,
}: {
  productName: string;
  variant: Variant & {'acom:safety_stock'?: number};
  mode: EditMode;
  onModeChange: (m: EditMode) => void;
  onClose: () => void;
  canWrite: boolean;
}) {
  const router = useRouter();
  const breakdown = usePriceBreakdown(variant.variant_id);
  const [newPrice, setNewPrice] = useState<number | null>(null);
  const [stock, setStock] = useState<number | null>(null);
  const [reason, setReason] = useState('');
  const currency = breakdown.data?.price_currency ?? null;
  const vid = encodeURIComponent(variant.variant_id);
  const name = `${productName}${variant.size || variant.color ? ` (${[variant.size, variant.color].filter(Boolean).join(', ')})` : ''}`;

  return (
    <Dialog isOpen onOpenChange={open => !open && onClose()} width={720} purpose="form">
      <DialogHeader title={name} subtitle={`${variant.variant_id}${variant.sku ? ` · SKU ${variant.sku}` : ''}`} onOpenChange={open => !open && onClose()} />
      <VStack gap={4} padding={4}>
        <TabList value={mode} onChange={v => onModeChange(v === 'stock' ? 'stock' : 'price')} hasDivider>
          <Tab value="price" label="Price" />
          <Tab value="stock" label="Stock" />
        </TabList>

        {mode === 'price' ? (
          <VStack gap={4}>
            {breakdown.loading ? <Spinner size="sm" label="Reading the current price…" /> : null}
            {breakdown.error ? <ErrorState error={breakdown.error} title="The current price could not be read." onRetry={breakdown.retry} /> : null}
            {breakdown.data ? <BreakdownFacts b={breakdown.data} /> : null}
            {!canWrite ? (
              <Text color="secondary">Your account can’t change catalog prices.</Text>
            ) : currency ? (
              <WriteFlow
                method="PUT"
                path={`/v1/catalog/variants/${vid}/price`}
                input={newPrice === null ? null : {variant_id: variant.variant_id, price: newPrice, price_currency: currency}}
                previewLabel="Preview the price"
                applyLabel="Apply the price"
                askWhenApprovalNeeded={`Prepare a change set to set the list price of ${name} (${variant.variant_id}) to ${price(newPrice, currency)}.`}
                onApplied={() => router.refresh()}>
                <TargetField
                  target={{
                    kind: 'price_input',
                    label: `New list price for ${name}: ${newPrice === null ? 'empty' : price(newPrice, currency)}`,
                    ids: {variant_id: variant.variant_id},
                    term: 'schema:price',
                  }}>
                  <NumberInput
                    label="New list price"
                    value={newPrice}
                    onChange={v => setNewPrice(Number.isFinite(v) ? v : null)}
                    min={0}
                    step={0.01}
                    units={currency}
                    hasClear
                    description="The server checks it against the price floor and says whether a person has to approve it."
                    width={280}
                  />
                </TargetField>
              </WriteFlow>
            ) : null}
          </VStack>
        ) : (
          <VStack gap={4}>
            <MetadataList columns="multi">
              <MetadataListItem label="Available now">{count(variant.inventory_level)}</MetadataListItem>
              {variant['acom:safety_stock'] !== undefined ? <MetadataListItem label="Safety stock">{count(variant['acom:safety_stock'])}</MetadataListItem> : null}
              <MetadataListItem label="Availability">{variant.availability ?? '—'}</MetadataListItem>
            </MetadataList>
            {!canWrite ? (
              <Text color="secondary">Your account can’t change stock.</Text>
            ) : (
              <WriteFlow
                method="PUT"
                path={`/v1/catalog/variants/${vid}/inventory`}
                input={stock === null ? null : {variant_id: variant.variant_id, inventory_level: stock, ...(reason.trim() ? {reason: reason.trim()} : {})}}
                previewLabel="Preview the stock"
                applyLabel="Apply the stock"
                askWhenApprovalNeeded={`Prepare a change set to set the stock of ${name} (${variant.variant_id}) to ${stock ?? ''}.`}
                onApplied={() => router.refresh()}>
                <TargetField
                  target={{
                    kind: 'stock_input',
                    label: `New stock for ${name}: ${stock === null ? 'empty' : stock}`,
                    ids: {variant_id: variant.variant_id},
                    term: 'schema:inventoryLevel',
                  }}>
                  <NumberInput
                    label="Available quantity"
                    value={stock}
                    onChange={v => setStock(Number.isFinite(v) ? v : null)}
                    min={0}
                    isIntegerOnly
                    hasClear
                    description="Set the quantity you can sell, not a difference."
                    width={280}
                  />
                </TargetField>
                <TextInput label="Reason" isOptional value={reason} onChange={setReason} placeholder="Stock count after delivery" width={420} />
              </WriteFlow>
            )}
          </VStack>
        )}
      </VStack>
    </Dialog>
  );
}
