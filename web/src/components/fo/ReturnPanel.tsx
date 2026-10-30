'use client';

// Returning a delivered line (fulfillment.md): the shopper says why, get_return_options prepares
// the choices (a replacement or a refund) with what each costs, who confirms it and how the item
// goes back. Sending the request is the shopper's PersonButton (POST /v1/fulfillment/returns) with
// the amounts the shopper saw; if the server's numbers differ it refuses with estimate_changed.

import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {Card} from '@astryxdesign/core/Card';
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
import {day, money, timeWindow, words} from '@/app/(storefront)/_lib/format';
import type {ClaimResult, OrderItem, ReturnOption} from '@/app/(storefront)/_lib/types';
import {api, type ApiErrorBody, type Page} from '@/lib/api';
import {PersonButton} from '@/components/PersonButton';

import {useSay} from './Ask';
import {ErrorNotice} from './ErrorNotice';
import {errorBody} from './write';

/** `acom:claim_reason` (docs/api/enums.md §2.2), in the shopper's words. */
const REASONS: {value: string; label: string}[] = [
  {value: 'defect', label: 'It is faulty'},
  {value: 'damaged_in_transit', label: 'It arrived damaged'},
  {value: 'not_as_described', label: 'Not as described'},
  {value: 'wrong_item_sent', label: 'Wrong item sent'},
  {value: 'wrong_option_sent', label: 'Wrong size or color sent'},
  {value: 'missing_parts', label: 'Parts are missing'},
  {value: 'wrong_size', label: 'The size does not fit'},
  {value: 'different_option_wanted', label: 'I want another size or color'},
  {value: 'changed_mind', label: 'I changed my mind'},
];

function OptionRow({option, itemName}: {option: ReturnOption; itemName: string}) {
  const target = useAiTarget({kind: 'return_option', label: `${option.title} for ${itemName}`, ids: {kind: option.kind}});
  return (
    <VStack gap={1} ref={target.ref} {...target.props}>
      <Text type="supporting" color="secondary">
        {option.facts.join(' · ')}
      </Text>
    </VStack>
  );
}

