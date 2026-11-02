'use client';

// The conversation + canvas workspace both surfaces use: the storefront's main page and /ask, and
// the back office's conversation view. It opens as one large conversation in the middle of the
// screen. When the agent answers with UI blocks (docs/api/chat.md §3) the conversation slides to
// the left and the canvas opens on the right, where the blocks are assembled as they arrive. The
// canvas shows one turn's blocks at a time: the latest, or the turn a person put there with the
// button under its answer. A handle between the two sets how wide the conversation is. On a
// narrow screen there is no room for a canvas, so the blocks stay in the conversation.

import {Button} from '@astryxdesign/core/Button';
import {Icon} from '@astryxdesign/core/Icon';
import {IconButton} from '@astryxdesign/core/IconButton';
import {HStack} from '@astryxdesign/core/Layout';
import {ResizeHandle, useResizable, type ResizableProps} from '@astryxdesign/core/Resizable';
import {Heading, Text} from '@astryxdesign/core/Text';
import {ChevronLeftIcon, ChevronRightIcon} from '@heroicons/react/24/outline';
import {useEffect, useRef, useState, type CSSProperties} from 'react';

import {useAi} from '@/ai/AiProvider';
import type {UiBlock} from '@/ai/types';
import {AilismAvatar} from '@/components/brand/Ailism';
import {BlockView} from '@/components/bo/BlockView';
import {ChatStyles, Composer, Starters, StepProgress, Transcript} from '@/components/chat/ChatParts';

/** Below this width of the workspace the canvas folds into the conversation. */
const NARROW = 900;
/** The conversation's width beside the canvas: where it starts, and how far a person can drag it. */
const CHAT_WIDTH = {defaultSize: 420, minSize: 320, maxSize: '60%'} as const;

