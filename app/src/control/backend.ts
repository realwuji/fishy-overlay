// Backend facade: typed Tauri commands/events with in-browser mocks so the whole UI is previewable.
import { call, inTauri, on } from '../shared/ipc';
import { demoSessionStatus } from '../shared/demo';
import { EVT, type AnalysisFrame, type BoardState, type CaptureTarget, type SessionPhase, type SessionStatus } from '../shared/types';
import { getSettings } from './store';
import { createStore } from './store';
import { toast } from './ui/toast';

export const statusStore = createStore<SessionStatus>({ running: false, permission: true, captureFps: 0, message: null });
export const boardStore = createStore<BoardState>({ geometry: null, fen: null, lastMove: null, state: 'needsCalibration', confidence: 0, calibrated: false });
export const targetsStore = createStore<CaptureTarget[]>([]);
/** Latest search progress (throttled), for the overview stepper. */
export interface SearchInfo { depth: number; targetDepth: number | null; nps: number; thinking: boolean }
export const analysisStore = createStore<SearchInfo | null>(null);
/** True from the Start click until the backend answered (engine spawning). */
export const startingStore = createStore<boolean>(false);
/** True while the capture target list is being fetched. */
export const targetsLoadingStore = createStore<boolean>(true);
export const overlayStore = createStore<{ visible: boolean; editable: boolean }>({ visible: true, editable: false });

const MOCK_TARGETS: CaptureTarget[] = [
  { id: 'monitor:1', kind: 'monitor', title: 'Built-in Retina Display', app: 'Display 1', x: 0, y: 0, width: 1512, height: 982, scale: 2 },
  { id: 'monitor:2', kind: 'monitor', title: 'LG UltraFine 27"', app: 'Display 2', x: 1512, y: 0, width: 2560, height: 1440, scale: 1 },
  { id: 'window:101', kind: 'window', title: 'Play Chess Online - Chess.com', app: 'Google Chrome', x: 120, y: 60, width: 1280, height: 840, scale: 2 },
  { id: 'window:102', kind: 'window', title: 'Daily Puzzle - Chess.com', app: 'Safari', x: 200, y: 90, width: 1180, height: 800, scale: 2 },
  { id: 'window:103', kind: 'window', title: 'Analysis Board', app: 'Microsoft Edge', x: 260, y: 120, width: 1100, height: 760, scale: 1 },
];

/** Phase from the shell, or derived from older payloads that do not carry one. */
export function currentPhase(): SessionPhase {
  const s = statusStore.get();
  if (!s.running) return 'idle';
  if (s.phase) return s.phase;
  const b = boardStore.get(), a = analysisStore.get();
  if (!b.geometry) return 'findingBoard';
  if (b.state !== 'tracking') return 'recognising';
  if (!a) return 'analysing';
  return a.targetDepth && a.thinking && a.depth < a.targetDepth ? 'analysing' : 'ready';
}

function fail(err: unknown): void {
  toast(typeof err === 'string' ? err : err instanceof Error ? err.message : 'Something went wrong.', { kind: 'error' });
}

export async function initBackend(): Promise<void> {
  if (inTauri()) {
    try {
      statusStore.set(await call('get_status'));
    } catch (err) { fail(err); }
    await on<SessionStatus>(EVT.status, (s) => { statusStore.set(s); if (!s.running) analysisStore.set(null); });
    await on<BoardState>(EVT.board, (b) => boardStore.set(b));
    await on<AnalysisFrame>(EVT.analysis, onAnalysis);
    void refreshTargets(true); // list loads in the background; the UI shows a skeleton meanwhile
  } else {
    statusStore.set({ running: false, permission: false, captureFps: 0, message: 'Preview mode: backend is mocked.', phase: 'idle' });
    void refreshTargets(true); // list loads in the background; the UI shows a skeleton meanwhile
  }
}

// Analysis frames arrive ~16 times a second; the control window only needs a calm summary.
let lastInfo: SearchInfo | null = null;
let lastEmit = 0;
let pendingInfo: SearchInfo | null = null;
let infoTimer = 0;
function emitInfo(i: SearchInfo): void {
  lastInfo = i;
  lastEmit = performance.now();
  pendingInfo = null;
  analysisStore.set(i);
}
function onAnalysis(f: AnalysisFrame): void {
  const i: SearchInfo = { depth: f.depth, targetDepth: f.targetDepth ?? null, nps: f.nps, thinking: f.thinking };
  const same = lastInfo && lastInfo.depth === i.depth && lastInfo.targetDepth === i.targetDepth && lastInfo.thinking === i.thinking
    && Math.abs(lastInfo.nps - i.nps) < Math.max(20_000, lastInfo.nps * 0.1);
  if (same) return;
  const wait = 250 - (performance.now() - lastEmit);
  if (wait <= 0) { emitInfo(i); return; }
  pendingInfo = i;
  if (!infoTimer) infoTimer = window.setTimeout(() => { infoTimer = 0; if (pendingInfo) emitInfo(pendingInfo); }, wait);
}

