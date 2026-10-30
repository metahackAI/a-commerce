'use client';

// The operator's home: the inbox of tasks (read again when a task changes, for example when a
// watcher refreshes one) with a composer to give the agent a task, and the full
// workspace (conversation + canvas of blocks) one switch away. Sending from the inbox, or pressing
// Ask on any task, switches to the conversation.

import {useEffect, useState} from 'react';
import {useRouter} from 'next/navigation';
import {HStack, Layout, LayoutContent, LayoutHeader, StackItem, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';
import {SegmentedControl, SegmentedControlItem} from '@astryxdesign/core/SegmentedControl';
import {Button} from '@astryxdesign/core/Button';

import {useAi} from '@/ai/AiProvider';
import type {ApiErrorBody} from '@/lib/api';
import {Workspace} from '@/components/workspace/Workspace';
import type {ChangeSet, TaskSummary} from '@/app/bo/_lib/types';

import {AgentComposer, useAgentDock} from './AgentDock';
import {ErrorState} from './ErrorState';
import {TaskGroups} from './TaskTable';
import {useStoreEvents} from './useStoreEvents';

export function HomeView({
  tasks,
  total,
  changeSets,
  error,
  initialView,
}: {
  tasks: TaskSummary[];
  total: number;
  changeSets: Record<string, ChangeSet>;
  error: ApiErrorBody | null;
  initialView: 'inbox' | 'agent';
}) {
  const ai = useAi();
  const router = useRouter();
  const {takeOver} = useAgentDock();
  const [view, setView] = useState<'inbox' | 'agent'>(initialView);

  useEffect(() => takeOver(() => setView('agent')), [takeOver]);
  // Watchers and change sets move tasks while the inbox is open: read the list again.
  useStoreEvents(['task.updated'], () => router.refresh());

  const needsYou = tasks.filter(t => t.state === 'needs_you').length;

  return (
    <Layout
      height="fill"
      header={
        <LayoutHeader hasDivider padding={4}>
          <HStack gap={3} vAlign="center" wrap="wrap">
            <StackItem size="fill">
              <HStack gap={2} vAlign="center">
                <Heading level={1}>Inbox</Heading>
                <Text type="supporting" color="secondary">
                  {needsYou > 0 ? `${needsYou} need you` : 'Nothing needs you'}
                </Text>
              </HStack>
            </StackItem>
            <SegmentedControl label="Show" value={view} onChange={v => setView(v === 'agent' ? 'agent' : 'inbox')} size="sm">
              <SegmentedControlItem value="inbox" label="Tasks" />
              <SegmentedControlItem value="agent" label={ai.messages.length > 0 ? 'Conversation · active' : 'Conversation'} />
            </SegmentedControl>
          </HStack>
        </LayoutHeader>
      }
      content={
        view === 'agent' ? (
          <LayoutContent padding={0} isScrollable={false}>
            <Workspace
              greeting="What should the store do next?"
              composerPlaceholder="Give the agent a task…"
              canvasTitle="Canvas"
              canvasEmpty={{
                title: 'Nothing to review yet',
                description: 'Change sets, return reviews, and explanations appear here. Nothing changes in the store until you approve.',
              }}
            />
          </LayoutContent>
        ) : (
          <LayoutContent role="main">
            <VStack gap={6} paddingBlock={2}>
              <AgentComposer
                placeholder="Give the agent a task. For example: “Run a 20% weekend sale on summer outerwear for VIP members.”"
                onSent={() => setView('agent')}
              />
              {error ? <ErrorState error={error} title="The tasks could not be loaded." /> : null}
              {!error ? <TaskGroups tasks={tasks} changeSets={changeSets} /> : null}
              {total > tasks.length ? (
                <HStack gap={2} vAlign="center">
                  <Text type="supporting" color="secondary">
                    {`Showing ${tasks.length} of ${total} tasks.`}
                  </Text>
                  <Button label="All change sets" size="sm" variant="ghost" href="/bo/change-sets" />
                </HStack>
              ) : null}
            </VStack>
          </LayoutContent>
        )
      }
    />
  );
}
