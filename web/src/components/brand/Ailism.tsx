'use client';

// The Ailism brand in one place: the wordmark lockup (the Ailism mark, then the product name in a
// lighter weight, as the Ailism site's bar shows it), the assistant's face, the voice of the
// brand, and the footer. The look comes from the Ailism theme (src/theme/ailism.ts).

import {Avatar} from '@astryxdesign/core/Avatar';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';

/** The brand's voice: two short beats, then a full stop. English first, Korean beside it. */
export const AILISM = {
  name: 'Ailism Commerce',
  assistant: 'Ailism',
  tagline: 'Say it once. See it through.',
  taglineKo: '한 번 말하고, 끝까지.',
  promise: 'You decide. Ailism prepares the rest.',
  promiseKo: '결정은 당신이, 준비는 Ailism이.',
  screen: 'Say it your way. The screen comes to you.',
  footer: 'The store that moves when you speak.',
  footerKo: '말로 움직이는 가게.',
} as const;

const HEIGHT = {sm: 16, md: 22, lg: 32} as const;

/** "Ailism Commerce": the wordmark and the product name, muted, as one lockup. */
export function AilismMark({size = 'md', product = 'Commerce'}: {size?: keyof typeof HEIGHT; product?: string | null}) {
  const h = HEIGHT[size];
  return (
    <HStack gap={1.5} vAlign="center" wrap="nowrap">
      {/* The wordmark is the brand's own artwork, not an icon: an <img> keeps its two colours. */}
      <img src="/brand/ailism-wordmark.png" alt="Ailism" height={h} width={Math.round(h * 3.09)} style={{display: 'block'}} />
      {product ? (
        <Text type={size === 'lg' ? 'large' : 'label'} color="secondary" weight="normal">
          {product}
        </Text>
      ) : null}
    </HStack>
  );
}

/** The assistant's face. */
export function AilismAvatar({size = 'sm'}: {size?: 'xsm' | 'sm' | 'md' | 'lg' | 'xl'}) {
  return <Avatar src="/brand/ailism-avatar.png" name={AILISM.assistant} size={size} />;
}

/** The storefront footer: the lockup, what the brand promises, and the year. */
export function AilismFooter() {
  return (
    <footer>
      <VStack gap={2} paddingBlock={8} paddingInline={6} hAlign="center">
        <AilismMark size="sm" />
        <Text type="supporting" color="secondary" justify="center">
          {`${AILISM.footer} ${AILISM.footerKo}`}
        </Text>
        <Text type="supporting" color="secondary" justify="center">
          {`${AILISM.promise} · © 2026 Ailism`}
        </Text>
      </VStack>
    </footer>
  );
}
