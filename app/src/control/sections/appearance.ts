import { h } from '../dom';
import { getSettings } from '../store';
import { at, colorPicker, reg, segmented, slider, toggle } from '../ui/controls';
import { card, field, sectionShell } from '../ui/layout';

const ACCENTS = ['#22d3b0', '#3b82ff', '#a78bfa', '#ff7ab6', '#f59e0b', '#ef476f', '#6fbf4a', '#06b6d4'];

export function appearanceSection(): HTMLElement {
  const accent = at<string>('theme.accent');
  const sws = ACCENTS.map((c) => h('button', {
    type: 'button', class: 'pal big', style: { '--c': c }, title: c, 'aria-label': `Accent ${c}`, onclick: () => accent.set(c),
  }));
  reg(() => sws.forEach((s, i) => s.setAttribute('aria-pressed', String(ACCENTS[i] === getSettings().theme.accent))));

  return sectionShell(
    { id: 'appearance', title: 'Appearance', desc: 'Make this control window feel right. The overlay on your board is styled in the other sections.', icon: 'appearance', reset: ['theme'] },
    card({ title: 'Theme' },
      field('Mode', 'System follows your operating system’s light or dark setting.',
        segmented(at<'dark' | 'light' | 'system'>('theme.mode'), [{ value: 'dark', label: 'Dark' }, { value: 'light', label: 'Light' }, { value: 'system', label: 'System' }])),
      field('Accent colour', 'Used for highlights, buttons and the brand gradient.',
        h('div', { class: 'accent-row' }, sws, colorPicker(accent, { label: 'Custom accent colour' })), { stack: true })),
    card({ title: 'Comfort' },
      field('Interface size', 'Scale all text and controls in this window.', slider(at<number>('theme.uiScale'), { min: 0.8, max: 1.4, step: 0.05, fmt: (v) => `${Math.round(v * 100)}%` })),
      field('Reduce motion', 'Turns off transitions and automatic preview cycling in this window.', toggle(at('theme.reduceMotion')))));
}
