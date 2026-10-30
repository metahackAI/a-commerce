'use client';

// The recipient's side of a gift sent by link (docs/api/sales.md §gift_order): what the gift is,
// who sent it and their card (get_gift_order, no prices), the sizes and colors they may switch to
// at the same price, and the address for this one order. Accepting is the person-only
// POST /v1/sales/gift-orders/{id}/accept; the held payment goes through then. The assistant can
// explain the gift but never accept it.

import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {Card} from '@astryxdesign/core/Card';
import {Grid} from '@astryxdesign/core/Grid';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Heading, Text} from '@astryxdesign/core/Text';
import {TextInput} from '@astryxdesign/core/TextInput';
import {ToggleButton} from '@astryxdesign/core/ToggleButton';
import {useRouter} from 'next/navigation';
import {useState, type ReactNode} from 'react';

import {useAiTarget} from '@/ai/useAiTarget';
import {dateTime, day} from '@/app/(storefront)/_lib/format';
import type {GiftView, GiftViewItem} from '@/app/(storefront)/_lib/types';
import {PersonButton} from '@/components/PersonButton';

import {ProductImage} from './ProductImage';

function unique(values: (string | null)[]): string[] {
  return [...new Set(values.filter((v): v is string => !!v))];
}

function describe(item: Pick<GiftViewItem, 'color' | 'size' | 'quantity'>): string {
  return [item.color, item.size, item.quantity > 1 ? `${item.quantity} pieces` : null].filter(Boolean).join(' · ');
}

function GiftItem({gift, item}: {gift: GiftView; item: GiftViewItem}) {
  const target = useAiTarget({kind: 'gift_item', label: item.name, ids: {gift_order_id: gift.id}, term: 'schema:OrderItem'});
  return (
    <HStack gap={4} vAlign="center" ref={target.ref} {...target.props}>
      <div style={{width: 120, flexShrink: 0}}>
        <ProductImage src={item.image} alt={item.name} />
      </div>
      <VStack gap={1}>
        <Text weight="semibold">{item.name}</Text>
        <Text color="secondary">{describe(item)}</Text>
      </VStack>
    </HStack>
  );
}

/** A form field the recipient can ask about ("why do you need the state?"). */
function Field({kind, label, children}: {kind: string; label: string; children: ReactNode}) {
  const target = useAiTarget({kind, label, ids: {field: kind}, term: 'schema:PostalAddress'});
  return (
    <VStack gap={1} ref={target.ref} {...target.props}>
      {children}
    </VStack>
  );
}

function OptionChip({
  kind,
  value,
  isSelected,
  isUnavailable,
  onSelect,
}: {
  kind: 'color' | 'size';
  value: string;
  isSelected: boolean;
  isUnavailable: boolean;
  onSelect: () => void;
}) {
  const target = useAiTarget({kind: kind === 'size' ? 'size_chip' : 'color_chip', label: `${kind} ${value}`, ids: {[kind]: value}});
  return (
    <ToggleButton
      ref={target.ref}
      {...target.props}
      label={value}
      size="sm"
      isPressed={isSelected}
      isDisabled={isUnavailable && !isSelected}
      tooltip={isUnavailable ? `${value} is sold out` : undefined}
      onPressedChange={pressed => {
        if (pressed) onSelect();
      }}>
      {value}
    </ToggleButton>
  );
}

/** What the gift is and who it is from. */
export function GiftSummary({gift}: {gift: GiftView}) {
  const card = useAiTarget({kind: 'gift_card_message', label: 'Card message', ids: {gift_order_id: gift.id}});
  return (
    <VStack gap={4}>
      {gift.items.map((item, i) => (
        <GiftItem key={`${item.name}-${i}`} gift={gift} item={item} />
      ))}
      {gift.card_message ? (
        <Card variant="muted" ref={card.ref} {...card.props}>
          <VStack gap={1}>
            <Text>{`“${gift.card_message}”`}</Text>
            {gift.from ? (
              <Text type="supporting" color="secondary">
                {gift.from}
              </Text>
            ) : null}
          </VStack>
        </Card>
      ) : null}
      <MetadataList>
        {gift.gift_wrap ? <MetadataListItem label="Wrapping">Gift wrapped</MetadataListItem> : null}
        <MetadataListItem label="Receipt">Gift receipt, no prices</MetadataListItem>
        {gift.exchange_until ? <MetadataListItem label="Exchange until">{day(gift.exchange_until)}</MetadataListItem> : null}
      </MetadataList>
    </VStack>
  );
}

