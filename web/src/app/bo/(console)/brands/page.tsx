import {BrandManager} from '@/components/bo/BrandManager';
import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';

import {read} from '../../_lib/server';
import type {Brand, Page} from '../../_lib/types';

// Read per request with the person's session, never prerendered.
export const dynamic = 'force-dynamic';

export const metadata = {title: 'Brands'};

// Brands (list_brands), with adding and editing one.
export default async function BrandsPage() {
  const list = await read<Page<Brand>>('/v1/catalog/brands', {limit: 100});
  return (
    <PageFrame title="Brands" description="The brands products are sold under. Open one to see its products." width={1000}>
      {list.ok ? <BrandManager brands={list.data.items} /> : <ErrorState error={list.error} title="The brands could not be loaded." />}
    </PageFrame>
  );
}
