import type {Metadata} from 'next';
import {BreadcrumbItem, Breadcrumbs} from '@astryxdesign/core/Breadcrumbs';
import {Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';

import {ErrorNotice} from '@/components/fo/ErrorNotice';
import {ProductSections} from '@/components/fo/ProductSections';
import {ProductShowcase} from '@/components/fo/ProductShowcase';
import {StorePopup} from '@/components/fo/StorePopup';
import type {Page} from '@/lib/api';

import {jsonLdScript, productJsonLd} from '../../_lib/jsonld';
import {load, shopper} from '../../_lib/server';
import type {
  Brand,
  CategoryDetail,
  DeliveryDate,
  Measurements,
  Popup,
  Product,
  ProductGroup,
  ProductQuestion,
  Profile,
  ReturnPolicy,
  Review,
  ReviewSummary,
} from '../../_lib/types';

type Params = Promise<{product_id: string}>;

export async function generateMetadata({params}: {params: Params}): Promise<Metadata> {
  const id = decodeURIComponent((await params).product_id);
  const product = await load<Product>(`/v1/catalog/products/${encodeURIComponent(id)}`, {fields: ['id', 'name']});
  return {title: product.ok ? product.data.name : 'Product'};
}

export default async function ProductPage({params, searchParams}: {params: Params; searchParams: Promise<{color?: string | string[]}>}) {
  const id = decodeURIComponent((await params).product_id);
  // `?color=Black` (links from the shopper's orders, subscriptions and alerts) opens on that color.
  const asked = (await searchParams).color;
  const initialColor = (Array.isArray(asked) ? asked[0] : asked) ?? null;
  const path = `/p/${encodeURIComponent(id)}`;
  const enc = encodeURIComponent(id);
  const who = await shopper();
  const [product, group, measurements, summary, reviews, policy, profile, questions, popups] = await Promise.all([
    load<Product>(`/v1/catalog/products/${enc}`),
    load<ProductGroup>(`/v1/catalog/product-groups/${enc}`, {fields: ['id', 'name', 'description', 'image', 'varies_by']}),
    load<Measurements>(`/v1/catalog/products/${enc}/measurements`),
    load<ReviewSummary>(`/v1/catalog/products/${enc}/reviews/summary`, {sample_limit: 3}),
    load<Page<Review>>('/v1/catalog/reviews', {product_id: id, sort: 'newest', limit: 5}),
    load<ReturnPolicy>(`/v1/catalog/products/${enc}/return-policy`),
    who.isShopper ? load<Profile>('/v1/customer/people/me', {fields: ['addresses']}) : null,
    load<Page<ProductQuestion>>('/v1/catalog/questions', {product_id: id, limit: 10}),
    load<Page<Popup>>('/v1/merchandising/popups', {page: 'product'}),
  ]);

  if (!product.ok) {
    return (
      <Layout
        height="auto"
        contentWidth={1200}
        content={
          <LayoutContent padding={6}>
            <ErrorNotice error={product.error} title="This product is not available." askAbout="Show me something similar." />
          </LayoutContent>
        }
      />
    );
  }

  const p = product.data;
  const g = group.ok ? group.data : null;
  // Shoppers cannot read get_product_group; get_product carries the same description.
  const description = g?.description ?? p.description;
  const addresses = profile?.ok ? (profile.data.addresses ?? []) : [];
  const address = addresses.find(a => a.is_primary) ?? addresses[0] ?? null;
  const [delivery, category, brand] = await Promise.all([
    address
      ? load<Page<DeliveryDate>>('/v1/fulfillment/delivery-dates', {product_id: id, address_mode: 'saved', address_id: address.id})
      : null,
    p.category_id ? load<CategoryDetail>(`/v1/merchandising/categories/${encodeURIComponent(p.category_id)}`, {fields: ['id', 'name']}) : null,
    p.brand_id ? load<Brand>(`/v1/catalog/brands/${encodeURIComponent(p.brand_id)}`) : null,
  ]);

  const ld = productJsonLd(p, g, summary.ok ? summary.data : null, path, brand?.ok ? brand.data.name : null);

  return (
    <Layout
      height="auto"
      contentWidth={1200}
      content={
        <LayoutContent padding={6}>
          <script type="application/ld+json" dangerouslySetInnerHTML={{__html: jsonLdScript(ld)}} />
          <VStack gap={6}>
            <StorePopup popups={popups.ok ? popups.data.items : []} />
            <Breadcrumbs variant="supporting">
              <BreadcrumbItem href="/">Home</BreadcrumbItem>
              {category?.ok ? <BreadcrumbItem href={`/c/${encodeURIComponent(category.data.id)}`}>{category.data.name}</BreadcrumbItem> : null}
              <BreadcrumbItem isCurrent>{p.name}</BreadcrumbItem>
            </Breadcrumbs>
            <ProductShowcase
              product={p}
              image={g?.image ?? p.image}
              description={description}
              addressId={address?.id ?? null}
              addressLabel={address ? (address.label ?? address.street_address) : null}
              initialDelivery={delivery?.ok ? (delivery.data.items[0] ?? null) : null}
              initialColor={initialColor}
            />
            <ProductSections product={p} reads={{measurements, summary, reviews, policy, questions}} />
          </VStack>
        </LayoutContent>
      }
    />
  );
}
