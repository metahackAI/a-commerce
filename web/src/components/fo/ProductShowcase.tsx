'use client';

// The product's picture beside its purchase panel. The picture follows the color the shopper
// picks: each variant in get_product's has_variant carries its color's picture, and a color
// without one shows the product's own picture.

import {Grid} from '@astryxdesign/core/Grid';
import {VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';
import {useState} from 'react';

import {imageUrl} from '@/app/(storefront)/_lib/format';
import type {DeliveryDate, ImageRef, Product} from '@/app/(storefront)/_lib/types';

import {ProductImage} from './ProductImage';
import {ProductPurchase} from './ProductPurchase';

export function ProductShowcase({
  product,
  image,
  description,
  addressId,
  addressLabel,
  initialDelivery,
}: {
  product: Product;
  /** The product's own picture, shown for a color that has none. */
  image: ImageRef;
  description: string | null | undefined;
  addressId: string | null;
  addressLabel: string | null;
  initialDelivery: DeliveryDate | null;
}) {
  const variants = product.has_variant ?? [];
  const first = variants.find(v => v.availability === 'in_stock') ?? variants[0];
  const [color, setColor] = useState<string | null>(first?.color ?? null);
  const shown = variants.find(v => v.color === color && imageUrl(v.image ?? null))?.image;
  return (
    <Grid columns={{minWidth: 320, repeat: 'fit'}} gap={6} align="start">
      <ProductImage src={imageUrl(shown ?? image)} alt={color ? `${product.name}, ${color}` : product.name} />
      <VStack gap={4}>
        <Heading level={1}>{product.name}</Heading>
        {description ? <Text color="secondary">{description}</Text> : null}
        <ProductPurchase
          product={product}
          addressId={addressId}
          addressLabel={addressLabel}
          initialDelivery={initialDelivery}
          color={color}
          onColorChange={setColor}
        />
      </VStack>
    </Grid>
  );
}
