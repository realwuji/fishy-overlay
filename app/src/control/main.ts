// Fishy control window entry: shell, routing, theming, wiring.
import './styles.css';
import { currentPhase, initBackend, startSession, startingStore, statusStore, stopSession } from './backend';
import { h, icon, spinner } from './dom';
import { hexToRgb, onColor, partner } from './color';
import { SECTION_IDS, type SectionId } from './nav';
import { createPreview } from './preview';
import { aboutSection } from './sections/about';
import { appearanceSection } from './sections/appearance';
import { arrowsSection } from './sections/arrows';
import { captureSection } from './sections/capture';
import { engineSection } from './sections/engine';
import { evalHudSection } from './sections/evalhud';
import { labelsSection } from './sections/labels';
import { overviewSection } from './sections/overview';
import { presetsSection } from './sections/presets';
import { getSettings, initSettings, saveState, subscribeSettings } from './store';
import { reg } from './ui/controls';
import { openFairPlay } from './ui/modal';
import type { Settings } from '../shared/types';

const NAV: { id: SectionId; label: string; icon: string }[] = [
  { id: 'overview', label: 'Overview', icon: 'overview' },
  { id: 'arrows', label: 'Arrows', icon: 'arrows' },
  { id: 'labels', label: 'Labels & Tags', icon: 'labels' },
  { id: 'evalhud', label: 'Eval bar & HUD', icon: 'evalhud' },
  { id: 'engine', label: 'Engine', icon: 'engine' },
  { id: 'capture', label: 'Capture & Hotkeys', icon: 'capture' },
  { id: 'appearance', label: 'Appearance', icon: 'appearance' },
  { id: 'presets', label: 'Presets', icon: 'presets' },
  { id: 'about', label: 'About & Fair play', icon: 'about' },
];

// ------------------------------------------------------------------- theme
const dark = matchMedia('(prefers-color-scheme: dark)');
function applyTheme(s: Settings): void {
  const root = document.documentElement;
  const t = s.theme;
  const resolved = t.mode === 'system' ? (dark.matches ? 'dark' : 'light') : t.mode;
  root.dataset.theme = resolved;
  root.style.colorScheme = resolved;
  const [r, g, b] = hexToRgb(t.accent);
  root.style.setProperty('--accent', t.accent);
  root.style.setProperty('--accent-rgb', `${r} ${g} ${b}`);
  root.style.setProperty('--accent-2', partner(t.accent));
  root.style.setProperty('--on-accent', onColor(t.accent));
  root.style.fontSize = `${10 * t.uiScale}px`;
  root.dataset.motion = t.reduceMotion ? 'reduced' : 'full';
  applyLayout();
}

/** Layout mode from the window width measured in rem, so it also reacts to interface size. */
function applyLayout(): void {
  const rem = window.innerWidth / (10 * getSettings().theme.uiScale);
  document.documentElement.dataset.layout = rem >= 100 ? 'full' : rem >= 84 ? 'rail' : 'stack';
}

// ------------------------------------------------------------------- shell
function build(): void {
  const app = document.getElementById('app')!;
  const sections = new Map<SectionId, HTMLElement>([
    ['overview', overviewSection()], ['arrows', arrowsSection()], ['labels', labelsSection()], ['evalhud', evalHudSection()],
    ['engine', engineSection()], ['capture', captureSection()], ['appearance', appearanceSection()], ['presets', presetsSection()], ['about', aboutSection()],
  ]);

  const links = NAV.map((n) => h('a', {
    class: 'nav-item', href: `#${n.id}`, title: n.label, 'data-id': n.id,
  }, icon(n.icon, 20), h('span', { class: 'nav-label' }, n.label)));
  const nav = h('nav', {
    class: 'nav', 'aria-label': 'Settings sections',
    onkeydown: (e: KeyboardEvent) => {
      if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
      const i = links.indexOf(document.activeElement as HTMLAnchorElement);
      if (i < 0) return;
      e.preventDefault();
      links[(i + (e.key === 'ArrowDown' ? 1 : -1) + links.length) % links.length].focus();
    },
  }, links);

  // Sidebar footer: save indicator + quick session control.
  const saveEl = h('div', { class: 'save', role: 'status', 'aria-live': 'polite' });
  const renderSave = () => {
    const s = saveState.get();
    saveEl.dataset.state = s;
    saveEl.replaceChildren(h('span', { class: 'dot' }), h('span', { class: 'save-text' },
      s === 'saving' ? 'Saving…' : s === 'error' ? 'Could not save' : s === 'saved' ? 'All changes saved' : 'Changes save automatically'));
  };
  saveState.subscribe(renderSave); renderSave();

  const mini = h('button', {
    type: 'button', class: 'btn primary mini',
    onclick: () => { void (statusStore.get().running ? stopSession() : startSession()); },
  });
  const renderMini = () => {
    const run = statusStore.get().running;
    const starting = startingStore.get() || (run && currentPhase() === 'startingEngine');
    mini.replaceChildren(starting ? spinner(14) : icon(run ? 'stop' : 'play', 14), h('span', { class: 'nav-label' }, starting ? 'Starting...' : run ? 'Stop' : 'Start'));
    mini.classList.toggle('stop', run && !starting);
    mini.classList.toggle('busy', starting);
    mini.disabled = starting || (!run && !getSettings().acceptedFairPlay);
    mini.title = starting ? 'Starting the engine' : run ? 'Stop the session' : getSettings().acceptedFairPlay ? 'Start the session' : 'Acknowledge the fair-play notice first';
  };
  statusStore.subscribe(renderMini); startingStore.subscribe(renderMini); reg(renderMini);

  const side = h('aside', { class: 'sidebar' },
    h('div', { class: 'brand' },
      h('img', { src: '/logo.svg', alt: '', width: 36, height: 36 }),
      h('div', { class: 'brand-text nav-label' }, h('strong', null, 'Fishy'), h('span', null, 'Study overlay'))),
    nav,
    h('div', { class: 'side-foot' }, mini, saveEl));

  const main = h('main', { class: 'main', id: 'main', tabIndex: -1 }, h('div', { class: 'content' }, Array.from(sections.values())));
  const preview = createPreview();
  app.replaceChildren(side, main, h('div', { class: 'pv-col' }, preview.el));

  const route = () => {
    const id = (location.hash.slice(1) as SectionId);
    const cur: SectionId = SECTION_IDS.includes(id) ? id : 'overview';
    sections.forEach((el, k) => { el.hidden = k !== cur; });
    links.forEach((a) => {
      const on = a.dataset.id === cur;
      if (on) a.setAttribute('aria-current', 'page'); else a.removeAttribute('aria-current');
    });
    main.scrollTop = 0;
    document.title = `${NAV.find((n) => n.id === cur)!.label} • Fishy`;
  };
  window.addEventListener('hashchange', route);
  route();
}

async function boot(): Promise<void> {
  await initSettings();
  applyTheme(getSettings());
  subscribeSettings(applyTheme);
  dark.addEventListener('change', () => applyTheme(getSettings()));
  window.addEventListener('resize', applyLayout);
  await initBackend();
  build();
  document.documentElement.dataset.ready = 'true';
  if (!getSettings().acceptedFairPlay) openFairPlay(true);
  // Re-open the notice if it becomes unacknowledged (e.g. settings pushed from elsewhere).
  subscribeSettings((s) => { if (!s.acceptedFairPlay && !document.querySelector('.modal-backdrop')) openFairPlay(true); });
}

void boot();
