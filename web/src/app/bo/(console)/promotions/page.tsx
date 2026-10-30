import {CouponCard} from '@/components/bo/PromotionView';
import {ErrorState} from '@/components/bo/ErrorState';
import {FilterBar} from '@/components/bo/FilterBar';
import {NavTabs} from '@/components/bo/NavTabs';
import {PageFrame, PageSection} from '@/components/bo/PageFrame';
import {Pager} from '@/components/bo/Pager';
import {CouponList, PromotionList} from '@/components/bo/lists/promotion';

import {endOfDay, isoDate, param, params, read, readOptional, type SearchParams} from '../../_lib/server';
import type {Coupon, CouponSummary, Page, PromotionSummary, Tier} from '../../_lib/types';

export const metadata = {title: 'Promotions'};

const STATUSES = ['draft', 'scheduled', 'running', 'paused', 'ended'];
const label = (s: string) => s.charAt(0).toUpperCase() + s.slice(1).replace(/_/g, ' ');

// Promotions in three views: all of them (search_promotions), the ones overlapping a period with
// their scope (list_active_promotions), and coupons (search_coupons, get_coupon for one).
export default async function PromotionsPage({searchParams}: {searchParams: SearchParams}) {
  const sp = await searchParams;
  const view = param(sp, 'view') ?? 'all';
  const cursor = param(sp, 'cursor');
  const tiers = await readOptional<Page<Tier>>('/v1/customer/tiers', {limit: 100});
  const tierOptions = [{value: 'everyone', label: 'Everyone'}, ...(tiers?.items ?? []).map(t => ({value: t.id, label: t.name}))];

  const tabs = (
    <NavTabs
      value={view}
      tabs={[
        {value: 'all', label: 'All promotions', href: '/bo/promotions'},
        {value: 'period', label: 'Running in a period', href: '/bo/promotions?view=period'},
        {value: 'coupons', label: 'Coupons', href: '/bo/promotions?view=coupons'},
      ]}
    />
  );

  if (view === 'period') {
    const values = {
      view,
      valid_from: param(sp, 'valid_from') ?? isoDate(0),
      valid_through: param(sp, 'valid_through') ?? isoDate(7),
      status: params(sp, 'status'),
      valid_for_member_tier: param(sp, 'valid_for_member_tier'),
    };
    const list = await read<Page<PromotionSummary>>('/v1/promotion/promotions/active', {
      valid_from: values.valid_from,
      valid_through: endOfDay(values.valid_through),
      status: values.status,
      valid_for_member_tier: values.valid_for_member_tier,
      limit: 100,
      cursor,
    });
    return (
      <PageFrame
        title="Promotions"
        description="Promotions that overlap the period, running and scheduled by default: the same data the overlap check reads."
        toolbar={
          <>
            {tabs}
            <FilterBar
              values={values}
              keep={{view}}
              filters={[
                {kind: 'date', key: 'valid_from', label: 'From'},
                {kind: 'date', key: 'valid_through', label: 'Through'},
                {kind: 'multi', key: 'status', label: 'Status', placeholder: 'Running and scheduled', options: STATUSES.map(s => ({value: s, label: label(s)}))},
                {kind: 'select', key: 'valid_for_member_tier', label: 'Reaches', options: tierOptions},
              ]}
            />
          </>
        }>
        {list.ok ? (
          <>
            <PromotionList items={list.data.items} />
            <Pager base="/bo/promotions" params={{...values, cursor}} shown={list.data.items.length} total={list.data.total} nextCursor={list.data.next_cursor} noun="promotions" />
          </>
        ) : (
          <ErrorState error={list.error} title="The promotions in this period could not be loaded." />
        )}
      </PageFrame>
    );
  }

  if (view === 'coupons') {
    const values = {
      view,
      code: param(sp, 'code'),
      promotion_id: param(sp, 'promotion_id'),
      application: param(sp, 'application'),
      valid_for_member_tier: param(sp, 'valid_for_member_tier'),
    };
    const couponId = param(sp, 'coupon');
    const [list, coupon] = await Promise.all([
      read<Page<CouponSummary>>('/v1/promotion/coupons', {
        code: values.code,
        promotion_id: values.promotion_id,
        application: values.application,
        valid_for_member_tier: values.valid_for_member_tier,
        limit: 50,
        cursor,
      }),
      couponId ? read<Coupon>(`/v1/promotion/coupons/${encodeURIComponent(couponId)}`) : Promise.resolve(null),
    ]);
    return (
      <PageFrame
        title="Promotions"
        description="Coupon definitions. Open one to see how many members hold, used or lost it."
        toolbar={
          <>
            {tabs}
            <FilterBar
              values={values}
              keep={{view}}
              filters={[
                {kind: 'text', key: 'code', label: 'Code', placeholder: 'VIPWEEKEND20', width: 180},
                {kind: 'text', key: 'promotion_id', label: 'Promotion id', placeholder: 'promo_…', width: 180},
                {
                  kind: 'select',
                  key: 'application',
                  label: 'Applies',
                  options: ['automatic_at_checkout', 'code_entry', 'download'].map(s => ({value: s, label: label(s)})),
                },
                {kind: 'select', key: 'valid_for_member_tier', label: 'For', options: tierOptions},
              ]}
            />
          </>
        }>
        {coupon ? (
          <PageSection title="Coupon">
            {coupon.ok ? <CouponCard coupon={coupon.data} /> : <ErrorState error={coupon.error} title="This coupon could not be loaded." />}
          </PageSection>
        ) : null}
        {list.ok ? (
          <>
            <CouponList items={list.data.items} />
            <Pager base="/bo/promotions" params={{...values, cursor}} shown={list.data.items.length} total={list.data.total} nextCursor={list.data.next_cursor} noun="coupons" />
          </>
        ) : (
          <ErrorState error={list.error} title="The coupons could not be loaded." />
        )}
      </PageFrame>
    );
  }

  const values = {
    query: param(sp, 'query'),
    status: params(sp, 'status'),
    discount_layer: param(sp, 'discount_layer'),
    valid_for_member_tier: param(sp, 'valid_for_member_tier'),
    valid_from: param(sp, 'valid_from'),
    valid_through: param(sp, 'valid_through'),
  };
  const list = await read<Page<PromotionSummary>>('/v1/promotion/promotions', {
    ...values,
    valid_through: endOfDay(values.valid_through),
    limit: 50,
    cursor,
  });
  return (
    <PageFrame
      title="Promotions"
      description="Every promotion, whatever its state. Open one to pause it, end it early, or see its coupons."
      toolbar={
        <>
          {tabs}
          <FilterBar
            values={values}
            filters={[
              {kind: 'text', key: 'query', label: 'Name', placeholder: 'VIP weekend'},
              {kind: 'multi', key: 'status', label: 'Status', options: STATUSES.map(s => ({value: s, label: label(s)}))},
              {kind: 'select', key: 'discount_layer', label: 'Layer', options: ['item_price', 'order_discount', 'add_on'].map(s => ({value: s, label: label(s)}))},
              {kind: 'select', key: 'valid_for_member_tier', label: 'For', options: tierOptions},
              {kind: 'date', key: 'valid_from', label: 'Overlaps from'},
              {kind: 'date', key: 'valid_through', label: 'Overlaps through'},
            ]}
          />
        </>
      }>
      {list.ok ? (
        <>
          <PromotionList items={list.data.items} />
          <Pager base="/bo/promotions" params={{...values, cursor}} shown={list.data.items.length} total={list.data.total} nextCursor={list.data.next_cursor} noun="promotions" />
        </>
      ) : (
        <ErrorState error={list.error} title="The promotions could not be loaded." />
      )}
    </PageFrame>
  );
}
