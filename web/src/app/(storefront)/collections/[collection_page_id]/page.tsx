import type {Metadata} from 'next';
import {BreadcrumbItem, Breadcrumbs} from '@astryxdesign/core/Breadcrumbs';
import {HStack, Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';
import {Token} from '@astryxdesign/core/Token';

import {ErrorNotice} from '@/components/fo/ErrorNotice';
import {ProductGrid} from '@/components/fo/ProductGrid';

import {longDate} from '../../_lib/format';
import {load} from '../../_lib/server';
import type {CollectionPage, Product, SlotItem} from '../../_lib/types';

type Params = Promise<{collection_page_id: string}>;

function itemId(item: SlotItem): string | null {
  if (typeof item === 'string') return item;
  return item.item_id ?? item.product_id ?? item.id ?? null;
}

export async function generateMetadata({params}: {params: Params}): Promise<Metadata> {
  const id = decodeURIComponent((await params).collection_page_id);
  const page = await load<CollectionPage>(`/v1/merchandising/collection-pages/${encodeURIComponent(id)}`, {fields: ['id', 'name']});
  return {title: page.ok ? page.data.name : 'Collection'};
}

// A collection page: its sections in the operator's order, each with its products at this
// shopper's own prices (get_product). Products the store's rules hide (`hidden_items`) are left
// out, as get_curation reports them.
export default async function CollectionPageView({params}: {params: Params}) {
  const id = decodeURIComponent((await params).collection_page_id);
  const page = await load<CollectionPage>(`/v1/merchandising/collection-pages/${encodeURIComponent(id)}`);
  if (!page.ok) {
    return (
      <Layout
        height="auto"
        contentWidth={1200}
        content={
          <LayoutContent padding={6}>
            <ErrorNotice error={page.error} title="This collection is not available." askAbout="What collections are on now?" />
          </LayoutContent>
        }
      />
    );
  }
  const c = page.data;
  const hidden = new Set((c.hidden_items ?? []).map(h => h.product_id));
  const sections = await Promise.all(
    c.has_part.map(async part => {
      const ids = part.item_list_element.map(itemId).filter((p): p is string => !!p && p.startsWith('prd_') && !hidden.has(p));
      const loaded = await Promise.all(ids.map(p => load<Product>(`/v1/catalog/products/${encodeURIComponent(p)}`)));
      return {name: part.name, products: loaded.flatMap(l => (l.ok ? [l.data] : []))};
    }),
  );
  return (
    <Layout
      height="auto"
      contentWidth={1200}
      content={
        <LayoutContent padding={6}>
          <VStack gap={6}>
            <Breadcrumbs variant="supporting">
              <BreadcrumbItem href="/">Home</BreadcrumbItem>
              <BreadcrumbItem href="/collections">Collections</BreadcrumbItem>
              <BreadcrumbItem isCurrent>{c.name}</BreadcrumbItem>
            </Breadcrumbs>
            <VStack gap={2}>
              <Heading level={1}>{c.name}</Heading>
              {c.valid_through ? <Text color="secondary">{`On until ${longDate(c.valid_through)}`}</Text> : null}
              {c.keywords && c.keywords.length > 0 ? (
                <HStack gap={1} wrap="wrap">
                  {c.keywords.map(k => (
                    <Token key={k} label={k} href={`/search?q=${encodeURIComponent(k)}`} />
                  ))}
                </HStack>
              ) : null}
            </VStack>
            {sections.map(s =>
              s.products.length > 0 ? (
                <VStack key={s.name} gap={3}>
                  <Heading level={2}>{s.name}</Heading>
                  <ProductGrid products={s.products} />
                </VStack>
              ) : null,
            )}
          </VStack>
        </LayoutContent>
      }
    />
  );
}
