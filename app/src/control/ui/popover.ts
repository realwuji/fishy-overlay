// One-at-a-time anchored popover (colour/icon pickers). Closes on outside click, Escape, scroll, resize.
import { h } from '../dom';

interface Open { anchor: HTMLElement; pop: HTMLElement; close: () => void }
let current: Open | null = null;

export function closePopover(): void { current?.close(); }

export function openPopover(anchor: HTMLElement, build: (close: () => void) => HTMLElement, label: string): void {
  if (current?.anchor === anchor) { current.close(); return; }
  current?.close();

  const pop = h('div', { class: 'popover', role: 'dialog', 'aria-label': label });
  let closed = false;
  const close = () => {
    if (closed) return;
    closed = true;
    document.removeEventListener('pointerdown', onDown, true);
    document.removeEventListener('keydown', onKey, true);
    window.removeEventListener('resize', close);
    document.removeEventListener('scroll', onScroll, true);
    pop.remove();
    anchor.setAttribute('aria-expanded', 'false');
    if (current?.pop === pop) current = null;
  };
  const onDown = (e: Event) => {
    const t = e.target as Node;
    if (!pop.contains(t) && !anchor.contains(t)) close();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') { e.stopPropagation(); close(); anchor.focus(); }
  };
  const onScroll = (e: Event) => { if (!pop.contains(e.target as Node)) close(); };

  pop.appendChild(build(close));
  document.body.appendChild(pop);
  anchor.setAttribute('aria-expanded', 'true');

  const r = anchor.getBoundingClientRect();
  const pw = pop.offsetWidth, ph = pop.offsetHeight;
  const left = Math.min(Math.max(8, r.left), window.innerWidth - pw - 8);
  let top = r.bottom + 8;
  if (top + ph > window.innerHeight - 8) top = Math.max(8, r.top - ph - 8);
  pop.style.left = `${left}px`;
  pop.style.top = `${top}px`;

  document.addEventListener('pointerdown', onDown, true);
  document.addEventListener('keydown', onKey, true);
  window.addEventListener('resize', close);
  document.addEventListener('scroll', onScroll, true);
  current = { anchor, pop, close };
  (pop.querySelector<HTMLElement>('input, button') ?? pop).focus({ preventScroll: true });
}
