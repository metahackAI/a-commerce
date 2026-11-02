'use client';

// The parts every conversation with Ailism is built from, on both sides of the store: the panel
// header, the transcript with what the agent is doing right now, the composer, and starter
// sentences. The back-office dock, the storefront drawer and the canvas workspace all use them,
// so the conversation looks and behaves the same wherever it is open.

import {
  ChatComposer,
  ChatComposerInput,
  ChatLayout,
  ChatMessage,
  ChatMessageBubble,
  ChatMessageList,
  ChatSystemMessage,
} from '@astryxdesign/core/Chat';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Spinner} from '@astryxdesign/core/Spinner';
import {Text} from '@astryxdesign/core/Text';
import {Token} from '@astryxdesign/core/Token';
import type {CSSProperties, ReactNode} from 'react';

import {useAi} from '@/ai/AiProvider';
import type {UiBlock} from '@/ai/types';
import {AILISM, AilismAvatar} from '@/components/brand/Ailism';
import {BlockView} from '@/components/bo/BlockView';
import {useVoiceInput} from '@/components/VoiceInput';

// The look the parts share. Plain CSS in a <style> tag (as the Astryx ai-chat template does), on
// theme tokens only: borders instead of shadows, the accent only on what a person presses.
export const CHAT_CSS = `
.acom-chat-panel { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--color-background-card); }
.acom-chat-header { display: flex; align-items: center; gap: var(--spacing-3); padding: var(--spacing-3) var(--spacing-4); border-bottom: 1px solid var(--color-border); flex-shrink: 0; }
.acom-chat-header-text { min-width: 0; flex: 1; display: flex; flex-direction: column; }
.acom-chat-header-text > * { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.acom-chat-body { flex: 1; min-height: 0; display: flex; flex-direction: column; }
.acom-chat-composer { padding: var(--spacing-2) var(--spacing-4) var(--spacing-4); display: flex; flex-direction: column; gap: var(--spacing-2); }
.acom-chat-step { padding: var(--spacing-1) 0; }
.acom-chat-starters { display: flex; flex-wrap: wrap; gap: var(--spacing-2); }
.acom-chat-starters[data-align="center"] { justify-content: center; }
.acom-chat-starter { all: unset; box-sizing: border-box; cursor: pointer; padding: var(--spacing-1) var(--spacing-3); border: 1px solid var(--color-border); border-radius: var(--radius-full); color: var(--color-text-primary); font-size: var(--font-size-sm); line-height: 1.6; background: var(--color-background-card); transition: border-color var(--duration-fast) ease, background-color var(--duration-fast) ease; }
.acom-chat-starter:hover { border-color: var(--color-border-emphasized); background: var(--color-background-muted); }
.acom-chat-starter:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 2px; }
.acom-chat-starter[aria-disabled="true"] { opacity: .5; cursor: default; }
.acom-chat-empty { padding: var(--spacing-6) var(--spacing-2); display: flex; flex-direction: column; gap: var(--spacing-3); }
@keyframes acom-rise { from { opacity: 0; transform: translateY(8px); } to { opacity: 1; transform: none; } }
.acom-rise { animation: acom-rise var(--duration-slow, 320ms) cubic-bezier(.2,.7,.2,1) both; }
@media (prefers-reduced-motion: reduce) { .acom-rise { animation: none; } }
`;

export function ChatStyles() {
  return <style>{CHAT_CSS}</style>;
}

/** The top of a chat panel: who is talking, one line on what they do, and the panel's actions. */
export function ChatHeader({title = 'Ask Ailism', subtitle, actions}: {title?: string; subtitle?: string; actions?: ReactNode}) {
  return (
    <div className="acom-chat-header">
      <AilismAvatar size="md" />
      <div className="acom-chat-header-text">
        <Text type="label" weight="semibold">
          {title}
        </Text>
        {subtitle ? (
          <Text type="supporting" color="secondary">
            {subtitle}
          </Text>
        ) : null}
      </div>
      {actions ? (
        <HStack gap={1} vAlign="center">
          {actions}
        </HStack>
      ) : null}
    </div>
  );
}

