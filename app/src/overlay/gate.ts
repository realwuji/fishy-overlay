// Display rule: arrows, labels and HUD lines for a position are shown only once the search is deep
// enough (frame.depth >= target) or finished. Until then the UI shows "analysing n/target" and the
// previous position's arrows fade out. It never blocks forever: after PRELIM_MS the best available
// lines are shown, flagged as preliminary. Pure logic, no DOM.
import type { AnalysisFrame } from '../shared/types';

export type GateState = 'none' | 'pending' | 'preliminary' | 'ready';

export interface GateView {
  state: GateState;
  depth: number;
  target: number;
  nps: number;
  side: 'w' | 'b' | null;
}

export const PRELIM_MS = 2500;
const PRELIM_MIN_DEPTH = 6;

export const NO_VIEW: GateView = { state: 'none', depth: 0, target: 0, nps: 0, side: null };

export interface GateResult {
  /** Frame the arrows/labels/HUD may draw; null while pending. */
  show: AnalysisFrame | null;
  view: GateView;
  /** Re-evaluate after this many ms (the preliminary timeout), or null. */
  recheckMs: number | null;
}

export class DisplayGate {
  private shownFen: string | null = null;
  private seenFen: string | null = null;
  private seenAt = 0;

  evaluate(frame: AnalysisFrame | null, minDisplayDepth: number, now: number): GateResult {
    if (!frame) {
      this.shownFen = null;
      this.seenFen = null;
      return { show: null, view: NO_VIEW, recheckMs: null };
    }
    if (frame.fen !== this.seenFen) {
      this.seenFen = frame.fen;
      this.seenAt = now;
    }
    const target = minDisplayDepth <= 0 ? 0 : Math.max(0, Math.round(frame.targetDepth ?? minDisplayDepth));
    const view: GateView = {
      state: 'ready', depth: frame.depth, target, nps: frame.nps, side: frame.sideToMove,
    };
    const done = target <= 0 || !frame.thinking || frame.depth >= target || frame.fen === this.shownFen;
    if (done) {
      this.shownFen = frame.fen;
      return { show: frame, view, recheckMs: null };
    }
    const waited = now - this.seenAt;
    if (waited >= PRELIM_MS && frame.candidates.length > 0 && frame.depth >= Math.min(PRELIM_MIN_DEPTH, target)) {
      view.state = 'preliminary';
      return { show: frame, view, recheckMs: null };
    }
    view.state = 'pending';
    // Past the timeout the next engine frame re-evaluates; before it, wake up exactly at the deadline.
    return { show: null, view, recheckMs: waited < PRELIM_MS ? PRELIM_MS - waited + 5 : null };
  }
}