export function ReturnPanel({orderId, item}: {orderId: string; item: OrderItem}) {
  const router = useRouter();
  const say = useSay();
  const [isOpen, setOpen] = useState(false);
  const [reason, setReason] = useState('');
  const [options, setOptions] = useState<ReturnOption[] | null>(null);
  const [kind, setKind] = useState<string>('');
  const [method, setMethod] = useState<string>('');
  const [note, setNote] = useState('');
  const [error, setError] = useState<ApiErrorBody | null>(null);
  const [done, setDone] = useState<ClaimResult | null>(null);
  const [version, setVersion] = useState(0);

  useEffect(() => {
    if (!isOpen || !reason) return;
    const ctl = new AbortController();
    setOptions(null);
    setError(null);
    api<Page<ReturnOption>>(
      `/v1/fulfillment/orders/${encodeURIComponent(orderId)}/items/${encodeURIComponent(item.id)}/return-options`,
      {query: {reason}, signal: ctl.signal},
    )
      .then(r => {
        setOptions(r.items);
        const first = r.items[0];
        setKind(k => (r.items.some(o => o.kind === k) ? k : (first?.kind ?? '')));
        setMethod(m => m || first?.send_back.methods[0] || '');
      })
      .catch(e => {
        if (!ctl.signal.aborted) {
          setOptions([]);
          setError(errorBody(e));
        }
      });
    return () => ctl.abort();
  }, [isOpen, reason, orderId, item.id, version]);

  if (done) {
    const refund = done.refund_estimate ?? done.refund;
    return (
      <Banner
        status="success"
        title={`Return request sent for ${item.name}`}
        description={`${words(done.status)}${done.confirmed_by ? ` · ${words(done.confirmed_by)}` : ''}${refund ? ` · ${money(refund.total)} expected back` : ''}`}
      />
    );
  }
  if (!isOpen) {
    return <Button label="Return or replace" size="sm" variant="secondary" onClick={() => setOpen(true)} />;
  }

  const option = options?.find(o => o.kind === kind) ?? null;
  const methods = option?.send_back.methods ?? [];
  const sendMethod = methods.includes(method as 'pickup' | 'drop_off') ? method : (methods[0] ?? '');
  const pickup = sendMethod === 'pickup' ? (option?.send_back.pickup_window ?? null) : null;

  return (
    <Card>
      <VStack gap={4}>
        <HStack gap={2} justify="between" vAlign="center">
          <Heading level={3}>Return {item.name}</Heading>
          <Button label="Close" size="sm" variant="ghost" onClick={() => setOpen(false)} />
        </HStack>
        <Selector label="What happened?" placeholder="Choose a reason" value={reason} onChange={setReason} options={REASONS} />
        {error ? (
          <ErrorNotice error={error} askAbout={`I want to return the ${item.name} from my order. What are my options?`} />
        ) : null}
        {reason && options === null ? (
          <VStack gap={2}>
            <Skeleton height={56} />
            <Skeleton height={56} index={1} />
          </VStack>
        ) : null}
        {options && options.length > 0 ? (
          <VStack gap={3}>
            <RadioList label="Choose one" value={kind} onChange={setKind}>
              {options.map(o => (
                <RadioListItem
                  key={o.kind}
                  value={o.kind}
                  label={`${o.title}${o.kind === 'refund' && o.refund_amount ? ` · ${money(o.refund_amount)}` : ''}`}
                  description={<OptionRow option={o} itemName={item.name} />}
                />
              ))}
            </RadioList>
            {option ? (
              <VStack gap={2}>
                {methods.length > 0 ? (
                  <RadioList label="How it goes back" value={sendMethod} onChange={setMethod} orientation="horizontal">
                    {methods.map(m => (
                      <RadioListItem key={m} value={m} label={words(m)} />
                    ))}
                  </RadioList>
                ) : null}
                <List density="compact">
                  {pickup ? <ListItem label="Pickup" endContent={<Text>{timeWindow(pickup.start, pickup.end)}</Text>} /> : null}
                  {option.send_back.cost ? (
                    <ListItem label="Sending it back" endContent={<Text hasTabularNumbers>{option.send_back.cost.value === 0 ? 'Free' : money(option.send_back.cost)}</Text>} />
                  ) : null}
                  {option.kind === 'replacement' && option.arrives_by ? (
                    <ListItem label="New one arrives" endContent={<Text>{day(option.arrives_by)}</Text>} />
                  ) : null}
                  {option.kind === 'refund' && option.refund_amount ? (
                    <ListItem
                      label={`Refund${option.refund_to ? ` to ${option.refund_to}` : ''}`}
                      endContent={<Text weight="semibold" hasTabularNumbers>{money(option.refund_amount)}</Text>}
                    />
                  ) : null}
                  <ListItem label="You pay" endContent={<Text hasTabularNumbers>{money(option.you_pay)}</Text>} />
                </List>
                <Text type="supporting" color="secondary">
                  {words(option.confirmed_by)}
                </Text>
                {option.charge_due && option.charge_due.value > 0 ? (
                  <Banner status="info" title={`${money(option.charge_due)} is due before the store takes this return`} description="Pay it from the request once it is sent." />
                ) : null}
                <TextArea label="Tell the store more" isOptional value={note} onChange={setNote} rows={2} />
                {reason === 'damaged_in_transit' || reason === 'defect' ? (
                  <HStack gap={2} wrap="wrap" vAlign="center">
                    <Text type="supporting" color="secondary">
                      Photos help the store decide. Send them to Northwind in the chat.
                    </Text>
                    <Button label="Send photos" size="sm" variant="ghost" onClick={() => say(`I have photos of the ${item.name} from order ${orderId}.`)} />
                  </HStack>
                ) : null}
                <HStack gap={2} wrap="wrap" vAlign="center">
                  <PersonButton
                    action={{
                      label: option.kind === 'refund' ? 'Send refund request' : 'Send replacement request',
                      method: 'POST',
                      path: '/v1/fulfillment/returns',
                      body: {
                        order_id: orderId,
                        items: [{order_item_id: item.id, quantity: item.order_quantity}],
                        reason,
                        kind: option.kind,
                        send_back_method: sendMethod,
                        ...(pickup ? {pickup_window: pickup} : {}),
                        ...(option.kind === 'refund' && option.refund_amount ? {expected_refund: option.refund_amount} : {}),
                        expected_charge: option.you_pay,
                        ...(note.trim() ? {reason_text: note.trim()} : {}),
                      },
                      confirm:
                        option.kind === 'refund'
                          ? `Send a refund request for the ${item.name}? ${option.refund_amount ? `${money(option.refund_amount)} back${option.refund_to ? ` to ${option.refund_to}` : ''}.` : ''}`
                          : `Send a replacement request for the ${item.name}?`,
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
