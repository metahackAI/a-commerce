'use client';

// return_options: what the shopper can do about an item (chat.md §3.3, get_return_options, scene
// F5): each choice with its facts as the store wrote them. Sending the request is the shopper's
// own person-only button, carried by the block (POST /v1/fulfillment/returns).

import {Card} from '@astryxdesign/core/Card';
import {Grid} from '@astryxdesign/core/Grid';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {List, ListItem} from '@astryxdesign/core/List';
import {Stepper, Step} from '@astryxdesign/core/Stepper';
import {Text} from '@astryxdesign/core/Text';

import type {UiBlock} from '@/ai/types';
import {useAiTarget} from '@/ai/useAiTarget';
import {day, timeWindow, words} from '@/app/(storefront)/_lib/format';
import {Intents} from '@/components/fo/Ask';

import type {BlockProps} from '../types';
import {ActionButton, BlockActions, BlockFrame, dataOf, intentsFor, list} from './common';
import type {ReturnOptionsData} from './data';
import {moneyText, textOf} from './values';
import {StatusTag} from '@/components/fo/StatusTag';

type Option = ReturnOptionsData['options'][number];

function sendBackText(o: Option): string {
  const sb = o.send_back;
  if (!sb) return '';
  const parts: string[] = [];
  if (sb.pickup) parts.push(typeof sb.pickup === 'string' ? `Pickup: ${sb.pickup}` : 'Pickup');
  if (sb.drop_off) parts.push(typeof sb.drop_off === 'string' ? `Drop off: ${sb.drop_off}` : 'Drop off');
  if (parts.length === 0 && Array.isArray(sb.methods)) parts.push(...sb.methods.map(words));
  if (sb.pickup_window) parts.push(timeWindow(sb.pickup_window.start, sb.pickup_window.end));
  return parts.join(' · ');
}

function OptionCard({block, option, isSelected}: {block: UiBlock; option: Option; isSelected: boolean}) {
  const target = useAiTarget({kind: 'return_option', label: option.title ?? words(option.kind), ids: {kind: option.kind}, term: 'schema:ReturnAction', blockId: block.block_id});
  const facts = list(option.facts);
  const amount = moneyText(option.refund_amount);
  const sendBack = sendBackText(option);
  const actions = block.actions.filter(a => a.kind === 'say' && a.args?.kind === option.kind);
  return (
    <Card ref={target.ref} {...target.props} variant={isSelected ? 'default' : 'muted'}>
      <VStack gap={2}>
        <HStack gap={2} vAlign="center" wrap="wrap">
          <Text type="body" weight="semibold">
            {option.title ?? words(option.kind)}
          </Text>
          {isSelected ? <StatusTag label="Chosen" tone="success" /> : null}
        </HStack>
        {amount ? (
          <Text type="large" weight="semibold" hasTabularNumbers>
            {amount}
          </Text>
        ) : null}
        {facts.length > 0 ? (
          <List density="compact" listStyle="disc">
            {facts.map(f => (
              <ListItem key={f} label={<Text type="body">{f}</Text>} />
            ))}
          </List>
        ) : null}
        {sendBack ? (
          <Text type="supporting" color="secondary">
            Send it back: {sendBack}
          </Text>
        ) : null}
        {actions.length > 0 ? (
          <HStack gap={1} wrap="wrap">
            {actions.map(a => (
              <ActionButton blockId={block.block_id} key={a.action_id} action={a} />
            ))}
          </HStack>
        ) : null}
        <Intents says={intentsFor(block, 'return_option', option.kind)} />
      </VStack>
    </Card>
  );
}

export function ReturnOptionsBlock({block}: BlockProps) {
  const data = dataOf<ReturnOptionsData>(block);
  const order = data.order ?? {};
  const options = list(data.options);
  const steps = list(data.steps);
  const optionActionIds = new Set(
    block.actions.filter(a => a.kind === 'say' && options.some(o => a.args?.kind === o.kind)).map(a => a.action_id),
  );
  const rest = {...block, actions: block.actions.filter(a => !optionActionIds.has(a.action_id))};
  const photos = Array.isArray(order.photos) ? order.photos.length : order.photos;
  const orderTarget = useAiTarget({
    kind: 'order_row',
    label: [order.order_number ? `#${order.order_number}` : null, order.item].filter(Boolean).join(' · ') || 'Order',
    ids: {order_number: order.order_number ?? block.block_id},
    term: 'schema:Order',
    blockId: block.block_id,
  });
  const activeStep = steps.length > 0 ? 0 : -1;

  return (
    <BlockFrame block={block} footer={<BlockActions block={rest} />}>
      {order.order_number || order.item ? (
        <VStack gap={0.5} ref={orderTarget.ref} {...orderTarget.props}>
          <Text type="body" weight="semibold">
            {[order.order_number ? `Order #${order.order_number}` : null, order.item].filter(Boolean).join(' · ')}
          </Text>
          <Text type="supporting" color="secondary">
            {[
              order.delivered_at ? `Delivered ${day(order.delivered_at)}` : null,
              typeof photos === 'number' && photos > 0 ? `${photos} photo${photos === 1 ? '' : 's'} attached` : null,
            ]
              .filter(Boolean)
              .join(' · ')}
          </Text>
        </VStack>
      ) : null}
      <Grid columns={{minWidth: 240}} gap={3}>
        {options.map(o => (
          <OptionCard key={o.kind} block={block} option={o} isSelected={data.selected === o.kind} />
        ))}
      </Grid>
      {steps.length > 0 ? (
        <Stepper activeStep={activeStep} orientation="vertical">
          {steps.map((s, i) => {
            const label = typeof s === 'string' ? s : textOf(s.label);
            const detail = typeof s === 'string' ? undefined : (s.detail ?? undefined);
            return <Step key={`${label}-${i}`} step={i} label={label} description={detail} />;
          })}
        </Stepper>
      ) : null}
    </BlockFrame>
  );
}
