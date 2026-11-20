// Reusable, accessible form controls bound to settings paths. Vanilla DOM, no framework.
import { MOVE_CLASSES } from '../../shared/types';
import { DEFAULT_SETTINGS } from '../../shared/defaults';
import { h, icon, clamp, nextId, isMac, type Child } from '../dom';
import { normHex } from '../color';
import { hotkeyError, normalizeHotkey } from '../sanitize';
import { edit, getSettings, subscribeSettings } from '../store';
import { openPopover } from './popover';

// ------------------------------------------------------------------ binding
export interface Bind<T> { get(): T; set(v: T): void }

/** Bind to a dotted path inside Settings, e.g. at<number>('arrows.count'). */
export function at<T>(path: string): Bind<T> {
  const p = path.split('.');
  return {
    get: () => p.reduce((o: any, k) => o?.[k], getSettings()) as T,
    set: (v) => edit((d) => {
      let o: any = d;
      for (let i = 0; i < p.length - 1; i++) o = o[p[i]];
      o[p[p.length - 1]] = v;
    }),
  };
}

const syncs: Array<() => void> = [];
/** Register a function that re-reads state into the DOM; runs once now and after every settings change. */
export function reg(fn: () => void): void { syncs.push(fn); fn(); }
export function syncAll(): void { for (const f of syncs) f(); }
subscribeSettings(syncAll);

const decimals = (step: number): number => (String(step).split('.')[1] ?? '').length;
const round = (v: number, step: number): number => Number((Math.round(v / step) * step).toFixed(decimals(step)));

interface Common { disabled?: () => boolean; title?: string }

// ------------------------------------------------------------------- toggle
export function toggle(b: Bind<boolean>, o: Common & { label?: string } = {}): HTMLButtonElement {
  const el = h('button', {
    type: 'button', class: 'switch', role: 'switch', 'data-ctl': '', title: o.title,
    'aria-label': o.label,
    onclick: () => b.set(!b.get()),
  }, h('span', { class: 'knob' }));
  reg(() => {
    const on = b.get();
    el.setAttribute('aria-checked', String(on));
    el.classList.toggle('on', on);
    el.disabled = o.disabled?.() ?? false;
  });
  return el;
}

// ------------------------------------------------------------------- slider
export interface SliderOpts extends Common {
  min: number; max: number; step: number; fmt?: (v: number) => string; log?: boolean; label?: string; ticks?: number[];
}

export function slider(b: Bind<number>, o: SliderOpts): HTMLElement {
  const log = !!o.log;
  const toPos = (v: number) => (log ? Math.round((1000 * Math.log(v / o.min)) / Math.log(o.max / o.min)) : v);
  const fromPos = (p: number) => {
    const v = log ? o.min * Math.pow(o.max / o.min, p / 1000) : p;
    return clamp(round(v, o.step), o.min, o.max);
  };
  const fmt = o.fmt ?? String;
  const input = h('input', {
    type: 'range', class: 'range', 'data-ctl': '', title: o.title, 'aria-label': o.label,
    min: log ? 0 : o.min, max: log ? 1000 : o.max, step: log ? 1 : o.step,
    oninput: () => b.set(fromPos(Number(input.value))),
  });
  const out = h('output', { class: 'range-out' });
  const root = h('div', { class: 'slider' }, input, out);
  reg(() => {
    const v = clamp(b.get(), o.min, o.max);
    const pos = toPos(v);
    if (Number(input.value) !== pos) input.value = String(pos);
    const lo = log ? 0 : o.min, hi = log ? 1000 : o.max;
    input.style.setProperty('--pct', `${((pos - lo) / (hi - lo)) * 100}%`);
    out.textContent = fmt(v);
    input.setAttribute('aria-valuetext', fmt(v));
    const dis = o.disabled?.() ?? false;
    input.disabled = dis;
    root.classList.toggle('disabled', dis);
  });
  return root;
}

