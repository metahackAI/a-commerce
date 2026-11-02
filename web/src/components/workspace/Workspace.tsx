'use client';

// The conversation + canvas workspace both surfaces use: the storefront's main page and /ask, and
// the back office's conversation view. It opens as one large conversation in the middle of the
// screen. When the agent answers with UI blocks (docs/api/chat.md §3) the conversation slides to
// the left and the canvas opens on the right, where the blocks are assembled as they arrive. On a
// narrow screen there is no room for a canvas, so the blocks stay in the conversation.

import {Button} from '@astryxdesign/core/Button';
import {Heading, Text} from '@astryxdesign/core/Text';
import {useEffect, useRef, useState} from 'react';

import {useAi} from '@/ai/AiProvider';
import type {UiBlock} from '@/ai/types';
import {AilismAvatar} from '@/components/brand/Ailism';
import {BlockView} from '@/components/bo/BlockView';
import {ChatStyles, Composer, Starters, StepProgress, Transcript} from '@/components/chat/ChatParts';

/** Below this width of the workspace the canvas folds into the conversation. */
const NARROW = 900;

// The three states share one tree so the change between them can animate: the chat column's width
// and the canvas's growth are transitioned, the blocks rise in as they arrive.
const WORKSPACE_CSS = `
.acom-ws { position: relative; display: flex; width: 100%; height: 100%; min-height: 0; overflow: hidden; background: var(--color-background-body); }
.acom-ws-chat { display: flex; flex-direction: column; min-width: 0; min-height: 0; flex: 1 1 100%; background: var(--color-background-card);
  transition: flex-basis 560ms cubic-bezier(.2,.7,.2,1), flex-grow 560ms cubic-bezier(.2,.7,.2,1); }
.acom-ws-chat-inner { display: flex; flex-direction: column; flex: 1; min-height: 0; width: 100%; max-width: 760px; margin: 0 auto; transition: max-width 560ms cubic-bezier(.2,.7,.2,1); }
.acom-ws[data-mode="split"] .acom-ws-chat { flex: 0 0 420px; border-right: 1px solid var(--color-border); }
.acom-ws[data-mode="split"] .acom-ws-chat-inner { max-width: 420px; }
.acom-ws-canvas { display: flex; flex-direction: column; min-width: 0; min-height: 0; flex: 0 0 0; opacity: 0; overflow: hidden;
  transition: flex-grow 560ms cubic-bezier(.2,.7,.2,1), opacity 400ms ease 160ms; }
.acom-ws[data-mode="split"] .acom-ws-canvas { flex: 1 1 0; opacity: 1; }
.acom-ws-canvas-bar { display: flex; align-items: center; justify-content: space-between; gap: var(--spacing-2); padding: var(--spacing-3) var(--spacing-5); border-bottom: 1px solid var(--color-border); background: var(--color-background-card); flex-shrink: 0; }
.acom-ws-canvas-body { flex: 1; min-height: 0; overflow-y: auto; padding: var(--spacing-5); }
.acom-ws-canvas-stack { display: flex; flex-direction: column; gap: var(--spacing-4); max-width: 1100px; margin: 0 auto; }
.acom-ws-hero { flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; justify-content: center; gap: var(--spacing-5); padding: var(--spacing-8) var(--spacing-5); }
.acom-ws-hero-head { display: flex; flex-direction: column; align-items: center; gap: var(--spacing-3); text-align: center; }
.acom-ws-hero .acom-ws-hero-below { display: flex; flex-direction: column; align-items: center; gap: var(--spacing-3); }
.acom-ws-new { outline: 2px solid var(--color-accent); outline-offset: 4px; border-radius: var(--radius-container); transition: outline-color 1200ms ease; }
.acom-ws-new[data-settled="true"] { outline-color: transparent; }
@media (prefers-reduced-motion: reduce) {
  .acom-ws-chat, .acom-ws-chat-inner, .acom-ws-canvas { transition: none; }
}
`;

type Mode = 'empty' | 'talk' | 'split';

function useWidth(ref: React.RefObject<HTMLElement | null>): number {
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const observer = new ResizeObserver(entries => setWidth(entries[0]?.contentRect.width ?? 0));
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref]);
  return width;
}

