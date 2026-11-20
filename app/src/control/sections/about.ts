import { h, icon } from '../dom';
import { getSettings } from '../store';
import { button, reg } from '../ui/controls';
import { callout, card, sectionShell } from '../ui/layout';
import { openFairPlay } from '../ui/modal';

export function aboutSection(): HTMLElement {
  const status = h('p', { class: 'ack' });
  reg(() => {
    const ok = getSettings().acceptedFairPlay;
    status.replaceChildren(icon(ok ? 'check' : 'warn', 16), ok ? 'You have acknowledged the fair-play notice on this device.' : 'You have not acknowledged the fair-play notice yet.');
    status.classList.toggle('ok', ok);
  });
  return sectionShell(
    { id: 'about', title: 'About & Fair play', desc: 'What Fishy is for, and what it must never be used for.', icon: 'about' },
    card({ cls: 'fair' },
      h('div', { class: 'fair-head' }, h('img', { src: '/logo.svg', alt: '', width: 56, height: 56 }), h('div', null, h('h3', null, 'Fishy is a study tool'), h('p', { class: 'help' }, 'Stockfish analysis, drawn on top of any chess board you can see.'))),
      h('div', { class: 'two-col' },
        h('div', { class: 'do' }, h('h4', null, icon('check', 16), 'Use it for'), h('ul', null,
          h('li', null, 'Studying and reviewing games'), h('li', null, 'Analysis boards and opening prep'), h('li', null, 'Puzzles and training positions'),
          h('li', null, 'Playing against bots'), h('li', null, 'Lessons and coaching'))),
        h('div', { class: 'dont' }, h('h4', null, icon('x', 16), 'Never use it for'), h('ul', null,
          h('li', null, 'Help during live games against other people'), h('li', null, 'Rated, unrated or tournament games with human opponents'),
          h('li', null, 'Anything that breaks a site’s fair-play rules')))),
      callout('warn', h('strong', null, 'Fair play matters. '), 'Getting engine help in a live game against another person is cheating. Chess sites ban for it, and it ruins the game for your opponent. Fishy does not hide itself and it never clicks or moves anything for you.'),
      status,
      button('Review the fair-play notice', () => openFairPlay(false), { cls: 'sm' })),
    card({ title: 'Privacy & safety' },
      h('ul', { class: 'checks' },
        h('li', null, icon('lock', 16), 'Screen frames stay in memory on this device. They are never written to disk or sent over a network.'),
        h('li', null, icon('lock', 16), 'No telemetry. The only download is Stockfish’s network file during the engine build.'),
        h('li', null, icon('lock', 16), 'Fishy never sends mouse or keyboard input to other apps. It only draws.'),
        h('li', null, icon('lock', 16), 'The overlay is a normal window. It does not try to hide from screen capture or sharing.'))),
    card({ title: 'Fishy' }, h('p', { class: 'help' }, 'Version 0.1.0. Built on Stockfish, a free and open-source chess engine (GPLv3). First target: chess.com, with more sites and board styles to follow.')));
}