/** A slider that can be switched off (value null) e.g. "Unlimited" / "Custom". */
export function optionalSlider(
  b: Bind<number | null>,
  o: SliderOpts & { offLabel: string; onLabel: string; initial: number },
): HTMLElement {
  let last = o.initial;
  const mode: Bind<string> = {
    get: () => (b.get() === null ? 'off' : 'on'),
    set: (v) => b.set(v === 'off' ? null : last),
  };
  const inner: Bind<number> = {
    get: () => { const v = b.get(); if (v !== null) last = v; return v ?? last; },
    set: (v) => { last = v; b.set(v); },
  };
  const seg = segmented(mode, [{ value: 'off', label: o.offLabel }, { value: 'on', label: o.onLabel }], { label: o.label });
  const sl = slider(inner, { ...o, disabled: () => b.get() === null });
  return h('div', { class: 'optional' }, seg, sl);
}

// ---------------------------------------------------------------- segmented
export interface SegOpt<T> { value: T; label: Child; title?: string; disabled?: boolean }

export function segmented<T extends string | number | boolean | null>(
  b: Bind<T>, opts: SegOpt<T>[], o: { label?: string; block?: boolean } = {},
): HTMLElement {
  const thumb = h('span', { class: 'seg-thumb', 'aria-hidden': 'true' });
  const btns = opts.map((op) => h('button', {
    type: 'button', class: 'seg-btn', role: 'radio', title: op.title, disabled: op.disabled,
    onclick: () => b.set(op.value),
  }, op.label));
  const root = h('div', {
    class: `segmented${o.block ? ' block' : ''}`, role: 'radiogroup', 'data-ctl': '', 'aria-label': o.label,
    onkeydown: (e: KeyboardEvent) => {
      const dir = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 0;
      if (!dir) return;
      e.preventDefault();
      let i = opts.findIndex((x) => x.value === b.get());
      for (let n = 0; n < opts.length; n++) {
        i = (i + dir + opts.length) % opts.length;
        if (!opts[i].disabled) break;
      }
      b.set(opts[i].value);
      btns[i].focus();
    },
  }, thumb, btns);
  const place = () => {
    const i = opts.findIndex((x) => x.value === b.get());
    const el = btns[i];
    if (!el || !el.offsetWidth) { thumb.style.opacity = '0'; return; }
    thumb.style.opacity = '1';
    thumb.style.width = `${el.offsetWidth}px`;
    thumb.style.transform = `translateX(${el.offsetLeft}px)`;
  };
  reg(() => {
    const cur = b.get();
    btns.forEach((el, i) => {
      const on = opts[i].value === cur;
      el.setAttribute('aria-checked', String(on));
      el.tabIndex = on || (!opts.some((x) => x.value === cur) && i === 0) ? 0 : -1;
      el.classList.toggle('on', on);
    });
    place();
  });
  new ResizeObserver(place).observe(root);
  return root;
}

// ------------------------------------------------------------------ stepper
export function stepper(b: Bind<number>, o: { min: number; max: number; step?: number; unit?: string; label?: string } & Common): HTMLElement {
  const step = o.step ?? 1;
  const input = h('input', {
    type: 'number', class: 'step-input', 'data-ctl': '', inputMode: 'numeric', min: o.min, max: o.max, step, title: o.title, 'aria-label': o.label,
    onchange: () => {
      const v = Number(input.value);
      if (Number.isFinite(v) && input.value !== '') b.set(clamp(round(v, step), o.min, o.max));
      input.value = String(b.get());
    },
    onkeydown: (e: KeyboardEvent) => {
      if (e.key === 'ArrowUp') { e.preventDefault(); bump(1); }
      if (e.key === 'ArrowDown') { e.preventDefault(); bump(-1); }
    },
  });
  const bump = (d: number) => b.set(clamp(round(b.get() + d * step, step), o.min, o.max));
  const dec = h('button', { type: 'button', class: 'step-btn', 'aria-label': 'Decrease', title: 'Decrease', onclick: () => bump(-1) }, '−');
  const inc = h('button', { type: 'button', class: 'step-btn', 'aria-label': 'Increase', title: 'Increase', onclick: () => bump(1) }, '+');
  reg(() => {
    const v = b.get();
    if (document.activeElement !== input) input.value = String(v);
    dec.disabled = v <= o.min;
    inc.disabled = v >= o.max;
    const dis = o.disabled?.() ?? false;
    input.disabled = dis;
    if (dis) { dec.disabled = true; inc.disabled = true; }
  });
  return h('div', { class: 'stepper' }, dec, input, o.unit && h('span', { class: 'step-unit' }, o.unit), inc);
}

