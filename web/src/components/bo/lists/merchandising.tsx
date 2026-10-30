'use client';

// Lists of the merchandising domain: display slots, banners, categories, and one slot's detail.

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';

import {count, period, value, words} from '@/app/bo/_lib/format';
import {entityHref, withParams} from '@/app/bo/_lib/links';
import type {Banner, Booking, Category, DisplaySlot, DisplaySlotSummary} from '@/app/bo/_lib/types';

import {StateToken} from '../StateToken';
import {DataTable, Figure, RecordCell} from '../tables';
import {TargetCard} from '../targets';

function Bookings({bookings}: {bookings: Booking[] | undefined}) {
  if (!bookings || bookings.length === 0) return <Text color="secondary">None</Text>;
  return (
    <VStack gap={1}>
      {bookings.map(b => (
        <VStack key={b.display_booking_id} gap={0}>
          <HStack gap={1} vAlign="center" wrap="wrap">
            <Text weight="medium">{b.headline ?? b.display_booking_id}</Text>
            {b.status ? <StateToken state={b.status} /> : null}
          </HStack>
          <Text type="supporting" color="secondary">
            {`${b.audience} · ${period(b.valid_from, b.valid_through)}`}
          </Text>
        </VStack>
      ))}
    </VStack>
  );
}

/** Free windows with the audiences that share each one, so a slot free for everyone reads once. */
function freeWindows(availability: DisplaySlotSummary['availability']): {window: string; audiences: string[]}[] {
  const out: {window: string; audiences: string[]}[] = [];
  for (const a of availability) {
    const window = period(a.free_from, a.free_until);
    const same = out.find(w => w.window === window);
    if (same) same.audiences.push(a.audience);
    else out.push({window, audiences: [a.audience]});
  }
  return out;
}

export function SlotList({items, keep}: {items: DisplaySlotSummary[]; keep: Record<string, string | string[] | undefined>}) {
  return (
    <DataTable
      rows={items}
      rowKey={r => r.id}
      empty={{title: 'No slots on this page'}}
      target={r => ({
        kind: 'display_slot',
        label: `${r.name} (${r.slot_type}) · ${r.bookings.length} bookings · free for ${r.availability.map(a => a.audience).join(', ') || 'nobody'}`,
        ids: {slot_id: r.id},
        term: 'acom:DisplaySlot',
      })}
      columns={[
        {header: 'Slot', cell: r => <RecordCell href={withParams('/bo/merchandising', {...keep, slot: r.id})} name={r.name} id={r.id} />},
        {header: 'Type', cell: r => <Text>{words(r.slot_type)}</Text>},
        {header: 'Booked', cell: r => <Bookings bookings={r.bookings} />},
        {
          header: 'Free',
          cell: r =>
            r.availability.length === 0 ? (
              <Text color="secondary">Not free in this period</Text>
            ) : (
              <VStack gap={0.5}>
                {freeWindows(r.availability).map(w => (
                  <VStack key={w.window} gap={0}>
                    <Text>{w.window}</Text>
                    <Text type="supporting" color="secondary">
                      {w.audiences.join(', ')}
                    </Text>
                  </VStack>
                ))}
              </VStack>
            ),
        },
      ]}
    />
  );
}

export function SlotDetail({slot}: {slot: DisplaySlot}) {
  return (
    <TargetCard
      target={{kind: 'display_slot', label: `Slot ${slot.name ?? slot.id}, live now: ${slot.live_now ? 'yes' : 'no'}`, ids: {slot_id: slot.id}, term: 'acom:DisplaySlot'}}
      heading={
        <>
          <Text weight="semibold">{slot.name ?? slot.id}</Text>
          <HStack gap={1} vAlign="center">
            <Text type="supporting" color="secondary">
              {`${slot.id} · ${words(slot.slot_type)}`}
            </Text>
            <StateToken state={slot.live_now ? 'live' : 'gray'} label={slot.live_now ? 'Showing now' : 'Not showing now'} />
          </HStack>
        </>
      }>
      <MetadataList columns="multi">
        <MetadataListItem label="Bookings">
          <Bookings bookings={slot.bookings} />
        </MetadataListItem>
        <MetadataListItem label="Items in the slot">{count(slot.item_list_element?.length ?? 0)}</MetadataListItem>
        <MetadataListItem label="Hidden now">
          {slot.hidden_items && slot.hidden_items.length > 0 ? (
            <VStack gap={0.5}>
              {slot.hidden_items.map(h => (
                <HStack key={h.product_id} gap={1}>
                  {entityHref(h.product_id) ? <Link href={entityHref(h.product_id) ?? '#'}>{h.product_id}</Link> : <Text>{h.product_id}</Text>}
                  <Text color="secondary">{words(h.reason)}</Text>
                </HStack>
              ))}
            </VStack>
          ) : (
            'Nothing hidden'
          )}
        </MetadataListItem>
      </MetadataList>
    </TargetCard>
  );
}

export function BannerList({items}: {items: Banner[]}) {
  return (
    <DataTable
      rows={items}
      rowKey={r => r.id}
      empty={{title: 'No banners match'}}
      target={r => ({kind: 'banner', label: `Banner ${r.id} → ${r.url ?? ''} on ${(r.channels ?? []).join(', ')}`, ids: {banner_id: r.id}, term: 'schema:ImageObject'})}
      columns={[
        {
          header: 'Banner',
          cell: r => (
            <VStack gap={0}>
              <Text weight="medium">{r.id}</Text>
              {r.content_url ? (
                <Link href={r.content_url} isExternalLink color="secondary">
                  Image
                </Link>
              ) : null}
            </VStack>
          ),
        },
        {header: 'Links to', cell: r => <Text>{r.url ?? '—'}</Text>},
        {header: 'Channels', cell: r => <Text>{(r.channels ?? []).map(words).join(', ') || '—'}</Text>},
        {header: 'Priority', cell: r => <Figure>{r.priority !== undefined ? count(r.priority) : '—'}</Figure>},
        {header: 'Booked', cell: r => <Bookings bookings={r.bookings} />},
      ]}
    />
  );
}

export function CategoryList({items}: {items: Category[]}) {
  const names = new Map(items.map(c => [c.id, c.name]));
  return (
    <DataTable
      rows={items}
      rowKey={r => r.id}
      empty={{title: 'No categories'}}
      target={r => ({kind: 'category', label: `Category ${r.name} (${r.term_set})${r.active ? '' : ', inactive'}`, ids: {category_id: r.id}, term: 'schema:CategoryCode'})}
      columns={[
        {header: 'Category', cell: r => <RecordCell href={withParams('/bo/products', {category_id: r.id})} name={r.name} id={r.id} />},
        {header: 'Code', cell: r => <Text color="secondary">{r.term_code ?? '—'}</Text>},
        {
          header: 'Parent',
          cell: r =>
            r.parent_id ? (
              <Link href={withParams('/bo/merchandising', {view: 'categories', parent_id: r.parent_id})}>{names.get(r.parent_id) ?? r.parent_id}</Link>
            ) : (
              <Text color="secondary">Top level</Text>
            ),
        },
        {header: 'Order', cell: r => <Figure muted>{value(r.display_order)}</Figure>},
        {header: 'State', cell: r => <StateToken state={r.active ? 'active' : 'gray'} label={r.active ? 'Active' : 'Inactive'} />},
      ]}
    />
  );
}
