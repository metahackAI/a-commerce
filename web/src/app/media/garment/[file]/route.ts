// Product pictures for the demo store: a flat drawing of the garment in its color, on a light tint
// of that color (the look of the storefront mockups in design/storefront). The catalog stores
// URLs such as `/media/garment/wind-slate.svg`; the first word is the shape, the rest the color.
// Unknown shapes or colors fall back to a folded garment in stone, so a picture always shows.

const OUTLINE = 'fill="none" stroke="#000" stroke-opacity=".16" stroke-width="1.6"';

// viewBox 0 0 120 140. The first six follow design/storefront; the rest are drawn the same way.
const SHAPES: Record<string, string> = {
  field: `<path fill="currentColor" d="M42 14 60 24 78 14 100 26 110 94 96 97 92 54 91 130H29L28 54 24 97 10 94 20 26Z"/><g ${OUTLINE}><path d="M42 14 51 32 60 24 69 32 78 14M60 24V130"/><rect x="36" y="58" width="16" height="15" rx="1.5"/><rect x="68" y="58" width="16" height="15" rx="1.5"/><rect x="35" y="92" width="18" height="18" rx="1.5"/><rect x="67" y="92" width="18" height="18" rx="1.5"/></g>`,
  wind: `<path fill="currentColor" d="M44 18Q60-2 76 18L100 28 110 94 96 97 92 56 91 126H29L28 56 24 97 10 94 20 28Z"/><g ${OUTLINE}><path d="M48 22Q60 10 72 22M60 24V72M38 92H82L86 114H34Z"/><path d="M29 120H91" stroke-dasharray="3 3"/></g>`,
  shirt: `<path fill="currentColor" d="M44 14 60 22 76 14 98 26 106 70 94 73 91 52 90 126H30L29 52 26 73 14 70 22 26Z"/><g ${OUTLINE}><path d="M44 14 53 34 60 22 67 34 76 14M60 26V126"/><rect x="67" y="44" width="14" height="14" rx="1.5"/></g><g fill="#000" fill-opacity=".18"><circle cx="63" cy="42" r="1.6"/><circle cx="63" cy="60" r="1.6"/><circle cx="63" cy="78" r="1.6"/><circle cx="63" cy="96" r="1.6"/><circle cx="63" cy="114" r="1.6"/></g>`,
  chore: `<path fill="currentColor" d="M42 14 60 24 78 14 100 26 110 96 96 99 92 54 92 134H28L28 54 24 99 10 96 20 26Z"/><g ${OUTLINE}><path d="M42 14 49 38 60 24 71 38 78 14M60 24V134"/><rect x="66" y="48" width="15" height="15" rx="1.5"/><rect x="34" y="94" width="20" height="21" rx="1.5"/><rect x="66" y="94" width="20" height="21" rx="1.5"/></g>`,
  parka: `<path fill="currentColor" d="M42 18Q60-4 78 18L100 28 110 100 96 103 92 58 93 138H27L28 58 24 103 10 100 20 28Z"/><g ${OUTLINE}><path d="M46 22Q60 6 74 22M60 24V138M28 84H92"/><path d="M36 98H52V112H36ZM68 98H84V112H68Z"/></g>`,
  boot: `<path fill="currentColor" d="M38 22H74V84Q74 92 82 95L104 104Q112 108 112 118V124H22Q18 124 18 118L20 96Q22 88 30 84 38 80 38 70Z"/><g ${OUTLINE}><path d="M46 26L52 66 58 26M22 114H112"/></g>`,
  tee: `<path fill="currentColor" d="M42 18 52 14Q60 22 68 14L78 18 104 34 96 54 84 48 84 124H36L36 48 24 54 16 34Z"/><g ${OUTLINE}><path d="M52 14Q60 28 68 14M36 116H84M24 54 30 40M96 54 90 40"/></g>`,
  knit: `<path fill="currentColor" d="M44 16Q60 26 76 16L98 26 110 102 98 104 90 50 90 126H30L30 50 22 104 10 102 22 26Z"/><g ${OUTLINE}><path d="M45 17Q60 32 75 17M30 116H90M11 94 23 96M97 96 109 94"/><path d="M36 116V126M44 116V126M52 116V126M60 116V126M68 116V126M76 116V126M84 116V126"/></g>`,
  vest: `<path fill="currentColor" d="M44 14 60 24 76 14 92 22Q86 40 92 50V124H28V50Q34 40 28 22Z"/><g ${OUTLINE}><path d="M44 14 52 30 60 24 68 30 76 14M60 24V124M28 62H92M28 86H92M28 110H92"/></g>`,
  pants: `<path fill="currentColor" d="M34 14H86L94 130H70L60 54 50 130H26Z"/><g ${OUTLINE}><path d="M34 24H86M60 24V46M38 24Q40 38 50 40M82 24Q80 38 70 40"/></g>`,
  shorts: `<path fill="currentColor" d="M30 34H90L100 96H66L60 66 54 96H20Z"/><g ${OUTLINE}><path d="M30 44H90M60 44V62M34 44Q36 56 46 58M86 44Q84 56 74 58"/></g>`,
  dress: `<path fill="currentColor" d="M46 14 52 22Q60 28 68 22L74 14 84 18 78 52 100 130H20L42 52 36 18Z"/><g ${OUTLINE}><path d="M52 22Q60 32 68 22M42 52H78M50 52 44 130M70 52 76 130"/></g>`,
  skirt: `<path fill="currentColor" d="M38 30H82L102 120H18Z"/><g ${OUTLINE}><path d="M38 40H82M48 40 40 120M60 40V120M72 40 80 120"/></g>`,
  shoe: `<path fill="currentColor" d="M14 92Q14 76 28 74L50 70Q60 52 76 54L94 60Q108 66 108 84V104H14Z"/><g ${OUTLINE}><path d="M14 96H108M58 66 66 76M66 62 74 72M74 60 82 70"/></g>`,
  bag: `<path fill="none" stroke="currentColor" stroke-opacity="1" stroke-width="6" d="M42 54V40Q42 24 60 24 78 24 78 40V54"/><path fill="currentColor" d="M22 52H98L106 128H14Z"/><g ${OUTLINE}><path d="M22 68H98M48 52V68M72 52V68"/></g>`,
  hat: `<circle cx="60" cy="30" r="9" fill="currentColor"/><path fill="currentColor" d="M24 100Q24 36 60 36 96 36 96 100Z"/><path fill="currentColor" d="M20 94H100V114H20Z"/><g ${OUTLINE}><path d="M20 94H100M30 94V114M40 94V114M50 94V114M60 94V114M70 94V114M80 94V114M90 94V114"/></g>`,
  socks: `<path fill="currentColor" d="M44 14H76V84L98 100Q108 108 102 120 96 130 84 124L50 106Q44 102 44 94Z"/><g ${OUTLINE}><path d="M44 24H76M44 30H76M46 92Q58 88 64 100"/></g>`,
  scarf: `<path fill="currentColor" d="M40 12H64V126H40Z"/><path fill="currentColor" d="M64 56H88V126H64Z"/><g ${OUTLINE}><path d="M40 118H64M64 118H88M44 126V134M50 126V134M56 126V134M68 126V134M76 126V134M84 126V134"/></g>`,
  belt: `<path fill="currentColor" d="M8 62H112V80H8Z"/><g ${OUTLINE}><rect x="22" y="56" width="20" height="30" rx="2"/><path d="M32 71H52M70 71H72M82 71H84M94 71H96"/></g>`,
  fold: `<path fill="currentColor" d="M20 42H100V112H20Z"/><g ${OUTLINE}><path d="M20 60H100M44 42V60M76 42V60"/></g>`,
};