// ------------------------------------------------------------------- select
export function select<T extends string>(b: Bind<T>, opts: { value: T; label: string }[], o: Common & { label?: string } = {}): HTMLElement {
  const el = h('select', {
    class: 'select', 'data-ctl': '', title: o.title, 'aria-label': o.label,
    onchange: () => b.set(el.value as T),
  }, opts.map((x) => h('option', { value: x.value }, x.label)));
  reg(() => { el.value = b.get(); el.disabled = o.disabled?.() ?? false; });
  return h('div', { class: 'select-wrap' }, el, icon('next', 14, 'select-caret'));
}

// --------------------------------------------------------------- text input
export function textInput(b: Bind<string>, o: Common & { max?: number; placeholder?: string; label?: string; cls?: string } = {}): HTMLInputElement {
  const el = h('input', {
    type: 'text', class: `input ${o.cls ?? ''}`, 'data-ctl': '', maxLength: o.max, placeholder: o.placeholder, title: o.title, 'aria-label': o.label,
    spellcheck: false, autocomplete: 'off',
    oninput: () => b.set(el.value),
  });
  reg(() => { if (document.activeElement !== el && el.value !== b.get()) el.value = b.get(); el.disabled = o.disabled?.() ?? false; });
  return el;
}

// ------------------------------------------------------------- colour picker
export const PALETTE = [
  '#22d3b0', '#2ee6c5', '#3b82ff', '#60a5fa', '#a78bfa', '#b15cff', '#ff7ab6', '#ef476f', '#ca3431', '#ff5d73',
  '#e58f2a', '#f59e0b', '#f7c631', '#ffd166', '#8cc152', '#6fbf4a', '#06d6a0', '#1bbfb0', '#5b8bb9', '#94a3b8',
  '#8d99ae', '#64748b', '#f4f8fc', '#101a26',
];

export function colorPicker(b: Bind<string>, o: Common & { label: string; palette?: string[] }): HTMLButtonElement {
  const btn = h('button', {
    type: 'button', class: 'swatch', 'data-ctl': '', title: o.title ?? `${o.label}: choose colour`, 'aria-label': o.label,
    'aria-haspopup': 'dialog', 'aria-expanded': 'false',
    onclick: () => openPopover(btn, () => build(), `${o.label} colour`),
  }, h('span', { class: 'swatch-chip' }));
  const build = (): HTMLElement => {
    const hex = h('input', {
      type: 'text', class: 'input mono', maxLength: 7, 'aria-label': 'Hex colour', spellcheck: false, value: b.get(),
      oninput: () => { const n = normHex(hex.value); if (n) { b.set(n); native.value = n; mark(); } },
      onblur: () => { hex.value = b.get(); },
    });
    const native = h('input', {
      type: 'color', class: 'native-color', 'aria-label': 'Custom colour', value: b.get(),
      oninput: () => { b.set(native.value); hex.value = native.value; mark(); },
    });
    const sws = (o.palette ?? PALETTE).map((c) => h('button', {
      type: 'button', class: 'pal', 'aria-label': c, title: c, style: { '--c': c },
      onclick: () => { b.set(c); hex.value = c; native.value = c; mark(); },
    }));
    const mark = () => sws.forEach((s) => s.setAttribute('aria-pressed', String(s.title === b.get())));
    mark();
    return h('div', { class: 'color-pop' }, h('div', { class: 'pal-grid' }, sws), h('div', { class: 'hex-row' }, hex, h('label', { class: 'native-wrap', title: 'Pick any colour' }, native, h('span', null, 'Custom'))));
  };
  reg(() => { btn.style.setProperty('--c', b.get()); btn.disabled = o.disabled?.() ?? false; });
  return btn;
}

