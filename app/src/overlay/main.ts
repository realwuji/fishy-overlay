// Overlay window entry. A full-window transparent canvas driven by the renderer.
// Inside Tauri it listens to the Rust events; in a plain browser (or with ?demo)
// it cycles through the demo frames over a fake board so it can be previewed.
import './overlay.css';
import { DEFAULT_SETTINGS } from '../shared/defaults';
import {
  DEMO_STATUSES, DEMO_TARGET_DEPTH, demoBoardState, demoFrames, demoGeometry, demoPendingFrame, demoSessionStatus, type DemoStatus,
} from '../shared/demo';
import { call, inTauri, on } from '../shared/ipc';
import { EVT } from '../shared/types';
import type { AnalysisFrame, ArrowStyle, BoardGeometry, BoardState, Corner, MoveClass, SessionStatus, Settings } from '../shared/types';
import { drawDemoBoard } from './demoBoard';
import { NO_VIEW } from './gate';
import { createOverlayRenderer, type GateView, type Rect } from './renderer';
import { StatusChip, chipText } from './statusChip';

const params = new URLSearchParams(location.search);
const demo = params.has('demo') || !inTauri();

const canvas = document.getElementById('stage') as HTMLCanvasElement;
const ui = document.getElementById('ui') as HTMLElement;

let settings: Settings = structuredClone(DEFAULT_SETTINGS);
let frame: AnalysisFrame | null = null;
let geometry: BoardGeometry | null = null;
let boardState: BoardState | null = null;
let status: SessionStatus | null = null;
let view: GateView = NO_VIEW;

const renderer = createOverlayRenderer(canvas, { settings, frame, geometry });
const chip = new StatusChip(ui);
let hudRect: Rect | null = null;

/** Re-evaluate the status chip from the latest board state, frame and geometry. */
function syncChip(): void {
  chip.update(chipText({ status, board: boardState, frame, geometry, view }), geometry, settings, hudRect);
}

function resize(): void {
  const w = window.innerWidth;
  const h = window.innerHeight;
  renderer.resize(w, h);
  if (demo) refreshDemo();
}
window.addEventListener('resize', resize);

// ------------------------------------------------------------ HUD edit mode
// The drag handle only receives events while the window is not click-through
// (the control window toggles that), so this is inert in normal use.
const hit = document.createElement('div');
hit.id = 'hud-hit';
ui.appendChild(hit);

renderer.onView((v) => {
  view = v;
  syncChip();
});

renderer.onLayout((r: Rect | null) => {
  hudRect = r ? { ...r } : null;
  syncChip();
  if (!r) {
    hit.style.display = 'none';
    return;
  }
  hit.style.display = 'block';
  hit.style.left = `${r.x}px`;
  hit.style.top = `${r.y}px`;
  hit.style.width = `${r.w}px`;
  hit.style.height = `${r.h}px`;
});

let dragging = false;
let grabX = 0;
let grabY = 0;

hit.addEventListener('pointerenter', () => renderer.hudHover(true));
hit.addEventListener('pointerleave', () => {
  if (!dragging) renderer.hudHover(false);
});
hit.addEventListener('pointerdown', (e) => {
  const b = renderer.hudBounds();
  if (!b) return;
  e.preventDefault();
  try {
    hit.setPointerCapture(e.pointerId);
  } catch {
    /* synthetic or already-released pointer: dragging still works while over the handle */
  }
  dragging = true;
  grabX = e.clientX - b.x;
  grabY = e.clientY - b.y;
  hit.classList.add('dragging');
  renderer.hudDragStart();
});
hit.addEventListener('pointermove', (e) => {
  if (dragging) renderer.hudDragMove(e.clientX - grabX, e.clientY - grabY);
});
const endDrag = (commit: boolean): void => {
  if (!dragging) return;
  dragging = false;
  hit.classList.remove('dragging');
  const corner = renderer.hudDragEnd();
  if (commit) setHudCorner(corner);
  renderer.hudHover(false);
};
hit.addEventListener('pointerup', () => endDrag(true));
hit.addEventListener('pointercancel', () => endDrag(false));
window.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') endDrag(false);
});

