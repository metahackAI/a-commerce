'use client';

// The main-page hero: the booking live for this shopper's audience on the hero slot
// (merchandising.md display_slot + banner). Copy and link come from the booking or its banner.

import {Button} from '@astryxdesign/core/Button';
import {Card} from '@astryxdesign/core/Card';
import {Grid} from '@astryxdesign/core/Grid';
import {VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';

import {useAiTarget} from '@/ai/useAiTarget';

import {AskButton} from './Ask';
import {ProductImage} from './ProductImage';

export interface HeroContent {
  bookingId: string;
  slotId: string;
  eyebrow?: string | null;
  headline: string;
  subline?: string | null;
  ctaLabel?: string | null;
  url?: string | null;
  image?: string | null;
}

export function HeroBanner({hero}: {hero: HeroContent}) {
  const target = useAiTarget({
    kind: 'banner',
    label: hero.headline,
    ids: {display_booking_id: hero.bookingId, slot_id: hero.slotId},
    term: 'schema:WebPageElement',
  });
  return (
    <Card padding={0} ref={target.ref} {...target.props}>
      <Grid columns={{minWidth: 300, repeat: 'fit'}} gap={0} align="center">
        <ProductImage src={hero.image} alt={hero.headline} ratio={4 / 3} />
        <VStack gap={3} padding={6} hAlign="start">
          {hero.eyebrow ? (
            <Text type="label" color="secondary" weight="semibold">
              {hero.eyebrow}
            </Text>
          ) : null}
          <Heading level={1} type="display-2">
            {hero.headline}
          </Heading>
          {hero.subline ? (
            <Text type="large" color="secondary">
              {hero.subline}
            </Text>
          ) : null}
          {hero.url ? <Button label={hero.ctaLabel ?? 'Shop now'} variant="primary" size="lg" href={hero.url} /> : null}
          <AskButton target={target} label={hero.headline} />
        </VStack>
      </Grid>
    </Card>
  );
}
