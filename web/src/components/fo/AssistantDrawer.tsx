'use client';

// The assistant on every storefront page: the same conversation as /ask (one AiProvider in the
// layout), in a drawer. Elements picked with "Ask about this" ride along as chips; the agent's UI
// blocks show under the conversation, drawn by the same block components as the full workspace.

import {
  ChatComposer,
  ChatComposerInput,
  ChatLayout,
  ChatMessage,
  ChatMessageBubble,
  ChatMessageList,
  ChatSystemMessage,
} from '@astryxdesign/core/Chat';
import {Card} from '@astryxdesign/core/Card';
import {Button} from '@astryxdesign/core/Button';
import {Drawer, DrawerHeader} from '@astryxdesign/core/Drawer';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Token} from '@astryxdesign/core/Token';
import {usePathname} from 'next/navigation';

import {useAi} from '@/ai/AiProvider';
import type {UiBlock} from '@/ai/types';
import {useAiTarget} from '@/ai/useAiTarget';
import {blockComponent} from '@/components/blocks/registry';

import {useStorefront} from './StorefrontContext';

const STARTERS = ['I need a light jacket for spring trips.', 'What goes with what is in my cart?', 'Where is my order?'];

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

function Block({block}: {block: UiBlock}) {
  const Component = blockComponent(block.ui);
  return Component ? <Component block={block} /> : <FallbackBlock block={block} />;
}

export function AssistantDrawer() {
  const ai = useAi();
  const {isAssistantOpen, setAssistantOpen} = useStorefront();
  const pathname = usePathname();
  // /ask is the full workspace; the drawer would show the same conversation twice.
  const isOpen = isAssistantOpen && pathname !== '/ask';

  const composer = (
    <ChatComposer
      onSubmit={text => void ai.send(text)}
      placeholder="Ask about fit, fabric, delivery, returns…"
      input={<ChatComposerInput />}
      density="compact"
      status={ai.isBusy ? {type: 'warning', message: 'Working…'} : undefined}
      headerActions={
        ai.picked.length > 0 ? (
          <HStack gap={1} wrap="wrap">
            {ai.picked.map(p => (
              <Token key={p.element_ref} label={p.label} size="sm" onRemove={() => ai.unpick(p.element_ref)} />
            ))}
          </HStack>
        ) : undefined
      }
    />
  );

  return (
    <Drawer
      isOpen={isOpen}
      onOpenChange={setAssistantOpen}
      label="Ask Northwind"
      width={480}
      padding={0}
      experimental={{isFullWidthOnMobile: true}}>
      <VStack height="100%">
        <DrawerHeader
          title="Ask Northwind"
          subtitle="Northwind can find, compare and prepare. Only you can place an order."
          onOpenChange={setAssistantOpen}
          endContent={<Button label="Open full view" size="sm" variant="ghost" href="/ask" onClick={() => setAssistantOpen(false)} />}
          hasDivider
        />
        <ChatLayout density="compact" composer={composer}>
          <ChatMessageList
            density="compact"
            isStreaming={ai.isBusy}
            emptyState={
              <EmptyState
                title="What are you looking for?"
                description="Ask in your own words, or pick something on the page and ask about it."
                isCompact
                actions={
                  <VStack gap={1} hAlign="start">
                    {STARTERS.map(s => (
                      <Button key={s} label={s} size="sm" variant="ghost" onClick={() => void ai.send(s)} />
                    ))}
                  </VStack>
                }
              />
            }>
            {ai.messages.map(m =>
              m.sender === 'system' ? (
                <ChatSystemMessage key={m.id}>{m.text}</ChatSystemMessage>
              ) : (
                <ChatMessage key={m.id} sender={m.sender} name={m.sender === 'assistant' ? 'Northwind' : undefined}>
                  <ChatMessageBubble>{m.text}</ChatMessageBubble>
                </ChatMessage>
              ),
            )}
            {ai.blocks.length > 0 ? (
              <VStack gap={3}>
                {ai.blocks.map(b => (
                  <Block key={b.block_id} block={b} />
                ))}
              </VStack>
            ) : null}
          </ChatMessageList>
        </ChatLayout>
      </VStack>
    </Drawer>
  );
}
