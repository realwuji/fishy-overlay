// Tiny DOM helpers: h() element factory, SVG icons, misc utilities. No framework.

export type Child = Node | string | number | null | undefined | false | Child[];
type Props = Record<string, unknown> | null | undefined;

function append(parent: Node, children: Child[]): void {
  for (const c of children) {
    if (c == null || c === false) continue;
    if (Array.isArray(c)) append(parent, c);
    else parent.appendChild(c instanceof Node ? c : document.createTextNode(String(c)));
  }
}

/** Create an element: h('button', { class: 'btn', onclick: fn }, 'Label'). */
export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props?: Props,
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  if (props) {
    for (const [k, v] of Object.entries(props)) {
      if (v == null) continue;
      if (k === 'class') el.className = String(v);
      else if (k === 'style') {
        if (typeof v === 'string') el.setAttribute('style', v);
        else for (const [sk, sv] of Object.entries(v as Record<string, string>)) el.style.setProperty(sk, sv);
      } else if (k.startsWith('on') && typeof v === 'function') {
        el.addEventListener(k.slice(2).toLowerCase(), v as EventListener);
      } else if (k.startsWith('aria-') || k.startsWith('data-') || k === 'role') {
        el.setAttribute(k, String(v));
      } else if (k === 'for') (el as unknown as HTMLLabelElement).htmlFor = String(v);
      else if (k in el) (el as unknown as Record<string, unknown>)[k] = v;
      else if (v === true) el.setAttribute(k, '');
      else if (v !== false) el.setAttribute(k, String(v));
    }
  }
  append(el, children);
  return el;
}

const NS = 'http://www.w3.org/2000/svg';

const ICONS: Record<string, string> = {
  overview: '<rect x="3" y="3" width="7" height="9" rx="2"/><rect x="14" y="3" width="7" height="5" rx="2"/><rect x="14" y="12" width="7" height="9" rx="2"/><rect x="3" y="16" width="7" height="5" rx="2"/>',
  arrows: '<path d="M7 17 17 7"/><path d="M8 7h9v9"/>',
  labels: '<path d="M20.6 13.4 13.4 20.6a2 2 0 0 1-2.8 0L3 13V3h10l7.6 7.6a2 2 0 0 1 0 2.8Z"/><circle cx="7.5" cy="7.5" r="1.2"/>',
  evalhud: '<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M8 4v16"/><path d="M12 15h6M12 11h4"/>',
  engine: '<rect x="6" y="6" width="12" height="12" rx="2.5"/><rect x="9.5" y="9.5" width="5" height="5" rx="1"/><path d="M9 2v3M15 2v3M9 19v3M15 19v3M2 9h3M2 15h3M19 9h3M19 15h3"/>',
  capture: '<rect x="3" y="4" width="18" height="12" rx="2.5"/><path d="M8 20h8M12 16v4"/><path d="M7.5 8.5V8h3M16.5 11.5V12h-3"/>',
  appearance: '<circle cx="12" cy="12" r="9"/><path d="M12 3a9 9 0 0 0 0 18Z" fill="currentColor" stroke="none"/>',
  presets: '<path d="M4 7h9M17 7h3M4 17h3M11 17h9"/><circle cx="15" cy="7" r="2"/><circle cx="9" cy="17" r="2"/>',
  about: '<path d="M12 3 4.5 6v5.5c0 4.4 3.1 8.1 7.5 9.5 4.4-1.4 7.5-5.1 7.5-9.5V6Z"/><path d="m9 12 2.2 2.2L15.5 10"/>',
  refresh: '<path d="M20 11a8 8 0 0 0-14.5-4M4 4v4h4"/><path d="M4 13a8 8 0 0 0 14.5 4M20 20v-4h-4"/>',
  reset: '<path d="M4 12a8 8 0 1 0 2.6-5.9"/><path d="M4 4v5h5"/>',
  play: '<path d="m7 4.5 12 7.5-12 7.5Z" fill="currentColor"/>',
  pause: '<rect x="6" y="5" width="4" height="14" rx="1" fill="currentColor"/><rect x="14" y="5" width="4" height="14" rx="1" fill="currentColor"/>',
  stop: '<rect x="6" y="6" width="12" height="12" rx="2.5" fill="currentColor"/>',
  prev: '<path d="m15 5-7 7 7 7"/>',
  next: '<path d="m9 5 7 7-7 7"/>',
  flip: '<path d="M7 4v14M7 18l-3-3M7 18l3-3M17 20V6M17 6l-3 3M17 6l3 3"/>',
  target: '<circle cx="12" cy="12" r="8"/><circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3"/>',
  check: '<path d="m5 12.5 4.5 4.5L19 7.5"/>',
  copy: '<rect x="9" y="9" width="11" height="11" rx="2.5"/><path d="M5 15V6a2 2 0 0 1 2-2h8"/>',
  download: '<path d="M12 4v11M7.5 10.5 12 15l4.5-4.5M5 20h14"/>',
  upload: '<path d="M12 16V5M7.5 9.5 12 5l4.5 4.5M5 20h14"/>',
  warn: '<path d="M12 3.5 2.8 19.5h18.4Z"/><path d="M12 10v4.5M12 17.2v.1"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v5.5M12 7.8v.1"/>',
  lock: '<rect x="5" y="10.5" width="14" height="10" rx="2.5"/><path d="M8 10.5V8a4 4 0 0 1 8 0v2.5"/>',
  external: '<path d="M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5"/>',
  sparkle: '<path d="m12 3 1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8Z"/><path d="m19 16 .7 2 2 .7-2 .7-.7 2-.7-2-2-.7 2-.7Z"/>',
  x: '<path d="m6 6 12 12M18 6 6 18"/>',
};

