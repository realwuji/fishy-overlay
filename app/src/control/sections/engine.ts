import { analysisStore, statusStore } from '../backend';
import { h, spinner } from '../dom';
import { maxThreads, recommendedThreads } from '../sanitize';
import { getSettings } from '../store';
import { at, button, optionalSlider, reg, slider, toggle, type Bind } from '../ui/controls';
import { callout, card, field, sectionShell } from '../ui/layout';

const fmtMb = (v: number) => (v >= 1024 ? `${(v / 1024).toFixed(v % 1024 ? 1 : 0)} GB` : `${v} MB`);
const fmtMs = (v: number) => (v >= 1000 ? `${(v / 1000).toFixed(v % 1000 ? 1 : 0)} s` : `${v} ms`);
const fmtNps = (n: number) => (n >= 1e6 ? `${(n / 1e6).toFixed(2)} Mn/s` : n >= 1e3 ? `${Math.round(n / 1e3)} kn/s` : `${Math.round(n)} n/s`);

const isAuto = (): boolean => getSettings().engine.auto !== false;

/** What the engine is actually using right now, from SessionStatus.engine (updates live). */
function liveFacts(): HTMLElement {
  const root = h('div', { class: 'facts engine-live', 'aria-live': 'polite' });
  const render = () => {
    const s = statusStore.get();
    const e = s.engine;
    const a = analysisStore.get();
    const fact = (label: string, value: string) => h('div', { class: 'fact' }, h('span', null, label), h('strong', null, value));
    if (!s.running) {
      root.replaceChildren(h('p', { class: 'help' }, 'Start a session to see what Stockfish is using on this computer.'));
    } else if (!e || !e.ready) {
      root.replaceChildren(spinner(14), h('span', { class: 'help' }, 'Starting Stockfish...'));
    } else {
      const nps = (a?.nps ?? e.nps) || 0;
      root.replaceChildren(
        fact('Engine', e.name ?? 'Stockfish'),
        fact('Threads', String(e.threads)),
        fact('Hash', fmtMb(e.hashMb)),
        fact('Speed', nps > 0 ? fmtNps(nps) : 'warming up'));
    }
  };
  statusStore.subscribe(render);
  analysisStore.subscribe(render);
  render();
  return root;
}

export function engineSection(): HTMLElement {
  const cores = navigator.hardwareConcurrency || 4;
  const rec = recommendedThreads();
  const threads = at<number>('engine.threads');
  const auto: Bind<boolean> = { get: isAuto, set: (v) => at<boolean>('engine.auto').set(v) };
  const manualOff = (): boolean => isAuto();

  const recBtn = button(`Use recommended (${rec})`, () => threads.set(rec), { cls: 'sm ghost' });
  reg(() => { recBtn.disabled = manualOff(); });
  const tHelp = h('div', { class: 'ctl-stack' },
    slider(threads, { min: 1, max: maxThreads(), step: 1, fmt: (v) => `${v} thread${v > 1 ? 's' : ''}`, disabled: manualOff }),
    h('div', { class: 'ctl-note' }, `This computer reports ${cores} logical cores.`, recBtn));

  const manual = card({ title: 'Performance', desc: 'Manual numbers, used only when Auto is off.' },
    field('Threads', `More threads let Stockfish search faster, but leave some cores free for your browser and the overlay. Recommended here: ${rec}.`, tHelp, { stack: true, disabled: manualOff }),
    field('Hash memory', 'Memory Stockfish uses to remember positions. 128 to 512 MB is plenty for study sessions.', slider(at<number>('engine.hashMb'), { min: 16, max: 4096, step: 16, log: true, fmt: fmtMb, disabled: manualOff }), { disabled: manualOff }),
    field('Lines to analyse (MultiPV)', 'How many different best moves Stockfish tracks. Needs to be at least your arrow count. More lines means slower depth.', slider(at<number>('engine.multiPv'), { min: 1, max: 8, step: 1, fmt: (v) => `${v} line${v > 1 ? 's' : ''}` })),
    (() => {
      const w = callout('warn', 'MultiPV is lower than your arrow count, so some arrows will not appear.');
      reg(() => { const s = getSettings(); w.hidden = s.engine.multiPv >= s.arrows.count; });
      return w;
    })());

  return sectionShell(
    { id: 'engine', title: 'Engine', desc: 'Control how hard Stockfish thinks. Stronger settings use more CPU and memory.', icon: 'engine', reset: ['engine'] },
    card({ cls: 'auto-card' },
      field('Auto (recommended)', 'Fishy picks the number of threads and the hash memory that suit this computer and keeps your browser and the overlay smooth. Turn it off to set them yourself.',
        toggle(auto, { label: 'Auto (recommended)' })),
      liveFacts()),
    manual,
    card({ title: 'When to show moves', desc: 'Early moves from a shallow search are often wrong. Wait a moment so what you see is reliable.' },
      field('Display after depth', 'Arrows and labels for a new position appear once the engine has searched this many half-moves. Until then Fishy shows "Analysing". If the search is slow, moves appear after about 2 seconds marked as preliminary. 0 shows moves immediately.',
        slider(at<number>('engine.minDisplayDepth'), { min: 0, max: 30, step: 1, fmt: (v) => (v === 0 ? 'Immediately' : `depth ${v}`), label: 'Display after depth' }), { stack: true })),
    card({ title: 'Search limits', desc: 'By default Fishy keeps thinking until the position changes. Add limits to save power.' },
      field('Depth limit', 'Stop searching after this many half-moves. Around 18 to 22 is already very strong.',
        optionalSlider(at<number | null>('engine.depthLimit'), { min: 1, max: 40, step: 1, offLabel: 'Unlimited', onLabel: 'Limit', initial: 20, fmt: (v) => `depth ${v}`, label: 'Depth limit' }), { stack: true }),
      field('Time per move', 'Stop after this much thinking time for each position.',
        optionalSlider(at<number | null>('engine.moveTimeMs'), { min: 50, max: 30000, step: 50, log: true, offLabel: 'Unlimited', onLabel: 'Limit', initial: 1000, fmt: fmtMs, label: 'Time per move' }), { stack: true })),
    card({ title: 'Strength', desc: 'Handy for lessons: weaker settings suggest more human, forgiving moves.' },
      field('Skill level', 'Full strength is Stockfish at its best. Lower levels (0 to 20) deliberately pick weaker moves.',
        optionalSlider(at<number | null>('engine.skillLevel'), { min: 0, max: 20, step: 1, offLabel: 'Full strength', onLabel: 'Limited', initial: 12, fmt: (v) => `level ${v}`, label: 'Skill level' }), { stack: true }),
      callout('info', 'Engine changes can restart the current analysis for a moment while Stockfish reloads.')));
}
