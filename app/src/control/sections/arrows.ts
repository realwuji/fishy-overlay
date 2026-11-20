import { h } from '../dom';
import { navigate } from '../nav';
import { getSettings } from '../store';
import { at, button, classOptions, colorPicker, reg, segmented, select, slider, toggle } from '../ui/controls';
import { callout, card, field, sectionShell } from '../ui/layout';
import type { ArrowStyle, MoveClass } from '../../shared/types';

const minClassBind = (() => {
  const b = at<MoveClass | null>('arrows.minClass');
  return { get: () => b.get() ?? 'none', set: (v: string) => b.set(v === 'none' ? null : (v as MoveClass)) };
})();

const pct = (v: number) => `${Math.round(v * 100)}%`;

export function arrowsSection(): HTMLElement {
  const on = () => getSettings().arrows.show;

  const warn = callout('warn',
    h('strong', null, 'Not enough engine lines. '),
    'Arrows are drawn from the engine’s MultiPV lines, and Engine is set lower than your arrow count. ',
    button('Open Engine settings', () => navigate('engine'), { cls: 'sm' }));
  reg(() => { const s = getSettings(); warn.hidden = s.engine.multiPv >= s.arrows.count || !s.arrows.show; });

  const ranks = ['1st', '2nd', '3rd', '4th', '5th'].map((r, i) => h('div', { class: 'rank-sw' },
    colorPicker(at<string>(`arrows.rankColors.${i}`), { label: `${r} best move colour`, disabled: () => getSettings().arrows.colorMode !== 'rank' }),
    h('span', null, r)));
  ranks.forEach((el, i) => reg(() => el.classList.toggle('dim', i >= getSettings().arrows.count)));

  const classNote = h('p', { class: 'help inline-note' },
    'Arrows take the colour of each move’s quality class. ',
    h('button', { type: 'button', class: 'link', onclick: () => navigate('labels') }, 'Edit class colours'));
  reg(() => { classNote.hidden = getSettings().arrows.colorMode !== 'classification'; });

  return sectionShell(
    { id: 'arrows', title: 'Arrows', desc: 'Choose how many candidate moves are drawn and how they look.', icon: 'arrows', reset: ['arrows'] },
    card({ title: 'Candidate arrows' },
      field('Show arrows', 'Draw the engine’s best moves as arrows on your board.', toggle(at('arrows.show'))),
      field('How many arrows', 'From 1 (just the best move) up to 5 candidates. Fewer arrows is easier to read.', slider(at<number>('arrows.count'), { min: 1, max: 5, step: 1, fmt: (v) => `${v} arrow${v > 1 ? 's' : ''}`, disabled: () => !on() }), { disabled: () => !on() }),
      warn,
      field('Show moves for', 'Which side’s moves get arrows. “Bottom” follows whoever sits at the bottom of your screen.',
        segmented(at<'sideToMove' | 'white' | 'black' | 'bottom'>('arrows.forSide'), [
          { value: 'sideToMove', label: 'Side to move' }, { value: 'white', label: 'White' }, { value: 'black', label: 'Black' }, { value: 'bottom', label: 'Bottom' },
        ]), { disabled: () => !on() }),
      field('Minimum move quality', 'Hide arrows for moves worse than this class. “Show all” keeps every candidate.',
        select<string>(minClassBind, classOptions.map((o) => ({ value: (o.value ?? 'none') as string, label: o.label })), { label: 'Minimum class' }),
        { disabled: () => !on() })),
    card({ title: 'Style' },
      field('Arrow style', 'Classic is a solid arrow, Slim is a thin line, Dotted is dashed, Pill is a rounded bar.',
        segmented(at<ArrowStyle>('arrows.style'), [
          { value: 'classic', label: 'Classic' }, { value: 'slim', label: 'Slim' }, { value: 'dotted', label: 'Dotted' }, { value: 'pill', label: 'Pill' },
        ]), { disabled: () => !on() }),
      field('Curve knight moves', 'Draw knight moves as an L-shape that follows the real path instead of a straight line.', toggle(at('arrows.curveKnightMoves')), { disabled: () => !on() }),
      field('Opacity', 'Lower this to see the pieces underneath.', slider(at<number>('arrows.opacity'), { min: 0.1, max: 1, step: 0.05, fmt: pct, disabled: () => !on() }), { disabled: () => !on() }),
      field('Thickness', 'Scale the arrow width up or down.', slider(at<number>('arrows.widthScale'), { min: 0.5, max: 1.6, step: 0.05, fmt: (v) => `×${v.toFixed(2)}`, disabled: () => !on() }), { disabled: () => !on() }),
      field('Glow', 'A soft halo that helps arrows stand out on busy boards.', toggle(at('arrows.glow')), { disabled: () => !on() }),
      field('Animate', 'Fade and slide arrows when the best moves change.', toggle(at('arrows.animate')), { disabled: () => !on() })),
    card({ title: 'Colours', desc: 'Pick how arrows are coloured.' },
      field('Colour by', 'Rank: the best move is always the same colour. Classification: colour reflects move quality.',
        segmented(at<'rank' | 'classification'>('arrows.colorMode'), [{ value: 'rank', label: 'Rank' }, { value: 'classification', label: 'Classification' }]), { disabled: () => !on() }),
      field('Rank colours', 'Dimmed swatches are ranks beyond your arrow count.', h('div', { class: 'rank-row' }, ranks), { stack: true }),
      classNote));
}