export async function refreshTargets(silent = false): Promise<void> {
  targetsLoadingStore.set(true);
  try {
    if (inTauri()) targetsStore.set(await call('list_capture_targets'));
    else {
      await new Promise((r) => setTimeout(r, silent ? 900 : 700)); // mock: show the loading skeleton
      targetsStore.set(MOCK_TARGETS.map((t) => ({ ...t })));
    }
    if (!silent) toast(`Found ${targetsStore.get().length} capture targets.`);
  } catch (err) { fail(err); }
  finally { targetsLoadingStore.set(false); }
}

// ------------------------------------------------ browser-preview mock session
// idle -> startingEngine -> findingBoard -> recognising -> analysing (depth 1..12) -> ready
let mockTimers: number[] = [];
const clearMock = (): void => { mockTimers.forEach((t) => clearTimeout(t)); mockTimers = []; };
const later = (ms: number, fn: () => void): void => { mockTimers.push(window.setTimeout(fn, ms)); };
const MOCK_FEN = 'rnbqkbnr/ppp2ppp/4p3/3p4/3P4/4P3/PPP2PPP/RNBQKBNR w KQkq - 0 4';
const MOCK_TARGET = 12;

function mockStatus(phase: SessionPhase, nps = 0): void {
  const base = demoSessionStatus(phase, nps);
  statusStore.set({ ...base, permission: statusStore.get().permission, captureFps: getSettings().capture.fps, message: null });
}

function runMockSession(): void {
  clearMock();
  analysisStore.set(null);
  boardStore.set({ geometry: null, fen: null, lastMove: null, state: 'needsCalibration', confidence: 0, calibrated: false });
  mockStatus('startingEngine');
  later(1300, () => mockStatus('findingBoard'));
  later(2500, () => {
    mockStatus('recognising');
    boardStore.set({ geometry: { x: 360, y: 120, size: 640, flipped: false }, fen: null, lastMove: null, state: 'needsCalibration', confidence: 0.4, calibrated: true });
  });
  later(3700, () => {
    mockStatus('analysing');
    boardStore.set({ ...boardStore.get(), fen: MOCK_FEN, state: 'tracking', confidence: 0.94 });
  });
  for (let d = 1; d <= MOCK_TARGET; d++) {
    later(3800 + d * 260, () => {
      const nps = 400_000 + d * 130_000;
      analysisStore.set({ depth: d, targetDepth: MOCK_TARGET, nps, thinking: true });
      if (d < MOCK_TARGET) mockStatus('analysing', nps);
    });
  }
  later(3800 + (MOCK_TARGET + 1) * 260, () => {
    analysisStore.set({ depth: 22, targetDepth: MOCK_TARGET, nps: 1_840_000, thinking: true });
    mockStatus('ready', 1_840_000);
  });
}

export async function startSession(): Promise<void> {
  if (startingStore.get()) return;
  startingStore.set(true);
  try {
    if (inTauri()) {
      const s = await call('start_session');
      statusStore.set(s);
      // The shell answered but did not start (for example a permission or engine error): say why.
      if (!s.running && s.message) toast(s.message, { kind: 'warn' });
    } else runMockSession();
  } catch (err) {
    fail(err);
  } finally {
    startingStore.set(false);
  }
}

export async function stopSession(): Promise<void> {
  try {
    if (inTauri()) statusStore.set(await call('stop_session'));
    else {
      clearMock();
      analysisStore.set(null);
      statusStore.set({ ...statusStore.get(), running: false, captureFps: 0, message: 'Stopped.', phase: 'idle', engine: null });
    }
  } catch (err) { fail(err); }
}

const mockBoard = (fen: string | null): BoardState => ({
  geometry: { x: 360, y: 120, size: 640, flipped: false }, fen, lastMove: null, state: 'tracking', confidence: 0.94, calibrated: true,
});

