'use client';

// Subtotal, discounts, shipping and total exactly as get_cart / prepare_order returned them
// (calculate_benefits runs on the server). Discount amounts are positive numbers the API subtracts;
// the minus sign is display only.

import {Divider} from '@astryxdesign/core/Divider';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';

import {useAiTarget} from '@/ai/useAiTarget';
import {day, money} from '@/app/(storefront)/_lib/format';
import type {DeliverySummary, Discount, Money, PaymentSummary, RecordedCoupon} from '@/app/(storefront)/_lib/types';

function Row({label, value, kind, id, strong, term}: {label: string; value: string; kind: string; id: string; strong?: boolean; term?: string}) {
  const target = useAiTarget({kind, label: `${label} ${value}`, ids: {[kind]: id}, term});
  return (
    <HStack gap={2} justify="between" vAlign="center" ref={target.ref} {...target.props}>
      <Text type={strong ? 'large' : 'body'} weight={strong ? 'semibold' : undefined} color={strong ? 'primary' : 'secondary'}>
        {label}
      </Text>
      <Text type={strong ? 'large' : 'body'} weight={strong ? 'semibold' : undefined} hasTabularNumbers>
        {value}
      </Text>
    </HStack>
  );
}

export function CartSummary({
  subtotal,
  discounts,
  shipping,
  total,
  giftWrap,
  delivery,
  payment,
  coupons,
}: {
  subtotal: Money;
  discounts: Discount[];
  shipping: Money;
  total: Money;
  giftWrap?: Money | null;
  delivery?: DeliverySummary | null;
  payment?: PaymentSummary | null;
  coupons?: RecordedCoupon[];
}) {
  return (
    <VStack gap={2}>
      <Row label="Subtotal" value={money(subtotal)} kind="subtotal" id="subtotal" />
      {discounts.map((d, i) => (
        <Row
          key={`${d.promotion_id ?? d.label}-${i}`}
          label={d.label}
          value={`−${money(d.amount)}`}
          kind="discount"
          id={d.promotion_id ?? d.label}
          term="schema:PriceSpecification"
        />
      ))}
      {giftWrap ? <Row label="Gift wrap" value={money(giftWrap)} kind="gift_wrap" id="gift_wrap" /> : null}
      <Row label="Shipping" value={shipping.value === 0 ? 'Free' : money(shipping)} kind="shipping" id="shipping" />
      <Divider />
      <Row label="Total" value={money(total)} kind="total" id="total" strong term="schema:PriceSpecification" />
      {coupons && coupons.length > 0 ? (
        <Text type="supporting" color="secondary">
          Recorded on the order: {coupons.map(c => c.code ?? c.coupon_id).join(', ')}
        </Text>
      ) : null}
      {delivery?.address_label || delivery?.arrives ? (
        <VStack gap={0.5}>
          <Text type="supporting" color="secondary">
            Ship to
          </Text>
          {delivery.address_label ? <Text>{delivery.address_label}</Text> : null}
          {delivery.arrives ? (
            <Text type="supporting" color="secondary">
              Arrives {day(delivery.arrives)}
            </Text>
          ) : null}
        </VStack>
      ) : null}
      {payment?.label ? (
        <VStack gap={0.5}>
          <Text type="supporting" color="secondary">
            Pay with
          </Text>
          <Text>{payment.label}</Text>
        </VStack>
      ) : null}
    </VStack>
  );
}
