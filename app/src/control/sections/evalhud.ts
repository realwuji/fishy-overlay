import { h } from '../dom';
import { getSettings } from '../store';
import { at, cornerPicker, colorPicker, segmented, slider, toggle } from '../ui/controls';
import { card, field, sectionShell } from '../ui/layout';

const pct = (v: number) => `${Math.round(v * 100)}%`;

export function evalHudSection(): HTMLElement {
  const eOn = () => getSettings().evalBar.show;
  const hOn = () => getSettings().hud.show;
  return sectionShell(
    { id: 'evalhud', title: 'Eval bar & HUD', desc: 'The evaluation bar beside the board and the info panel in a screen corner.', icon: 'evalhud', reset: ['evalBar', 'hud'] },
    card({ title: 'Eval bar', desc: 'A vertical bar showing who is ahead.' },
      field('Show eval bar', 'White fills from the bottom when White is better.', toggle(at('evalBar.show'))),
      field('Position', 'Which side of the board the bar sits on.', segmented(at<'left' | 'right'>('evalBar.position'), [{ value: 'left', label: 'Left' }, { value: 'right', label: 'Right' }]), { disabled: () => !eOn() }),
      field('Width', 'Thickness of the bar in pixels.', slider(at<number>('evalBar.width'), { min: 6, max: 40, step: 1, fmt: (v) => `${v} px`, disabled: () => !eOn() }), { disabled: () => !eOn() }),
      field('Show number', 'Print the evaluation (like +0.8 or M3) on the bar.', toggle(at('evalBar.showNumber')), { disabled: () => !eOn() }),
      field('Style', 'Gradient is softer; solid has a hard edge.', segmented(at<'solid' | 'gradient'>('evalBar.style'), [{ value: 'gradient', label: 'Gradient' }, { value: 'solid', label: 'Solid' }]), { disabled: () => !eOn() }),
      field('Animate', 'Glide smoothly when the evaluation changes.', toggle(at('evalBar.animate')), { disabled: () => !eOn() }),
      field('Colours', 'White’s side and Black’s side of the bar.',
        h('div', { class: 'inline-row' },
          h('label', { class: 'mini-label' }, 'White', colorPicker(at<string>('evalBar.whiteColor'), { label: 'Eval bar White colour', disabled: () => !eOn() })),
          h('label', { class: 'mini-label' }, 'Black', colorPicker(at<string>('evalBar.blackColor'), { label: 'Eval bar Black colour', disabled: () => !eOn() }))), { disabled: () => !eOn() })),
    card({ title: 'HUD', desc: 'A compact panel with the engine’s thoughts.' },
      field('Show HUD', 'Toggle the panel. You can also use the HUD hotkey.', toggle(at('hud.show'))),
      field('Corner', 'Where the HUD sits on the board.', cornerPicker(at('hud.corner')), { disabled: () => !hOn() }),
      field('Lines', 'How many engine lines are listed.', slider(at<number>('hud.lines'), { min: 1, max: 5, step: 1, disabled: () => !hOn() }), { disabled: () => !hOn() }),
      field('Show best line (PV)', 'The sequence of moves the engine expects.', toggle(at('hud.showPv')), { disabled: () => !hOn() }),
      field('Show explanation', 'A plain-language reason for the top move, when available.', toggle(at('hud.showExplanation')), { disabled: () => !hOn() }),
      field('Show depth', 'How deep the engine has searched. Higher is more reliable.', toggle(at('hud.showDepth')), { disabled: () => !hOn() }),
      field('Show win %', 'Expected score converted to a percentage.', toggle(at('hud.showWinPct')), { disabled: () => !hOn() }),
      field('Opacity', 'See-through HUDs get out of the way.', slider(at<number>('hud.opacity'), { min: 0.2, max: 1, step: 0.02, fmt: pct, disabled: () => !hOn() }), { disabled: () => !hOn() }),
      field('Scale', 'Make the HUD bigger or smaller.', slider(at<number>('hud.scale'), { min: 0.6, max: 1.6, step: 0.05, fmt: (v) => `×${v.toFixed(2)}`, disabled: () => !hOn() }), { disabled: () => !hOn() }),
      field('Compact', 'Tighter spacing and smaller type.', toggle(at('hud.compact')), { disabled: () => !hOn() })));
}
