import {NewProduct} from '@/components/bo/NewProduct';
import {PageFrame, PageSection} from '@/components/bo/PageFrame';

import {readOptional} from '../../../_lib/server';
import type {Brand, Category, Page} from '../../../_lib/types';

// Read per request with the person's session, never prerendered.
export const dynamic = 'force-dynamic';

export const metadata = {title: 'New product'};

// A new product (create_product_group), then its variants and prices on the product page
// (set_variants). Standard categories and brands come from list_categories and list_brands.
export default async function NewProductPage() {
  const [categories, brands] = await Promise.all([
    readOptional<Page<Category>>('/v1/merchandising/categories', {term_set: 'standard', limit: 100}),
    readOptional<Page<Brand>>('/v1/catalog/brands', {limit: 100}),
  ]);
  return (
    <PageFrame back={{href: '/bo/products', label: 'Products'}} title="New product" width={960}>
      <PageSection title="Product" description="Name it, file it under a standard category, and say which options it varies by.">
        <NewProduct categories={categories?.items ?? []} brands={brands?.items ?? []} />
      </PageSection>
    </PageFrame>
  );
}
