import { h, icon } from '../dom';
import { edit } from '../store';

let open: HTMLElement | null = null;

/** Fair-play notice. firstRun = must be acknowledged (sets settings.acceptedFairPlay) before Start is enabled. */
export function openFairPlay(firstRun: boolean): void {
  if (open) return;
  const app = document.getElementById('app');
  const prevFocus = document.activeElement as HTMLElement | null;

  const check = h('input', { type: 'checkbox', id: 'fp-ack', onchange: () => { accept.disabled = !check.checked; } });
  const accept = h('button', {
    type: 'button', class: 'btn primary', disabled: firstRun,
    onclick: () => { if (firstRun) edit((d) => { d.acceptedFairPlay = true; }); close(); },
  }, firstRun ? 'I understand, continue' : 'Close');

  const dialog = h('div', { class: 'modal', role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': 'fp-title', 'aria-describedby': 'fp-desc' },
    h('img', { class: 'modal-logo', src: '/logo.svg', alt: '', width: 64, height: 64 }),
    h('h2', { id: 'fp-title' }, 'Before you start: fair play'),
    h('p', { id: 'fp-desc', class: 'modal-lead' }, 'Fishy shows live Stockfish analysis on top of any chess board on your screen. It is built for learning.'),
    h('div', { class: 'two-col' },
      h('div', { class: 'do' }, h('h4', null, icon('check', 16), 'Great for'), h('ul', null,
        h('li', null, 'Study and game review'), h('li', null, 'Analysis boards'), h('li', null, 'Puzzles'), h('li', null, 'Playing bots'), h('li', null, 'Lessons and coaching'))),
      h('div', { class: 'dont' }, h('h4', null, icon('x', 16), 'Never for'), h('ul', null,
        h('li', null, 'Live games against other people'), h('li', null, 'Anything that breaks a site’s fair-play rules')))),
    h('p', { class: 'modal-note' }, 'Using engine help in a live game against another person is cheating and ruins the game for them. Fishy does not hide itself and never moves pieces or clicks for you.'),
    firstRun && h('label', { class: 'check', for: 'fp-ack' }, check, h('span', null, 'I will only use Fishy for study, analysis, puzzles, bots, lessons or coaching, and never to get help in live games against other people.')),
    h('div', { class: 'modal-actions' }, accept));

  const backdrop = h('div', { class: 'modal-backdrop' }, dialog);
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape' && !firstRun) { e.stopPropagation(); close(); }
    if (e.key === 'Tab') {
      const f = Array.from(dialog.querySelectorAll<HTMLElement>('button:not(:disabled), input')).filter((x) => x.offsetParent !== null);
      if (!f.length) return;
      const first = f[0], last = f[f.length - 1];
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
    }
  };
  function close() {
    document.removeEventListener('keydown', onKey, true);
    backdrop.classList.add('out');
    window.setTimeout(() => backdrop.remove(), 180);
    app?.removeAttribute('inert');
    open = null;
    prevFocus?.focus?.();
  }
  document.addEventListener('keydown', onKey, true);
  document.body.appendChild(backdrop);
  app?.setAttribute('inert', '');
  open = backdrop;
  (firstRun ? check : accept).focus();
}
