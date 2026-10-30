'use client';

// One UI block from the agent: its own component when one is registered, otherwise a card with
// its title and why-line (the same fallback the workspace uses).

import {VStack} from '@astryxdesign/core/Layout';
import {Card} from '@astryxdesign/core/Card';
import {Text} from '@astryxdesign/core/Text';

import {useAiTarget} from '@/ai/useAiTarget';
import type {UiBlock} from '@/ai/types';
import {blockComponent} from '@/components/blocks/registry';

export function BlockView({block}: {block: UiBlock}) {
  const Component = blockComponent(block.ui);
  return Component ? <Component block={block} /> : <FallbackBlock block={block} />;
}

function FallbackBlock({block}: {block: UiBlock}) {
  const target = useAiTarget({kind: 'block', label: block.title ?? block.ui, ids: {block_id: block.block_id}, blockId: block.block_id});
  return (
    <Card ref={target.ref} {...target.props}>
      <VStack gap={1}>
        <Text type="label" weight="semibold">
          {block.title ?? block.ui}
        </Text>
        {block.why ? (
          <Text type="supporting" color="secondary">
            {block.why}
          </Text>
        ) : null}
      </VStack>
    </Card>
  );
}
