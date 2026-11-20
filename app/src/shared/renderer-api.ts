// CONTRACT FILE: implemented by src/overlay/renderer.ts (overlay owner),
// consumed by the overlay window AND the control window's live preview.
import type { AnalysisFrame, BoardGeometry, Settings } from './types';

export interface RendererInput {
  settings: Settings;
  frame: AnalysisFrame | null;
  geometry: BoardGeometry | null;
}

export interface FishyRenderer {
  /** Replace any part of the input; the renderer diffs and animates transitions. */
  update(partial: Partial<RendererInput>): void;
  /** Resize the backing canvas (CSS px) and handle devicePixelRatio. */
  resize(width: number, height: number): void;
  destroy(): void;
}

/** Overlay owner exports this from src/overlay/renderer.ts */
export type CreateRenderer = (canvas: HTMLCanvasElement, initial: RendererInput) => FishyRenderer;