export function icon(name: string, size = 18, cls = ''): SVGSVGElement {
  const s = document.createElementNS(NS, 'svg');
  s.setAttribute('viewBox', '0 0 24 24');
  s.setAttribute('width', String(size));
  s.setAttribute('height', String(size));
  s.setAttribute('fill', 'none');
  s.setAttribute('stroke', 'currentColor');
  s.setAttribute('stroke-width', '1.8');
  s.setAttribute('stroke-linecap', 'round');
  s.setAttribute('stroke-linejoin', 'round');
  s.setAttribute('aria-hidden', 'true');
  s.setAttribute('focusable', 'false');
  if (cls) s.setAttribute('class', cls);
  s.innerHTML = ICONS[name] ?? '';
  return s;
}

/** Loading indicator: a spinning ring, or three static dots when motion is reduced (see styles.css). */
export function spinner(size = 16): HTMLSpanElement {
  const el = document.createElement('span');
  el.className = 'spin';
  el.setAttribute('aria-hidden', 'true');
  el.style.setProperty('--sz', `${size}px`);
  el.innerHTML = '<i class="ring"></i><i class="dots"><b></b><b></b><b></b></i>';
  return el;
}

export const clamp = (v: number, lo: number, hi: number): number => Math.min(hi, Math.max(lo, v));

export const isMac = (): boolean => /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

export function debounce<A extends unknown[]>(fn: (...a: A) => void, ms: number): ((...a: A) => void) & { flush(): void } {
  let t: number | undefined;
  let last: A | undefined;
  const run = () => { t = undefined; if (last) { const a = last; last = undefined; fn(...a); } };
  const d = ((...a: A) => { last = a; if (t !== undefined) clearTimeout(t); t = window.setTimeout(run, ms); }) as ((...a: A) => void) & { flush(): void };
  d.flush = () => { if (t !== undefined) { clearTimeout(t); run(); } };
  return d;
}

export const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

let uid = 0;
export const nextId = (p: string): string => `${p}-${++uid}`;
