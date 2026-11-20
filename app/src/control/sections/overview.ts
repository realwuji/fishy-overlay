import { inTauri } from '../../shared/ipc';
import type { CaptureTarget, SessionPhase } from '../../shared/types';
import {
  analysisStore, boardStore, calibrate, currentPhase, startingStore, targetsLoadingStore, openPermissionSettings, overlayStore, redetect, refreshTargets, requestPermission,
  resumeFromFen, setOverlayEditable, setOverlayVisible, startSession, statusStore, stopSession, targetsStore,
} from '../backend';
import { h, icon, isMac, spinner } from '../dom';
import { checkFen } from '../sanitize';
import { edit, getSettings } from '../store';
import { at, button, keycaps, reg, segmented, toggle, type Bind } from '../ui/controls';
import { callout, card, field, sectionShell } from '../ui/layout';
import { openFairPlay } from '../ui/modal';
import { navigate } from '../nav';

const START_FEN = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1';

function targetLabel(t: CaptureTarget): string {
  return `${t.app}: ${t.title} (${t.width}×${t.height})`;
}

function targetSelect(): HTMLElement {
  const sel = h('select', {
    class: 'select', 'data-ctl': '', 'aria-label': 'Capture target',
    onchange: () => edit((d) => { d.capture.targetId = sel.value || null; }),
  });
  const wrap = h('div', { class: 'select-wrap grow' }, sel, icon('next', 14, 'select-caret'));
  const render = () => {
    const ts = targetsStore.get();
    const cur = getSettings().capture.targetId ?? '';
    const monitors = ts.filter((t) => t.kind === 'monitor'), windows = ts.filter((t) => t.kind === 'window');
    const opt = (t: CaptureTarget) => h('option', { value: t.id }, targetLabel(t));
    const missing = cur && !ts.some((t) => t.id === cur);
    sel.replaceChildren(
      h('option', { value: '' }, 'Automatic: whole primary screen'),
      ...(missing ? [h('option', { value: cur }, 'Saved target (not currently available)')] : []),
      ...(monitors.length > 0 ? [h('optgroup', { label: 'Screens' }, monitors.map(opt))] : []),
      ...(windows.length > 0 ? [h('optgroup', { label: 'Windows' }, windows.map(opt))] : []),
    );
    sel.value = cur;
  };
  reg(render);
  targetsStore.subscribe(render);
  // Skeleton while the list of screens and windows loads.
  const skel = h('div', { class: 'select-skel grow', 'aria-hidden': 'true' }, h('span', { class: 'skel w60' }), h('span', { class: 'skel w30' }));
  const refresh = h('button', { type: 'button', class: 'icon-only bordered', title: 'Refresh the list of screens and windows', 'aria-label': 'Refresh capture targets', onclick: () => { void refreshTargets(); } }, icon('refresh', 16));
  const row = h('div', { class: 'row-ctl' }, wrap, skel, refresh);
  const loading = () => {
    const on = targetsLoadingStore.get();
    wrap.hidden = on;
    skel.hidden = !on;
    sel.disabled = on;
    refresh.disabled = on;
    refresh.classList.toggle('busy', on);
    row.setAttribute('aria-busy', String(on));
  };
  targetsLoadingStore.subscribe(loading);
  loading();
  return row;
}

/** Plain-language board/tracking state from EVT.board + EVT.status. */
function trackingInfo(running: boolean): { tone: 'ok' | 'warn' | 'idle'; label: string; guide: string } {
  const b = boardStore.get();
  if (!running) return { tone: 'idle', label: 'Not running', guide: 'Press Start session. Fishy then looks for the board on the screen or window you picked.' };
  if (!b.geometry) return { tone: 'idle', label: 'Looking for a board', guide: 'Make sure the whole board is visible on the selected screen or window, with nothing covering it.' };
  if (b.state === 'tracking') return { tone: 'ok', label: `Tracking (${Math.round(b.confidence * 100)}%)`, guide: 'Fishy is following the game. Arrows appear on the board after each move.' };
  if (b.state === 'lost') return { tone: 'warn', label: 'Lost track of the game', guide: 'Hold still for a moment. If it does not recover, press Re-detect board, or Calibrate with the starting position.' };
  return b.calibrated
    ? { tone: 'idle', label: 'Board found, reading pieces', guide: 'Fishy is recognising the pieces. This takes a moment.' }
    : { tone: 'warn', label: 'Board found, pieces not recognised yet', guide: 'On chess.com with the default pieces this should work by itself. For another piece set or site, calibrate with the starting position.' };
}

