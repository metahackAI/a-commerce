'use client';

// Cancelling before shipment (fulfillment.md): get_cancel_options prepares what can be cancelled,
// the reasons, and the refund estimate for the chosen lines and reason. Sending the cancellation is
// the shopper's PersonButton (POST /v1/fulfillment/orders/{order_id}/cancel) with the estimate the
// shopper saw; if the server's numbers differ it refuses with estimate_changed and the panel reads
// the options again.

import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {Card} from '@astryxdesign/core/Card';
import {CheckboxList, CheckboxListItem} from '@astryxdesign/core/CheckboxList';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {List, ListItem} from '@astryxdesign/core/List';
import {RadioList, RadioListItem} from '@astryxdesign/core/RadioList';
import {Selector} from '@astryxdesign/core/Selector';
import {Skeleton} from '@astryxdesign/core/Skeleton';
import {Heading, Text} from '@astryxdesign/core/Text';
import {TextArea} from '@astryxdesign/core/TextArea';
import {useRouter} from 'next/navigation';
import {useEffect, useState} from 'react';

import {useAiTarget} from '@/ai/useAiTarget';
import {money, words} from '@/app/(storefront)/_lib/format';
import type {CancelOptions, ClaimResult} from '@/app/(storefront)/_lib/types';
import {api, type ApiErrorBody} from '@/lib/api';
import {PersonButton} from '@/components/PersonButton';

import {ErrorNotice} from './ErrorNotice';
import {errorBody} from './write';