export function GiftAcceptFlow({gift, canAccept, signInHref}: {gift: GiftView; canAccept: boolean; signInHref: string}) {
  const router = useRouter();
  const item = gift.items.length === 1 ? gift.items[0] : null;
  const options = item?.options ?? [];
  const colors = unique(options.map(o => o.color));
  const sizes = unique(options.map(o => o.size));
  const [color, setColor] = useState<string | null>(item?.color ?? null);
  const [size, setSize] = useState<string | null>(item?.size ?? null);
  const [street, setStreet] = useState('');
  const [town, setTown] = useState('');
  const [region, setRegion] = useState('');
  const [postal, setPostal] = useState('');
  const [country, setCountry] = useState(gift.ships_to.length === 1 ? gift.ships_to[0] : '');
  const [accepted, setAccepted] = useState(false);
  const form = useAiTarget({kind: 'gift_accept', label: `Accept the gift for ${gift.recipient_name}`, ids: {gift_order_id: gift.id}});

  const chosen = options.find(o => o.color === color && o.size === size) ?? null;
  const optionOk = options.length === 0 || (chosen?.available ?? false);
  const ready = street.trim() !== '' && town.trim() !== '' && /^[A-Za-z]{2}$/.test(country.trim()) && optionOk;
  const changed = item !== null && (color !== item.color || size !== item.size);
  const option = [color, size].filter(Boolean).join(' · ');

  if (accepted) {
    return (
      <Banner status="success" title="Gift accepted" description="It is on its way to you. The address was used for this delivery only." />
    );
  }

  return (
    <Card ref={form.ref} {...form.props}>
      <VStack gap={4}>
        <VStack gap={1}>
          <Heading level={2}>Accept your gift</Heading>
          <Text type="supporting" color="secondary">
            {`Accept by ${dateTime(gift.expires)}. ${gift.from ?? 'The sender'} has already paid for it; nothing is charged to you.`}
          </Text>
        </VStack>

        {options.length > 1 ? (
          <VStack gap={3}>
            {colors.length > 1 ? (
              <VStack gap={2}>
                <Text type="label">{`Color${color ? `: ${color}` : ''}`}</Text>
                <HStack gap={1} wrap="wrap">
                  {colors.map(c => (
                    <OptionChip
                      key={c}
                      kind="color"
                      value={c}
                      isSelected={c === color}
                      isUnavailable={!options.some(o => o.color === c && o.available)}
                      onSelect={() => setColor(c)}
                    />
                  ))}
                </HStack>
              </VStack>
            ) : null}
            {sizes.length > 1 ? (
              <VStack gap={2}>
                <Text type="label">{`Size${size ? `: ${size}` : ''}`}</Text>
                <HStack gap={1} wrap="wrap">
                  {sizes.map(s => (
                    <OptionChip
                      key={s}
                      kind="size"
                      value={s}
                      isSelected={s === size}
                      isUnavailable={!options.some(o => o.size === s && (colors.length <= 1 || o.color === color) && o.available)}
                      onSelect={() => setSize(s)}
                    />
                  ))}
                </HStack>
              </VStack>
            ) : null}
            <Text type="supporting" color="secondary">
              {chosen && !chosen.available
                ? `${option} is sold out; choose another.`
                : 'You can switch to another size or color at the same price.'}
            </Text>
          </VStack>
        ) : null}

        <Field kind="gift_address" label="Where the gift goes">
          <TextInput label="Street" value={street} onChange={setStreet} isRequired autoComplete="street-address" />
          <HStack gap={2} wrap="wrap">
            <TextInput label="Town" value={town} onChange={setTown} isRequired autoComplete="address-level2" />
            <TextInput label="State" value={region} onChange={setRegion} isOptional autoComplete="address-level1" />
          </HStack>
          <HStack gap={2} wrap="wrap">
            <TextInput label="Postal code" value={postal} onChange={setPostal} isOptional autoComplete="postal-code" />
            <TextInput
              label="Country"
              value={country}
              onChange={setCountry}
              isRequired
              autoComplete="country"
              description={gift.ships_to.length > 0 ? `Ships to ${gift.ships_to.join(', ')}` : undefined}
            />
          </HStack>
          <Text type="supporting" color="secondary">
            Used for this delivery only.
          </Text>
        </Field>

        {canAccept ? (
          <PersonButton
            isDisabled={!ready}
            action={{
              label: 'Accept gift',
              method: 'POST',
              path: `/v1/sales/gift-orders/${encodeURIComponent(gift.id)}/accept`,
              body: {
                delivery_address: {
                  street_address: street.trim(),
                  address_locality: town.trim(),
                  ...(region.trim() ? {address_region: region.trim()} : {}),
                  ...(postal.trim() ? {postal_code: postal.trim()} : {}),
                  address_country: country.trim().toUpperCase(),
                },
                ...(changed && size ? {size} : {}),
                ...(changed && color ? {color} : {}),
              },
              confirm: `Accept ${item?.name ?? 'the gift'}${option ? ` in ${option}` : ''} and have it sent to ${street.trim()}, ${town.trim()}?`,
            }}
            onDone={() => {
              setAccepted(true);
              router.refresh();
            }}
          />
        ) : (
          <Banner
            status="info"
            title="Sign in to accept"
            description="Accepting asks for your address, so it needs a signed-in session."
            endContent={<Button label="Sign in" size="sm" variant="primary" href={signInHref} />}
          />
        )}
      </VStack>
    </Card>
  );
}

/** The gift and either the way to accept it or where it stands. */
export function GiftPage({gift, canAccept, signInHref}: {gift: GiftView; canAccept: boolean; signInHref: string}) {
  const status =
    gift.action_status === 'completed' ? (
      <Banner status="success" title="This gift has been accepted" description="It is on its way to the address given when it was accepted." />
    ) : gift.action_status === 'failed' ? (
      <Banner
        status="warning"
        title="This gift can no longer be accepted"
        description={`It was open until ${dateTime(gift.expires)}.${gift.from ? ` Ask ${gift.from} about it.` : ''}`}
      />
    ) : null;
  return (
    <Grid columns={{minWidth: 320, max: 2}} gap={6} align="start">
      <GiftSummary gift={gift} />
      {status ?? <GiftAcceptFlow gift={gift} canAccept={canAccept} signInHref={signInHref} />}
    </Grid>
  );
}
