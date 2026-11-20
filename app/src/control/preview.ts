// Sticky live preview: a fake board drawn here, with the real overlay renderer on top, cycling demo frames.
import { createRenderer } from '../overlay/renderer';
import type { FishyRenderer } from '../shared/renderer-api';
import { demoFrames, demoGeometry } from '../shared/demo';
import type { AnalysisFrame, BoardGeometry, CandidateMove, MoveClass, Settings, TacticTag } from '../shared/types';
import { h, icon } from './dom';
import { getSettings, subscribeSettings } from './store';
import { at, reg, toggle } from './ui/controls';

// ------------------------------------------------------------ fallback data
function cand(rank: number, uci: string, san: string, cls: MoveClass, cp: number, tags: TacticTag[] = [], explanation: string | null = null): CandidateMove {
  const win = 50 + 50 * (2 / (1 + Math.exp(-0.00368208 * cp)) - 1);
  return {
    rank, uci, san, from: uci.slice(0, 2), to: uci.slice(2, 4), scoreCp: cp, mate: null, winPct: win,
    lossPct: Math.max(0, 54 - win), cls, tags, pvUci: [uci], pvSan: [san], depth: 18, explanation,
  };
}

const FALLBACK: AnalysisFrame[] = [
  {
    fen: 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1', sideToMove: 'w', depth: 20, nps: 1_200_000, evalCp: 28, mate: null, winPct: 54, thinking: false, ts: 0,
    candidates: [
      cand(1, 'e2e4', 'e4', 'best', 28, [], 'Claims the centre and opens lines for the bishop and queen.'),
      cand(2, 'd2d4', 'd4', 'excellent', 24), cand(3, 'g1f3', 'Nf3', 'good', 18), cand(4, 'c2c4', 'c4', 'inaccuracy', 8), cand(5, 'b1c3', 'Nc3', 'mistake', -10),
    ],
  },
  {
    fen: 'r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4', sideToMove: 'w', depth: 22, nps: 1_400_000, evalCp: null, mate: 1, winPct: 100, thinking: false, ts: 0,
    candidates: [
      { ...cand(1, 'h5f7', 'Qxf7#', 'brilliant', 10000, ['mate', 'capture'], 'The queen takes f7 with support from the bishop on c4. Checkmate.'), mate: 1, scoreCp: null, winPct: 100 },
      cand(2, 'h5e5', 'Qxe5+', 'good', 120, ['check', 'capture']), cand(3, 'b1c3', 'Nc3', 'mistake', -90, [], null), cand(4, 'd2d3', 'd3', 'blunder', -400),
    ],
  },
  {
    fen: 'r1bq1rk1/ppp2ppp/2np1n2/2b1p3/2B1P3/2NP1N2/PPP2PPP/R1BQ1RK1 w - - 0 7', sideToMove: 'w', depth: 21, nps: 1_100_000, evalCp: 35, mate: null, winPct: 55, thinking: false, ts: 0,
    candidates: [
      cand(1, 'c1g5', 'Bg5', 'best', 35, ['pin'], 'Pins the knight on f6 to the queen and adds pressure on d6.'),
      cand(2, 'c3a4', 'Na4', 'great', 31, ['sacrifice']), cand(3, 'h2h3', 'h3', 'good', 22), cand(4, 'a2a3', 'a3', 'inaccuracy', 5), cand(5, 'c4f7', 'Bxf7+', 'blunder', -300, ['check', 'capture']),
    ],
  },
];

const REF_H = 720;
const frames = (): AnalysisFrame[] => (demoFrames.length ? demoFrames : FALLBACK);

// -------------------------------------------------------------- board paint
const BOARD_THEMES = {
  chessCom: { light: '#eeeed2', dark: '#769656' },
  lichess: { light: '#f0d9b5', dark: '#b58863' },
  generic: { light: '#dfe6ee', dark: '#8497b0' },
} as const;
const GLYPH: Record<string, string> = { k: '♚', q: '♛', r: '♜', b: '♝', n: '♞', p: '♟' };

