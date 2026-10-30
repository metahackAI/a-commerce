import {Button} from '@astryxdesign/core/Button';
import {Grid} from '@astryxdesign/core/Grid';
import {HStack, Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';
import {Token} from '@astryxdesign/core/Token';

import {AskEntry} from '@/components/fo/AskEntry';
import {ErrorNotice} from '@/components/fo/ErrorNotice';
import {HeroBanner, type HeroContent} from '@/components/fo/HeroBanner';
import {ProductGrid} from '@/components/fo/ProductGrid';
import type {Page} from '@/lib/api';

import {imageUrl} from './_lib/format';
import {load, orElse, shopper} from './_lib/server';
import type {
  Banner,
  Brand,
  Category,
  DisplayBooking,
  DisplaySlot,
  DisplaySlotSummary,
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

function isLive(b: DisplayBooking): boolean {
  return b.status === undefined || b.status === 'active';
}

/** The booking this shopper sees: one for their tier if there is one, else the one for everyone. */
function bookingFor(slot: DisplaySlot, tierId: string | null): DisplayBooking | null {
  const live = slot.bookings.filter(isLive);
  return (tierId ? live.find(b => b.audience === tierId) : undefined) ?? live.find(b => b.audience === 'everyone') ?? null;
}

function bannerFor(booking: DisplayBooking, banners: Banner[]): Banner | undefined {
  return banners.find(
    b => (booking.banner_id && b.id === booking.banner_id) || b.bookings?.some(x => x.display_booking_id === booking.display_booking_id),
  );
}

async function heroContent(slot: DisplaySlot, booking: DisplayBooking, banners: Banner[]): Promise<HeroContent | null> {
  const banner = bannerFor(booking, banners);
  const c = booking.content ?? {};
  const headline = c.headline ?? booking.headline ?? banner?.headline;
  if (!headline) return null;
  let image = c.content_url ?? banner?.content_url ?? null;
  const productId = c.image_product_id ?? banner?.image_product_id ?? banner?.image_product_group_id ?? null;
  if (!image && productId) {
    const group = await load<ProductGroup>(`/v1/catalog/product-groups/${encodeURIComponent(productId)}`, {fields: ['id', 'name', 'image']});
    image = group.ok ? imageUrl(group.data.image) : null;
  }
  return {
    bookingId: booking.display_booking_id,
    slotId: slot.id,
    eyebrow: c.eyebrow ?? booking.eyebrow ?? banner?.eyebrow,
    headline,
    subline: c.alternative_headline ?? booking.alternative_headline ?? banner?.alternative_headline,
    ctaLabel: c.cta_label ?? booking.cta_label ?? banner?.cta_label,
    url: c.url ?? booking.url ?? banner?.url,
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

function isoDay(d: Date): string {
  return d.toISOString().slice(0, 10);
}

type Section =
  | {kind: 'hero'; id: string; hero: HeroContent}
  | {kind: 'products'; id: string; title: string; products: ProductSummary[]}
  | {kind: 'banners'; id: string; title: string; heroes: HeroContent[]}
  | {kind: 'brands'; id: string; title: string; brands: Brand[]};

export default async function StorefrontHome() {
  const who = await shopper();
  const now = new Date();
  const [slotList, bannerList, categories, featured, profile, brandList] = await Promise.all([
    load<Page<DisplaySlotSummary>>('/v1/merchandising/display-slots', {
      page: 'main',
      valid_from: isoDay(now),
      valid_through: isoDay(new Date(now.getTime() + 86_400_000)),
      limit: 20,
    }),
    load<Page<Banner>>('/v1/merchandising/banners', {page: 'main', channel: 'web', limit: 50}),
    load<Page<Category>>('/v1/merchandising/categories', {term_set: 'standard', limit: 50}),
    load<Page<ProductSummary>>('/v1/search/products', {sort: 'newest', limit: GRID_LIMIT}),
    who.isShopper ? load<Profile>('/v1/customer/people/me', {fields: ['name', 'tier_id']}) : null,
    load<Page<Brand>>('/v1/catalog/brands', {limit: 50}),
  ]);
  const tierId = profile?.ok ? profile.data.tier_id : null;
  const banners = orElse(bannerList, {items: [], total: 0}).items;
  const brands = orElse(brandList, {items: [], total: 0}).items;

  const summaries = orElse(slotList, {items: [], total: 0}).items.sort((a, b) => (a.position ?? 0) - (b.position ?? 0));
  const slots = await Promise.all(summaries.map(s => load<DisplaySlot>(`/v1/merchandising/display-slots/${encodeURIComponent(s.id)}`)));

  const sections: Section[] = [];
  for (const [i, loaded] of slots.entries()) {
    if (!loaded.ok || loaded.data.live_now === false) continue;
    const slot = loaded.data;
    const title = slot.name ?? summaries[i].name ?? '';
    if (slot.slot_type === 'hero') {
      const booking = bookingFor(slot, tierId);
      const hero = booking ? await heroContent(slot, booking, banners) : null;
      if (hero) sections.push({kind: 'hero', id: slot.id, hero});
    } else if (slot.slot_type === 'product_grid') {
      const products = await slotProducts(slot);
      if (products.length > 0) sections.push({kind: 'products', id: slot.id, title, products});
    } else if (slot.slot_type === 'banner_board') {
      const live = slot.bookings.filter(b => isLive(b) && (b.audience === 'everyone' || b.audience === tierId));
      const heroes = (await Promise.all(live.map(b => heroContent(slot, b, banners)))).filter((h): h is HeroContent => !!h);
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
            <AskEntry name={profile?.ok ? profile.data.name : null} />
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
          </VStack>
        </LayoutContent>
      }
    />
  );
}
