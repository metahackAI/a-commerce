'use client';

// The personas Ailism Customer Bot shops as (list_shopper_personas): one card per member, with
// who they are, what they wear and spend, and how often they have shopped.

import {Card} from '@astryxdesign/core/Card';
import {Grid} from '@astryxdesign/core/Grid';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {Text} from '@astryxdesign/core/Text';

import {count, dateTime} from '@/app/bo/_lib/format';
import type {ShopperPersona} from '@/app/bo/_lib/types';

import {TargetBlock} from '../targets';
import {MemberLink, PersonaFacts} from './parts';

export function PersonaGrid({personas}: {personas: ShopperPersona[]}) {
  if (personas.length === 0) {
    return (
      <Text type="supporting" color="secondary">
        No persona yet. A member gets one the first time the bot shops as them.
      </Text>
    );
  }
  return (
    <Grid columns={{minWidth: 320}} gap={3}>
      {personas.map(p => (
        <TargetBlock
          key={p.id}
          target={{kind: 'shopper_persona', label: p.description, ids: {customer_id: p.customer_id}, term: 'acom:ShopperPersona'}}>
          <Card>
            <VStack gap={2}>
              <HStack gap={2} vAlign="center" justify="between" wrap="wrap">
                <MemberLink id={p.customer_id} />
                <Text type="supporting" color="secondary">
                  {`${count(p.sessions)} session${p.sessions === 1 ? '' : 's'}${p.last_session ? ` · last ${dateTime(p.last_session)}` : ''}`}
                </Text>
              </HStack>
              <Text weight="semibold">{p.description}</Text>
              <PersonaFacts p={p.persona} />
              <HStack gap={3}>
                <Link href={`/bo/customer-bot/sessions?customer_id=${encodeURIComponent(p.customer_id)}`}>Their sessions</Link>
                {p.built_by_session_id ? <Link href={`/bo/customer-bot/sessions/${encodeURIComponent(p.built_by_session_id)}`}>How it was made</Link> : null}
              </HStack>
            </VStack>
          </Card>
        </TargetBlock>
      ))}
    </Grid>
  );
}