// -------------------------------------------------------------- icon picker
export const ICON_CHOICES = [
  '!!', '!', '?!', '?', '??', '★', '✓', '✗', '+', '#', '×', '=', '→', '↗',
  '\u{1F44D}', '\u{1F525}', '\u{1F48E}', '⚡', '\u{1F3AF}', '✨', '\u{1F9E0}', '\u{1F480}', '⚠', '⚓', '\u{1F4CC}', '⚔',
  '\u{1F6E1}', '\u{1F451}', '\u{1F680}', '⛓', '⑂', '◐', '⟿', '♛', '♜', '♞', '♝', '♚', '♟',
];

export function iconPicker(b: Bind<string>, o: Common & { label: string }): HTMLButtonElement {
  const btn = h('button', {
    type: 'button', class: 'icon-btn', 'data-ctl': '', title: o.title ?? `${o.label}: choose icon`, 'aria-label': o.label,
    'aria-haspopup': 'dialog', 'aria-expanded': 'false',
    onclick: () => openPopover(btn, (close) => {
      const inp = h('input', {
        type: 'text', class: 'input', maxLength: 6, 'aria-label': 'Custom icon or text', placeholder: 'Type any emoji or text', value: b.get(), spellcheck: false,
        oninput: () => b.set(inp.value),
        onkeydown: (e: KeyboardEvent) => { if (e.key === 'Enter') close(); },
      });
      return h('div', { class: 'icon-pop' }, inp,
        h('div', { class: 'icon-grid' }, ICON_CHOICES.map((c) => h('button', {
          type: 'button', class: 'icon-choice', title: c, 'aria-label': c, onclick: () => { b.set(c); inp.value = c; close(); btn.focus(); },
        }, c))),
        h('button', { type: 'button', class: 'btn sm ghost', onclick: () => { b.set(''); close(); btn.focus(); } }, 'No icon'));
    }, `${o.label} picker`),
  });
  reg(() => { btn.textContent = b.get() || '∅'; btn.classList.toggle('empty', !b.get()); btn.disabled = o.disabled?.() ?? false; });
  return btn;
}

// ------------------------------------------------------------ hotkey input
const KEY_NAMES: Record<string, string> = {
  Space: 'Space', Enter: 'Enter', Tab: 'Tab', ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right',
  Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown', Insert: 'Insert', Delete: 'Delete',
  Minus: 'Minus', Equal: 'Equal', Comma: 'Comma', Period: 'Period', Slash: 'Slash', Semicolon: 'Semicolon', Quote: 'Quote',
  Backquote: 'Backquote', BracketLeft: 'BracketLeft', BracketRight: 'BracketRight', Backslash: 'Backslash',
};

function keyFromEvent(e: KeyboardEvent): string | null {
  if (/^Key[A-Z]$/.test(e.code)) return e.code.slice(3);
  if (/^Digit\d$/.test(e.code)) return e.code.slice(5);
  if (/^F\d{1,2}$/.test(e.code)) return e.code;
  return KEY_NAMES[e.code] ?? null;
}

const MAC_SYMBOLS: Record<string, string> = { Ctrl: '⌃', Alt: '⌥', Shift: '⇧', Cmd: '⌘', Up: '↑', Down: '↓', Left: '←', Right: '→', Enter: '↩' };

/** Render an accelerator as <kbd> keycaps. */
export function keycaps(accel: string): HTMLElement {
  const mac = isMac();
  const parts = accel.split('+');
  return h('span', { class: 'keycaps' }, parts.map((p) => h('kbd', null, mac ? MAC_SYMBOLS[p] ?? p : p === 'Cmd' ? 'Win' : p)));
}

