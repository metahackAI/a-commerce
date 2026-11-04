'use client';

// Inside the workspace's canvas, what a person opens stays on the canvas: a product, an order or a
// collection comes up as the next canvas page, with the way back above it. Outside the canvas (the
// storefront's own pages, a phone, the back office) there is no provider and a link is a link. A
// ctrl- or cmd-click, and the "Open in new tab" button, still open the full page in a tab.

import {Button} from '@astryxdesign/core/Button';
import {Icon} from '@astryxdesign/core/Icon';
import {ArrowTopRightOnSquareIcon} from '@heroicons/react/24/outline';
import {createContext, useContext, type MouseEvent} from 'react';

export interface CanvasPage {
  kind: 'product' | 'order' | 'collection';
  id: string;
  /** A product's color to open on. */
  color?: string | null;
  /** The name to title the page with. */
  label?: string | null;
  /** The full page, for "Open in new tab". */
  href: string;
}

/** The canvas page a storefront link stands for, when it has one. */
export function canvasPageOf(href: string): Omit<CanvasPage, 'label'> | null {
  const [path, search] = href.split('?');
  const [, section, raw, rest] = path.split('/');
  if (!raw || rest) return null;
  const id = decodeURIComponent(raw);
  if (section === 'p') return {kind: 'product', id, color: new URLSearchParams(search).get('color'), href};
  if (section === 'orders') return {kind: 'order', id, href};
  if (section === 'collections') return {kind: 'collection', id, href};
  return null;
}

const Ctx = createContext<((page: CanvasPage) => void) | null>(null);

export const CanvasPagesProvider = Ctx.Provider;

/** Whether this element is on the canvas. */
export function useOnCanvas(): boolean {
  return useContext(Ctx) !== null;
}

/**
 * Props for a link: on the canvas, a plain click opens the next canvas page. A page with no canvas
 * form opens in a new tab, so the conversation stays where it is.
 */
export function useCanvasLink(href: string, label?: string | null): {href: string; target?: string; onClick?: (e: MouseEvent<HTMLElement>) => void} {
  const open = useContext(Ctx);
  if (!open) return {href};
  const page = canvasPageOf(href);
  if (!page) return {href, target: '_blank'};
  return {
    href,
    onClick: e => {
      if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return;
      e.preventDefault();
      open({...page, label});
    },
  };
}

/** "Open in new tab" beside what opens on the canvas. Off the canvas a link already goes there. */
export function OpenInNewTab({href, label = 'Open in new tab', showLabel = false}: {href: string; label?: string; showLabel?: boolean}) {
  const onCanvas = useOnCanvas();
  if (!onCanvas) return null;
  return (
    <Button
      label={label}
      isIconOnly={!showLabel}
      tooltip={showLabel ? undefined : label}
      variant="ghost"
      size="sm"
      icon={<Icon icon={ArrowTopRightOnSquareIcon} />}
      href={href}
      target="_blank"
    />
  );
}