const PHASE_ORDER: SessionPhase[] = ['startingEngine', 'findingBoard', 'recognising', 'analysing', 'ready'];

const fmtNps = (n: number): string => (n >= 1e6 ? `${(n / 1e6).toFixed(2)} Mn/s` : n >= 1e3 ? `${Math.round(n / 1e3)} kn/s` : '');
const fmtHash = (mb: number): string => (mb >= 1024 ? `${(mb / 1024).toFixed(mb % 1024 ? 1 : 0)} GB` : `${mb} MB`);

/** Plain-language one-liner for the hero card, from the phase. */
function phaseMessage(phase: SessionPhase): string {
  switch (phase) {
    case 'startingEngine': return 'Starting the engine...';
    case 'findingBoard': return 'Looking for your board...';
    case 'recognising': return 'Reading the pieces...';
    case 'analysing': return 'Analysing the position...';
    case 'ready': return 'Watching your game. Arrows appear after each move.';
    default: return 'Pick a screen or window, then press Start.';
  }
}

interface StepDef { title: string; pending: string; active: () => string; done: () => string }

function stepper(): HTMLElement {
  const steps: StepDef[] = [
    {
      title: 'Engine', pending: 'Waits for you to press Start.',
      active: () => 'Starting Stockfish...',
      done: () => {
        const e = statusStore.get().engine;
        return e?.name ? `${e.name} ready` : 'Stockfish ready';
      },
    },
    {
      title: 'Board', pending: 'Looks for the board on the chosen screen.',
      active: () => 'Looking for the board on your screen...',
      done: () => 'Board found',
    },
    {
      title: 'Pieces', pending: 'Reads which piece stands where.',
      active: () => 'Reading the pieces...',
      done: () => 'Position recognised',
    },
    {
      title: 'Analysis', pending: 'Searches for the best moves.',
      active: () => {
        const a = analysisStore.get();
        const t = a?.targetDepth ?? getSettings().engine.minDisplayDepth ?? 12;
        return a && a.depth > 0 ? (t > 0 ? `Searching: depth ${a.depth} of ${t}` : `Searching: depth ${a.depth}`) : 'Starting the search...';
      },
      done: () => {
        const a = analysisStore.get();
        return a ? `Depth ${a.depth}${a.nps ? ` at ${fmtNps(a.nps)}` : ''}` : 'Analysing';
      },
    },
  ];
  const nodes = steps.map((st, i) => {
    const mark = h('span', { class: 'st-mark' });
    const text = h('span', { class: 'st-text' });
    const root = h('li', { class: 'st pending' },
      h('span', { class: 'st-rail' }, mark, i < steps.length - 1 && h('span', { class: 'st-line' })),
      h('span', { class: 'st-body' }, h('strong', null, st.title), text));
    return { root, mark, text, st, i };
  });
  const list = h('ol', { class: 'stepper', 'aria-label': 'Start-up progress' }, nodes.map((n) => n.root));

  // Engine facts: shown as soon as they are known.
  const facts = h('div', { class: 'facts' });
  const bar = h('div', { class: 'st-progress', hidden: true }, h('i'));

  const render = () => {
    const s = statusStore.get();
    const phase = currentPhase();
    const idx = phase === 'idle' ? -1 : PHASE_ORDER.indexOf(phase);
    for (const n of nodes) {
      // 'ready' finishes every step; 'analysing' keeps the last one active.
      const done = phase === 'ready' ? true : n.i < idx;
      const active = !done && n.i === idx;
      n.root.className = `st ${done ? 'done' : active ? 'active' : 'pending'}`;
      n.mark.replaceChildren(done ? icon('check', 14) : active ? spinner(14) : String(n.i + 1));
      n.text.textContent = done ? n.st.done() : active ? n.st.active() : n.st.pending;
      if (active) n.root.setAttribute('aria-current', 'step'); else n.root.removeAttribute('aria-current');
    }
    // Slim depth bar under the Analysis step while it searches.
    const a = analysisStore.get();
    const t = a?.targetDepth ?? 0;
    const showBar = phase === 'analysing' && !!a && t > 0;
    bar.hidden = !showBar;
    if (showBar && a) (bar.firstElementChild as HTMLElement).style.transform = `scaleX(${Math.max(0.03, Math.min(1, a.depth / t))})`;
    if (showBar) nodes[3].root.querySelector('.st-body')?.appendChild(bar);

    const e = s.engine;
    const auto = getSettings().engine.auto !== false;
    const fact = (label: string, value: string) => h('div', { class: 'fact' }, h('span', null, label), h('strong', null, value));
    if (s.running && e && (e.name || e.threads)) {
      const nps = (a?.nps ?? e.nps) || 0;
      facts.replaceChildren(
        fact('Engine', e.name ?? 'Stockfish'),
        fact('Threads', String(e.threads)),
        fact('Hash', fmtHash(e.hashMb)),
        fact('Speed', nps > 0 ? fmtNps(nps) : 'warming up'),
        h('span', { class: `chip ${auto ? 'ok' : ''}` }, auto ? 'Auto-tuned for this computer' : 'Manual settings'));
      facts.hidden = false;
    } else {
      facts.replaceChildren();
      facts.hidden = true;
    }
  };
  statusStore.subscribe(render);
  boardStore.subscribe(render);
  analysisStore.subscribe(render);
  reg(render);
  return h('div', { class: 'stepper-wrap' }, list, facts);
}

