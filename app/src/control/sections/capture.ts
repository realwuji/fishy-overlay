import { at, hotkeyInput, slider } from '../ui/controls';
import { callout, card, field, sectionShell } from '../ui/layout';

export function captureSection(): HTMLElement {
  return sectionShell(
    { id: 'capture', title: 'Capture & Hotkeys', desc: 'How often Fishy looks at the screen, and the keyboard shortcuts that control the overlay.', icon: 'capture', reset: ['capture', 'hotkeys'] },
    card({ title: 'Capture', desc: 'Frames are analysed in memory on this device and are never saved or sent anywhere.' },
      field('Capture rate', 'How many times per second Fishy checks your board for new moves. 6 to 10 is plenty; higher reacts faster but uses more CPU.',
        slider(at<number>('capture.fps'), { min: 1, max: 30, step: 1, fmt: (v) => `${v} fps` }))),
    card({ title: 'Global hotkeys', desc: 'Click a shortcut, then press the new combination. Esc cancels.' },
      field('Show / hide overlay', 'Instantly toggle everything Fishy draws.', hotkeyInput('toggleOverlay', { label: 'Toggle overlay hotkey' })),
      field('Show / hide HUD', 'Toggle only the info panel.', hotkeyInput('toggleHud', { label: 'Toggle HUD hotkey' })),
      field('Cycle arrow count', 'Step through 1 to 5 arrows without opening this window.', hotkeyInput('cycleArrows', { label: 'Cycle arrows hotkey' })),
      field('Re-calibrate board', 'Re-run board detection, for example after moving the browser window.', hotkeyInput('recalibrate', { label: 'Recalibrate hotkey' })),
      callout('info', 'Shortcuts need at least one of Ctrl, Alt or Cmd (F-keys can stand alone) so they never clash with typing.')));
}
