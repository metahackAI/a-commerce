'use client';

// gift_picker: a few picks for someone else, the gift options, and where it goes (chat.md §3.3,
// scene F4). The recipient's details are for this order only; the block says so. Ordering stays
// with the shopper: the `open` button leads to the order confirmation.

import {Card} from '@astryxdesign/core/Card';
import {Grid} from '@astryxdesign/core/Grid';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Text} from '@astryxdesign/core/Text';

import type {UiBlock} from '@/ai/types';
import {useAiTarget} from '@/ai/useAiTarget';
import {day, imageUrl} from '@/app/(storefront)/_lib/format';
import type {ImageRef} from '@/app/(storefront)/_lib/types';
import {ProductCard} from '@/components/fo/ProductCard';

import type {BlockProps} from '../types';
import {ActionButton, BlockActions, BlockFrame, dataOf, intentsFor, list} from './common';
import type {GiftPickerData} from './data';
import {cardPrice, moneyText, textOf} from './values';
import {StatusTag} from '@/components/fo/StatusTag';

type Pick = GiftPickerData['picks'][number];

/** The say button that chooses this pick (`args.product_id`), if the block carries one. */
function chooseAction(block: UiBlock, productId: string) {
  return block.actions.find(a => a.kind === 'say' && a.args?.product_id === productId);
}

function PickCard({block, pick}: {block: UiBlock; pick: Pick}) {
  const action = chooseAction(block, pick.product_id);
  return (
    <Card variant={pick.chosen_size ? 'default' : 'muted'}>
      <VStack gap={2}>
        <ProductCard
          productId={pick.product_id}
          name={pick.name}
          {...cardPrice(pick.price, pick.list_price, pick.price_label)}
          image={imageUrl(pick.image as ImageRef)}
          caption={pick.why}
          blockId={block.block_id}
          intents={intentsFor(block, 'product_card', pick.product_id)}
          canCompare={false}
        />
        <HStack gap={2} vAlign="center" wrap="wrap">
          {pick.chosen_size ? <StatusTag label={`Chosen · ${pick.chosen_size}`} tone="success" /> : null}
          {action && !pick.chosen_size ? <ActionButton blockId={block.block_id} action={action} /> : null}
        </HStack>
      </VStack>
    </Card>
  );
}

function Total({block, text}: {block: UiBlock; text: string}) {
  const target = useAiTarget({kind: 'price', label: `Gift order total · ${text}`, ids: {total: block.block_id}, term: 'schema:PriceSpecification', blockId: block.block_id});
  return (
    <VStack ref={target.ref} {...target.props}>
      <Text type="body" weight="semibold" hasTabularNumbers>
        {text}
      </Text>
    </VStack>
  );
}

function yesNo(v: unknown, yes: string, no: string): string {
  if (typeof v === 'boolean') return v ? yes : no;
  return textOf(v);
}

export function GiftPickerBlock({block}: BlockProps) {
  const data = dataOf<GiftPickerData>(block);
  const picks = list(data.picks);
  const options = data.gift_options ?? {};
  const delivery = data.delivery ?? {};
  const chooseIds = new Set(picks.map(p => chooseAction(block, p.product_id)?.action_id).filter(Boolean));
  const rest = {...block, actions: block.actions.filter(a => !chooseIds.has(a.action_id))};
  const total = moneyText(data.total);

  return (
    <BlockFrame
      block={block}
      footer={
        <HStack gap={3} justify="between" vAlign="center" wrap="wrap">
          <Text type="supporting" color="secondary">
            Northwind prepares the order. Only you can place it.
          </Text>
          <BlockActions block={rest} />
        </HStack>
      }>
      <Grid columns={{minWidth: 200}} gap={3}>
        {picks.map(p => (
          <PickCard key={p.product_id} block={block} pick={p} />
        ))}
      </Grid>
      <Grid columns={{minWidth: 260}} gap={3}>
        <Card>
          <VStack gap={2}>
            <Text type="label" weight="semibold">
              Gift options
            </Text>
            <MetadataList>
              {options.wrap !== undefined && options.wrap !== null ? (
                <MetadataListItem label="Wrap">{yesNo(options.wrap, 'Gift wrap', 'No wrap')}</MetadataListItem>
              ) : null}
              {options.card_message ? <MetadataListItem label="Card message">{`“${options.card_message}”`}</MetadataListItem> : null}
              {options.receipt !== undefined && options.receipt !== null ? (
                <MetadataListItem label="Receipt">{yesNo(options.receipt, 'Gift receipt, no prices shown', 'Regular receipt')}</MetadataListItem>
              ) : null}
              {options.exchange_until ? <MetadataListItem label="Exchange">{`Size exchange until ${day(options.exchange_until)}`}</MetadataListItem> : null}
            </MetadataList>
          </VStack>
        </Card>
        <Card>
          <VStack gap={2}>
            <Text type="label" weight="semibold">
              Delivery
            </Text>
            <MetadataList>
              {delivery.to ? <MetadataListItem label="To">{delivery.to}</MetadataListItem> : null}
              {delivery.arrives ? <MetadataListItem label="Arrives">{day(delivery.arrives)}</MetadataListItem> : null}
              {delivery.address_saved === false ? <MetadataListItem label="Address">Used for this order only</MetadataListItem> : null}
              {total ? (
                <MetadataListItem label="Total">
                  <Total block={block} text={total} />
                </MetadataListItem>
              ) : null}
            </MetadataList>
          </VStack>
        </Card>
      </Grid>
    </BlockFrame>
  );
}