function statusCard(): HTMLElement {
  const pill = h('span', { class: 'pill' });
  const msg = h('p', { class: 'status-msg' });
  const chips = h('div', { class: 'chips' });
  const startBtn = h('button', { type: 'button', class: 'btn primary lg', onclick: () => { void (statusStore.get().running ? stopSession() : startSession()); } });
  const hint = h('p', { class: 'help gate' });
  const state = h('div', { class: 'state-row' });
  const steps = stepper();
  const render = () => {
    const s = statusStore.get(), b = boardStore.get(), ok = getSettings().acceptedFairPlay;
    const phase = currentPhase();
    // Spinner and "Starting..." from the click until the engine is up; after that Stop is always available.
    const starting = startingStore.get() || (s.running && phase === 'startingEngine');
    const loading = s.running && phase !== 'ready' && phase !== 'idle';
    pill.className = `pill ${s.running ? (loading ? 'busy' : 'live') : starting ? 'busy' : 'idle'}`;
    pill.replaceChildren(loading || starting ? spinner(12) : h('span', { class: 'dot' }), starting ? 'Starting' : loading ? 'Getting ready' : s.running ? 'Running' : 'Stopped');
    msg.textContent = s.message ?? (starting ? phaseMessage('startingEngine') : s.running ? phaseMessage(phase) : phaseMessage('idle'));
    chips.replaceChildren(
      h('span', { class: `chip ${s.permission ? 'ok' : 'warn'}` }, s.permission ? 'Screen access granted' : 'Needs screen recording permission'),
      h('span', { class: 'chip' }, s.running ? `${s.captureFps || getSettings().capture.fps} fps` : `${getSettings().capture.fps} fps when running`),
      h('span', { class: `chip ${!b.geometry ? '' : b.state === 'tracking' ? 'ok' : b.state === 'lost' ? 'warn' : ''}` },
        !b.geometry ? 'No board yet' : b.state === 'tracking' ? 'Board tracked' : b.state === 'lost' ? 'Board lost' : 'Board found'));
    const t = trackingInfo(s.running);
    state.className = `state-row ${t.tone}`;
    state.replaceChildren(h('span', { class: 'dot' }), h('div', null, h('strong', null, t.label), h('p', { class: 'help' }, t.guide)));
    startBtn.replaceChildren(starting ? spinner(16) : icon(s.running ? 'stop' : 'play', 16), h('span', null, starting ? 'Starting...' : s.running ? 'Stop session' : 'Start session'));
    startBtn.classList.toggle('stop', s.running && !starting);
    startBtn.classList.toggle('busy', starting);
    startBtn.disabled = starting || (!s.running && !ok);
    startBtn.setAttribute('aria-busy', String(starting));
    hint.replaceChildren(...(ok || s.running ? [] : ['Acknowledge the fair-play notice to enable Start. ', h('button', { type: 'button', class: 'link', onclick: () => openFairPlay(true) }, 'Open notice')]));
    hint.hidden = ok || s.running;
  };
  reg(render);
  statusStore.subscribe(render);
  boardStore.subscribe(render);
  startingStore.subscribe(render);
  return card({ cls: 'hero' },
    h('div', { class: 'hero-row' },
      h('div', { class: 'hero-text' }, pill, msg),
      h('div', { class: 'hero-cta' }, startBtn)),
    chips, steps, state, hint);
}

