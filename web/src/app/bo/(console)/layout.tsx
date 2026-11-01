import type {ReactNode} from 'react';
import {AppShell} from '@astryxdesign/core/AppShell';

import {AiProvider} from '@/ai/AiProvider';
import {AgentDockProvider} from '@/components/bo/AgentDock';
import {BoNav} from '@/components/bo/BoNav';
import {BoSessionProvider} from '@/components/bo/BoSession';

import {read, requireOperator} from '../_lib/server';
import type {Page, TaskSummary} from '../_lib/types';

// The operator's console: only signed-in operators get here (everyone else goes to /bo/login).
// Operators talk to the agent from any page; the canvas and pages show change sets, reviews and
// explanations, and nothing changes in the store until a person approves.
export default async function ConsoleLayout({children}: {children: ReactNode}) {
  const session = await requireOperator();
  const user = session.user;
  const role = session.scopes.includes('human:approve') ? 'Operator · can approve' : 'Operator';
  // The inbox's count in the navigation; read again whenever the page refreshes after a decision.
  const needsYou = await read<Page<TaskSummary>>('/v1/platform/tasks', {state: 'needs_you', limit: 1});

  return (
    <BoSessionProvider session={session}>
      <AiProvider surface="bo">
        <AgentDockProvider>
          <AppShell height="fill" variant="section" sideNav={<BoNav userName={user?.name ?? user?.id ?? 'Operator'} role={role} needsYou={needsYou.ok ? needsYou.data.total : null} />}>
            {children}
          </AppShell>
        </AgentDockProvider>
      </AiProvider>
    </BoSessionProvider>
  );
}
