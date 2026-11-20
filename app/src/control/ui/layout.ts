// Layout building blocks: fields, cards, section shells, undoable actions.
import { DEFAULT_SETTINGS } from '../../shared/defaults';
import type { Settings } from '../../shared/types';
import { h, icon, nextId, clone, type Child } from '../dom';
import { edit, getSettings, replaceSettings } from '../store';
import { reg } from './controls';
import { toast } from './toast';

/** Label + help text on the left, control on the right. Wires aria-labelledby/describedby. */
export function field(
  label: string, help: string | null, control: HTMLElement,
  o: { stack?: boolean; tip?: string; hidden?: () => boolean; disabled?: () => boolean } = {},
): HTMLElement {
  const lid = nextId('lbl'), hid = nextId('hlp');
  const labelEl = h('label', { class: 'field-label', id: lid }, label);
  const primaries = control.matches('[data-ctl]') ? [control] : Array.from(control.querySelectorAll<HTMLElement>('[data-ctl]'));
  primaries.forEach((p, i) => {
    if (!p.getAttribute('aria-label')) p.setAttribute('aria-labelledby', lid);
    if (help) p.setAttribute('aria-describedby', hid);
    if (i === 0 && !p.id && /^(INPUT|SELECT|BUTTON)$/.test(p.tagName)) p.id = nextId('ctl');
    if (i === 0 && p.id && /^(INPUT|SELECT)$/.test(p.tagName)) labelEl.htmlFor = p.id;
  });
  const root = h('div', { class: `field${o.stack ? ' stack' : ''}`, title: o.tip ?? help ?? undefined },
    h('div', { class: 'field-text' }, labelEl, help && h('p', { class: 'help', id: hid }, help)),
    h('div', { class: 'field-ctl' }, control));
  if (o.hidden || o.disabled) {
    reg(() => {
      root.hidden = o.hidden?.() ?? false;
      root.classList.toggle('is-disabled', o.disabled?.() ?? false);
    });
  }
  return root;
}

export function card(o: { title?: string; desc?: string; actions?: Child; cls?: string }, ...children: Child[]): HTMLElement {
  return h('section', { class: `card ${o.cls ?? ''}` },
    (o.title || o.actions) && h('header', { class: 'card-head' },
      h('div', null, o.title && h('h3', null, o.title), o.desc && h('p', { class: 'help' }, o.desc)),
      o.actions && h('div', { class: 'card-actions' }, o.actions)),
    h('div', { class: 'card-body' }, children));
}

export function callout(kind: 'info' | 'warn' | 'ok', ...children: Child[]): HTMLElement {
  return h('div', { class: `callout ${kind}`, role: kind === 'warn' ? 'note' : undefined },
    icon(kind === 'ok' ? 'check' : kind === 'warn' ? 'warn' : 'info', 18), h('div', null, children));
}

/** Run a mutation and offer Undo via toast. */
export function undoable(message: string, fn: () => void): void {
  const before = clone(getSettings());
  fn();
  toast(message, { action: { label: 'Undo', run: () => replaceSettings(before) } });
}

export function resetKeys(keys: (keyof Settings)[], message: string): void {
  undoable(message, () => edit((d) => { for (const k of keys) (d as unknown as Record<string, unknown>)[k] = clone(DEFAULT_SETTINGS[k]); }));
}

export interface SectionDef { id: string; title: string; desc: string; icon: string; reset?: (keyof Settings)[] }

export function sectionShell(def: SectionDef, ...children: Child[]): HTMLElement {
  return h('section', { class: 'section', id: `sec-${def.id}`, 'aria-labelledby': `h-${def.id}`, hidden: true },
    h('header', { class: 'section-head' },
      h('div', { class: 'section-title' },
        h('span', { class: 'section-ico' }, icon(def.icon, 22)),
        h('div', null, h('h2', { id: `h-${def.id}`, tabIndex: -1 }, def.title), h('p', { class: 'section-desc' }, def.desc))),
      def.reset && h('button', {
        type: 'button', class: 'btn sm ghost', title: 'Restore this section to the default settings',
        onclick: () => resetKeys(def.reset!, `${def.title} reset to defaults.`),
      }, icon('reset', 14), h('span', null, 'Reset section'))),
    h('div', { class: 'section-body' }, children));
}

/** Small icon-only reset button that shows only when `isModified()`. */
export function rowReset(isModified: () => boolean, onReset: () => void, what: string): HTMLButtonElement {
  const b = h('button', { type: 'button', class: 'icon-only', title: `Reset ${what} to default`, 'aria-label': `Reset ${what} to default`, onclick: onReset }, icon('reset', 15));
  reg(() => { b.style.visibility = isModified() ? 'visible' : 'hidden'; b.disabled = !isModified(); });
  return b;
}

export const subhead = (text: string, desc?: string): HTMLElement =>
  h('div', { class: 'subhead' }, h('h4', null, text), desc && h('p', { class: 'help' }, desc));
