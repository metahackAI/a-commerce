import type {Metadata} from 'next';
import {Layout, LayoutContent} from '@astryxdesign/core/Layout';

import {ErrorNotice} from '@/components/fo/ErrorNotice';

import {categoryIdFromSlug} from '../../_lib/format';
import {ResultsView, parseSearchParams, type RawParams} from '../../_lib/ResultsView';
import {load} from '../../_lib/server';
import type {CategoryDetail} from '../../_lib/types';

type Params = Promise<{category_id: string}>;

export async function generateMetadata({params}: {params: Params}): Promise<Metadata> {
  const id = categoryIdFromSlug((await params).category_id);
  const category = await load<CategoryDetail>(`/v1/merchandising/categories/${encodeURIComponent(id)}`, {fields: ['id', 'name']});
  return {title: category.ok ? category.data.name : 'Category'};
}

// Banner links use the term code (`/c/fall-layers`); the nav uses the id (`/c/cat_fall_layers`).
export default async function CategoryPage({params, searchParams}: {params: Params; searchParams: Promise<RawParams>}) {
  const id = categoryIdFromSlug((await params).category_id);
  const [category, raw] = await Promise.all([
    load<CategoryDetail>(`/v1/merchandising/categories/${encodeURIComponent(id)}`),
    searchParams,
  ]);
  if (!category.ok) {
    return (
      <Layout
        height="auto"
        contentWidth={1200}
        content={
          <LayoutContent padding={6}>
            <ErrorNotice error={category.error} title="This category is not available." />
          </LayoutContent>
        }
      />
    );
  }
  const state = parseSearchParams(raw);
  return (
    <ResultsView
      pathname={`/c/${encodeURIComponent(id)}`}
      state={{...state, category_id: undefined}}
      title={category.data.name}
      fixedCategoryId={id}
    />
  );
}
