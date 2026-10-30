import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';
import {PromotionView} from '@/components/bo/PromotionView';

import {read, readOptional} from '../../../_lib/server';
import type {Coupon, CouponSummary, Page, Promotion} from '../../../_lib/types';

export const metadata = {title: 'Promotion'};

// One promotion (get_promotion), its coupons (search_coupons) and each coupon's counts
// (get_coupon).
export default async function PromotionPage({params}: {params: Promise<{promotion_id: string}>}) {
  const {promotion_id: id} = await params;
  const promotion = await read<Promotion>(`/v1/promotion/promotions/${encodeURIComponent(id)}`);
  if (!promotion.ok) {
    return (
      <PageFrame back={{href: '/bo/promotions', label: 'Promotions'}} title={id}>
        <ErrorState error={promotion.error} title="This promotion could not be loaded." />
      </PageFrame>
    );
  }
  const coupons = await read<Page<CouponSummary>>('/v1/promotion/coupons', {promotion_id: id, limit: 50});
  const couponIds = [...new Set([...(promotion.data.coupon_ids ?? []), ...(coupons.ok ? coupons.data.items.map(c => c.id) : [])])].slice(0, 10);
  const details = (await Promise.all(couponIds.map(c => readOptional<Coupon>(`/v1/promotion/coupons/${encodeURIComponent(c)}`)))).filter(
    (c): c is Coupon => c !== null,
  );

  return (
    <PromotionView
      promotion={promotion.data}
      coupons={coupons.ok ? coupons.data.items : null}
      couponsError={coupons.ok ? null : coupons.error}
      couponDetails={details}
    />
  );
}
