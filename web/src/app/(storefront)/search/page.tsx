import type {Metadata} from 'next';

import {ResultsView, parseSearchParams, type RawParams} from '../_lib/ResultsView';

export async function generateMetadata({searchParams}: {searchParams: Promise<RawParams>}): Promise<Metadata> {
  const {q} = parseSearchParams(await searchParams);
  return {title: q ? `“${q}”` : 'Shop all'};
}

export default async function SearchPage({searchParams}: {searchParams: Promise<RawParams>}) {
  const state = parseSearchParams(await searchParams);
  return (
    <ResultsView
      pathname="/search"
      state={state}
      title={state.q ? `Results for “${state.q}”` : 'Shop all'}
      subtitle={state.q ? undefined : 'Everything in the store, with your prices.'}
    />
  );
}
