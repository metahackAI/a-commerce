'use client';

// Subtotal, discounts, shipping and total exactly as the block carries them (cart, checkout_card,
// order_detail). Discounts are positive amounts taken off (sales.md); they show with a minus sign.

import {Divider} from '@astryxdesign/core/Divider';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';

import {useAiTarget} from '@/ai/useAiTarget';

import {list} from './common';
import type {MoneyLike} from './data';
import {isZeroMoney, moneyText} from './values';

function Row({label, value, strong}: {label: string; value: string; strong?: boolean}) {
  return (
    <HStack gap={2} justify="between" vAlign="center">
      <Text type={strong ? 'body' : 'supporting'} weight={strong ? 'semibold' : undefined} color={strong ? 'primary' : 'secondary'}>
        {label}
      </Text>
      <Text type={strong ? 'large' : 'body'} weight={strong ? 'semibold' : undefined} hasTabularNumbers>
        {value}
      </Text>
    </HStack>
  );
}

function DiscountRow({blockId, label, amount}: {blockId: string; label: string; amount: MoneyLike}) {
  const value = moneyText(amount);
  const target = useAiTarget({kind: 'discount', label: `${label} · −${value}`, ids: {discount: label}, term: 'schema:PriceSpecification', blockId});
  return (
    <HStack gap={2} justify="between" vAlign="center" ref={target.ref} {...target.props}>
      <Text type="supporting" color="secondary">
        {label}
      </Text>
      <Text type="body" hasTabularNumbers>
        {value ? `−${value}` : ''}
      </Text>
    </HStack>
  );
}

export function Totals({
  blockId,
  subtotal,
  discounts,
  shipping,
  extra,
  total,
  totalLabel = 'Total',
}: {
  blockId: string;
  subtotal?: MoneyLike;
  discounts?: {label: string; amount: MoneyLike}[] | null;
  shipping?: MoneyLike;
  /** Other lines the block carries as they are (gift wrap). */
  extra?: {label: string; value: MoneyLike}[];
  total?: MoneyLike;
  totalLabel?: string;
}) {
  const totalText = moneyText(total);
  const target = useAiTarget({kind: 'price', label: `${totalLabel} · ${totalText}`, ids: {total: blockId}, term: 'schema:PriceSpecification', blockId});
  return (
    <VStack gap={1.5}>
      {subtotal !== undefined && subtotal !== null ? <Row label="Subtotal" value={moneyText(subtotal)} /> : null}
      {list(discounts).map(d => (
        <DiscountRow key={d.label} blockId={blockId} label={d.label} amount={d.amount} />
      ))}
      {(extra ?? []).map(e => (
        <Row key={e.label} label={e.label} value={moneyText(e.value)} />
      ))}
      {shipping !== undefined && shipping !== null ? (
        <Row label="Shipping" value={isZeroMoney(shipping) ? 'Free' : moneyText(shipping)} />
      ) : null}
      {totalText ? (
        <>
          <Divider />
          <VStack ref={target.ref} {...target.props}>
            <Row label={totalLabel} value={totalText} strong />
          </VStack>
        </>
      ) : null}
    </VStack>
  );
}