/** Sentences a person can start with, as quiet chips. */
export function Starters({items, align = 'start'}: {items: string[]; align?: 'start' | 'center'}) {
  const ai = useAi();
  return (
    <div className="acom-chat-starters" data-align={align} role="group" aria-label="Ways to start">
      {items.map(s => (
        <button
          key={s}
          type="button"
          className="acom-chat-starter"
          aria-disabled={ai.isBusy}
          onClick={() => {
            if (!ai.isBusy) void ai.send(s);
          }}>
          {s}
        </button>
      ))}
    </div>
  );
}

/** A chat panel's empty state: a line on what to do, and starters. */
export function ChatEmpty({title, description, starters}: {title: string; description?: string; starters?: string[]}) {
  return (
    <div className="acom-chat-empty">
      <VStack gap={1}>
        <Text type="label" weight="semibold">
          {title}
        </Text>
        {description ? (
          <Text type="supporting" color="secondary">
            {description}
          </Text>
        ) : null}
      </VStack>
      {starters && starters.length > 0 ? <Starters items={starters} /> : null}
    </div>
  );
}

/** What the agent is doing right now (the running step of the turn), while a turn is open. */
export function StepProgress() {
  const ai = useAi();
  if (!ai.isBusy) return null;
  const running = [...ai.steps].reverse().find(s => s.status === 'running') ?? ai.steps[ai.steps.length - 1];
  return (
    <HStack gap={2} vAlign="center" className="acom-chat-step" role="status" aria-live="polite">
      <Spinner size="sm" />
      <Text type="supporting" color="secondary">
        {running?.summary ?? 'Thinking…'}
      </Text>
    </HStack>
  );
}

/** The composer: what was picked on the page rides along as chips; voice beside send. */
export function Composer({placeholder, size = 'md', onSent}: {placeholder: string; size?: 'md' | 'lg'; onSent?: () => void}) {
  const ai = useAi();
  const voice = useVoiceInput(size === 'lg' ? 'md' : 'sm');
  return (
    <ChatComposer
      onSubmit={text => {
        void ai.send(text);
        onSent?.();
      }}
      placeholder={placeholder}
      input={<ChatComposerInput />}
      density={size === 'lg' ? 'spacious' : 'compact'}
      elevation="none"
      isDisabled={ai.isBusy}
      sendActions={voice.button}
      status={voice.status}
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
}

const fill: CSSProperties = {flex: 1, minHeight: 0};

/**
 * The conversation: messages, what the agent is doing, and (where there is no canvas beside it)
 * the agent's UI blocks under the messages. Fills its parent; the composer sits at the bottom.
 */
export function Transcript({
  placeholder,
  emptyState,
  showBlocks,
  renderBlock = b => <BlockView block={b} />,
  aboveComposer,
}: {
  placeholder: string;
  emptyState: ReactNode;
  /** Show the blocks in the conversation (a panel with no canvas beside it). */
  showBlocks: boolean;
  renderBlock?: (block: UiBlock) => ReactNode;
  /** Shown over the composer (starters while a conversation runs, for example). */
  aboveComposer?: ReactNode;
}) {
  const ai = useAi();
  const composer = (
    <div className="acom-chat-composer">
      {aboveComposer}
      <Composer placeholder={placeholder} />
    </div>
  );
  return (
    <ChatLayout density="compact" style={fill} composer={composer}>
      <ChatMessageList density="compact" isStreaming={ai.isBusy} emptyState={emptyState}>
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
          ...(ai.isBusy ? [<StepProgress key="step" />] : []),
          ...(showBlocks
            ? ai.blocks.map(b => (
                <div key={b.block_id} className="acom-rise">
                  {renderBlock(b)}
                </div>
              ))
            : []),
        ]}
      </ChatMessageList>
    </ChatLayout>
  );
}

/** A chat panel (dock or drawer): the header over the conversation, its blocks inline. */
export function ChatPanel({header, placeholder, emptyState}: {header: ReactNode; placeholder: string; emptyState: ReactNode}) {
  return (
    <div className="acom-chat-panel">
      <ChatStyles />
      {header}
      <div className="acom-chat-body">
        <Transcript placeholder={placeholder} emptyState={emptyState} showBlocks />
      </div>
    </div>
  );
}
