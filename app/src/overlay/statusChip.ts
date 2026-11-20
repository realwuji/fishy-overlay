// Small DOM chip anchored to the board that explains why no arrows are showing.
// Pure presentation: never receives pointer events, respects reduced motion.
import type { AnalysisFrame, BoardGeometry, BoardState, SessionStatus, Settings } from '../shared/types';
import type { GateView } from './gate';

const COMPACT_H = 22;

export interface Rect { x: number; y: number; w: number; h: number }

export interface ChipText {
  title: string;
  sub?: string;
  tone: 'info' | 'warn';
  /** Something is loading: spinner (static dots when motion is reduced). */
  busy?: boolean;
  /** 0..1 fills a slim progress bar under the text. Absent = no bar. */
  progress?: number;
  /** Shown even without a board (before it was found). */
  anchorless?: boolean;
}

export interface ChipInput {
  status: SessionStatus | null;
  board: BoardState | null;
  frame: AnalysisFrame | null;
  geometry: BoardGeometry | null;
  view: GateView;
}

/** What the chip should say, or null when everything is fine (analysis flowing). */
export function chipText(c: ChipInput): ChipText | null {
  const { status, board: b, frame, geometry, view } = c;
  switch (status?.phase) {
    case 'startingEngine':
      return { title: 'Starting engine\u2026', sub: 'Warming up Stockfish for this computer.', tone: 'info', busy: true, anchorless: true };
    case 'findingBoard':
      return { title: 'Looking for the board\u2026', sub: 'Keep the whole board visible on the captured screen.', tone: 'info', busy: true, anchorless: true };
    case 'recognising':
      return { title: 'Reading the pieces\u2026', sub: 'Working out the position from the screen.', tone: 'info', busy: true, anchorless: !geometry };
    default:
  }
  if (!geometry) return null;
  // No board event yet: judge by the analysis frame alone.
  const state = b?.state ?? 'tracking';
  if (state === 'needsCalibration') {
    if (b && b.calibrated) return { title: 'Fishy is looking at the board', sub: 'Recognising pieces\u2026', tone: 'info', busy: true };
    return {
      title: 'Reading the board\u2026',
      sub: 'Not picking up the pieces? Show the starting position once, then press Calibrate (Fishy \u203a Overview).',
      tone: 'info',
    };
  }
  if (state === 'lost') return { title: 'Lost track of the game', sub: 'Hold still for a moment, or re-calibrate.', tone: 'warn' };
  const frac = view.target > 0 ? Math.max(0, Math.min(1, view.depth / view.target)) : undefined;
  if (view.state === 'pending') {
    return {
      title: view.target > 0 ? `Analysing ${view.depth}/${view.target}` : 'Analysing\u2026',
      sub: 'Moves appear once the search is deep enough.', tone: 'info', busy: true, progress: frac,
    };
  }
  if (view.state === 'preliminary') {
    return {
      title: `Preliminary \u00b7 depth ${view.depth}/${view.target}`,
      sub: 'Still searching deeper. These moves may change.', tone: 'info', busy: true, progress: frac,
    };
  }
  if (!frame) return { title: status?.phase === 'analysing' ? 'Analysing\u2026' : 'Thinking\u2026', tone: 'info', busy: true };
  return null;
}

export class StatusChip {
  private el: HTMLDivElement;
  private dot: HTMLSpanElement;
  private title: HTMLDivElement;
  private sub: HTMLDivElement;
  private bar: HTMLDivElement;
  private barFill: HTMLElement;
  private shown = false;
  private key = '';

  constructor(parent: HTMLElement) {
    this.el = document.createElement('div');
    this.el.id = 'status-chip';
    this.el.setAttribute('role', 'status');
    this.el.setAttribute('aria-live', 'polite');
    this.dot = document.createElement('span');
    this.dot.className = 'sc-dot';
    const spin = document.createElement('span');
    spin.className = 'sc-spin';
    const dots = document.createElement('span');
    dots.className = 'sc-dots';
    dots.append(document.createElement('i'), document.createElement('i'), document.createElement('i'));
    const body = document.createElement('div');
    body.className = 'sc-body';
    this.title = document.createElement('div');
    this.title.className = 'sc-title';
    this.sub = document.createElement('div');
    this.sub.className = 'sc-sub';
    this.bar = document.createElement('div');
    this.bar.className = 'sc-bar';
    this.barFill = document.createElement('i');
    this.bar.appendChild(this.barFill);
    body.append(this.title, this.sub, this.bar);
    this.el.append(this.dot, spin, dots, body);
    parent.appendChild(this.el);
  }

