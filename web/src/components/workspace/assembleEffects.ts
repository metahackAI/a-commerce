// What a piece of a block does as it assembles (Assembling): its text fills in fast and its pictures
// form from a shifting mosaic. Neither changes the page's layout or anything React wrote. Text is
// hidden with the CSS Custom Highlight API, which paints over text nodes without touching them, so
// lines never rewrap. A picture is covered by a canvas that is taken away when it is done. A browser
// without highlights shows the text at once.

/** One character's time, and the most one run of text takes, in ms. */
const PER_CHAR = 5;
const LONGEST_TEXT = 420;
/** Text in one piece runs one after another, but never later than this, in ms. */
const LONGEST_PIECE = 700;
/** A picture's whole mosaic, and how long to wait for one still loading, in ms. */
const MOSAIC = 700;
const WAIT_FOR_PICTURE = 3000;
/** Pictures smaller than this (an icon, a small thumbnail) just arrive with their piece. */
const SMALLEST = 56;
/** Cells across a picture, and how many finer cells each divides into on its way to clear. */
const CELLS = 20;
const FINER = 2;

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

/** The picture in `cols` by `rows` cells, cropped as `object-fit: cover` crops it. */
function mosaicLayer(img: HTMLImageElement, cols: number, rows: number, width: number, height: number): HTMLCanvasElement {
  const layer = document.createElement('canvas');
  layer.width = cols;
  layer.height = rows;
  const scale = Math.max(width / img.naturalWidth, height / img.naturalHeight);
  const sw = width / scale;
  const sh = height / scale;
  layer.getContext('2d')?.drawImage(img, (img.naturalWidth - sw) / 2, (img.naturalHeight - sh) / 2, sw, sh, 0, 0, cols, rows);
  return layer;
}

/**
 * Covers a picture with a canvas that resolves it, starting `delay` ms from now: from the top left
 * down, each small cell goes from nothing through a brief coarse and fine mosaic, then fades into the
 * picture. Most of a cell's time is the fade, so it reads as the picture forming, not as pixels.
 */
export function mosaicIn(img: HTMLImageElement, delay: number): void {
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
  ctx.imageSmoothingEnabled = false;
  const blank = blankColor(parent, ctx);
  ctx.fillStyle = blank;
  ctx.fillRect(0, 0, width, height);
  parent.appendChild(canvas);

  const cellSize = width / CELLS;
  const cols = CELLS;
  const rows = Math.max(1, Math.round(height / cellSize));
  const cw = Math.ceil(width / cols);
  const ch = Math.ceil(height / rows);

  const start = () => {
    // A picture that failed, or has no size of its own, is left as it is.
    if (img.naturalWidth === 0 || img.naturalHeight === 0) {
      canvas.remove();
      return;
    }
    const coarse = mosaicLayer(img, cols, rows, width, height);
    const fine = mosaicLayer(img, cols * FINER, rows * FINER, width, height);
    const startAt = performance.now() + delay;
    const draw = (now: number) => {
      if (!canvas.isConnected) return;
      const elapsed = (now - startAt) / MOSAIC;
      if (elapsed >= 1) {
        canvas.remove();
        return;
      }
      for (let r = 0; r < rows; r++) {
        for (let c = 0; c < cols; c++) {
          // Mostly downward, leaning from the left, so it starts at the top left corner.
          const order = (r / rows) * 0.7 + (c / cols) * 0.3;
          const t = (elapsed - order * 0.55) / 0.45;
          const x = Math.floor((c * width) / cols);
          const y = Math.floor((r * height) / rows);
          if (t < 0) {
            ctx.fillStyle = blank;
            ctx.fillRect(x, y, cw, ch);
          } else if (t < 0.15) {
            ctx.drawImage(coarse, c, r, 1, 1, x, y, cw, ch);
          } else if (t < 0.35) {
            ctx.drawImage(fine, c * FINER, r * FINER, FINER, FINER, x, y, cw, ch);
          } else {
            ctx.clearRect(x, y, cw, ch);
            if (t < 1) {
              ctx.globalAlpha = 1 - (t - 0.35) / 0.65;
              ctx.drawImage(fine, c * FINER, r * FINER, FINER, FINER, x, y, cw, ch);
              ctx.globalAlpha = 1;
            }
          }
        }
      }
      requestAnimationFrame(draw);
    };
    requestAnimationFrame(draw);
  };

  if (img.complete) {
    start();
    return;
  }
  // Still loading: the blank cover holds its place, and goes if the picture never comes.
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
