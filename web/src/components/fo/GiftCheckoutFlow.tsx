'use client';

// Gift checkout (scene F4): `prepare_gift_order` previews a gift from the cart — who it is for,
// the card, the wrap, a gift receipt without prices, and an address for this order only
// (POST /v1/sales/gift-orders/prepare?preview=true). The card shows what the sender pays, the
// server's checks, and the person-only "Place gift order" button (POST /v1/sales/orders
// {preview_id}): the assistant can prepare a gift but never send it.
//
// The recipient's address goes with this one order and is never saved to the sender's address
// book. The page computes no amount: the wrap fee, the total and the delivery date all come
// from the preview. A gift the recipient accepts by link holds its payment until they do.

import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {Card} from '@astryxdesign/core/Card';
import {Grid} from '@astryxdesign/core/Grid';
import {Icon} from '@astryxdesign/core/Icon';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {List, ListItem} from '@astryxdesign/core/List';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {RadioList, RadioListItem} from '@astryxdesign/core/RadioList';
import {Switch} from '@astryxdesign/core/Switch';
import {Heading, Text} from '@astryxdesign/core/Text';
import {TextArea} from '@astryxdesign/core/TextArea';
import {TextInput} from '@astryxdesign/core/TextInput';
import {CheckCircleIcon, ExclamationTriangleIcon} from '@heroicons/react/24/outline';
import {useRouter} from 'next/navigation';
import {useRef, useState, type ReactNode} from 'react';

import {useAiTarget} from '@/ai/useAiTarget';
import {dateTime, day, money} from '@/app/(storefront)/_lib/format';
import type {Cart, GiftDraft, Money, PlacedOrder} from '@/app/(storefront)/_lib/types';
import {preview as previewWrite, type ApiErrorBody, type Preview} from '@/lib/api';
import {PersonButton} from '@/components/PersonButton';

import {CartSummary} from './CartSummary';
import {ErrorNotice} from './ErrorNotice';
import {errorBody} from './write';

const TOTAL_CHECK = 'checkout_validation.server_total_matches_client';
const CARD_MESSAGE_LIMIT = 300;

type Mode = 'address_known' | 'recipient_accepts';

/** A form field the shopper can ask about ("why do you need the town?"). */
function Field({kind, label, children}: {kind: string; label: string; children: ReactNode}) {
  const target = useAiTarget({kind, label, ids: {field: kind}, term: 'schema:PostalAddress'});
  return (
    <VStack gap={1} ref={target.ref} {...target.props}>
      {children}
    </VStack>
  );
}

function CheckRow({check}: {check: Preview['checks'][number]}) {
  const target = useAiTarget({kind: 'check_row', label: check.detail ?? check.rule, ids: {rule: check.rule}});
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      startContent={
        <Icon
          icon={check.passed ? CheckCircleIcon : ExclamationTriangleIcon}
          color={check.passed ? 'success' : 'warning'}
          size="sm"
          label={check.passed ? 'Passed' : 'Needs attention'}
        />
      }
      label={check.detail ?? check.rule}
    />
  );
}

/** The gift options as the preview states them; nothing is worked out here. */
function GiftOptions({draft}: {draft: GiftDraft}) {
  const options = draft.gift_options ?? {};
  const target = useAiTarget({
    kind: 'gift_options',
    label: `Gift options: ${options.wrap ? `wrapped (${money(options.wrap_fee ?? null)})` : 'not wrapped'}`,
    ids: {gift_options: 'gift'},
    term: 'acom:GiftOptions',
  });
  const delivery = draft.delivery;
  return (
    <VStack gap={2} ref={target.ref} {...target.props}>
      <MetadataList>
        <MetadataListItem label="To">{delivery?.address_label ?? delivery?.to ?? draft.recipient?.name ?? ''}</MetadataListItem>
        {delivery?.arrives ? <MetadataListItem label="Arrives">{day(delivery.arrives)}</MetadataListItem> : null}
        <MetadataListItem label="Address">
          {draft.recipient?.mode === 'recipient_accepts' ? 'They give it when they accept the gift' : 'Used for this order only, not saved'}
        </MetadataListItem>
        <MetadataListItem label="Wrap">{options.wrap ? `Gift wrap, +${money(options.wrap_fee ?? null)}` : 'No wrap'}</MetadataListItem>
        {options.card_message ? <MetadataListItem label="Card">{`“${options.card_message}”`}</MetadataListItem> : null}
        <MetadataListItem label="Receipt">
          {options.gift_receipt ? (options.receipt_hides_prices ? 'Gift receipt, no prices shown' : 'Gift receipt') : 'Regular receipt'}
        </MetadataListItem>
        {options.exchange_until ? <MetadataListItem label="Exchange">{`They can swap the size until ${day(options.exchange_until)}`}</MetadataListItem> : null}
      </MetadataList>
    </VStack>
  );
}