function calibrateCard(): HTMLElement {
  const input = h('input', {
    type: 'text', class: 'input mono', 'data-ctl': '', placeholder: START_FEN, spellcheck: false, autocomplete: 'off', 'aria-label': 'FEN position',
    'aria-describedby': 'fen-msg',
    oninput: () => { msg.textContent = ''; input.removeAttribute('aria-invalid'); },
    onkeydown: (e: KeyboardEvent) => { if (e.key === 'Enter') go(); },
  });
  const msg = h('p', { class: 'field-error', id: 'fen-msg', role: 'alert' });
  const go = () => {
    const r = checkFen(input.value);
    if (!r.ok) { msg.textContent = r.error; input.setAttribute('aria-invalid', 'true'); return; }
    input.value = r.fen;
    void resumeFromFen(r.fen);
  };
  const step = (n: number, title: string, body: string) => h('li', { class: 'step' }, h('span', { class: 'step-n' }, String(n)), h('div', null, h('strong', null, title), h('p', { class: 'help' }, body)));

  const boardLine = h('p', { class: 'board-line' });
  const render = () => {
    const b = boardStore.get();
    boardLine.className = `board-line ${b.state}`;
    boardLine.replaceChildren(h('span', { class: 'dot' }),
      b.state === 'tracking' ? `Tracking. ${b.fen ? 'Position: ' + b.fen.split(' ')[0] : ''}` : b.state === 'lost' ? 'Board lost. Try Re-detect board.' : b.geometry ? 'Board found. Pieces not recognised yet.' : 'No board found yet.');
  };
  boardStore.subscribe(render); render();

  return card({ title: 'Board and pieces', desc: 'On chess.com, Fishy already knows the default pieces and can join a game mid-way. Calibration is for anything else.' },
    h('ol', { class: 'steps' },
      step(1, 'Show the whole board', 'Open a chess.com game, analysis board or puzzle, then press Start session. Keep the board fully visible and unobstructed.'),
      step(2, 'chess.com default pieces: nothing else to do', 'Fishy recognises them built in, so it works from any position, even a game already in progress.'),
      step(3, 'Other piece set or site: calibrate', 'Put the starting position on screen (all 32 pieces home, no arrows over them) and press Calibrate. Fishy learns your piece set from it.')),
    h('div', { class: 'btn-row' },
      button('Calibrate (starting position on screen)', () => { void calibrate(); }, { cls: 'primary lg', icon: 'target' }),
      button('Re-detect board', () => { void redetect(); }, { cls: 'lg', icon: 'refresh', title: 'Find the board again, for example after moving or resizing the window' })),
    boardLine,
    h('div', { class: 'divider' }),
    field('Resume from FEN', 'Joining a game already in progress? Paste its FEN to start tracking from that exact position.',
      h('div', { class: 'fen-row' }, input, button('Resume', go, { cls: 'sm' }), button('Use start', () => { input.value = START_FEN; msg.textContent = ''; }, { cls: 'sm ghost', title: 'Fill in the standard starting position' })),
      { stack: true }),
    msg);
}