function drawBoard(canvas: HTMLCanvasElement, w: number, h2: number, geo: BoardGeometry, fen: string, site: Settings['site'], k: number): void {
  const dpr = (window.devicePixelRatio || 1) * k;
  canvas.width = Math.round(w * dpr);
  canvas.height = Math.round(h2 * dpr);
  const ctx = canvas.getContext('2d');
  if (!ctx) return;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h2);
  const th = BOARD_THEMES[site];
  const sq = geo.size / 8;
  const rows = fen.split(' ')[0].split('/').map((r) => r.replace(/\d/g, (d) => '.'.repeat(Number(d))));

  ctx.save();
  ctx.shadowColor = 'rgba(0,0,0,0.35)';
  ctx.shadowBlur = 18;
  ctx.shadowOffsetY = 6;
  ctx.fillStyle = th.dark;
  ctx.fillRect(geo.x, geo.y, geo.size, geo.size);
  ctx.restore();

  ctx.font = `${sq * 0.8}px "Segoe UI Symbol","Apple Symbols","Noto Sans Symbols 2","DejaVu Sans",serif`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  for (let r = 0; r < 8; r++) {
    for (let f = 0; f < 8; f++) {
      const dr = geo.flipped ? 7 - r : r, df = geo.flipped ? 7 - f : f;
      const x = geo.x + df * sq, y = geo.y + dr * sq;
      ctx.fillStyle = (r + f) % 2 === 0 ? th.light : th.dark;
      ctx.fillRect(x, y, sq, sq);
      const ch = rows[r]?.[f];
      if (ch && ch !== '.') {
        const white = ch === ch.toUpperCase();
        const g = (GLYPH[ch.toLowerCase()] ?? '') + '︎';
        ctx.lineWidth = Math.max(1, sq * 0.05);
        ctx.strokeStyle = white ? '#2b2b2b' : '#f2f2f2';
        ctx.fillStyle = white ? '#ffffff' : '#1c1c1c';
        ctx.strokeText(g, x + sq / 2, y + sq * 0.54);
        ctx.fillText(g, x + sq / 2, y + sq * 0.54);
      }
    }
  }
  // coordinates
  ctx.font = `600 ${Math.max(8, sq * 0.16)}px system-ui, sans-serif`;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'top';
  for (let i = 0; i < 8; i++) {
    const dark = (i + 7) % 2 === 1;
    ctx.fillStyle = dark ? th.light : th.dark;
    const file = String.fromCharCode(97 + (geo.flipped ? 7 - i : i));
    ctx.textAlign = 'right';
    ctx.textBaseline = 'bottom';
    ctx.fillText(file, geo.x + (i + 1) * sq - sq * 0.06, geo.y + geo.size - sq * 0.04);
    const rank = String(geo.flipped ? i + 1 : 8 - i);
    ctx.textAlign = 'left';
    ctx.textBaseline = 'top';
    ctx.fillStyle = i % 2 === 0 ? th.dark : th.light;
    ctx.fillText(rank, geo.x + sq * 0.06, geo.y + i * sq + sq * 0.05);
  }
}

// ------------------------------------------------------------------ widget
function quick(label: string, path: string): HTMLElement {
  const sw = toggle(at<boolean>(path), { label: `Show ${label}` });
  return h('label', { class: 'quick' }, h('span', null, label), sw);
}