function setHudCorner(corner: Corner): void {
  if (settings.hud.corner === corner) {
    renderer.update({ settings: { ...settings } });
    return;
  }
  settings = { ...settings, hud: { ...settings.hud, corner } };
  renderer.update({ settings });
  if (inTauri()) {
    call('set_settings', { settings })
      .then((saved) => {
        settings = saved;
        renderer.update({ settings });
      })
      .catch((err) => console.error('set_settings failed', err));
  }
}

// ------------------------------------------------------------- peek mode
// Analysis and drawing keep running; peek only fades the whole overlay in and out.
const dot = document.createElement('div');
dot.id = 'peek-dot';
document.body.appendChild(dot);
let revealed = false;

function applyPeek(): void {
  const hidden = settings.peek.mode !== 'off' && !revealed;
  const ms = settings.theme.reduceMotion ? 0 : settings.peek.fadeMs;
  document.documentElement.style.setProperty('--peek-ms', `${ms}ms`);
  document.body.classList.toggle('peek-hidden', hidden);
  dot.classList.toggle('on', hidden && settings.peek.indicator);
  const w = window.innerWidth, hgt = window.innerHeight;
  const x = geometry ? geometry.x + geometry.size + 10 : w - 20;
  const y = geometry ? geometry.y + geometry.size - 14 : hgt - 20;
  dot.style.left = `${Math.max(4, Math.min(w - 14, x))}px`;
  dot.style.top = `${Math.max(4, Math.min(hgt - 14, y))}px`;
}

// -------------------------------------------------------------- live mode
async function startLive(): Promise<void> {
  try {
    settings = await call('get_settings');
    renderer.update({ settings });
    revealed = await call('get_peek').catch(() => false);
    applyPeek();
  } catch (err) {
    console.error('get_settings failed', err);
  }
  await on<AnalysisFrame>(EVT.analysis, (f) => {
    frame = f;
    renderer.update({ frame });
    syncChip();
  });
  await on<BoardState>(EVT.board, (b) => {
    boardState = b;
    geometry = b.geometry;
    renderer.update({ geometry });
    applyPeek();
    syncChip();
  });
  await on<SessionStatus>(EVT.status, (s) => {
    status = s;
    syncChip();
  });
  try {
    status = await call('get_status');
    syncChip();
  } catch (err) {
    console.error('get_status failed', err);
  }
  await on<{ revealed: boolean }>(EVT.peek, (p) => {
    revealed = p.revealed;
    applyPeek();
  });
  await on<Settings>(EVT.settings, (s) => {
    settings = s;
    renderer.update({ settings });
    applyPeek();
    syncChip();
  });
  // The overlay window moved or resized on the desktop: re-sync the canvas size.
  await on<unknown>(EVT.overlayGeometry, () => resize());
}

// -------------------------------------------------------------- demo mode
let demoCanvas: HTMLCanvasElement | null = null;
let demoIndex = Math.max(0, Math.min(demoFrames.length - 1, Number(params.get('frame') ?? 0) || 0));
let demoFlip: boolean | null = params.has('flip') ? params.get('flip') !== '0' : null;
let demoStatus: DemoStatus = DEMO_STATUSES.includes(params.get('status') as DemoStatus) ? (params.get('status') as DemoStatus) : 'none';
let demoPaused = params.has('hold') || demoStatus !== 'none';
// Elapsed ms in the looping ?status=loading sequence.
let loadStart = performance.now();
let loadTimer = 0;
let demoTimer = 0;

const STYLES: ArrowStyle[] = ['classic', 'slim', 'dotted', 'pill'];
const PLACEMENTS = ['cornerBadge', 'arrowTail', 'arrowHead'] as const;
const LABEL_STYLES = ['icon', 'text', 'iconText'] as const;