  update(text: ChipText | null, geometry: BoardGeometry | null, settings: Settings, hud: Rect | null = null): void {
    const el = this.el;
    const showable = !!text && (!!geometry || !!text.anchorless);
    // Skip all DOM and layout work when nothing the chip depends on changed (zero idle cost).
    const key = showable
      ? [text!.title, text!.sub, text!.tone, text!.busy, text!.progress === undefined ? '' : Math.round(text!.progress * 100),
        geometry ? `${geometry.x}|${geometry.y}|${geometry.size}` : '-', hud ? `${hud.x}|${hud.y}|${hud.w}|${hud.h}` : '-',
        settings.theme.reduceMotion, settings.theme.accent, window.innerWidth, window.innerHeight].join('~')
      : 'hidden';
    if (key === this.key) return;
    this.key = key;
    el.classList.toggle('still', settings.theme.reduceMotion);
    el.style.setProperty('--sc-accent', settings.theme.accent);
    if (!text || !showable) {
      this.shown = false;
      el.classList.remove('show');
      return;
    }
    this.title.textContent = text.title;
    this.sub.textContent = text.sub ?? '';
    this.sub.style.display = text.sub ? '' : 'none';
    el.classList.toggle('warn', text.tone === 'warn');
    el.classList.toggle('busy', !!text.busy);
    const hasBar = text.progress !== undefined;
    this.bar.style.display = hasBar ? '' : 'none';
    this.barFill.style.transform = `scaleX(${hasBar ? Math.max(0.03, text.progress as number) : 0})`;
    const size = geometry ? geometry.size : Math.min(window.innerWidth, window.innerHeight) * 0.8;
    el.style.maxWidth = `${Math.max(160, Math.min(size - 16, 440))}px`;

    if (!geometry) {
      // No board yet: float near the top centre of the window.
      el.classList.remove('compact');
      const W0 = window.innerWidth;
      const w0 = el.offsetWidth || 220;
      el.style.left = `${Math.round(Math.max(6, (W0 - w0) / 2))}px`;
      el.style.top = `${Math.round(Math.max(24, window.innerHeight * 0.1))}px`;
      this.shown = true;
      el.classList.add('show');
      return;
    }

    const W = window.innerWidth;
    const H = window.innerHeight;
    const boardTop = geometry.y;
    const boardBottom = geometry.y + geometry.size;
    const cx = geometry.x + geometry.size / 2;
    const GAP = 8;
    const EDGE = 6;

    const measure = (compact: boolean): { w: number; h: number } => {
      el.classList.toggle('compact', compact);
      return { w: el.offsetWidth || 200, h: el.offsetHeight || (compact ? COMPACT_H : 40) };
    };
    const full = measure(false);
    const compact = measure(true);

    // Candidate slots in order of preference. Only the last one may touch the board,
    // and then only its top border strip (22px max, translucent).
    interface Slot { compact: boolean; w: number; h: number; top: number }
    const slots: Slot[] = [
      { compact: false, ...full, top: boardTop - full.h - GAP },
      { compact: false, ...full, top: boardBottom + GAP },
      { compact: true, ...compact, top: boardTop - compact.h - 4 },
      { compact: true, ...compact, top: boardBottom + 4 },
    ];
    const fits = (s: Slot): boolean => s.top >= EDGE && s.top + s.h <= H - EDGE;
    const pad = 6;
    const hitsHud = (x: number, s: Slot): boolean =>
      !!hud && x < hud.x + hud.w + pad && x + s.w > hud.x - pad && s.top < hud.y + hud.h + pad && s.top + s.h > hud.y - pad;
    /** Left edge for the slot: centred over the board, clamped to the window, nudged clear of the HUD. */
    const place = (s: Slot): number | null => {
      const clamp = (x: number): number => Math.max(EDGE, Math.min(W - s.w - EDGE, x));
      const x0 = clamp(cx - s.w / 2);
      if (!hitsHud(x0, s)) return x0;
      if (!hud) return x0;
      for (const x of [clamp(hud.x - pad - s.w), clamp(hud.x + hud.w + pad)]) if (!hitsHud(x, s)) return x;
      return null;
    };

    let chosen: Slot | null = null;
    let left = 0;
    for (const s of slots) {
      if (!fits(s)) continue;
      const x = place(s);
      if (x === null) continue;
      chosen = s;
      left = x;
      break;
    }
    if (!chosen) {
      // No clear room around the board: a compact strip on the window's top edge, centred over the
      // board, overlapping only its top border area (never lower than needed to stay in the window).
      const top = Math.max(0, Math.min(boardTop - compact.h + 6, H - compact.h));
      const s: Slot = { compact: true, ...compact, top };
      chosen = s;
      const x = place(s);
      left = x ?? Math.max(EDGE, Math.min(W - s.w - EDGE, cx - s.w / 2));
      if (x === null && hud) {
        // Still blocked by the HUD: slide the strip below it if that stays near the board top.
        s.top = Math.min(H - compact.h, hud.y + hud.h + pad);
      }
    }
    measure(chosen.compact);
    el.style.left = `${Math.round(left)}px`;
    el.style.top = `${Math.round(chosen.top)}px`;
    this.shown = true;
    el.classList.add('show');
  }

  get visible(): boolean { return this.shown; }
}
