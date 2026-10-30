'use client';

import {Center} from '@astryxdesign/core/Center';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {Button} from '@astryxdesign/core/Button';
import {HStack} from '@astryxdesign/core/Layout';

// A page that failed for a reason other than an API answer (those are shown in place).
export default function ConsoleError({reset}: {error: Error; reset: () => void}) {
  return (
    <Center axis="both" padding={10} minHeight="60dvh">
      <EmptyState
        title="This page could not be shown"
        description="Something went wrong while loading it. Try again, or go back to the inbox."
        actions={
          <HStack gap={2}>
            <Button label="Try again" variant="primary" onClick={reset} />
            <Button label="Inbox" href="/bo" />
          </HStack>
        }
      />
    </Center>
  );
}
