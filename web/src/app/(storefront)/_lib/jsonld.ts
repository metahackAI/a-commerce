// schema.org JSON-LD for product pages (docs/frontend/README.md §1.6), built only from the API
// response: get_product (prices, variants, availability), get_product_group (description, image)
// and get_review_summary (rating). External agents read the page the same way people see it.

import {imageUrl} from './format';
import type {Availability, Product, ProductGroup, ReviewSummary} from './types';

const AVAILABILITY: Record<Availability, string> = {
  in_stock: 'https://schema.org/InStock',
  out_of_stock: 'https://schema.org/OutOfStock',
  pre_order: 'https://schema.org/PreOrder',
  back_order: 'https://schema.org/BackOrder',
  discontinued: 'https://schema.org/Discontinued',
};

export function productJsonLd(product: Product, group: ProductGroup | null, reviews: ReviewSummary | null, path: string) {
  const image = imageUrl(group?.image ?? product.image);
  const offer = (sku: string | undefined, availability: Availability | undefined, extra: Record<string, unknown> = {}) => ({
    '@type': 'Offer',
    ...(sku ? {sku} : {}),
    price: product.price,
    priceCurrency: product.price_currency,
    ...(availability ? {availability: AVAILABILITY[availability]} : {}),
    ...(product.member_price
      ? {
          priceSpecification: [
            {'@type': 'UnitPriceSpecification', priceType: 'https://schema.org/ListPrice', price: product.price, priceCurrency: product.price_currency},
            {
              '@type': 'UnitPriceSpecification',
              priceType: 'https://schema.org/SalePrice',
              price: product.member_price.price,
              priceCurrency: product.member_price.price_currency ?? product.price_currency,
              name: product.member_price.name,
              ...(product.member_price.valid_through ? {validThrough: product.member_price.valid_through} : {}),
            },
          ],
        }
      : {}),
    url: path,
    potentialAction: {'@type': 'BuyAction', target: path},
    ...extra,
  });

  const variants = product.has_variant ?? [];
  return {
    '@context': 'https://schema.org',
    '@type': variants.length > 0 ? 'ProductGroup' : 'Product',
    productGroupID: product.id,
    name: product.name,
    ...(group?.description ? {description: group.description} : {}),
    ...(image ? {image} : {}),
    ...(product.varies_by ? {variesBy: product.varies_by.map(v => `https://schema.org/${v}`)} : {}),
    ...(product.additional_property
      ? {
          additionalProperty: product.additional_property.map(p => ({
            '@type': 'PropertyValue',
            name: p.name,
            value: p.value,
            ...(p.unit_code ? {unitCode: p.unit_code} : {}),
          })),
        }
      : {}),
    ...(reviews && reviews.review_count > 0 && reviews.rating_value !== null
      ? {aggregateRating: {'@type': 'AggregateRating', ratingValue: reviews.rating_value, reviewCount: reviews.review_count}}
      : {}),
    ...(variants.length > 0
      ? {
          hasVariant: variants.map(v => ({
            '@type': 'Product',
            sku: v.sku ?? v.variant_id,
            name: [product.name, v.color, v.size].filter(Boolean).join(', '),
            ...(v.color ? {color: v.color} : {}),
            ...(v.size ? {size: v.size} : {}),
            offers: offer(v.sku ?? v.variant_id, v.availability),
          })),
        }
      : {offers: offer(undefined, product.availability)}),
  };
}

/** JSON for a `<script type="application/ld+json">`, safe to put in HTML. */
export function jsonLdScript(data: unknown): string {
  return JSON.stringify(data).replace(/</g, '\\u003c');
}