function permissionCard(): HTMLElement {
  const body = h('div');
  const render = () => {
    const ok = statusStore.get().permission;
    body.replaceChildren(
      callout(ok ? 'ok' : 'warn',
        h('strong', null, ok ? 'Screen recording is allowed.' : 'Fishy cannot see your screen yet.'),
        ' ', ok ? 'Fishy can read the board. Frames never leave memory.' : (isMac() ? 'macOS needs your permission before any app can read the screen. Request it, enable Fishy in System Settings, then restart the app.' : 'Allow screen capture for Fishy, then try again.')),
      h('div', { class: 'btn-row' },
        button('Request permission', () => { void requestPermission(); }, { cls: 'sm', icon: 'lock', disabled: ok }),
        button('Open system settings', () => { void openPermissionSettings(); }, { cls: 'sm ghost', icon: 'external' })));
  };
  statusStore.subscribe(render); render();
  return card({ title: 'Screen recording permission' }, body);
}

function overlayCard(): HTMLElement {
  const vis: Bind<boolean> = { get: () => overlayStore.get().visible, set: (v) => { void setOverlayVisible(v); } };
  const ed: Bind<boolean> = { get: () => overlayStore.get().editable, set: (v) => { void setOverlayEditable(v); } };
  const t1 = toggle(vis, { label: 'Show overlay' }), t2 = toggle(ed, { label: 'Move HUD' });
  overlayStore.subscribe(() => { t1.setAttribute('aria-checked', String(vis.get())); t1.classList.toggle('on', vis.get()); t2.setAttribute('aria-checked', String(ed.get())); t2.classList.toggle('on', ed.get()); });
  return card({ title: 'Overlay' },
    field('Show overlay', 'Hide or show everything Fishy draws without stopping analysis.', t1),
    field('Move the HUD', 'Unlock the overlay so you can drag the HUD. Click-through is restored when you turn this off.', t2));
}

function hotkeyCard(): HTMLElement {
  const list = h('dl', { class: 'hk-list' });
  const rows: [string, keyof ReturnType<typeof getSettings>['hotkeys']][] = [
    ['Show or hide overlay', 'toggleOverlay'], ['Show or hide HUD', 'toggleHud'], ['Cycle arrow count', 'cycleArrows'], ['Re-detect board', 'recalibrate'],
  ];
  reg(() => list.replaceChildren(...rows.flatMap(([label, k]) => [h('dt', null, label), h('dd', null, keycaps(getSettings().hotkeys[k]))])));
  return card({ title: 'Hotkey cheat sheet', actions: button('Edit', () => navigate('capture'), { cls: 'sm ghost' }) }, list);
}

export function overviewSection(): HTMLElement {
  const site: Bind<string> = at('site');
  return sectionShell(
    { id: 'overview', title: 'Overview', desc: 'Start a session and point Fishy at your board. Calibrate only for other piece sets or sites.', icon: 'overview' },
    statusCard(),
    card({ title: 'What to watch' },
      field('Capture target', inTauri() ? 'Choose the screen or the browser window that shows your board.' : 'Choose the screen or browser window that shows your board. (Preview mode lists sample targets.)', targetSelect(), { stack: true }),
      field('Site preset', 'Board detection is tuned per site. chess.com themes are fully tuned; others use the general detector for now.',
        segmented(site as Bind<'chessCom' | 'lichess' | 'generic'>, [
          { value: 'chessCom', label: 'chess.com', title: 'Tuned for chess.com board and piece themes' },
          { value: 'lichess', label: 'lichess', title: 'Uses the general detector. Tuning is coming.' },
          { value: 'generic', label: 'Other', title: 'General detector for any board' },
        ]))),
    calibrateCard(),
    permissionCard(),
    overlayCard(),
    hotkeyCard());
}
