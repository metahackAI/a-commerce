'use client';

// The conversation + canvas workspace both surfaces use (design/ mockups: chat on one side, the
// result as UI blocks on the other). Built from the Astryx `ai-chat` page template (`npm run
// astryx -- template ai-chat`): a ChatLayout transcript beside a resizable panel. The canvas shows
// the UI blocks the agent sends (docs/api/chat.md §3); until block components exist (M4) it shows
// each block's title and why-line.

import type {CSSProperties} from 'react';
import {HStack, VStack, Layout, LayoutContent} from '@astryxdesign/core/Layout';
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
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {Text} from '@astryxdesign/core/Text';
import {Token} from '@astryxdesign/core/Token';
import {Toolbar} from '@astryxdesign/core/Toolbar';
import {ResizeHandle, useResizable} from '@astryxdesign/core/Resizable';

import {AILISM, AilismAvatar} from '@/components/brand/Ailism';
import {useAi} from '@/ai/AiProvider';
import {useAiTarget} from '@/ai/useAiTarget';
import type {UiBlock} from '@/ai/types';
import {blockComponent} from '@/components/blocks/registry';

// Below this width the canvas collapses under the chat (same breakpoint as the template).
const MOBILE_MAX_WIDTH = 767;

const root: CSSProperties = {height: '100%', width: '100%', containerType: 'inline-size', containerName: 'workspace'};
const chatColumn: CSSProperties = {flex: 1, width: '100%', minWidth: 0, height: '100%'};
const chatLayout: CSSProperties = {flex: 1, minHeight: 0};
const canvasScroll: CSSProperties = {flex: 1, overflowY: 'auto'};
const canvasWidth = (size: number | string): CSSProperties =>
  ({'--workspace-canvas-width': typeof size === 'number' ? `${size}px` : size}) as CSSProperties;

// The container query lives in a plain <style> tag, as in the template, so no CSS compiler is
// needed.
const WORKSPACE_CSS = `
.acom-canvas-handle { display: flex; }
.acom-canvas { overflow: hidden; display: flex; flex-direction: column; width: var(--workspace-canvas-width); flex-shrink: 0; }
[data-acom-highlight="true"] { outline: 2px solid var(--color-accent); outline-offset: 2px; }
@container workspace (max-width: ${MOBILE_MAX_WIDTH}px) {
  .acom-canvas-handle { display: none; }
  .acom-canvas { width: 100%; flex-shrink: 1; }
}
`;

/** The block's own component, or a card with its title and why-line. */
function Block({block}: {block: UiBlock}) {
  const Component = blockComponent(block.ui);
  return Component ? <Component block={block} /> : <BlockCard block={block} />;
}

function BlockCard({block}: {block: UiBlock}) {
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

export interface WorkspaceProps {
  /** Shown in the empty transcript. */
  greeting: string;
  composerPlaceholder: string;
  canvasTitle: string;
  canvasEmpty: {title: string; description: string};
}

export function Workspace({greeting, composerPlaceholder, canvasTitle, canvasEmpty}: WorkspaceProps) {
  const ai = useAi();
  const canvas = useResizable({defaultSize: 560, minSize: 400, maxSize: 900, autoSaveId: `acom-canvas-${ai.surface}`});

  const composer = (
    <ChatComposer
      onSubmit={text => void ai.send(text)}
      placeholder={composerPlaceholder}
      input={<ChatComposerInput />}
      status={ai.isBusy ? {type: 'warning', message: 'Working…'} : undefined}
      headerActions={
        ai.picked.length > 0 ? (
          <HStack gap={1}>
            {ai.picked.map(p => (
              <Token key={p.element_ref} label={p.label} onRemove={() => ai.unpick(p.element_ref)} />
            ))}
          </HStack>
        ) : undefined
      }
    />
  );

  return (
    <VStack style={root}>
      <style>{WORKSPACE_CSS}</style>
      <Layout
        height="fill"
        content={
          <LayoutContent padding={0}>
            <HStack height="100%">
              <VStack style={chatColumn}>
                <ChatLayout density="spacious" style={chatLayout} composer={composer}>
                  <ChatMessageList emptyState={<EmptyState title={greeting} />}>
                    {ai.messages.map(m =>
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
                    )}
                  </ChatMessageList>
                </ChatLayout>
              </VStack>
              <ResizeHandle
                direction="horizontal"
                resizable={canvas.props}
                isReversed
                pillPlacement="start"
                hasDivider
                label="Resize canvas"
                className="acom-canvas-handle"
              />
              <Card variant="transparent" height="100%" className="acom-canvas" style={canvasWidth(canvas.size)}>
                <Toolbar
                  label="Canvas"
                  dividers={['bottom']}
                  startContent={
                    <Text type="label" weight="semibold">
                      {canvasTitle}
                    </Text>
                  }
                />
                <VStack gap={3} padding={4} style={canvasScroll}>
                  {ai.blocks.length === 0 ? (
                    <EmptyState title={canvasEmpty.title} description={canvasEmpty.description} />
                  ) : (
                    ai.blocks.map(b => <Block key={b.block_id} block={b} />)
                  )}
                </VStack>
              </Card>
            </HStack>
          </LayoutContent>
        }
      />
    </VStack>
  );
}