function applyDemoParams(): void {
  const s = structuredClone(settings);
  const p = params;
  const style = p.get('style') as ArrowStyle | null;
  if (style && STYLES.includes(style)) s.arrows.style = style;
  if (p.has('count')) s.arrows.count = Math.max(1, Math.min(5, Number(p.get('count')) || 3));
  if (p.get('color') === 'class') s.arrows.colorMode = 'classification';
  if (p.has('place') && PLACEMENTS.includes(p.get('place') as (typeof PLACEMENTS)[number])) s.labels.placement = p.get('place') as (typeof PLACEMENTS)[number];
  if (p.has('labels') && LABEL_STYLES.includes(p.get('labels') as (typeof LABEL_STYLES)[number])) s.labels.style = p.get('labels') as (typeof LABEL_STYLES)[number];
  if (p.has('size')) s.labels.size = Number(p.get('size')) || 1;
  if (p.has('hud')) s.hud.corner = p.get('hud') as Corner;
  if (p.has('lines')) s.hud.lines = Number(p.get('lines')) || 3;
  if (p.has('compact')) s.hud.compact = p.get('compact') !== '0';
  if (p.has('bar')) s.evalBar.position = p.get('bar') === 'right' ? 'right' : 'left';
  if (p.has('glow')) s.arrows.glow = p.get('glow') !== '0';
  if (p.has('knight')) s.arrows.curveKnightMoves = p.get('knight') !== '0';
  if (p.has('width')) s.arrows.widthScale = Number(p.get('width')) || 1;
  if (p.has('min')) s.arrows.minClass = p.get('min') as MoveClass;
  if (p.get('theme') === 'light' || p.get('theme') === 'dark') s.theme.mode = p.get('theme') as 'light' | 'dark';
  if (p.has('still')) s.theme.reduceMotion = true;
  if (p.get('peek') === 'hold' || p.get('peek') === 'toggle') { s.peek.mode = p.get('peek') as 'hold' | 'toggle'; s.peek.indicator = true; }
  settings = s;
}

function currentFlip(): boolean {
  if (demoFlip !== null) return demoFlip;
  return demoFrames[demoIndex].sideToMove === 'b';
}

function refreshDemo(): void {
  const w = window.innerWidth;
  const h = window.innerHeight;
  const f = demoFrames[demoIndex];
  const g = demoGeometry(w, h, currentFlip());
  // Preview-only overrides to exercise tight layouts: ?by=<board top px>&bs=<board size px>.
  if (params.has('bs')) g.size = Math.max(80, Math.min(Number(params.get('bs')) || g.size, w, h));
  if (params.has('by')) g.y = Number(params.get('by')) || 0;
  if (params.has('bs') || params.has('by')) g.x = (w - g.size) / 2;
  if (demoCanvas) drawDemoBoard(demoCanvas, w, h, g, f.fen);
  const scene = demoScene(f, g);
  frame = scene.frame;
  geometry = scene.geometry;
  boardState = scene.board;
  status = scene.status;
  renderer.update({ frame, geometry, settings });
  syncChip();
}

interface DemoScene { frame: AnalysisFrame | null; geometry: BoardGeometry | null; board: BoardState | null; status: SessionStatus | null }

/** Loading sequence timeline (ms): engine, board, pieces, then the search climbs to the target depth. */
const SEQ = { engine: 1500, board: 1500, pieces: 1500, step: 300 };

function loadingScene(f: AnalysisFrame, g: BoardGeometry): DemoScene {
  const t = performance.now() - loadStart;
  const t1 = SEQ.engine, t2 = t1 + SEQ.board, t3 = t2 + SEQ.pieces;
  const tSearch = DEMO_TARGET_DEPTH * SEQ.step;
  const t4 = t3 + tSearch, t5 = t4 + 3500;
  if (t >= t5) { loadStart = performance.now(); return loadingScene(f, g); }
  if (t < t1) return { frame: null, geometry: null, board: null, status: demoSessionStatus('startingEngine') };
  if (t < t2) return { frame: null, geometry: null, board: null, status: demoSessionStatus('findingBoard') };
  if (t < t3) return { frame: null, geometry: g, board: demoBoardState('recognising', g, null), status: demoSessionStatus('recognising') };
  const board = demoBoardState('none', g, f.fen);
  if (t < t4) {
    const depth = 1 + Math.floor((t - t3) / SEQ.step);
    return { frame: demoPendingFrame(f, depth), geometry: g, board, status: demoSessionStatus('analysing', f.nps) };
  }
  return { frame: f, geometry: g, board, status: demoSessionStatus('ready', f.nps) };
}