function draftOf(p: Preview): GiftDraft | null {
  const change = p.changes.find(c => c.entity === 'order' && c.op === 'create') ?? p.changes[0];
  const to = change?.to;
  return to && typeof to === 'object' ? (to as GiftDraft) : null;
}

export function GiftCheckoutFlow({cart}: {cart: Cart}) {
  const router = useRouter();
  const [mode, setMode] = useState<Mode>('address_known');
  const [name, setName] = useState('');
  const [street, setStreet] = useState('');
  const [town, setTown] = useState('');
  const [region, setRegion] = useState('');
  const [postal, setPostal] = useState('');
  const [country, setCountry] = useState('US');
  const [contact, setContact] = useState('');
  const [message, setMessage] = useState('');
  const [wrap, setWrap] = useState(true);
  const [receipt, setReceipt] = useState(true);
  const [prepared, setPrepared] = useState<Preview | null>(null);
  const [placed, setPlaced] = useState<PlacedOrder | null>(null);
  const [error, setError] = useState<ApiErrorBody | null>(null);
  const [isPreparing, setPreparing] = useState(false);
  // What the sender has seen: the cart's total, then the gift's total once shown.
  const seenTotal = useRef<Money>(cart.total);

  const ready =
    name.trim() !== '' && (mode === 'address_known' ? street.trim() !== '' && town.trim() !== '' && country.trim() !== '' : contact.trim() !== '');

  async function prepare(clientTotal: Money) {
    if (!ready) return;
    seenTotal.current = clientTotal;
    setPreparing(true);
    setError(null);
    try {
      const p = await previewWrite('/v1/sales/gift-orders/prepare', {
        recipient_mode: mode,
        recipient_name: name.trim(),
        ...(mode === 'address_known'
          ? {
              delivery_address: {
                street_address: street.trim(),
                address_locality: town.trim(),
                ...(region.trim() ? {address_region: region.trim()} : {}),
                ...(postal.trim() ? {postal_code: postal.trim()} : {}),
                address_country: country.trim().toUpperCase(),
              },
            }
          : {recipient_contact: contact.trim()}),
        ...(message.trim() ? {card_message: message.trim()} : {}),
        gift_wrap: wrap,
        gift_receipt: receipt,
        client_total: clientTotal,
      });
      setPrepared(p);
    } catch (e) {
      setPrepared(null);
      setError(errorBody(e));
    } finally {
      setPreparing(false);
    }
  }

  const draft = prepared ? draftOf(prepared) : null;
  const failed = prepared?.checks.filter(c => !c.passed) ?? [];
  const totalChanged = failed.some(c => c.rule === TOTAL_CHECK);
  const canPlace = !!prepared && !!draft && failed.length === 0 && !isPreparing && !placed;
  const card = useAiTarget({
    kind: 'checkout_card',
    label: draft ? `Gift to pay ${money(draft.total)}` : 'Gift being prepared',
    ids: {preview_id: prepared?.preview_id ?? 'pending'},
    term: 'schema:OrderAction',
  });
  const to = draft?.delivery?.address_label ?? draft?.recipient?.name ?? name;

  return (
    <Grid columns={{minWidth: 320, max: 2}} gap={6} align="start">
      <VStack gap={5}>
        <VStack gap={2}>
          <Heading level={2}>Who it is for</Heading>
          <Text type="supporting" color="secondary">
            Nothing about them is saved to your profile or your addresses.
          </Text>
          <Field kind="gift_recipient" label="Recipient's name">
            <TextInput label="Their name" value={name} onChange={setName} placeholder="Dad" isRequired />
          </Field>
          <RadioList label="How it reaches them" value={mode} onChange={v => setMode(v as Mode)}>
            <RadioListItem value="address_known" label="I know the address" description="It ships straight to them." />
            <RadioListItem
              value="recipient_accepts"
              label="Send them a link"
              description="They accept and give their address and size; you pay when they accept."
            />
          </RadioList>
          {mode === 'address_known' ? (
            <Field kind="gift_address" label="Address for this gift only">
              <TextInput label="Street" value={street} onChange={setStreet} placeholder="48 Pine Rd" isRequired autoComplete="off" />
              <HStack gap={2} wrap="wrap">
                <TextInput label="Town" value={town} onChange={setTown} placeholder="Lakeview" isRequired autoComplete="off" />
                <TextInput label="State" value={region} onChange={setRegion} isOptional autoComplete="off" />
              </HStack>
              <HStack gap={2} wrap="wrap">
                <TextInput label="Postal code" value={postal} onChange={setPostal} isOptional autoComplete="off" />
                <TextInput label="Country" value={country} onChange={setCountry} isRequired autoComplete="off" />
              </HStack>
              <Text type="supporting" color="secondary">
                Used for this order only.
              </Text>
            </Field>
          ) : (
            <Field kind="gift_contact" label="Where the accept link goes">
              <TextInput label="Their email or phone" value={contact} onChange={setContact} isRequired autoComplete="off" />
            </Field>
          )}
        </VStack>
        <VStack gap={2}>
          <Heading level={2}>Gift options</Heading>
          <Field kind="gift_card_message" label="Card message">
            <TextArea label="Card message" value={message} onChange={setMessage} isOptional rows={2} maxLength={CARD_MESSAGE_LIMIT} />
          </Field>
          <Field kind="gift_wrap" label="Gift wrap">
            <Switch label="Wrap it" description="The fee is shown with the total." value={wrap} onChange={setWrap} />
          </Field>
          <Field kind="gift_receipt" label="Gift receipt">
            <Switch label="Gift receipt" description="Put a receipt in the parcel." value={receipt} onChange={setReceipt} />
          </Field>
        </VStack>
        <VStack gap={2}>
          <Heading level={2}>In this gift</Heading>
          <List hasDividers>
            {(draft?.lines ?? cart.lines).map((l, i) => (
              <ListItem
                key={l.cart_line_id ?? `${l.variant_id}-${i}`}
                label={l.name}
                description={[[l.color, l.size].filter(Boolean).join(' · '), `Qty ${l.quantity}`, `${money(l.price)} each`].filter(Boolean).join(' · ')}
              />
            ))}
          </List>
        </VStack>
      </VStack>

      <Card ref={card.ref} {...card.props}>
        <VStack gap={4}>
          <Heading level={2}>What you will pay</Heading>
          {error ? <ErrorNotice error={error} /> : null}
          {placed ? (
            <Banner
              status="success"
              title={`Gift order ${placed.order_number} placed`}
              description={
                placed.accept_url
                  ? `${to} accepts it through a link and gives their address and size. Your card is charged when they accept.`
                  : 'It is on its way. You can follow it in your orders.'
              }
              endContent={
                <HStack gap={2}>
                  {placed.accept_url ? <Button label="Their link" size="sm" href={placed.accept_url} /> : null}
                  <Button label="Your order" size="sm" href={`/orders/${encodeURIComponent(placed.id)}`} />
                </HStack>
              }
            />
          ) : null}
          {draft ? (
            <VStack gap={3}>
              {totalChanged ? (
                <Banner
                  status="warning"
                  title={`The gift now comes to ${money(draft.total)}`}
                  description={`You saw ${money(seenTotal.current)}. Check the numbers below, then prepare the gift again with the new total.`}
                  endContent={<Button label="Use the new total" size="sm" variant="primary" clickAction={() => prepare(draft.total)} />}
                />
              ) : null}
              <CartSummary
                subtotal={draft.subtotal}
                discounts={draft.discounts ?? []}
                shipping={draft.shipping}
                total={draft.total}
                giftWrap={draft.gift_wrap}
                payment={draft.payment}
                coupons={draft.coupons_recorded}
              />
              <GiftOptions draft={draft} />
              <List density="compact">
                {prepared?.checks.map(c => (
                  <CheckRow key={c.rule} check={c} />
                ))}
              </List>
              {canPlace && prepared ? (
                <VStack gap={2}>
                  <PersonButton
                    action={{
                      label: `Place gift order · ${money(draft.total)}`,
                      method: 'POST',
                      path: '/v1/sales/orders',
                      body: {preview_id: prepared.preview_id},
                      confirm: `Send this gift to ${to} and pay ${money(draft.total)}${draft.payment?.label ? ` with ${draft.payment.label}` : ''}?`,
                    }}
                    onDone={result => {
                      const order = result as PlacedOrder | null;
                      if (order?.accept_url) {
                        setPlaced(order);
                        return;
                      }
                      router.push(order?.id ? `/orders/${encodeURIComponent(order.id)}?placed=1` : '/orders');
                      router.refresh();
                    }}
                    onError={err => {
                      const stale = ['estimate_changed', 'preview_expired', 'preview_superseded'];
                      if (!stale.includes(err.code)) return true;
                      setError(err);
                      void prepare(seenTotal.current);
                    }}
                  />
                  <Text type="supporting" color="secondary">
                    Ailism can prepare a gift, but only you can send it.
                    {prepared.expires_at ? ` Prepared until ${dateTime(prepared.expires_at)}.` : ''}
                  </Text>
                </VStack>
              ) : null}
            </VStack>
          ) : (
            <Text color="secondary">Say who it is for and where it goes, then prepare the gift to see what you will pay.</Text>
          )}
          {!placed ? (
            <HStack>
              <Button
                label={draft ? 'Changed something? Prepare again' : 'Prepare the gift'}
                variant={draft ? 'ghost' : 'primary'}
                size={draft ? 'sm' : 'md'}
                isDisabled={!ready}
                isLoading={isPreparing}
                clickAction={() => prepare(draft?.goods_total ?? cart.total)}
              />
            </HStack>
          ) : null}
        </VStack>
      </Card>
    </Grid>
  );
}
