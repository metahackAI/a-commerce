import type {Metadata} from 'next';

import {StorefrontWorkspace} from '@/components/fo/StorefrontWorkspace';

import {greetingName, shopper} from '../_lib/server';

export const metadata: Metadata = {title: 'Ask Ailism'};

// The same conversation as the main page, for links that name it.
export default async function AskPage() {
  return <StorefrontWorkspace name={await greetingName(await shopper())} />;
}
