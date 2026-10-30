'use client';

// `task_list` block (docs/api/chat.md §3.3): the operator's home in the conversation. Tasks in
// their groups (needs you, working, done) with the agents working on the store. Opening an item
// goes to its task; dismiss and undo are person buttons.

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {MetadataList} from '@astryxdesign/core/MetadataList';

import type {TaskListBlockData} from '@/app/bo/_lib/blocks';
import {value} from '@/app/bo/_lib/format';
import type {TaskSummary} from '@/app/bo/_lib/types';
import {TASK_GROUPS, TaskTable} from '@/components/bo/TaskTable';
import {TargetFact} from '@/components/bo/targets';

import type {BlockProps} from '../types';
import {BlockFrame, intentsLookup} from './parts';

export function TaskListBlock({block}: BlockProps) {
  const d = block.data as TaskListBlockData;
  const intentsFor = intentsLookup(block);
  const groups = d.groups ?? [];

  return (
    <BlockFrame block={block} kind="task_list" id={block.block_id} label="Tasks">
      {groups.length === 0 ? (
        <Text color="secondary">No tasks.</Text>
      ) : (
        groups.map(g => {
          const tasks: TaskSummary[] = g.items.map(i => ({
            id: i.task_id,
            title: i.title,
            state: g.state,
            origin: i.origin,
            at: i.at,
            summary: i.summary ?? null,
            tag: i.tag ?? null,
            change_set_id: i.change_set_id ?? null,
            watcher: null,
          }));
          const title = TASK_GROUPS.find(x => x.state === g.state)?.title ?? g.state;
          return (
            <VStack key={g.state} gap={2}>
              <HStack gap={2} vAlign="center">
                <Text type="label" weight="semibold">
                  {title}
                </Text>
              </HStack>
              {tasks.length === 0 ? (
                <Text type="supporting" color="secondary">
                  Nothing here.
                </Text>
              ) : (
                <TaskTable tasks={tasks} changeSets={{}} blockId={block.block_id} intentsFor={intentsFor} />
              )}
            </VStack>
          );
        })
      )}
      {d.agents && d.agents.length > 0 ? (
        <MetadataList title="Agents working on this store" columns="multi">
          {d.agents.map(a => (
            <TargetFact key={a.name} label={a.name} target={{kind: 'agent', label: `${a.name}: ${value(a.stats)}`, ids: {agent: a.name}, blockId: block.block_id}}>
              <Text>{value(a.stats)}</Text>
            </TargetFact>
          ))}
        </MetadataList>
      ) : null}
    </BlockFrame>
  );
}
