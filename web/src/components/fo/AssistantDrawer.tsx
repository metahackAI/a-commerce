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
import {HStack, Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Token} from '@astryxdesign/core/Token';
import {usePathname} from 'next/navigation';
import type {CSSProperties} from 'react';

import {useAi} from '@/ai/AiProvider';
import type {UiBlock} from '@/ai/types';
import {useAiTarget} from '@/ai/useAiTarget';
import {blockComponent} from '@/components/blocks/registry';

import {AILISM, AilismAvatar} from '@/components/brand/Ailism';
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

/** The chat fills the drawer under its header, as in the workspace (components/workspace). */
const fill: CSSProperties = {height: '100%', minHeight: 0};
const grow: CSSProperties = {flex: 1, minHeight: 0};

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
      label="Ask Ailism"
      width={480}
      experimental={{isFullWidthOnMobile: true}}>
      {/* Drawer → Layout with the DrawerHeader in its header slot keeps the header on the drawer's
          inset (Astryx DrawerHeader). */}
      <Layout
        height="fill"
        header={
          <DrawerHeader
            startContent={<AilismAvatar size="md" />}
            title="Ask Ailism"
            subtitle={`${AILISM.screen} Only you can place an order.`}
            onOpenChange={setAssistantOpen}
            endContent={<Button label="Open full view" size="sm" variant="ghost" href="/ask" onClick={() => setAssistantOpen(false)} />}
            hasDivider
          />
        }
        content={
          <LayoutContent padding={0} isScrollable={false}>
            <VStack style={fill}>
        <ChatLayout density="compact" style={grow} composer={composer}>
          <ChatMessageList
            density="compact"
            isStreaming={ai.isBusy}
            emptyState={
              <EmptyState
                title={AILISM.tagline}
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
            {/* One flat list: ChatMessageList shows its empty state only when its children are an
                empty array. */}
            {[
              ...ai.messages.map(m =>
                m.sender === 'system' ? (
                  <ChatSystemMessage key={m.id}>{m.text}</ChatSystemMessage>
                ) : (
                  <ChatMessage
                    key={m.id}
                    sender={m.sender}
                    name={m.sender === 'assistant' ? AILISM.assistant : undefined}
                    avatar={m.sender === 'assistant' ? <AilismAvatar size="sm" /> : undefined}>
                    <ChatMessageBubble>{m.text}</ChatMessageBubble>
                  </ChatMessage>
                ),
              ),
              ...(ai.blocks.length > 0
                ? [
                    <VStack key="blocks" gap={3}>
                      {ai.blocks.map(b => (
                        <Block key={b.block_id} block={b} />
                      ))}
                    </VStack>,
                  ]
                : []),
            ]}
          </ChatMessageList>
        </ChatLayout>
            </VStack>
          </LayoutContent>
        }
      />
    </Drawer>
  );
}
