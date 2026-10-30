import {Button} from '@astryxdesign/core/Button';

import {ErrorState} from '@/components/bo/ErrorState';
import {FilterBar} from '@/components/bo/FilterBar';
import {NavTabs} from '@/components/bo/NavTabs';
import {PageFrame, PageSection} from '@/components/bo/PageFrame';
import {Pager} from '@/components/bo/Pager';
import {BannerList, CategoryList, SlotDetail, SlotList} from '@/components/bo/lists/merchandising';

import {endOfDay, isoDate, param, read, readOptional, type SearchParams} from '../../_lib/server';
import type {Banner, Category, DisplaySlot, DisplaySlotSummary, Page, Tier} from '../../_lib/types';

export const metadata = {title: 'Merchandising'};

const label = (s: string) => s.charAt(0).toUpperCase() + s.slice(1).replace(/_/g, ' ');
const PAGES = ['main', 'category', 'product', 'search'];

// Merchandising in three views: display slots with their bookings and free time for a period
// (list_display_slots, get_display_slot for one), banners (list_banners), categories
// (list_categories).
export default async function MerchandisingPage({searchParams}: {searchParams: SearchParams}) {
  const sp = await searchParams;
  const view = param(sp, 'view') ?? 'slots';
  const cursor = param(sp, 'cursor');
  const tabs = (
    <NavTabs
      value={view}
      tabs={[
        {value: 'slots', label: 'Display slots', href: '/bo/merchandising'},
        {value: 'banners', label: 'Banners', href: '/bo/merchandising?view=banners'},
        {value: 'categories', label: 'Categories', href: '/bo/merchandising?view=categories'},
      ]}
    />
  );

  if (view === 'banners') {
    const values = {view, page: param(sp, 'page'), channel: param(sp, 'channel'), as_of: param(sp, 'as_of')};
    const list = await read<Page<Banner>>('/v1/merchandising/banners', {page: values.page, channel: values.channel, as_of: values.as_of, limit: 50, cursor});
    return (
      <PageFrame
        title="Merchandising"
        description="Banner images, where they link, and the bookings that show them."
        toolbar={
          <>
            {tabs}
            <FilterBar
              values={values}
              keep={{view}}
              filters={[
                {kind: 'select', key: 'page', label: 'Page', options: PAGES.map(p => ({value: p, label: label(p)}))},
                {kind: 'select', key: 'channel', label: 'Channel', options: ['web', 'mobile_web', 'app'].map(c => ({value: c, label: label(c)}))},
                {kind: 'date', key: 'as_of', label: 'Showing on'},
              ]}
            />
          </>
        }>
        {list.ok ? (
          <>
            <BannerList items={list.data.items} />
            <Pager base="/bo/merchandising" params={{...values, cursor}} shown={list.data.items.length} total={list.data.total} nextCursor={list.data.next_cursor} noun="banners" />
          </>
        ) : (
          <ErrorState error={list.error} title="The banners could not be loaded." />
        )}
      </PageFrame>
    );
  }

  if (view === 'categories') {
    const values = {view, term_set: param(sp, 'term_set') ?? 'standard', parent_id: param(sp, 'parent_id')};
    const list = await read<Page<Category>>('/v1/merchandising/categories', {term_set: values.term_set, parent_id: values.parent_id, limit: 100, cursor});
    return (
      <PageFrame
        title="Merchandising"
        description="The category tree. A category under an inactive parent shows as inactive too."
        actions={<Button label="Manage categories" href="/bo/categories" />}
        toolbar={
          <>
            {tabs}
            <FilterBar
              values={values}
              keep={{view}}
              filters={[
                {
                  kind: 'select',
                  key: 'term_set',
                  label: 'Tree',
                  options: [
                    {value: 'standard', label: 'Standard'},
                    {value: 'display', label: 'Display'},
                  ],
                },
                {kind: 'text', key: 'parent_id', label: 'Under', placeholder: 'cat_…', width: 180},
              ]}
            />
          </>
        }>
        {list.ok ? (
          <>
            <CategoryList items={list.data.items} />
            <Pager base="/bo/merchandising" params={{...values, cursor}} shown={list.data.items.length} total={list.data.total} nextCursor={list.data.next_cursor} noun="categories" />
          </>
        ) : (
          <ErrorState error={list.error} title="The categories could not be loaded." />
        )}
      </PageFrame>
    );
  }

  const values = {
    page: param(sp, 'page') ?? 'main',
    valid_from: param(sp, 'valid_from') ?? isoDate(0),
    valid_through: param(sp, 'valid_through') ?? isoDate(7),
    audience: param(sp, 'audience'),
    slot_type: param(sp, 'slot_type'),
  };
  const slotId = param(sp, 'slot');
  const [list, slot, tiers] = await Promise.all([
    read<Page<DisplaySlotSummary>>('/v1/merchandising/display-slots', {
      ...values,
      valid_through: endOfDay(values.valid_through),
      limit: 50,
      cursor,
    }),
    slotId ? read<DisplaySlot>(`/v1/merchandising/display-slots/${encodeURIComponent(slotId)}`) : Promise.resolve(null),
    readOptional<Page<Tier>>('/v1/customer/tiers', {limit: 100}),
  ]);

  return (
    <PageFrame
      title="Merchandising"
      description="Each slot of a page, what is booked in it for the period, and when it is still free for each audience."
      toolbar={
        <>
          {tabs}
          <FilterBar
            values={values}
            filters={[
              {kind: 'select', key: 'page', label: 'Page', options: PAGES.map(p => ({value: p, label: label(p)}))},
              {kind: 'date', key: 'valid_from', label: 'From'},
              {kind: 'date', key: 'valid_through', label: 'Through'},
              {
                kind: 'select',
                key: 'audience',
                label: 'Free for',
                options: [{value: 'everyone', label: 'Everyone'}, ...(tiers?.items ?? []).map(t => ({value: t.id, label: t.name}))],
              },
              {
                kind: 'select',
                key: 'slot_type',
                label: 'Slot type',
                options: ['hero', 'banner_board', 'product_grid', 'brand_strip', 'collection_feed'].map(s => ({value: s, label: label(s)})),
              },
            ]}
          />
        </>
      }>
      {slot ? (
        <PageSection title="Slot">
          {slot.ok ? <SlotDetail slot={slot.data} /> : <ErrorState error={slot.error} title="This slot could not be loaded." />}
        </PageSection>
      ) : null}
      {list.ok ? (
        <>
          <SlotList items={list.data.items} keep={values} />
          <Pager base="/bo/merchandising" params={{...values, cursor}} shown={list.data.items.length} total={list.data.total} nextCursor={list.data.next_cursor} noun="slots" />
        </>
      ) : (
        <ErrorState error={list.error} title="The display slots could not be loaded." />
      )}
    </PageFrame>
  );
}
