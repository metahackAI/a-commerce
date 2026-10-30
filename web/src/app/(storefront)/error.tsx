'use client';

import {Button} from '@astryxdesign/core/Button';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {HStack, Layout, LayoutContent} from '@astryxdesign/core/Layout';

// A page failed in a way the page itself did not handle. Offer to try again or go home.
export default function StorefrontError({reset}: {error: Error & {digest?: string}; reset: () => void}) {
  return (
    <Layout
      height="auto"
      contentWidth={800}
      content={
        <LayoutContent padding={6}>
          <EmptyState
            title="This page could not be shown"
            description="Something went wrong while reading the store. Try again, or start from the main page."
            actions={
              <HStack gap={2}>
                <Button label="Try again" variant="primary" onClick={reset} />
                <Button label="Main page" href="/" />
              </HStack>
            }
          />
        </LayoutContent>
      }
    />
  );
}