// The demo store's color words as hex. Several words map to the same family (`navy stripe`).
const COLORS: Record<string, string> = {
  white: '#ECE9E3',
  natural: '#E3D9C6',
  oatmeal: '#D9CFBC',
  sand: '#CDB894',
  stone: '#B9B3A6',
  tan: '#C4A27C',
  clay: '#C2876B',
  rust: '#B5654A',
  olive: '#8F9773',
  sky: '#9DB9D3',
  slate: '#7F8FA0',
  'heather-grey': '#A9A9AB',
  grey: '#9A9A9A',
  charcoal: '#55595E',
  navy: '#3E4C66',
  'navy-stripe': '#3E4C66',
  indigo: '#4A5A86',
  black: '#3A3A3A',
};

function hex(c: string): [number, number, number] {
  return [1, 3, 5].map(i => parseInt(c.slice(i, i + 2), 16)) as [number, number, number];
}

/** The background: the color mixed into the page's paper tone, so each picture reads as one hue. */
function tint(c: string): string {
  const paper: [number, number, number] = [244, 242, 238];
  const rgb = hex(c);
  const light = (rgb[0] * 299 + rgb[1] * 587 + rgb[2] * 114) / 1000 > 215;
  const share = light ? 0.55 : 0.24;
  const mixed = rgb.map((v, i) => Math.round(paper[i] * (1 - share) + v * share * (light ? 0.86 : 1)));
  return `#${mixed.map(v => v.toString(16).padStart(2, '0')).join('')}`;
}

export function GET(_req: Request, {params}: {params: Promise<{file: string}>}) {
  return params.then(({file}) => {
    const name = file.replace(/\.svg$/, '');
    const dash = name.indexOf('-');
    const shape = SHAPES[dash > 0 ? name.slice(0, dash) : name] ?? SHAPES.fold;
    const color = COLORS[dash > 0 ? name.slice(dash + 1) : ''] ?? COLORS.stone;
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 500" role="img" aria-hidden="true"><rect width="400" height="500" fill="${tint(color)}"/><g transform="translate(80 110) scale(2)" color="${color}" stroke="#000" stroke-opacity=".06" stroke-width=".6">${shape}</g></svg>`;
    return new Response(svg, {
      headers: {'content-type': 'image/svg+xml', 'cache-control': 'public, max-age=86400'},
    });
  });
}