/** The canvas: the blocks in the order they came, the newest scrolled into view and marked briefly. */
function Canvas({title, blocks}: {title: string; blocks: UiBlock[]}) {
  const body = useRef<HTMLDivElement | null>(null);
  const [newest, setNewest] = useState<string | null>(null);
  const [settled, setSettled] = useState(false);
  const last = blocks[blocks.length - 1]?.block_id ?? null;

  useEffect(() => {
    if (!last) return;
    setNewest(last);
    setSettled(false);
    const el = body.current?.querySelector<HTMLElement>(`[data-block-id="${CSS.escape(last)}"]`);
    el?.scrollIntoView({behavior: 'smooth', block: 'start'});
    const t = window.setTimeout(() => setSettled(true), 900);
    return () => window.clearTimeout(t);
  }, [last]);

  return (
    <>
      <div className="acom-ws-canvas-bar">
        <Text type="label" weight="semibold">
          {title}
        </Text>
        <Text type="supporting" color="secondary">
          {blocks.length === 1 ? '1 result' : `${blocks.length} results`}
        </Text>
      </div>
      <div className="acom-ws-canvas-body" ref={body}>
        <div className="acom-ws-canvas-stack">
          {blocks.map(b => (
            <div
              key={b.block_id}
              data-block-id={b.block_id}
              className={b.block_id === newest ? 'acom-rise acom-ws-new' : 'acom-rise'}
              data-settled={b.block_id === newest ? settled : undefined}>
              <BlockView block={b} />
            </div>
          ))}
        </div>
      </div>
    </>
  );
}

export interface WorkspaceProps {
  /** The large question on the empty workspace. */
  greeting: string;
  /** One line under it. */
  intro?: string;
  composerPlaceholder: string;
  canvasTitle: string;
  starters?: string[];
  /** Under the starters on the empty workspace (links to browse, for example). */
  belowStarters?: React.ReactNode;
  /** @deprecated The canvas opens only when there is something on it. */
  canvasEmpty?: {title: string; description: string};
}

export function Workspace({greeting, intro, composerPlaceholder, canvasTitle, starters = [], belowStarters}: WorkspaceProps) {
  const ai = useAi();
  const root = useRef<HTMLDivElement | null>(null);
  const width = useWidth(root);
  const narrow = width > 0 && width < NARROW;
  const hasBlocks = ai.blocks.length > 0;
  const mode: Mode = ai.messages.length === 0 && !hasBlocks ? 'empty' : hasBlocks && !narrow ? 'split' : 'talk';

  return (
    <div className="acom-ws" data-mode={mode} ref={root}>
      <ChatStyles />
      <style>{WORKSPACE_CSS}</style>
      <section className="acom-ws-chat" aria-label="Conversation">
        <div className="acom-ws-chat-inner">
          {mode === 'empty' ? (
            <div className="acom-ws-hero">
              <div className="acom-ws-hero-head acom-rise">
                <AilismAvatar size="lg" />
                <Heading level={1} type="display-3">
                  {greeting}
                </Heading>
                {intro ? <Text color="secondary">{intro}</Text> : null}
              </div>
              <div className="acom-rise">
                <Composer placeholder={composerPlaceholder} size="lg" />
              </div>
              <div className="acom-ws-hero-below acom-rise">
                {starters.length > 0 ? <Starters items={starters} align="center" /> : null}
                {belowStarters}
              </div>
              <StepProgress />
            </div>
          ) : (
            <Transcript placeholder={composerPlaceholder} emptyState={null} showBlocks={mode === 'talk'} />
          )}
        </div>
      </section>
      <section className="acom-ws-canvas" aria-label={canvasTitle} aria-hidden={mode !== 'split'}>
        {hasBlocks && !narrow ? <Canvas title={canvasTitle} blocks={ai.blocks} /> : null}
      </section>
    </div>
  );
}

/** A small link-like button for under the starters. */
export function WorkspaceLink({label, href}: {label: string; href: string}) {
  return <Button label={label} variant="ghost" size="sm" href={href} />;
}
