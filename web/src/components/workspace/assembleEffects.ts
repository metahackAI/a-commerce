// What a piece of a block does as it assembles (Assembling): its text fills in fast and its pictures
// slide into view from under a soft cover. Neither changes the page's layout or anything React wrote. Text is
// hidden with the CSS Custom Highlight API, which paints over text nodes without touching them, so
// lines never rewrap. A picture is covered by a canvas that is taken away when it is done. A browser
// without highlights shows the text at once.

/** One character's time, and the most one run of text takes, in ms. */
const PER_CHAR = 5;
const LONGEST_TEXT = 420;
/** Text in one piece runs one after another, but never later than this, in ms. */
const LONGEST_PIECE = 700;
/** A picture's whole reveal, and how long to wait for one still loading, in ms. */
const REVEAL = 800;
const WAIT_FOR_PICTURE = 3000;
/** Pictures smaller than this (an icon, a small thumbnail) just arrive with their piece. */
const SMALLEST = 56;
/** How wide the cover's soft edge is, as a share of the picture. */
const SOFT_EDGE = 0.55;
/** The way the cover moves: mostly downward, leaning right, so it starts at the top left corner. */
const SWEEP = {x: 0.35, y: 0.94};

interface Filling {
  node: Text;
  range: Range;
  startAt: number;
  duration: number;
}

const HIGHLIGHT = 'acom-unfilled';
const filling: Filling[] = [];
let unfilled: Highlight | null = null;
let ticking = false;

function fill(now: number) {
  for (let i = filling.length - 1; i >= 0; i--) {
    const f = filling[i];
    const length = f.node.length;
    const typed = Math.floor(Math.max(0, Math.min(1, (now - f.startAt) / f.duration)) * length);
    if (!f.node.isConnected || typed >= length) {
      unfilled?.delete(f.range);
      filling.splice(i, 1);
    } else {
      f.range.setStart(f.node, typed);
      f.range.setEnd(f.node, length);
    }
  }
  if (filling.length > 0) requestAnimationFrame(fill);
  else ticking = false;
}

/** Fills the text under `piece` in, starting `delay` ms from now. */
export function fillTextIn(piece: HTMLElement, delay: number): void {
  if (typeof Highlight === 'undefined' || !CSS.highlights) return;
  if (!unfilled) {
    unfilled = new Highlight();
    CSS.highlights.set(HIGHLIGHT, unfilled);
  }
  const walker = document.createTreeWalker(piece, NodeFilter.SHOW_TEXT);
  const now = performance.now();
  let spent = 0;
  for (let node = walker.nextNode() as Text | null; node; node = walker.nextNode() as Text | null) {
    if (!node.data.trim() || node.parentElement?.closest('style, script')) continue;
    const range = new Range();
    range.selectNodeContents(node);
    unfilled.add(range);
    const duration = Math.min(LONGEST_TEXT, 40 + node.length * PER_CHAR);
    filling.push({node, range, startAt: now + delay + spent, duration});
    spent = Math.min(LONGEST_PIECE, spent + duration * 0.7);
  }
  if (filling.length > 0 && !ticking) {
    ticking = true;
    requestAnimationFrame(fill);
  }
}

/**
 * The muted color as a canvas can use it. The token's own text may be a form a canvas does not parse,
 * and a canvas ignores what it cannot parse and stays black, so it is read as the browser resolved it,
 * and anything else keeps the light fallback.
 */
function blankColor(parent: HTMLElement, ctx: CanvasRenderingContext2D): string {
  const probe = document.createElement('div');
  probe.style.cssText = 'display:none;background-color:var(--color-background-muted);';
  parent.appendChild(probe);
  const resolved = getComputedStyle(probe).backgroundColor;
  probe.remove();
  ctx.fillStyle = '#ececec';
  if (resolved && resolved !== 'transparent' && resolved !== 'rgba(0, 0, 0, 0)') ctx.fillStyle = resolved;
  return String(ctx.fillStyle);
}

/**
 * Covers a picture with a canvas that lifts off it, starting `delay` ms from now: a soft edge moves
 * from the top left corner down, and the picture shows through behind it.
 */
export function revealPicture(img: HTMLImageElement, delay: number): void {
  const parent = img.parentElement;
  const width = img.offsetWidth;
  const height = img.offsetHeight;
  if (!parent || width < SMALLEST || height < SMALLEST) return;
  if (getComputedStyle(parent).position === 'static') parent.style.position = 'relative';

  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  canvas.setAttribute('aria-hidden', 'true');
  canvas.style.cssText = `position:absolute;left:${img.offsetLeft}px;top:${img.offsetTop}px;width:${width}px;height:${height}px;max-width:none;pointer-events:none;border-radius:inherit;z-index:1;`;
  const ctx = canvas.getContext('2d');
  if (!ctx) return;
  const blank = blankColor(parent, ctx);
  ctx.fillStyle = blank;
  ctx.fillRect(0, 0, width, height);
  parent.appendChild(canvas);

  // The sweep runs from the top left corner to the far corner along its direction.
  const length = width * SWEEP.x + height * SWEEP.y;
  const edgeAt = (x: number, front: number) => Math.max(0, Math.min(1, (x - front) / SOFT_EDGE));

  const start = () => {
    const startAt = performance.now() + delay;
    const draw = (now: number) => {
      if (!canvas.isConnected) return;
      const elapsed = Math.max(0, (now - startAt) / REVEAL);
      if (elapsed >= 1) {
        canvas.remove();
        return;
      }
      // Eases out; the front starts a soft edge before the picture and ends past its far corner.
      const front = (1 - (1 - elapsed) ** 2) * (1 + SOFT_EDGE) - SOFT_EDGE;
      const shade = ctx.createLinearGradient(0, 0, SWEEP.x * length, SWEEP.y * length);
      shade.addColorStop(0, `rgba(0,0,0,${edgeAt(0, front)})`);
      if (front > 0 && front < 1) shade.addColorStop(front, 'rgba(0,0,0,0)');
      if (front + SOFT_EDGE > 0 && front + SOFT_EDGE < 1) shade.addColorStop(front + SOFT_EDGE, 'rgba(0,0,0,1)');
      shade.addColorStop(1, `rgba(0,0,0,${edgeAt(1, front)})`);
      ctx.clearRect(0, 0, width, height);
      ctx.globalCompositeOperation = 'source-over';
      ctx.fillStyle = shade;
      ctx.fillRect(0, 0, width, height);
      // The shape above, in the cover's color.
      ctx.globalCompositeOperation = 'source-in';
      ctx.fillStyle = blank;
      ctx.fillRect(0, 0, width, height);
      ctx.globalCompositeOperation = 'source-over';
      requestAnimationFrame(draw);
    };
    requestAnimationFrame(draw);
  };

  if (img.complete) {
    start();
    return;
  }
  // Still loading: the cover holds its place, and goes if the picture never comes.
  const giveUp = window.setTimeout(() => canvas.remove(), WAIT_FOR_PICTURE);
  const ready = () => {
    window.clearTimeout(giveUp);
    if (canvas.isConnected) start();
  };
  img.addEventListener('load', ready, {once: true});
  img.addEventListener(
    'error',
    () => {
      window.clearTimeout(giveUp);
      canvas.remove();
    },
    {once: true},
  );
}
