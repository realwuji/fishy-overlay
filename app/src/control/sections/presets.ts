import { MOVE_CLASSES, TACTIC_TAGS, type Settings } from '../../shared/types';
import { h, icon } from '../dom';
import { deepMerge, type DeepPartial } from '../sanitize';
import { getSettings, replaceSettings, resetAllSettings } from '../store';
import { button } from '../ui/controls';
import { callout, card, sectionShell, undoable } from '../ui/layout';
import { toast } from '../ui/toast';

interface Preset { id: string; name: string; blurb: string; chips: string[]; patch: DeepPartial<Settings> }

const allClasses = Object.fromEntries(MOVE_CLASSES.map((k) => [k, { enabled: true }]));
const allTags = Object.fromEntries(TACTIC_TAGS.map((k) => [k, { enabled: true }]));

export const PRESETS: Preset[] = [
  {
    id: 'beginner', name: 'Beginner', blurb: 'One clear arrow with big, friendly labels. Nothing to decode.',
    chips: ['1 arrow', 'Big labels', 'Short tips'],
    patch: {
      arrows: { show: true, count: 1, style: 'classic', glow: true, colorMode: 'rank', minClass: null },
      labels: { show: true, placement: 'cornerBadge', style: 'iconText', size: 1.4, showTags: true, maxTags: 1 },
      evalBar: { show: true, showNumber: false, width: 16 },
      hud: { show: true, lines: 1, showPv: false, showExplanation: true, showDepth: false, showWinPct: false, compact: false, scale: 1.2 },
      engine: { multiPv: 3 },
    },
  },
  {
    id: 'coach', name: 'Coach', blurb: 'Three candidates, tactic tags and the reasoning behind the top move.',
    chips: ['3 arrows', 'Tactic tags', 'Explanations'],
    patch: {
      arrows: { show: true, count: 3, style: 'classic', glow: true, colorMode: 'rank', minClass: null },
      labels: { show: true, placement: 'cornerBadge', style: 'iconText', size: 1.1, showTags: true, maxTags: 2 },
      evalBar: { show: true, showNumber: true, width: 14 },
      hud: { show: true, lines: 3, showPv: true, showExplanation: true, showDepth: false, showWinPct: true, compact: false, scale: 1 },
      engine: { multiPv: 3 },
    },
  },
  {
    id: 'minimal', name: 'Minimal', blurb: 'A single slim arrow and nothing else. Stays out of your way.',
    chips: ['1 slim arrow', 'No HUD', 'No labels'],
    patch: {
      arrows: { show: true, count: 1, style: 'slim', glow: false, animate: false, opacity: 0.75, minClass: null },
      labels: { show: false },
      evalBar: { show: true, showNumber: false, width: 8, style: 'solid' },
      hud: { show: false },
      engine: { multiPv: 1 },
    },
  },
  {
    id: 'analyst', name: 'Analyst', blurb: 'Five arrows, every label and tag, and the full HUD for deep study.',
    chips: ['5 arrows', 'All labels', 'Full HUD'],
    patch: {
      arrows: { show: true, count: 5, style: 'classic', glow: true, colorMode: 'classification', minClass: null },
      labels: { show: true, placement: 'cornerBadge', style: 'iconText', size: 1, showTags: true, maxTags: 3, classes: allClasses, tags: allTags },
      evalBar: { show: true, showNumber: true, width: 14 },
      hud: { show: true, lines: 5, showPv: true, showExplanation: true, showDepth: true, showWinPct: true, compact: false, scale: 1 },
      engine: { multiPv: 5 },
    },
  },
];

export function applyPreset(p: Preset): void {
  undoable(`${p.name} preset applied.`, () => replaceSettings(deepMerge(getSettings(), p.patch)));
}

function exportJson(): string {
  return JSON.stringify(getSettings(), null, 2);
}

