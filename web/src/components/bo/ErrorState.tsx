'use client';

// An API error shown as the API wrote it (docs/api/errors.md): its sentence, its code, and the
// next step it suggests, as a link to the page that answers that next tool. The agent can be
// asked about the error too.

import {HStack} from '@astryxdesign/core/Layout';
import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';

import {useAi} from '@/ai/AiProvider';
import type {ApiErrorBody} from '@/lib/api';
import {nextToolLink} from '@/app/bo/_lib/links';

import {useAgentDock} from './AgentDock';

export function ErrorState({error, title, onRetry}: {error: ApiErrorBody; title?: string; onRetry?: () => void}) {
  const ai = useAi();
  const dock = useAgentDock();
  const next = nextToolLink(error.next_tool);
  const signedOut = error.class === 'unauthenticated' || error.code === 'unauthenticated';
  return (
    <Banner
      status={error.class === 'not_found' ? 'warning' : 'error'}
      title={title ?? error.message}
      description={title ? error.message : `Code: ${error.code}`}
      endContent={
        <HStack gap={2} wrap="wrap">
          {signedOut ? <Button label="Sign in again" size="sm" href="/bo/login" /> : null}
          {next ? <Button label={next.label} size="sm" href={next.href} /> : null}
          {onRetry ? <Button label="Try again" size="sm" onClick={onRetry} /> : null}
          <Button
            label="Ask the agent"
            size="sm"
            variant="ghost"
            onClick={() => {
              dock.open();
              void ai.send(`I got this error: "${error.message}" (${error.code}). What should I do?`);
            }}
          />
        </HStack>
      }
    />
  );
}