export function createPreview(): { el: HTMLElement; destroy(): void } {
  const boardCanvas = h('canvas', { class: 'pv-canvas', 'aria-hidden': 'true' });
  const overlayCanvas = h('canvas', { class: 'pv-canvas', role: 'img', 'aria-label': 'Live preview of the overlay on a sample chess board' });
  // The scene is laid out at a fixed logical height (like a real screen) and scaled down to fit the card,
  // so the overlay renderer shows HUD/labels at their true proportions.
  const scene = h('div', { class: 'pv-scene' }, boardCanvas, overlayCanvas);
  const stage = h('div', { class: 'pv-stage' }, scene);
  let k = 1;

  let idx = 0;
  let flipped = false;
  let playing = !(getSettings().theme.reduceMotion || matchMedia('(prefers-reduced-motion: reduce)').matches);
  let w = 0, hh = 0;
  let renderer: FishyRenderer | null = null;

  const caption = h('span', { class: 'pv-caption' });
  const dots = h('div', { class: 'pv-dots', role: 'tablist', 'aria-label': 'Sample positions' });
  const playBtn = h('button', { type: 'button', class: 'icon-only', onclick: () => { playing = !playing; paint(); } });

  const geometry = (): BoardGeometry => demoGeometry(w, hh, flipped);
  const currentFrame = (): AnalysisFrame => frames()[idx % frames().length];

  const redraw = () => {
    if (!w || !hh) return;
    const f = currentFrame();
    drawBoard(boardCanvas, w, hh, geometry(), f.fen, getSettings().site, k);
    if (!renderer) renderer = createRenderer(overlayCanvas, { settings: getSettings(), frame: f, geometry: geometry() });
    renderer.resize(w, hh);
    renderer.update({ settings: getSettings(), frame: f, geometry: geometry() });
  };

  const paint = () => {
    const n = frames().length;
    caption.textContent = `Sample ${(idx % n) + 1} of ${n}`;
    dots.replaceChildren(...frames().map((_, i) => h('button', {
      type: 'button', class: `pv-dot${i === idx % n ? ' on' : ''}`, role: 'tab', 'aria-selected': String(i === idx % n),
      'aria-label': `Sample position ${i + 1}`, onclick: () => { idx = i; paint(); redraw(); },
    })));
    playBtn.replaceChildren(icon(playing ? 'pause' : 'play', 14));
    playBtn.title = playBtn.ariaLabel = playing ? 'Pause cycling' : 'Play cycling';
    redraw();
  };

  const step = (d: number) => { const n = frames().length; idx = (idx + d + n) % n; paint(); };
  const timer = window.setInterval(() => { if (playing && !document.hidden) step(1); }, 5200);

  const ro = new ResizeObserver(() => {
    const r = stage.getBoundingClientRect();
    if (!r.width || !r.height) return;
    k = r.height / REF_H;
    const nw = Math.round(r.width / k);
    if (nw === w && hh === REF_H && scene.style.transform === `scale(${k})`) return;
    w = nw; hh = REF_H;
    scene.style.width = `${w}px`; scene.style.height = `${hh}px`;
    scene.style.transform = `scale(${k})`;
    redraw();
  });
  ro.observe(stage);
  const unsub = subscribeSettings(() => redraw());

  const siteName = h('span', { class: 'pv-site' });
  reg(() => { siteName.textContent = ({ chessCom: 'chess.com', lichess: 'lichess', generic: 'generic' } as const)[getSettings().site]; });

  const el = h('aside', { class: 'preview card', 'aria-label': 'Live preview' },
    h('header', { class: 'pv-head' },
      h('div', { class: 'pv-title' }, h('span', { class: 'live-dot', 'aria-hidden': 'true' }), h('h3', null, 'Live preview')),
      siteName),
    stage,
    h('div', { class: 'pv-bar' },
      h('button', { type: 'button', class: 'icon-only', title: 'Previous sample', 'aria-label': 'Previous sample', onclick: () => step(-1) }, icon('prev', 15)),
      playBtn,
      h('button', { type: 'button', class: 'icon-only', title: 'Next sample', 'aria-label': 'Next sample', onclick: () => step(1) }, icon('next', 15)),
      caption, dots,
      h('button', { type: 'button', class: 'icon-only', title: 'Flip board', 'aria-label': 'Flip board', onclick: () => { flipped = !flipped; redraw(); } }, icon('flip', 15))),
    h('div', { class: 'pv-quick' }, quick('Arrows', 'arrows.show'), quick('Labels', 'labels.show'), quick('Eval bar', 'evalBar.show'), quick('HUD', 'hud.show')),
  );
  paint();
  return { el, destroy() { window.clearInterval(timer); ro.disconnect(); unsub(); renderer?.destroy(); } };
}
