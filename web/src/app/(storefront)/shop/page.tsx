import {Button} from '@astryxdesign/core/Button';
import {Grid} from '@astryxdesign/core/Grid';
import {HStack, Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';
import {Token} from '@astryxdesign/core/Token';

import {AskEntry} from '@/components/fo/AskEntry';
import {ErrorNotice} from '@/components/fo/ErrorNotice';
import {HeroBanner, type HeroContent} from '@/components/fo/HeroBanner';
import {ProductGrid} from '@/components/fo/ProductGrid';
import {CollectionCards, NoticeList} from '@/components/fo/StoreContent';
import {StorePopup} from '@/components/fo/StorePopup';
import type {Page} from '@/lib/api';

import {collectionPreviews} from './_lib/collections';
import {imageUrl} from './_lib/format';
import {load, orElse, shopper} from './_lib/server';
import type {
  Brand,
  Category,
  CollectionPageSummary,
  DisplayBooking,
  DisplaySlot,
  DisplaySlotSummary,
  NoticeSummary,
  Popup,
  Product,
  ProductGroup,
  ProductSummary,
  Profile,
  SlotItem,
} from './_lib/types';

const GRID_LIMIT = 8;

function itemId(item: SlotItem): string | null {
  if (typeof item === 'string') return item;
  return item.item_id ?? item.product_id ?? item.id ?? null;
}

/**
 * The hero content of a booking. For a shopper, `get_display_slot` returns only the booking that
 * shows to them now (their tier's, else everyone's), so the page never decides who sees what.
 */
async function heroContent(slot: DisplaySlot, booking: DisplayBooking): Promise<HeroContent | null> {
  const banner = booking.banner ?? null;
  const c = booking.content ?? {};
  const headline = banner?.headline ?? c.headline ?? booking.headline;
  if (!headline) return null;
  let image = banner?.content_url ?? c.content_url ?? null;
  const productId = banner?.image_product_id ?? c.image_product_id ?? null;
  if (!image && productId) {
    const group = await load<ProductGroup>(`/v1/catalog/product-groups/${encodeURIComponent(productId)}`, {fields: ['id', 'name', 'image']});
    image = group.ok ? imageUrl(group.data.image) : null;
  }
  return {
    bookingId: booking.display_booking_id,
    slotId: slot.id,
    eyebrow: banner?.eyebrow ?? c.eyebrow ?? booking.eyebrow,
    headline,
    subline: banner?.alternative_headline ?? c.alternative_headline ?? booking.alternative_headline,
    ctaLabel: banner?.cta_label ?? c.cta_label ?? booking.cta_label,
    url: banner?.url ?? c.url ?? booking.url,
    image,
  };
}

async function slotProducts(slot: DisplaySlot): Promise<ProductSummary[]> {
  const hidden = new Set((slot.hidden_items ?? []).map(h => h.product_id));
  const ids = slot.item_list_element
    .map(itemId)
    .filter((id): id is string => !!id && id.startsWith('prd_') && !hidden.has(id))
    .slice(0, GRID_LIMIT);
  const loaded = await Promise.all(ids.map(id => load<Product>(`/v1/catalog/products/${encodeURIComponent(id)}`)));
  return loaded.flatMap(l => (l.ok ? [l.data] : []));
}

/** The store's own day (`2026-11-06`) and the next, from `get_store`'s clock, never the server's. */
function storeDays(now: string | null): [string, string] {
  const today = now ? now.slice(0, 10) : new Date().toISOString().slice(0, 10);
  const next = new Date(`${today}T00:00:00Z`);
  next.setUTCDate(next.getUTCDate() + 1);
  return [today, next.toISOString().slice(0, 10)];
}

type Section =
  | {kind: 'hero'; id: string; hero: HeroContent}
  | {kind: 'products'; id: string; title: string; products: ProductSummary[]}
  | {kind: 'banners'; id: string; title: string; heroes: HeroContent[]}
  | {kind: 'brands'; id: string; title: string; brands: Brand[]};

export default async function StorefrontHome() {
  const who = await shopper();
  const store = await load<{now: string}>('/v1/platform/store');
  const [today, tomorrow] = storeDays(store.ok ? store.data.now : null);
  const [slotList, categories, featured, profile, brandList, collections, notices, popups] = await Promise.all([
    load<Page<DisplaySlotSummary>>('/v1/merchandising/display-slots', {
      page: 'main',
      valid_from: today,
      valid_through: tomorrow,
      limit: 20,
    }),
    load<Page<Category>>('/v1/merchandising/categories', {term_set: 'standard', limit: 50}),
    load<Page<ProductSummary>>('/v1/search/products', {sort: 'newest', limit: GRID_LIMIT}),
    who.isShopper ? load<Profile>('/v1/customer/people/me', {fields: ['name']}) : null,
    load<Page<Brand>>('/v1/catalog/brands', {limit: 50}),
    load<Page<CollectionPageSummary>>('/v1/merchandising/collection-pages', {limit: 6}),
    load<Page<NoticeSummary>>('/v1/customer/notices', {limit: 3}),
    load<Page<Popup>>('/v1/merchandising/popups', {page: 'main'}),
  ]);
  const openCollections = await collectionPreviews(orElse(collections, {items: [], total: 0}).items);
  const latestNotices = orElse(notices, {items: [], total: 0}).items;
  const brands = orElse(brandList, {items: [], total: 0}).items;

  const summaries = orElse(slotList, {items: [], total: 0}).items.sort((a, b) => (a.position ?? 0) - (b.position ?? 0));
  const slots = await Promise.all(summaries.map(s => load<DisplaySlot>(`/v1/merchandising/display-slots/${encodeURIComponent(s.id)}`)));

  const sections: Section[] = [];
  for (const [i, loaded] of slots.entries()) {
    if (!loaded.ok || loaded.data.live_now === false) continue;
    const slot = loaded.data;
    const title = slot.name ?? summaries[i].name ?? '';
    if (slot.slot_type === 'hero') {
      const booking = slot.bookings[0];
      const hero = booking ? await heroContent(slot, booking) : null;
      if (hero) sections.push({kind: 'hero', id: slot.id, hero});
    } else if (slot.slot_type === 'product_grid') {
      const products = await slotProducts(slot);
      if (products.length > 0) sections.push({kind: 'products', id: slot.id, title, products});
    } else if (slot.slot_type === 'banner_board') {
      const heroes = (await Promise.all(slot.bookings.map(b => heroContent(slot, b)))).filter((h): h is HeroContent => !!h);
      if (heroes.length > 0) sections.push({kind: 'banners', id: slot.id, title, heroes});
    } else if (slot.slot_type === 'brand_strip') {
      const ids = new Set(slot.item_list_element.map(itemId));
      const shown = brands.filter(b => ids.has(b.id));
      if (shown.length > 0) sections.push({kind: 'brands', id: slot.id, title, brands: shown});
    }
  }

  const topCategories = orElse(categories, {items: [], total: 0})
    .items.filter(c => c.active !== false && !c.parent_id)
    .sort((a, b) => (a.display_order ?? 0) - (b.display_order ?? 0));

  return (
    <Layout
      height="auto"
      contentWidth={1200}
      content={
        <LayoutContent padding={6}>
          <VStack gap={8}>
            {/* AI-native: the first thing on the page is the way to ask. */}
            <AskEntry name={profile?.ok ? profile.data.name : null} />
            <StorePopup popups={orElse(popups, {items: [], total: 0}).items} />
            {!slotList.ok ? <ErrorNotice error={slotList.error} title="The main page could not be loaded." /> : null}
            {sections.map(s =>
              s.kind === 'hero' ? (
                <HeroBanner key={s.id} hero={s.hero} />
              ) : s.kind === 'products' ? (
                <VStack key={s.id} gap={3}>
                  <Heading level={2}>{s.title}</Heading>
                  <ProductGrid products={s.products} />
                </VStack>
              ) : s.kind === 'banners' ? (
                <VStack key={s.id} gap={3}>
                  {s.title ? <Heading level={2}>{s.title}</Heading> : null}
                  <Grid columns={{minWidth: 320}} gap={4}>
                    {s.heroes.map(h => (
                      <HeroBanner key={h.bookingId} hero={h} />
                    ))}
                  </Grid>
                </VStack>
              ) : (
                <VStack key={s.id} gap={2}>
                  <Text type="label" color="secondary">
                    {s.title || 'Brands'}
                  </Text>
                  <HStack gap={2} wrap="wrap">
                    {s.brands.map(b => (
                      <Token key={b.id} label={b.name} href={`/search?q=${encodeURIComponent(b.name)}`} />
                    ))}
                  </HStack>
                </VStack>
              ),
            )}
            {openCollections.length > 0 ? (
              <VStack gap={3}>
                <HStack gap={2} vAlign="center" justify="between" wrap="wrap">
                  <Heading level={2}>Collections</Heading>
                  <Button label="All collections" variant="ghost" href="/collections" />
                </HStack>
                <CollectionCards collections={openCollections} />
              </VStack>
            ) : null}
            {topCategories.length > 0 ? (
              <VStack gap={3}>
                <Heading level={2}>Shop by category</Heading>
                <HStack gap={2} wrap="wrap">
                  {topCategories.map(c => (
                    <Button key={c.id} label={c.name} variant="secondary" href={`/c/${encodeURIComponent(c.id)}`} />
                  ))}
                </HStack>
              </VStack>
            ) : null}
            <VStack gap={3}>
              <HStack gap={2} vAlign="center" justify="between" wrap="wrap">
                <Heading level={2}>Just in</Heading>
                <Button label="Shop all" variant="ghost" href="/search?sort=newest" />
              </HStack>
              {featured.ok ? (
                featured.data.items.length > 0 ? (
                  <ProductGrid products={featured.data.items} />
                ) : (
                  <Text color="secondary">Nothing new to show yet.</Text>
                )
              ) : (
                <ErrorNotice error={featured.error} askAbout="What's new in the store?" />
              )}
            </VStack>
            {latestNotices.length > 0 ? (
              <VStack gap={3}>
                <HStack gap={2} vAlign="center" justify="between" wrap="wrap">
                  <Heading level={2}>From the store</Heading>
                  <Button label="Help and notices" variant="ghost" href="/help" />
                </HStack>
                <NoticeList notices={latestNotices} />
              </VStack>
            ) : null}
          </VStack>
        </LayoutContent>
      }
    />
  );
}