function demoScene(f: AnalysisFrame, g: BoardGeometry): DemoScene {
  switch (demoStatus) {
    case 'none': return { frame: f, geometry: g, board: null, status: null };
    case 'loading': return loadingScene(f, g);
    case 'starting': return { frame: null, geometry: null, board: null, status: demoSessionStatus('startingEngine') };
    case 'finding': return { frame: null, geometry: null, board: null, status: demoSessionStatus('findingBoard') };
    case 'recognising': return { frame: null, geometry: g, board: demoBoardState('recognising', g, f.fen), status: demoSessionStatus('recognising') };
    case 'analysing': {
      // Hold at depth 7 of the target: after ~2.5 s the gate flips to "preliminary", like a slow search would.
      return { frame: demoPendingFrame(f, 7), geometry: g, board: demoBoardState('none', g, f.fen), status: demoSessionStatus('analysing', f.nps) };
    }
    default: return { frame: null, geometry: g, board: demoBoardState(demoStatus, g, f.fen), status: null };
  }
}

/** Drive the looping loading sequence (only while ?status=loading is selected). */
function syncLoadTimer(): void {
  window.clearInterval(loadTimer);
  loadTimer = 0;
  if (demoStatus !== 'loading') return;
  loadStart = performance.now();
  loadTimer = window.setInterval(refreshDemo, 100);
}

function startDemo(): void {
  document.body.classList.add('demo');
  demoCanvas = document.createElement('canvas');
  demoCanvas.id = 'demo-board';
  document.body.insertBefore(demoCanvas, canvas);
  const hint = document.createElement('div');
  hint.id = 'demo-hint';
  hint.textContent = 'demo  ← → position   F flip   S arrow style   L labels   M placement   P pause   C status / loading';
  document.body.appendChild(hint);

  applyDemoParams();
  applyPeek();
  window.addEventListener('keydown', (e) => { if ((e.key === 'x' || e.key === 'X') && !e.repeat) { revealed = settings.peek.mode === 'toggle' ? !revealed : true; applyPeek(); } });
  window.addEventListener('keyup', (e) => { if ((e.key === 'x' || e.key === 'X') && settings.peek.mode === 'hold') { revealed = false; applyPeek(); } });
  refreshDemo();
  syncLoadTimer();

  const cycle = (): void => {
    if (demoPaused) return;
    demoIndex = (demoIndex + 1) % demoFrames.length;
    refreshDemo();
  };
  const restart = (): void => {
    window.clearInterval(demoTimer);
    demoTimer = window.setInterval(cycle, 5200);
  };
  restart();

  const nextIn = <T,>(list: readonly T[], cur: T): T => list[(list.indexOf(cur) + 1) % list.length];
  window.addEventListener('keydown', (e) => {
    switch (e.key) {
      case 'ArrowRight': demoIndex = (demoIndex + 1) % demoFrames.length; break;
      case 'ArrowLeft': demoIndex = (demoIndex + demoFrames.length - 1) % demoFrames.length; break;
      case 'f': case 'F': demoFlip = !currentFlip(); break;
      case 's': case 'S': settings = { ...settings, arrows: { ...settings.arrows, style: nextIn(STYLES, settings.arrows.style) } }; break;
      case 'l': case 'L': settings = { ...settings, labels: { ...settings.labels, style: nextIn(LABEL_STYLES, settings.labels.style) } }; break;
      case 'm': case 'M': settings = { ...settings, labels: { ...settings.labels, placement: nextIn(PLACEMENTS, settings.labels.placement) } }; break;
      case 'c': case 'C': demoStatus = nextIn(DEMO_STATUSES, demoStatus); demoPaused = demoStatus !== 'none'; syncLoadTimer(); break;
      case 'p': case 'P': demoPaused = !demoPaused; return;
      default: return;
    }
    restart();
    refreshDemo();
  });
}

// ------------------------------------------------------------------ boot
resize();
if (demo) {
  startDemo();
  // Keep any HUD-corner drag made in the demo.
} else {
  void startLive();
}
