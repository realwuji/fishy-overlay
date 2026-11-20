import { DEFAULT_SETTINGS } from '../../shared/defaults';
import { MOVE_CLASSES, TACTIC_TAGS, type MoveClass, type TacticTag } from '../../shared/types';
import { h } from '../dom';
import { edit, getSettings } from '../store';
import { at, button, colorPicker, iconPicker, reg, segmented, slider, textInput, toggle } from '../ui/controls';
import { card, field, rowReset, sectionShell, undoable } from '../ui/layout';
import { jsonEqual } from '../sanitize';
import { clone } from '../dom';

const CLASS_HELP: Record<MoveClass, string> = {
  brilliant: 'A strong, hard-to-find move, often a sound sacrifice.',
  great: 'A move that changes the game or is the only good option.',
  best: 'The engine’s top choice.',
  excellent: 'Almost as good as the best move.',
  good: 'A solid move that keeps the position healthy.',
  inaccuracy: 'Gives away a small edge.',
  mistake: 'Hands over a clear advantage.',
  blunder: 'A serious error that may lose material or the game.',
};
const TAG_HELP: Record<TacticTag, string> = {
  fork: 'One piece attacks two or more targets at once.',
  pin: 'A piece cannot move without exposing a more valuable one.',
  skewer: 'A valuable piece is forced to move, exposing one behind it.',
  discovered: 'Moving one piece uncovers an attack from another.',
  hanging: 'A piece that is undefended and can be captured.',
  check: 'The move gives check.',
  mate: 'The move gives or forces checkmate.',
  capture: 'The move captures a piece.',
  promotion: 'A pawn reaches the last rank.',
  sacrifice: 'Material is given up for a bigger gain.',
  castle: 'The king castles.',
  enPassant: 'A pawn captures en passant.',
};
const pretty = (k: string) => k.replace(/([A-Z])/g, ' $1').replace(/^./, (c) => c.toUpperCase());

type Kind = 'classes' | 'tags';

function row(kind: Kind, key: string, help: string): HTMLElement {
  const base = `labels.${kind}.${key}`;
  const def = (DEFAULT_SETTINGS.labels[kind] as Record<string, unknown>)[key];
  const get = () => (getSettings().labels[kind] as Record<string, { enabled: boolean; label: string; color: string; icon: string }>)[key];

  const chipIcon = h('span', { class: 'chip-ico' });
  const chipLabel = h('span', { class: 'chip-label' });
  const chip = h('span', { class: 'chip-prev' }, chipIcon, chipLabel);
  const on = at<boolean>(`${base}.enabled`);
  const lab = textInput(at<string>(`${base}.label`), { max: 24, label: `${pretty(key)} label text`, placeholder: pretty(key), cls: 'tiny' });
  const clr = colorPicker(at<string>(`${base}.color`), { label: `${pretty(key)} colour` });
  const ico = iconPicker(at<string>(`${base}.icon`), { label: `${pretty(key)} icon` });
  const rst = rowReset(() => !jsonEqual(get(), def), () => edit((d) => { (d.labels[kind] as Record<string, unknown>)[key] = clone(def); }), `${pretty(key)}`);

  const el = h('div', { class: 'trow', title: help },
    toggle(on, { label: `Enable ${pretty(key)}` }),
    h('div', { class: 'tname' }, chip, h('span', { class: 'tdesc' }, help)),
    h('div', { class: 'tedit' }, lab, clr, ico),
    rst);
  reg(() => {
    const s = get();
    chip.style.setProperty('--c', s.color);
    chipIcon.textContent = s.icon;
    chipIcon.hidden = !s.icon;
    chipLabel.textContent = s.label || pretty(key);
    el.classList.toggle('off', !s.enabled);
  });
  return el;
}

function setAll(kind: Kind, enabled: boolean): void {
  undoable(enabled ? 'Everything enabled.' : 'Everything disabled.', () => edit((d) => {
    for (const v of Object.values(d.labels[kind])) (v as { enabled: boolean }).enabled = enabled;
  }));
}

export function labelsSection(): HTMLElement {
  const on = () => getSettings().labels.show;
  const tagsOn = () => on() && getSettings().labels.showTags;

  return sectionShell(
    { id: 'labels', title: 'Labels & Tags', desc: 'Name every move’s quality and call out tactics like forks and pins.', icon: 'labels', reset: ['labels'] },
    card({ title: 'Labels' },
      field('Show labels', 'Master switch for move-quality badges and tactic chips.', toggle(at('labels.show'))),
      field('Placement', 'Corner badge sits on the destination square. Tail and head attach to the arrow ends.',
        segmented(at<'cornerBadge' | 'arrowTail' | 'arrowHead'>('labels.placement'), [
          { value: 'cornerBadge', label: 'Corner badge' }, { value: 'arrowTail', label: 'Arrow tail' }, { value: 'arrowHead', label: 'Arrow head' },
        ]), { disabled: () => !on() }),
      field('Style', 'Icon only is compact; text only is clearest; both is the default.',
        segmented(at<'icon' | 'text' | 'iconText'>('labels.style'), [{ value: 'icon', label: 'Icon' }, { value: 'text', label: 'Text' }, { value: 'iconText', label: 'Icon + text' }]), { disabled: () => !on() }),
      field('Size', 'Scale label badges up for readability or down to reduce clutter.', slider(at<number>('labels.size'), { min: 0.6, max: 1.6, step: 0.05, fmt: (v) => `×${v.toFixed(2)}`, disabled: () => !on() }), { disabled: () => !on() }),
      field('Show tactic tags', 'Little chips such as Fork or Hanging next to a move.', toggle(at('labels.showTags')), { disabled: () => !on() }),
      field('Max tags per move', 'Limit how many chips appear on one move (0 hides them).', slider(at<number>('labels.maxTags'), { min: 0, max: 3, step: 1, disabled: () => !tagsOn() }), { disabled: () => !tagsOn() })),
    card({
      title: 'Move classes', desc: 'Every quality class can be switched off or restyled. Click a swatch or icon to edit.',
      actions: [
        button('All on', () => setAll('classes', true), { cls: 'sm ghost' }),
        button('All off', () => setAll('classes', false), { cls: 'sm ghost' }),
        button('Reset', () => resetKeysClasses('classes'), { cls: 'sm ghost', icon: 'reset', title: 'Restore class defaults' }),
      ],
    }, h('div', { class: 'ttable', role: 'group', 'aria-label': 'Move classes' }, MOVE_CLASSES.map((k) => row('classes', k, CLASS_HELP[k])))),
    card({
      title: 'Tactic tags', desc: 'Short chips that explain why a move is interesting.',
      actions: [
        button('All on', () => setAll('tags', true), { cls: 'sm ghost' }),
        button('All off', () => setAll('tags', false), { cls: 'sm ghost' }),
        button('Reset', () => resetKeysClasses('tags'), { cls: 'sm ghost', icon: 'reset', title: 'Restore tag defaults' }),
      ],
    }, h('div', { class: 'ttable', role: 'group', 'aria-label': 'Tactic tags' }, TACTIC_TAGS.map((k) => row('tags', k, TAG_HELP[k])))));
}

function resetKeysClasses(kind: Kind): void {
  undoable(kind === 'classes' ? 'Move classes reset to defaults.' : 'Tactic tags reset to defaults.', () => edit((d) => {
    (d.labels as unknown as Record<string, unknown>)[kind] = clone(DEFAULT_SETTINGS.labels[kind]);
  }));
}
