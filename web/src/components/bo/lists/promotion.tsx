'use client';

// Lists of the promotion domain: promotions and coupons.

import {VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';

import {count, period, words} from '@/app/bo/_lib/format';
import {withParams} from '@/app/bo/_lib/links';
import type {AppliesTo, CouponSummary, PromotionSummary} from '@/app/bo/_lib/types';

import {StateToken} from '../StateToken';
import {DataTable, RecordCell} from '../tables';

export function appliesToText(a: AppliesTo | undefined): string {
  if (!a) return '—';
  if (a.scope === 'order') return 'Whole order';
  const parts = [words(a.scope)];
  if (a.category_ids && a.category_ids.length > 0) parts.push(a.category_ids.join(', '));
  if (typeof a.product_count === 'number') parts.push(`${count(a.product_count)} products`);
  return parts.join(' · ');
}

export function PromotionList({items}: {items: PromotionSummary[]}) {
  return (
    <DataTable
      rows={items}
      rowKey={r => r.id}
      empty={{title: 'No promotions match', description: 'Try other filters or another period.'}}
      target={r => ({
        kind: 'promotion_row',
        label: `${r.name} · ${r.status} · ${r.valid_for_member_tier} · ${period(r.valid_from, r.valid_through)}`,
        ids: {promotion_id: r.id},
        term: 'schema:Offer',
      })}
      columns={[
        {header: 'Promotion', cell: r => <RecordCell href={`/bo/promotions/${encodeURIComponent(r.id)}`} name={r.name} id={r.id} />},
        {header: 'Status', cell: r => <StateToken state={r.status} />},
        {header: 'Discount layer', cell: r => <Text>{words(r.discount_layer)}</Text>},
        {header: 'For', cell: r => <Text>{r.valid_for_member_tier}</Text>},
        {header: 'When', cell: r => <Text>{period(r.valid_from, r.valid_through)}</Text>},
        {header: 'Applies to', cell: r => <Text color="secondary">{appliesToText(r.applies_to)}</Text>},
      ]}
    />
  );
}

export function CouponList({items}: {items: CouponSummary[]}) {
  return (
    <DataTable
      rows={items}
      rowKey={r => r.id}
      empty={{title: 'No coupons match'}}
      target={r => ({
        kind: 'coupon_row',
        label: `Coupon ${r.code} · ${r.valid_for_member_tier} · ${words(r.application)}`,
        ids: {coupon_id: r.id},
        term: 'acom:Coupon',
      })}
      columns={[
        {header: 'Coupon', cell: r => <RecordCell href={withParams('/bo/promotions', {view: 'coupons', coupon: r.id})} name={r.code} id={r.id} />},
        {
          header: 'Promotion',
          cell: r => (r.promotion_id ? <Link href={`/bo/promotions/${encodeURIComponent(r.promotion_id)}`}>{r.promotion_id}</Link> : <Text color="secondary">—</Text>),
        },
        {header: 'For', cell: r => <Text>{r.valid_for_member_tier}</Text>},
        {
          header: 'How it applies',
          cell: r => (
            <VStack gap={0}>
              <Text>{words(r.application)}</Text>
              <Text type="supporting" color="secondary">
                {words(r.stacking)}
              </Text>
            </VStack>
          ),
        },
        {header: 'When', cell: r => <Text>{period(r.valid_from, r.valid_through)}</Text>},
      ]}
    />
  );
}