export function CancelPanel({orderId, orderNumber}: {orderId: string; orderNumber: string}) {
  const router = useRouter();
  const [isOpen, setOpen] = useState(false);
  const [options, setOptions] = useState<CancelOptions | null>(null);
  const [selected, setSelected] = useState<string[] | null>(null);
  const [reason, setReason] = useState<string>('');
  const [note, setNote] = useState('');
  const [refundTo, setRefundTo] = useState('store_credit');
  const [error, setError] = useState<ApiErrorBody | null>(null);
  const [done, setDone] = useState<ClaimResult | null>(null);
  const [version, setVersion] = useState(0);
  const target = useAiTarget({kind: 'cancel_options', label: `Cancel order #${orderNumber}`, ids: {order_id: orderId}, term: 'schema:CancelAction'});

  const cancellable = (options?.items ?? []).filter(i => i.path !== 'not_allowed' && i.cancellable_quantity > 0);
  const chosen = selected ?? cancellable.map(i => i.order_item_id);
  const isWhole = cancellable.length > 0 && chosen.length === cancellable.length;
  const items = isWhole
    ? undefined
    : cancellable.filter(i => chosen.includes(i.order_item_id)).map(i => ({order_item_id: i.order_item_id, quantity: i.cancellable_quantity}));

  // Read the options (and the estimate for the chosen lines and reason) again whenever they change.
  const itemsKey = items ? items.map(i => `${i.order_item_id}:${i.quantity}`).join(',') : 'all';
  useEffect(() => {
    if (!isOpen) return;
    const ctl = new AbortController();
    setError(null);
    api<CancelOptions>(`/v1/fulfillment/orders/${encodeURIComponent(orderId)}/cancel-options`, {
      query: {items: itemsKey === 'all' ? undefined : itemsKey.split(','), reason: reason || undefined},
      signal: ctl.signal,
    })
      .then(o => {
        setOptions(o);
        setReason(r => r || o.reasons[0]?.reason || '');
      })
      .catch(e => {
        if (!ctl.signal.aborted) setError(errorBody(e));
      });
    return () => ctl.abort();
  }, [isOpen, orderId, itemsKey, reason, version]);

  if (done) {
    return (
      <Banner
        status="success"
        title={done.status === 'requested' ? 'Cancellation requested' : 'Cancelled'}
        description={
          done.status === 'requested'
            ? 'A person at the store confirms it before it ships. The refund follows their decision.'
            : done.refund
              ? `${money(done.refund.total)} goes back: ${done.refund.breakdown.map(b => `${money(b.amount)} to ${b.to}`).join(', ')}.`
              : 'Nothing was charged.'
        }
      />
    );
  }

  if (!isOpen) {
    return (
      <HStack>
        <Button label="Cancel items…" variant="secondary" onClick={() => setOpen(true)} />
      </HStack>
    );
  }

  const estimate = options?.estimate;
  return (
    <Card ref={target.ref} {...target.props}>
      <VStack gap={4}>
        <HStack gap={2} justify="between" vAlign="center">
          <Heading level={2}>Cancel before it ships</Heading>
          <Button label="Close" size="sm" variant="ghost" onClick={() => setOpen(false)} />
        </HStack>
        {error ? <ErrorNotice error={error} askAbout={`I want to cancel order #${orderNumber}. What can I do?`} /> : null}
        {!options && !error ? (
          <VStack gap={2}>
            <Skeleton height={32} />
            <Skeleton height={32} index={1} />
          </VStack>
        ) : null}
        {options ? (
          <VStack gap={4}>
            {cancellable.length === 0 ? (
              <Text color="secondary">Nothing in this order can be cancelled now.</Text>
            ) : (
              <CheckboxList label="Lines to cancel" value={chosen} onChange={setSelected}>
                {cancellable.map(i => (
                  <CheckboxListItem
                    key={i.order_item_id}
                    value={i.order_item_id}
                    label={i.name}
                    description={`${i.cancellable_quantity} of ${i.ordered} · ${money(i.unit_paid)} each · ${words(i.path)}${i.whole_quantity_only ? ' · whole quantity only' : ''}`}
                  />
                ))}
              </CheckboxList>
            )}
            {options.not_cancellable.length > 0 ? (
              <List density="compact">
                {options.not_cancellable.map(n => (
                  <ListItem key={n.order_item_id} label={`${n.order_item_id}: ${words(n.why)}`} description={n.next_tool === 'get_return_options' ? 'Already shipped — return it once it arrives.' : undefined} />
                ))}
              </List>
            ) : null}
            {options.reasons.length > 0 ? (
              <Selector
                label="Why are you cancelling?"
                value={reason}
                onChange={setReason}
                options={options.reasons.map(r => ({
                  value: r.reason,
                  label: words(r.reason),
                  description: r.needs_store_review ? 'The store reviews this reason' : undefined,
                }))}
              />
            ) : null}
            <TextArea label="Anything to add?" isOptional value={note} onChange={setNote} rows={2} />
            {estimate && chosen.length > 0 ? (
              <VStack gap={2}>
                <Text type="label">If you cancel</Text>
                <List density="compact">
                  <ListItem label="Cancelled" endContent={<Text hasTabularNumbers>{money(estimate.cancelled_amount)}</Text>} />
                  {estimate.extra_charge.map(c => (
                    <ListItem key={c.kind} label={words(c.kind)} description={c.why} endContent={<Text hasTabularNumbers>−{money(c.amount)}</Text>} />
                  ))}
                  {estimate.refund_breakdown.map(b => (
                    <ListItem key={b.to} label={`Back to ${b.to}`} endContent={<Text hasTabularNumbers>{money(b.amount)}</Text>} />
                  ))}
                  <ListItem label="Refund" endContent={<Text weight="semibold" hasTabularNumbers>{money(estimate.refund_total)}</Text>} />
                  {estimate.charge_due.value > 0 ? (
                    <ListItem label="You pay first" endContent={<Text weight="semibold" hasTabularNumbers>{money(estimate.charge_due)}</Text>} />
                  ) : null}
                </List>
                <Text type="supporting" color="secondary">
                  {words(estimate.confirmed_by)}
                  {estimate.money_back_business_days ? ` · money back ${estimate.money_back_business_days.min}–${estimate.money_back_business_days.max} business days after` : ''}
                  {estimate.revocable ? '' : ' · a cancellation cannot be taken back'}
                </Text>
                {estimate.needs_refund_destination ? (
                  <RadioList label="Where should the refund go?" value={refundTo} onChange={setRefundTo}>
                    <RadioListItem value="store_credit" label="Store credit" description="The original payment can't be refunded directly." />
                  </RadioList>
                ) : null}
                <HStack gap={2} wrap="wrap" vAlign="center">
                  <PersonButton
                    action={{
                      label: isWhole ? 'Cancel order' : 'Cancel selected',
                      method: 'POST',
                      path: `/v1/fulfillment/orders/${encodeURIComponent(orderId)}/cancel`,
                      body: {
                        ...(items ? {items} : {}),
                        reason,
                        ...(note.trim() ? {reason_text: note.trim()} : {}),
                        expected_refund: estimate.refund_total,
                        expected_charge: estimate.charge_due,
                        ...(estimate.needs_refund_destination ? {refund_to: {type: refundTo, ref: null}} : {}),
                      },
                      confirm: `Cancel ${isWhole ? `order #${orderNumber}` : 'the selected lines'}? ${money(estimate.refund_total)} comes back to you.`,
                    }}
                    onDone={result => {
                      setDone(result as ClaimResult);
                      router.refresh();
                    }}
                  />
                  <Button label="Numbers changed? Check again" size="sm" variant="ghost" onClick={() => setVersion(v => v + 1)} />
                </HStack>
              </VStack>
            ) : null}
          </VStack>
        ) : null}
      </VStack>
    </Card>
  );
}
