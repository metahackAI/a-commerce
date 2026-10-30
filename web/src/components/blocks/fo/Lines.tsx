'use client';

// The lines of a cart, a prepared order, or a placed order as rows. Each row is an AI target
// (`cart_line` or `order_row`) carrying the agent's suggested sentences for it.

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {List, ListItem} from '@astryxdesign/core/List';
import {Text} from '@astryxdesign/core/Text';
import {Thumbnail} from '@astryxdesign/core/Thumbnail';

import type {UiBlock} from '@/ai/types';
import {useAiTarget} from '@/ai/useAiTarget';
import {imageUrl, words} from '@/app/(storefront)/_lib/format';
import type {ImageRef} from '@/app/(storefront)/_lib/types';
import {AskButton, Intents} from '@/components/fo/Ask';

import {intentsFor} from './common';
import type {MoneyLike} from './data';
import {moneyText} from './values';

export interface LineView {
  /** cart_line_id or order item id, when the block has one. */
  id?: string;
  product_id?: string;
  name: string;
  color?: string | null;
  size?: string | null;
  quantity?: number;
  price?: MoneyLike;
  note?: string | null;
  status?: string | null;
  image?: unknown;
}

function Line({block, line, kind, index}: {block: UiBlock; line: LineView; kind: 'cart_line' | 'order_row'; index: number}) {
  const id = line.id ?? line.product_id ?? `${index}`;
  const idKey = kind === 'cart_line' ? 'cart_line_id' : 'order_item_id';
  const ids: Record<string, string> = {[idKey]: id};
  if (line.product_id) ids.product_id = line.product_id;
  const details = [line.color, line.size, typeof line.quantity === 'number' ? `Qty ${line.quantity}` : null].filter(Boolean).join(' · ');
  const target = useAiTarget({kind, label: [line.name, details].filter(Boolean).join(' · '), ids, term: 'schema:OrderItem', blockId: block.block_id});
  const price = moneyText(line.price);
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      align="start"
      startContent={<Thumbnail src={imageUrl(line.image as ImageRef) ?? undefined} alt={line.name} label={line.name} />}
      label={
        line.product_id ? (
          <Link href={`/p/${encodeURIComponent(line.product_id)}`}>
            <Text type="body" weight="semibold">
              {line.name}
            </Text>
          </Link>
        ) : (
          <Text type="body" weight="semibold">
            {line.name}
          </Text>
        )
      }
      description={
        <VStack gap={0.5}>
          {details ? (
            <Text type="supporting" color="secondary">
              {details}
            </Text>
          ) : null}
          {line.status ? (
            <Text type="supporting" color="secondary">
              {words(line.status)}
            </Text>
          ) : null}
          {line.note ? (
            <Text type="supporting" color="secondary">
              {line.note}
            </Text>
          ) : null}
          <Intents says={intentsFor(block, kind, id)} />
        </VStack>
      }
      endContent={
        <HStack gap={1} vAlign="center">
          {price ? (
            <Text type="body" hasTabularNumbers>
              {price}
            </Text>
          ) : null}
          <AskButton target={target} label={line.name} />
        </HStack>
      }
    />
  );
}

export function Lines({block, lines, kind}: {block: UiBlock; lines: LineView[]; kind: 'cart_line' | 'order_row'}) {
  if (lines.length === 0) {
    return (
      <Text type="supporting" color="secondary">
        No lines yet.
      </Text>
    );
  }
  return (
    <List hasDividers edge="flush" density="compact">
      {lines.map((l, i) => (
        <Line key={l.id ?? `${l.product_id ?? l.name}-${i}`} block={block} line={l} kind={kind} index={i} />
      ))}
    </List>
  );
}
