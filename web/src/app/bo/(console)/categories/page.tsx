import {CategoryManager} from '@/components/bo/CategoryManager';
import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';

import {read} from '../../_lib/server';
import type {Category, Page} from '../../_lib/types';

// Read per request with the person's session, never prerendered.
export const dynamic = 'force-dynamic';

export const metadata = {title: 'Categories'};

// Category management (list_categories, both trees): create, rename, move, switch off, and map
// standard categories into the display menu.
export default async function CategoriesPage() {
  const list = await read<Page<Category>>('/v1/merchandising/categories', {limit: 100});
  return (
    <PageFrame
      title="Categories"
      description="Products are filed in standard categories; shoppers browse display categories. A mapping shows a standard category’s products in the menu. A category under an inactive one reads as inactive."
      width={1100}>
      {list.ok ? <CategoryManager categories={list.data.items} /> : <ErrorState error={list.error} title="The categories could not be loaded." />}
    </PageFrame>
  );
}