// The three states share one tree so the change between them can animate: the chat column's width
// and the canvas's growth are transitioned, the blocks rise in as they arrive.
const WORKSPACE_CSS = `
.acom-ws { position: relative; display: flex; width: 100%; height: 100%; min-height: 0; overflow: hidden; background: var(--color-background-body); }
.acom-ws-chat { display: flex; flex-direction: column; min-width: 0; min-height: 0; flex: 1 1 100%; background: var(--color-background-card);
  transition: flex-basis 560ms cubic-bezier(.2,.7,.2,1), flex-grow 560ms cubic-bezier(.2,.7,.2,1); }
.acom-ws-chat-inner { display: flex; flex-direction: column; flex: 1; min-height: 0; width: 100%; max-width: 760px; margin: 0 auto; transition: max-width 560ms cubic-bezier(.2,.7,.2,1); }
.acom-ws[data-mode="split"] .acom-ws-chat { flex: 0 0 var(--acom-ws-chat-width, 420px); }
.acom-ws[data-mode="split"] .acom-ws-chat-inner { max-width: 100%; }
.acom-ws-canvas { display: flex; flex-direction: column; min-width: 0; min-height: 0; flex: 0 0 0; opacity: 0; overflow: hidden;
  transition: flex-grow 560ms cubic-bezier(.2,.7,.2,1), opacity 400ms ease 160ms; }
.acom-ws[data-mode="split"] .acom-ws-canvas { flex: 1 1 0; opacity: 1; }
.acom-ws-canvas-bar { display: flex; align-items: center; justify-content: space-between; gap: var(--spacing-2); padding: var(--spacing-3) var(--spacing-5); border-bottom: 1px solid var(--color-border); background: var(--color-background-card); flex-shrink: 0; }
.acom-ws-canvas-for { display: flex; flex-direction: column; min-width: 0; }
.acom-ws-canvas-for > * { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
/* While the handle is dragged the widths follow the pointer, not an animation. */
.acom-ws[data-resizing="true"] .acom-ws-chat, .acom-ws[data-resizing="true"] .acom-ws-chat-inner, .acom-ws[data-resizing="true"] .acom-ws-canvas { transition: none; }
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

/** Moving between the turns that put results on the canvas. */
interface CanvasNav {
  position: number;
  total: number;
  onEarlier?: () => void;
  onLater?: () => void;
  onLatest?: () => void;
}

/**
 * The canvas: one turn's blocks in the order they came. While the turn runs, the newest is
 * scrolled into view and marked briefly; a turn a person puts here opens at its top.
 */
function Canvas({
  title,
  turnId,
  question,
  blocks,
  isLive,
  nav,
}: {
  title: string;
  turnId: string;
  /** What the person said in that turn. */
  question: string | null;
  blocks: UiBlock[];
  isLive: boolean;
  nav: CanvasNav;
}) {
  const body = useRef<HTMLDivElement | null>(null);
  const shownTurn = useRef<string | null>(null);
  const [newest, setNewest] = useState<string | null>(null);
  const [settled, setSettled] = useState(false);
  const last = blocks[blocks.length - 1]?.block_id ?? null;

  useEffect(() => {
    if (!last) return;
    const switched = shownTurn.current !== turnId;
    shownTurn.current = turnId;
    if (!isLive) {
      if (switched) body.current?.scrollTo({top: 0});
      setNewest(null);
      return;
    }
    setNewest(last);
    setSettled(false);
    const el = body.current?.querySelector<HTMLElement>(`[data-block-id="${CSS.escape(last)}"]`);
    el?.scrollIntoView({behavior: 'smooth', block: 'start'});
    const t = window.setTimeout(() => setSettled(true), 900);
    return () => window.clearTimeout(t);
  }, [last, turnId, isLive]);

  const results = blocks.length === 1 ? '1 result' : `${blocks.length} results`;
  return (
    <>
      <div className="acom-ws-canvas-bar">
        <div className="acom-ws-canvas-for">
          <Text type="label" weight="semibold">
            {title}
          </Text>
          <Text type="supporting" color="secondary">
            {question ? `${results} for “${question}”` : results}
          </Text>
        </div>
        {nav.total > 1 ? (
          <HStack gap={1} vAlign="center">
            {nav.onLatest ? <Button label="Latest" variant="ghost" size="sm" onClick={nav.onLatest} /> : null}
            <IconButton
              label="Earlier results"
              variant="ghost"
              size="sm"
              icon={<Icon icon={ChevronLeftIcon} />}
              isDisabled={!nav.onEarlier}
              onClick={nav.onEarlier}
            />
            <Text type="supporting" color="secondary" hasTabularNumbers>
              {nav.position} / {nav.total}
            </Text>
            <IconButton
              label="Later results"
              variant="ghost"
              size="sm"
              icon={<Icon icon={ChevronRightIcon} />}
              isDisabled={!nav.onLater}
              onClick={nav.onLater}
            />
          </HStack>
        ) : null}
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

  const chat = useResizable({...CHAT_WIDTH, containerRef: root, autoSaveId: `acom-ws-chat-${ai.surface}`});
  const [resizing, setResizing] = useState(false);
  // The handle reports a drag through these; marking the workspace while one runs stops the
  // widths from animating behind the pointer.
  const handle: ResizableProps = {
    ...chat.props,
    _onResizeStart: () => {
      setResizing(true);
      chat.props._onResizeStart();
    },
    _onResizeEnd: () => {
      setResizing(false);
      chat.props._onResizeEnd();
    },
    _onResizeCancel: () => {
      setResizing(false);
      chat.props._onResizeCancel?.();
    },
  };
  const style = {'--acom-ws-chat-width': `${chat.size}px`} as CSSProperties;

  const turns = ai.resultTurns;
  const turn = ai.canvasTurn;
  const at = turn ? turns.indexOf(turn) : -1;
  const nav: CanvasNav = {
    position: at + 1,
    total: turns.length,
    onEarlier: at > 0 ? () => ai.showOnCanvas(turns[at - 1]) : undefined,
    onLater: at >= 0 && at < turns.length - 1 ? () => ai.showOnCanvas(turns[at + 1]) : undefined,
    onLatest: at >= 0 && at < turns.length - 1 ? () => ai.showOnCanvas(null) : undefined,
  };
  const question = ai.messages.find(m => m.turn_id === turn && m.sender === 'user')?.text ?? null;

  return (
    <div className="acom-ws" data-mode={mode} data-resizing={resizing} ref={root} style={style}>
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
      {mode === 'split' ? (
        <ResizeHandle direction="horizontal" resizable={handle} hasDivider label="Resize the conversation" />
      ) : null}
      <section className="acom-ws-canvas" aria-label={canvasTitle} aria-hidden={mode !== 'split'}>
        {turn && !narrow ? (
          <Canvas
            title={canvasTitle}
            turnId={turn}
            question={question}
            blocks={ai.blocksOf(turn)}
            isLive={ai.liveTurn === turn}
            nav={nav}
          />
        ) : null}
      </section>
    </div>
  );
}

/** A small link-like button for under the starters. */
export function WorkspaceLink({label, href}: {label: string; href: string}) {
  return <Button label={label} variant="ghost" size="sm" href={href} />;
}