export function hotkeyInput(key: keyof typeof DEFAULT_SETTINGS.hotkeys, o: { label: string }): HTMLElement {
  const b = at<string>(`hotkeys.${key}`);
  const msg = h('p', { class: 'field-error', role: 'alert' });
  const caps = h('span', { class: 'hk-caps' });
  const btn: HTMLButtonElement = h('button', {
    type: 'button', class: 'hk-btn', 'data-ctl': '', 'aria-label': o.label, title: 'Click, then press the new shortcut. Esc cancels.',
    onclick: () => { listening = true; msg.textContent = ''; render(); },
    onblur: () => { listening = false; render(); },
    onkeydown: (e: KeyboardEvent) => {
      if (!listening) return;
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); listening = false; msg.textContent = ''; render(); return; }
      if (e.key === 'Tab' && !e.ctrlKey && !e.altKey && !e.metaKey) return;
      e.preventDefault();
      if (['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) return; // wait for the main key
      const k = keyFromEvent(e);
      if (!k) { msg.textContent = 'That key is not supported as a hotkey.'; return; }
      const mods = [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', e.metaKey && 'Cmd'].filter(Boolean) as string[];
      const combo = normalizeHotkey([...mods, k].join('+'));
      const err = hotkeyError(combo);
      if (err) { msg.textContent = err; return; }
      const s = getSettings().hotkeys;
      const clash = (Object.keys(s) as (keyof typeof s)[]).find((o2) => o2 !== key && s[o2] === combo);
      if (clash) { msg.textContent = `Already used by another action (${combo}).`; return; }
      b.set(combo);
      msg.textContent = '';
      listening = false;
      render();
    },
  }, caps);
  let listening = false;
  const render = () => {
    btn.classList.toggle('listening', listening);
    caps.replaceChildren(listening ? h('span', { class: 'hk-prompt' }, 'Press shortcut…') : keycaps(b.get()));
  };
  const reset = h('button', {
    type: 'button', class: 'icon-only', title: 'Reset to default', 'aria-label': `Reset ${o.label} to default`,
    onclick: () => { b.set(DEFAULT_SETTINGS.hotkeys[key]); msg.textContent = ''; },
  }, icon('reset', 15));
  reg(() => { if (!listening) render(); reset.hidden = b.get() === DEFAULT_SETTINGS.hotkeys[key]; });
  return h('div', { class: 'hotkey' }, h('div', { class: 'hk-row' }, btn, reset), msg);
}

// ------------------------------------------------------------ corner picker
const CORNER_NAMES = { tl: 'Top left', tr: 'Top right', bl: 'Bottom left', br: 'Bottom right' } as const;

export function cornerPicker(b: Bind<'tl' | 'tr' | 'bl' | 'br'>): HTMLElement {
  const order = ['tl', 'tr', 'bl', 'br'] as const;
  const btns = order.map((c) => h('button', {
    type: 'button', class: `corner c-${c}`, role: 'radio', title: CORNER_NAMES[c], 'aria-label': CORNER_NAMES[c],
    onclick: () => b.set(c),
  }, h('span', { class: 'hud-block' })));
  const root = h('div', {
    class: 'corner-picker', role: 'radiogroup', 'data-ctl': '', 'aria-label': 'HUD corner',
    onkeydown: (e: KeyboardEvent) => {
      const i = order.indexOf(b.get());
      const col = i % 2, row = i >> 1;
      let n = i;
      if (e.key === 'ArrowRight' && col === 0) n = i + 1;
      else if (e.key === 'ArrowLeft' && col === 1) n = i - 1;
      else if (e.key === 'ArrowDown' && row === 0) n = i + 2;
      else if (e.key === 'ArrowUp' && row === 1) n = i - 2;
      else return;
      e.preventDefault();
      b.set(order[n]);
      btns[n].focus();
    },
  }, h('div', { class: 'cp-board' }), btns);
  reg(() => btns.forEach((el, i) => {
    const on = order[i] === b.get();
    el.setAttribute('aria-checked', String(on));
    el.tabIndex = on ? 0 : -1;
    el.classList.toggle('on', on);
  }));
  return root;
}

export const classOptions = [{ value: null as string | null, label: 'Show all' }, ...MOVE_CLASSES.map((c) => ({ value: c as string | null, label: c[0].toUpperCase() + c.slice(1) }))];

// ----------------------------------------------------------------- buttons
export function button(label: Child, onclick: (e: MouseEvent) => void, o: { cls?: string; title?: string; icon?: string; disabled?: boolean; type?: string } = {}): HTMLButtonElement {
  return h('button', { type: 'button', class: `btn ${o.cls ?? ''}`, title: o.title, disabled: o.disabled, onclick },
    o.icon && icon(o.icon, 16), h('span', null, label));
}

export { nextId };
