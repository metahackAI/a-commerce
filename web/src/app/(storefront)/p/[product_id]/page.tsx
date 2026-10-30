import type {Metadata} from 'next';
import {BreadcrumbItem, Breadcrumbs} from '@astryxdesign/core/Breadcrumbs';
import {Collapsible, CollapsibleGroup} from '@astryxdesign/core/Collapsible';
import {Grid} from '@astryxdesign/core/Grid';
import {Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Heading, Text} from '@astryxdesign/core/Text';

import {ErrorNotice} from '@/components/fo/ErrorNotice';
import {MeasurementsTable, ProductFacts, ReviewList, ReviewSummaryView} from '@/components/fo/ProductDetails';
import {ProductImage} from '@/components/fo/ProductImage';
import {ProductPurchase} from '@/components/fo/ProductPurchase';
import type {Page} from '@/lib/api';

import {imageUrl, words} from '../../_lib/format';
import {jsonLdScript, productJsonLd} from '../../_lib/jsonld';
import {load, shopper} from '../../_lib/server';
import type {
  CategoryDetail,
  DeliveryDate,
  Measurements,
  Product,
  ProductGroup,
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

export default async function ProductPage({params}: {params: Params}) {
  const id = decodeURIComponent((await params).product_id);
  const path = `/p/${encodeURIComponent(id)}`;
  const enc = encodeURIComponent(id);
  const who = await shopper();
  const [product, group, measurements, summary, reviews, policy, profile] = await Promise.all([
    load<Product>(`/v1/catalog/products/${enc}`),
    load<ProductGroup>(`/v1/catalog/product-groups/${enc}`, {fields: ['id', 'name', 'description', 'image', 'varies_by']}),
    load<Measurements>(`/v1/catalog/products/${enc}/measurements`),
    load<ReviewSummary>(`/v1/catalog/products/${enc}/reviews/summary`, {sample_limit: 3}),
    load<Page<Review>>('/v1/catalog/reviews', {product_id: id, sort: 'newest', limit: 5}),
    load<ReturnPolicy>(`/v1/catalog/products/${enc}/return-policy`),
    who.isShopper ? load<Profile>('/v1/customer/people/me', {fields: ['addresses']}) : null,
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
  const addresses = profile?.ok ? (profile.data.addresses ?? []) : [];
  const address = addresses.find(a => a.is_primary) ?? addresses[0] ?? null;
  const [delivery, category] = await Promise.all([
    address
      ? load<Page<DeliveryDate>>('/v1/fulfillment/delivery-dates', {product_id: id, address_mode: 'saved', address_id: address.id})
      : null,
    p.category_id ? load<CategoryDetail>(`/v1/merchandising/categories/${encodeURIComponent(p.category_id)}`, {fields: ['id', 'name']}) : null,
  ]);

  const rp = policy.ok ? policy.data : null;
  const nonReturnable = rp ? (rp['acom:non_returnable_when'] ?? rp.non_returnable_when ?? []) : [];
  const sizeExchangeDays = rp ? (rp['acom:size_exchange_days'] ?? rp.size_exchange_days ?? null) : null;
  const ld = productJsonLd(p, g, summary.ok ? summary.data : null, path);

  return (
    <Layout
      height="auto"
      contentWidth={1200}
      content={
        <LayoutContent padding={6}>
          <script type="application/ld+json" dangerouslySetInnerHTML={{__html: jsonLdScript(ld)}} />
          <VStack gap={6}>
            <Breadcrumbs variant="supporting">
              <BreadcrumbItem href="/">Home</BreadcrumbItem>
              {category?.ok ? <BreadcrumbItem href={`/c/${encodeURIComponent(category.data.id)}`}>{category.data.name}</BreadcrumbItem> : null}
              <BreadcrumbItem isCurrent>{p.name}</BreadcrumbItem>
            </Breadcrumbs>
            <Grid columns={{minWidth: 320, repeat: 'fit'}} gap={6} align="start">
              <ProductImage src={imageUrl(g?.image ?? p.image)} alt={p.name} />
              <VStack gap={4}>
                <Heading level={1}>{p.name}</Heading>
                {g?.description ? <Text color="secondary">{g.description}</Text> : null}
                <ProductPurchase
                  product={p}
                  addressId={address?.id ?? null}
                  addressLabel={address ? (address.label ?? address.street_address) : null}
                  initialDelivery={delivery?.ok ? (delivery.data.items[0] ?? null) : null}
                />
              </VStack>
            </Grid>
            <CollapsibleGroup type="multiple" defaultValue={['details', 'fit', 'reviews', 'returns']} hasDividers>
              {p.additional_property && p.additional_property.length > 0 ? (
                <Collapsible value="details" trigger={<Heading level={2}>Details</Heading>}>
                  <ProductFacts productId={p.id} facts={p.additional_property} />
                </Collapsible>
              ) : null}
              <Collapsible value="fit" trigger={<Heading level={2}>Measurements and fit</Heading>}>
                {measurements.ok ? (
                  <MeasurementsTable productName={p.name} measurements={measurements.data} />
                ) : (
                  <Text color="secondary">{measurements.error.message}</Text>
                )}
              </Collapsible>
              <Collapsible value="reviews" trigger={<Heading level={2}>Reviews</Heading>}>
                <VStack gap={4}>
                  {summary.ok ? (
                    <ReviewSummaryView productName={p.name} summary={summary.data} />
                  ) : (
                    <Text color="secondary">{summary.error.code === 'no_reviews' ? 'No reviews yet.' : summary.error.message}</Text>
                  )}
                  {reviews.ok && reviews.data.items.length > 0 ? <ReviewList reviews={reviews.data.items} /> : null}
                </VStack>
              </Collapsible>
              <Collapsible value="returns" trigger={<Heading level={2}>Delivery and returns</Heading>}>
                {rp ? (
                  <MetadataList>
                    <MetadataListItem label="Returns">Within {rp.merchant_return_days} days of delivery</MetadataListItem>
                    <MetadataListItem label="Return shipping">{words(rp.return_fees)}</MetadataListItem>
                    {sizeExchangeDays ? <MetadataListItem label="Size exchange">Within {sizeExchangeDays} days</MetadataListItem> : null}
                    {nonReturnable.length > 0 ? (
                      <MetadataListItem label="Not returnable when">{nonReturnable.map(words).join(', ')}</MetadataListItem>
                    ) : null}
                  </MetadataList>
                ) : (
                  <Text color="secondary">{policy.ok ? '' : policy.error.message}</Text>
                )}
              </Collapsible>
            </CollapsibleGroup>
          </VStack>
        </LayoutContent>
      }
    />
  );
}
