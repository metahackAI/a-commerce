'use client';

// One promotion (get_promotion) with its coupons (search_coupons, get_coupon for the counts), and
// the writes an operator makes on it without a change set: pause a running promotion, end one
// early. Both are previewed first; the reason is part of the input.

import {useState} from 'react';
import {useRouter} from 'next/navigation';
import {VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {Button} from '@astryxdesign/core/Button';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {TextInput} from '@astryxdesign/core/TextInput';

import type {ApiErrorBody} from '@/lib/api';
import {count, period, words} from '@/app/bo/_lib/format';
import type {Coupon, CouponSummary, Promotion} from '@/app/bo/_lib/types';

import {useBoSession} from './BoSession';
import {ErrorState} from './ErrorState';
import {PageFrame, PageSection} from './PageFrame';
import {StateToken} from './StateToken';
import {appliesToText, CouponList} from './lists/promotion';
import {TargetCard, TargetFact} from './targets';
import {WriteFlow} from './WriteFlow';

type Verb = 'pause' | 'end';

/** One coupon definition with its counts by coupon status. */
export function CouponCard({coupon}: {coupon: Coupon}) {
  return (
    <TargetCard
      target={{kind: 'coupon', label: `Coupon ${coupon.code}`, ids: {coupon_id: coupon.id}, term: 'acom:Coupon'}}
      heading={
        <>
          <Text weight="semibold">{coupon.code}</Text>
          <Text type="supporting" color="secondary">
            {coupon.id}
          </Text>
        </>
      }>
      <MetadataList columns="multi">
        <MetadataListItem label="Promotion">
          {coupon.promotion_id ? <Link href={`/bo/promotions/${encodeURIComponent(coupon.promotion_id)}`}>{coupon.promotion_id}</Link> : '—'}
        </MetadataListItem>
        <MetadataListItem label="For">{coupon.valid_for_member_tier}</MetadataListItem>
        <MetadataListItem label="Applies">{words(coupon.application)}</MetadataListItem>
        <MetadataListItem label="Stacking">{words(coupon.stacking)}</MetadataListItem>
        <MetadataListItem label="When">{period(coupon.valid_from, coupon.valid_through)}</MetadataListItem>
        {coupon.records_benefit_only !== undefined ? (
          <MetadataListItem label="Discount">{coupon.records_benefit_only ? 'Records the member price; no second discount' : 'Gives its own discount'}</MetadataListItem>
        ) : null}
      </MetadataList>
      {coupon.counts ? (
        <MetadataList columns="multi" title="Member coupons by status">
          {Object.entries(coupon.counts).map(([k, n]) => (
            <TargetFact key={k} label={words(k)} target={{kind: 'coupon_count', label: `${coupon.code} ${k}: ${n}`, ids: {coupon_status: `${coupon.id}:${k}`, coupon_id: coupon.id}}}>
              <Text hasTabularNumbers>{count(n)}</Text>
            </TargetFact>
          ))}
        </MetadataList>
      ) : null}
    </TargetCard>
  );
}

export function PromotionView({
  promotion: p,
  coupons,
  couponsError,
  couponDetails,
}: {
  promotion: Promotion;
  coupons: CouponSummary[] | null;
  couponsError: ApiErrorBody | null;
  couponDetails: Coupon[];
}) {
  const router = useRouter();
  const session = useBoSession();
  const canWrite = session?.scopes.includes('tools:write:promotion') ?? false;
  const [verb, setVerb] = useState<Verb | null>(null);
  const [reason, setReason] = useState('');
  const canPause = p.status === 'running';
  const canEnd = p.status === 'scheduled' || p.status === 'running' || p.status === 'paused';
  const ids = {promotion_id: p.id};

  return (
    <PageFrame
      back={{href: '/bo/promotions', label: 'Promotions'}}
      title={p.name}
      meta={
        <>
          <Text type="supporting" color="secondary">
            {p.id}
          </Text>
          <StateToken state={p.status} size="md" />
        </>
      }
      actions={
        canWrite ? (
          <>
            {canPause ? <Button label="Pause…" onClick={() => setVerb('pause')} /> : null}
            {canEnd ? <Button label="End early…" variant="destructive" onClick={() => setVerb('end')} /> : null}
          </>
        ) : undefined
      }
      panel={
        <MetadataList title="Made by">
          <MetadataListItem label="Change set">
            {p.change_set_id ? <Link href={`/bo/change-sets/${encodeURIComponent(p.change_set_id)}`}>{p.change_set_id}</Link> : '—'}
          </MetadataListItem>
        </MetadataList>
      }>
      <PageSection title="Offer">
        <MetadataList columns="multi">
          <TargetFact label="For" target={{kind: 'promotion_audience', label: `${p.name} is for ${p.valid_for_member_tier}`, ids}}>
            <Text weight="medium">{p.valid_for_member_tier}</Text>
          </TargetFact>
          <TargetFact label="When" target={{kind: 'promotion_period', label: `${p.name}: ${period(p.valid_from, p.valid_through)}`, ids}}>
            <Text weight="medium">{period(p.valid_from, p.valid_through)}</Text>
          </TargetFact>
          <TargetFact label="Discount layer" target={{kind: 'promotion_layer', label: `${p.name} layer ${p.discount_layer}`, ids}}>
            <Text weight="medium">{words(p.discount_layer)}</Text>
          </TargetFact>
          <TargetFact label="Applies to" target={{kind: 'promotion_scope', label: `${p.name} applies to ${appliesToText(p.applies_to)}`, ids}}>
            <Text weight="medium">{appliesToText(p.applies_to)}</Text>
          </TargetFact>
          {typeof p.price_specification_count === 'number' ? (
            <TargetFact label="Member prices" target={{kind: 'promotion_prices', label: `${p.name}: ${p.price_specification_count} member prices`, ids}}>
              <Text weight="medium">{count(p.price_specification_count)}</Text>
            </TargetFact>
          ) : null}
        </MetadataList>
      </PageSection>

      <PageSection title="Coupons">
        {couponsError ? <ErrorState error={couponsError} title="The coupons could not be loaded." /> : null}
        {coupons ? <CouponList items={coupons} /> : null}
        {couponDetails.map(c => (
          <CouponCard key={c.id} coupon={c} />
        ))}
      </PageSection>

      {verb ? (
        <Dialog isOpen onOpenChange={open => !open && setVerb(null)} width={680} purpose="form">
          <DialogHeader
            title={verb === 'pause' ? `Pause ${p.name}` : `End ${p.name} early`}
            subtitle={
              verb === 'pause'
                ? 'While paused, its member prices and coupons do not apply.'
                : 'Ending is final: its member prices stop applying from now on.'
            }
            onOpenChange={open => !open && setVerb(null)}
          />
          <VStack gap={4} padding={4}>
            <WriteFlow
              method="POST"
              path={`/v1/promotion/promotions/${encodeURIComponent(p.id)}/${verb}`}
              input={{promotion_id: p.id, ...(reason.trim() ? {reason: reason.trim()} : {})}}
              previewLabel={verb === 'pause' ? 'Preview pausing' : 'Preview ending'}
              applyLabel={verb === 'pause' ? 'Pause it' : 'End it'}
              askWhenApprovalNeeded={`Prepare a change set to ${verb} the promotion ${p.name} (${p.id})${reason.trim() ? ` because ${reason.trim()}` : ''}.`}
              onApplied={() => router.refresh()}>
              <TextInput label="Reason" isOptional value={reason} onChange={setReason} placeholder="Stock ran out early" width={480} />
            </WriteFlow>
          </VStack>
        </Dialog>
      ) : null}
    </PageFrame>
  );
}