export function presetsSection(): HTMLElement {
  const cards = PRESETS.map((p) => h('article', { class: 'preset' },
    h('div', { class: 'preset-top' }, h('h4', null, p.name), icon('sparkle', 16)),
    h('p', { class: 'help' }, p.blurb),
    h('div', { class: 'chips' }, p.chips.map((c) => h('span', { class: 'chip' }, c))),
    button(`Apply ${p.name}`, () => applyPreset(p), { cls: 'sm', title: `Apply the ${p.name} preset on top of your current settings` })));

  const area = h('textarea', {
    class: 'json', rows: 8, spellcheck: false, 'aria-label': 'Settings JSON', placeholder: 'Paste exported settings JSON here, then press Import.',
  });
  const file = h('input', {
    type: 'file', accept: 'application/json,.json', hidden: true, 'aria-label': 'Choose settings file',
    onchange: async () => {
      const f = file.files?.[0];
      if (f) area.value = await f.text();
      file.value = '';
    },
  });

  const doImport = () => {
    let raw: unknown;
    try { raw = JSON.parse(area.value); } catch { toast('That is not valid JSON.', { kind: 'error' }); return; }
    if (!raw || typeof raw !== 'object') { toast('Settings must be a JSON object.', { kind: 'error' }); return; }
    undoable('Settings imported.', () => {
      const incoming = { ...(raw as Record<string, unknown>), acceptedFairPlay: getSettings().acceptedFairPlay };
      replaceSettings(deepMerge(getSettings(), incoming));
    });
  };

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(exportJson());
      toast('Settings copied to the clipboard.', { kind: 'ok' });
    } catch {
      area.value = exportJson(); area.select();
      toast('Copy was blocked. The JSON is in the box below, press Ctrl/Cmd+C.', { kind: 'warn' });
    }
  };
  const download = () => {
    const a = h('a', { href: URL.createObjectURL(new Blob([exportJson()], { type: 'application/json' })), download: 'fishy-settings.json' });
    document.body.appendChild(a); a.click(); a.remove();
    window.setTimeout(() => URL.revokeObjectURL(a.href), 2000);
  };

  let armed = 0;
  const resetBtn = button('Reset all settings', () => {
    if (!armed) {
      armed = window.setTimeout(() => { armed = 0; resetBtn.querySelector('span')!.textContent = 'Reset all settings'; resetBtn.classList.remove('armed'); }, 4000);
      resetBtn.querySelector('span')!.textContent = 'Click again to confirm';
      resetBtn.classList.add('armed');
      return;
    }
    clearTimeout(armed); armed = 0;
    resetBtn.querySelector('span')!.textContent = 'Reset all settings';
    resetBtn.classList.remove('armed');
    undoable('Everything reset to defaults.', () => { void resetAllSettings(); });
  }, { cls: 'danger', icon: 'reset' });

  return sectionShell(
    { id: 'presets', title: 'Presets', desc: 'One-click starting points. They merge into your settings, so your hotkeys, theme and engine power stay yours.', icon: 'presets' },
    h('div', { class: 'preset-grid' }, cards),
    card({ title: 'Import & export', desc: 'Back up your look, or share it with a friend or student.' },
      h('div', { class: 'btn-row' },
        button('Copy settings', copy, { icon: 'copy', cls: 'sm' }),
        button('Download .json', download, { icon: 'download', cls: 'sm' }),
        button('Choose file…', () => file.click(), { icon: 'upload', cls: 'sm' }), file),
      area,
      h('div', { class: 'btn-row' }, button('Import from JSON', doImport, { icon: 'check' }),
        h('span', { class: 'help' }, 'Values are validated and clamped. Your fair-play acknowledgement is never changed by an import.'))),
    card({ title: 'Reset', desc: 'Restore every setting to its factory value. You can undo right after.' },
      h('div', { class: 'btn-row' }, resetBtn),
      callout('info', 'Your acknowledgement of the fair-play notice is kept when you reset.')));
}
