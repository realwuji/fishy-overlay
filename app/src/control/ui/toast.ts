import { h, icon } from '../dom';

let region: HTMLElement | null = null;

export interface ToastOpts { kind?: 'info' | 'ok' | 'warn' | 'error'; action?: { label: string; run: () => void } }

export function toast(message: string, o: ToastOpts = {}): void {
  if (!region) {
    region = h('div', { class: 'toasts', role: 'status', 'aria-live': 'polite' });
    document.body.appendChild(region);
  }
  const kind = o.kind ?? 'info';
  const el = h('div', { class: `toast toast-${kind}` },
    icon(kind === 'ok' ? 'check' : kind === 'info' ? 'info' : 'warn', 16),
    h('span', { class: 'toast-msg' }, message),
    o.action && h('button', { type: 'button', class: 'toast-act', onclick: () => { o.action!.run(); close(); } }, o.action.label),
  );
  const close = () => { el.classList.add('out'); window.setTimeout(() => el.remove(), 200); };
  region.appendChild(el);
  while (region.children.length > 3) region.firstElementChild?.remove();
  window.setTimeout(close, o.action ? 7000 : kind === 'error' ? 6000 : 3800);
}