export const NOT_START_HINT = 'This is not the starting position. Paste the current position (FEN) below and press Calibrate from this position.';
/** Dev-only: this FEN makes the mock fail, so the error copy can be seen in browser preview. */
export const MOCK_FAIL_FEN = '8/8/8/8/8/8/8/K6k w - - 0 1';

export async function calibrate(): Promise<void> {
  try {
    const b = inTauri() ? await call('calibrate_start_position') : mockBoard('rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1');
    boardStore.set(b);
    toast(b.state === 'tracking' ? 'Board calibrated. Tracking moves.' : 'Calibration finished, but the board was not found. Check it is fully visible.', { kind: b.state === 'tracking' ? 'ok' : 'warn' });
  } catch (err) {
    const m = typeof err === 'string' ? err : err instanceof Error ? err.message : '';
    if (m.includes('start position not recognised')) {
      toast(NOT_START_HINT, { kind: 'warn' });
      const fen = document.getElementById('fen-input') as HTMLInputElement | null;
      fen?.focus();
    } else fail(err);
  }
}

/** Learn the piece set from the screen plus the position the user states, then track from it. */
export async function calibrateFromPosition(fen: string): Promise<void> {
  try {
    const known = boardStore.get().calibrated; // read before the call
    let b: BoardState;
    if (inTauri()) b = await call('calibrate_from_position', { fen });
    else {
      if (fen.split(/\s+/)[0] === MOCK_FAIL_FEN.split(' ')[0]) {
        throw 'The picture does not match this position (3/2 pieces seen). Check the FEN, that the board is fully visible, and that nothing covers the pieces (arrows, popups).';
      }
      b = mockBoard(fen);
    }
    boardStore.set(b);
    const note = statusStore.get().message;
    if (note && note.startsWith('Pieces learned. Not seen yet')) toast(note, { kind: 'warn' });
    else toast(known ? 'Position set. Tracking moves.' : 'Pieces learned from this position. Tracking moves.', { kind: 'ok' });
  } catch (err) { fail(err); }
}

/** Forget the saved calibration for this site and use the built-in pieces again. */
export async function resetPieces(): Promise<void> {
  try {
    const b = inTauri() ? await call('reset_pieces') : mockBoard(boardStore.get().fen);
    boardStore.set(b);
    toast('Back to the built-in pieces.', { kind: 'ok' });
  } catch (err) { fail(err); }
}

export async function redetect(): Promise<void> {
  try {
    const b = inTauri() ? await call('recalibrate_board') : mockBoard(boardStore.get().fen);
    boardStore.set(b);
    toast(b.geometry ? 'Board re-detected.' : 'No board found. Make sure it is fully on screen.', { kind: b.geometry ? 'ok' : 'warn' });
  } catch (err) { fail(err); }
}

export async function resumeFromFen(fen: string): Promise<void> {
  try {
    const b = inTauri() ? await call('set_known_position', { fen }) : mockBoard(fen);
    boardStore.set(b);
    toast('Position set. Tracking resumed from your FEN.', { kind: 'ok' });
  } catch (err) { fail(err); }
}

export async function requestPermission(): Promise<void> {
  try {
    const ok = inTauri() ? await call('request_screen_permission') : true;
    statusStore.set({ ...statusStore.get(), permission: ok });
    toast(ok ? 'Screen recording permission granted.' : 'Permission not granted yet. Open System Settings and enable Fishy, then restart the app.', { kind: ok ? 'ok' : 'warn' });
  } catch (err) { fail(err); }
}

export async function openPermissionSettings(): Promise<void> {
  try {
    if (inTauri()) await call('open_screen_permission_settings');
    else toast('Would open the system screen-recording settings.');
  } catch (err) { fail(err); }
}

export async function setOverlayVisible(visible: boolean): Promise<void> {
  overlayStore.set({ ...overlayStore.get(), visible });
  try { if (inTauri()) await call('set_overlay_visible', { visible }); } catch (err) { fail(err); }
}

/** Reveal / hide the overlay content in peek mode (used by the "Hold to try" button). */
export async function setPeek(revealed: boolean): Promise<void> {
  try { if (inTauri()) await call('set_peek', { revealed }); } catch (err) { fail(err); }
}

/** Editable = overlay accepts the mouse so you can drag the HUD (click-through off). */
export async function setOverlayEditable(editable: boolean): Promise<void> {
  overlayStore.set({ ...overlayStore.get(), editable });
  try { if (inTauri()) await call('set_click_through', { enabled: !editable }); } catch (err) { fail(err); }
}
