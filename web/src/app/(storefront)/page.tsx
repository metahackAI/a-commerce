import {StorefrontWorkspace} from '@/components/fo/StorefrontWorkspace';

import {greetingName, shopper} from './_lib/server';

// The storefront opens as a conversation; the page is assembled beside it as the shopper talks.
// The usual store front (slots, collections, notices) is at /shop.
export default async function StorefrontHome() {
  return <StorefrontWorkspace name={await greetingName(await shopper())} />;
}
