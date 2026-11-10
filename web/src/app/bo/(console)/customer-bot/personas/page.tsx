import {Text} from '@astryxdesign/core/Text';

import {PersonaGrid} from '@/components/bo/customer-bot/PersonaGrid';
import {CustomerBotTabs} from '@/components/bo/customer-bot/parts';
import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';

import {read} from '../../../_lib/server';
import type {Page, ShopperPersona} from '../../../_lib/types';

export const metadata = {title: 'Shopper personas'};

// The personas the bot shops as (list_shopper_personas), the most recently used first.
export default async function PersonasPage() {
  const personas = await read<Page<ShopperPersona>>('/v1/platform/customer-bot/personas', {limit: 60});
  return (
    <PageFrame
      title="Personas"
      back={{href: '/bo/customer-bot', label: 'Ailism Customer Bot'}}
      description="Who each member is when the bot shops as them. A persona is made once, from the member's own orders, the first time the bot signs in as them, and is kept for every later visit."
      toolbar={<CustomerBotTabs value="personas" />}>
      {personas.ok ? (
        <>
          <Text type="supporting" color="secondary">{`${personas.data.total} personas`}</Text>
          <PersonaGrid personas={personas.data.items} />
        </>
      ) : (
        <ErrorState error={personas.error} title="The personas could not be read." />
      )}
    </PageFrame>
  );
}
