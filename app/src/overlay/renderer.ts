// Fishy overlay renderer: a zero-dependency, canvas based painter.
//
// Layers (back to front): eval bar, candidate arrows, class badges + tactic chips, HUD.
// Everything is driven by springs; a requestAnimationFrame loop only runs while
// something is moving or the scene is dirty, so an idle overlay costs no CPU.
import type { Corner } from '../shared/types';
import type { CreateRenderer, FishyRenderer, RendererInput } from '../shared/renderer-api';
import { Spring, clamp as clamp01x } from './anim';
const clamp01 = (v: number): number => clamp01x(v, 0, 1);
import { ArrowLayer } from './arrows';
import { Env, Rect, makePalette } from './env';
import { EvalBar } from './evalbar';
import { Hud } from './hud';
import { DisplayGate, NO_VIEW, type GateView } from './gate';
import type { AnalysisFrame } from '../shared/types';

export type { Rect } from './env';
export type { GateView } from './gate';

/** The renderer plus the extras the overlay window needs (HUD editing). */
export interface OverlayRenderer extends FishyRenderer {
  /** Current animated HUD rectangle in CSS px, or null when the HUD is hidden. */
  hudBounds(): Rect | null;
  /** Called whenever the HUD rectangle moves or resizes. */
  onLayout(cb: (hud: Rect | null) => void): void;
  /** Show or hide the dashed edit outline around the HUD. */
  hudHover(on: boolean): void;
  hudDragStart(): void;
  /** Move the HUD so its top-left corner is at (x, y). */
  hudDragMove(x: number, y: number): void;
  /** Finish dragging; returns the corner the HUD should snap to. */
  hudDragEnd(): Corner;
  /** Display-rule state (pending / preliminary / ready) for the status chip. Fires only on change. */
  onView(cb: (v: GateView) => void): void;
}

const geometryEqual = (a: RendererInput['geometry'], b: RendererInput['geometry']): boolean =>
  a === b || (!!a && !!b && a.x === b.x && a.y === b.y && a.size === b.size && a.flipped === b.flipped);

export function createOverlayRenderer(canvas: HTMLCanvasElement, initial: RendererInput): OverlayRenderer {
  const ctx = canvas.getContext('2d');
  let input: RendererInput = { ...initial };
  let dpr = 1;
  let raf = 0;
  let last = 0;
  let destroyed = false;
  let flipped = false;
  let hasGeometry = false;
  let layoutCb: ((r: Rect | null) => void) | null = null;
  let lastHud: Rect | null = null;
  const gate = new DisplayGate();
  let view: GateView = NO_VIEW;
  let viewCb: ((v: GateView) => void) | null = null;
  let recheck = 0;
  let barFrame: AnalysisFrame | null = null;
  let motion = true;
  let lastDraw = 0;

  const gx = new Spring(0, 26, 1, 0.04);
  const gy = new Spring(0, 26, 1, 0.04);
  const gs = new Spring(0, 26, 1, 0.04);
  const vis = new Spring(0, 14, 1, 0.004);

  const board = { x: 0, y: 0, s: 0, flipped: false };
  const arrows = new ArrowLayer();
  const bar = new EvalBar();
  const hud = new Hud();

  const scheme = typeof window.matchMedia === 'function' ? window.matchMedia('(prefers-color-scheme: dark)') : null;
  const env: Env = {
    ctx: ctx as CanvasRenderingContext2D,
    dpr: 1,
    W: 0,
    H: 0,
    pal: makePalette(input.settings.theme, scheme ? scheme.matches : true),
    t: 0,
  };

  const schedule = (): void => {
    if (!raf && !destroyed) raf = requestAnimationFrame(tick);
  };
  const markDirty = (): void => {
    schedule();
  };

  function applyInput(): void {
    if (!ctx) return;
    const st = input.settings;
    const animate = !st.theme.reduceMotion;
    env.pal = makePalette(st.theme, scheme ? scheme.matches : true);

    const g = input.geometry;
    if (g) {
      const jump = !hasGeometry || vis.x < 0.05 || !animate;
      gx.set(g.x, !jump);
      gy.set(g.y, !jump);
      gs.set(g.size, !jump);
      if (jump) { gx.snap(g.x); gy.snap(g.y); gs.snap(g.size); }
      flipped = g.flipped;
      hasGeometry = true;
    }
    vis.set(g ? 1 : 0, animate);

    motion = animate;

    // Display rule: new-position arrows/lines wait for the target depth (or the preliminary timeout).
    const ev = gate.evaluate(input.frame, st.engine.minDisplayDepth ?? 12, performance.now());
    const pending = ev.view.state === 'pending';
    const prelim = ev.view.state === 'preliminary';
    if (ev.show) barFrame = ev.show;
    else if (ev.view.state === 'none') barFrame = null;
    const frac = ev.view.target > 0 ? clamp01(ev.view.depth / ev.view.target) : 0;
    arrows.sync(env, ev.show, g, st, animate);
    bar.update(barFrame, st.evalBar, animate);
    bar.setLoading(pending || prelim, frac, animate);
    hud.update(env, ev.show, st, animate, pending || prelim ? { depth: ev.view.depth, target: ev.view.target, nps: ev.view.nps, side: ev.view.side, frac } : null, prelim);
    if (view.state !== ev.view.state || view.depth !== ev.view.depth || view.target !== ev.view.target) {
      view = ev.view;
      viewCb?.(view);
    } else {
      view = ev.view;
    }
    window.clearTimeout(recheck);
    recheck = 0;
    if (ev.recheckMs !== null) recheck = window.setTimeout(applyInput, ev.recheckMs);
    markDirty();
  }

  function setupCanvas(): void {
    dpr = Math.max(1, window.devicePixelRatio || 1);
    env.dpr = dpr;
    const w = Math.max(1, Math.round(env.W * dpr));
    const h = Math.max(1, Math.round(env.H * dpr));
    if (canvas.width !== w) canvas.width = w;
    if (canvas.height !== h) canvas.height = h;
    canvas.style.width = `${env.W}px`;
    canvas.style.height = `${env.H}px`;
  }

  function stepAll(dt: number): boolean {
    const a = gx.step(dt);
    const b = gy.step(dt);
    const c = gs.step(dt);
    const d = vis.step(dt);
    const e = arrows.step(dt);
    const f = bar.step(dt);
    const h = hud.step(dt);
    return a || b || c || d || e || f || h;
  }

  function draw(): void {
    if (!ctx || env.W === 0) return;
    if ((window.devicePixelRatio || 1) !== dpr) setupCanvas();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, env.W, env.H);
    const v = vis.x;
    if (v >= 0.004) {
      ctx.textBaseline = 'middle';
      bar.draw(env, gx.x, gy.x, gs.x, flipped, v);
      const hb = hud.visible ? hud.bounds() : null;
      board.x = gx.x; board.y = gy.x; board.s = gs.x; board.flipped = flipped;
      arrows.draw(env, board, v, hb, bar.visible ? bar.rect(env, gx.x, gy.x, gs.x) : null);
      hud.drawGuides(env, v);
      hud.draw(env, v);
    }
    if (layoutCb) {
      const b = hud.bounds();
      const same =
        (b === null && lastHud === null) ||
        (b !== null && lastHud !== null &&
          Math.abs(b.x - lastHud.x) < 0.25 && Math.abs(b.y - lastHud.y) < 0.25 &&
          Math.abs(b.w - lastHud.w) < 0.25 && Math.abs(b.h - lastHud.h) < 0.25);
      if (!same) {
        lastHud = b ? { ...b } : null;
        layoutCb(b);
      }
    }
  }

  function tick(ts: number): void {
    raf = 0;
    if (destroyed) return;
    const dt = last ? Math.min(0.05, Math.max(0.001, (ts - last) / 1000)) : 1 / 60;
    last = ts;
    env.t = ts / 1000;
    const springs = stepAll(dt);
    const shimmer = motion && (bar.loading || hud.loading);
    // Shimmer alone needs no more than ~30 fps; springs keep full rate.
    if (!springs && shimmer && ts - lastDraw < 30) {
      schedule();
      return;
    }
    lastDraw = ts;
    draw();
    if (springs || shimmer) schedule();
    else last = 0;
  }

  const onScheme = (): void => {
    if (input.settings.theme.mode === 'system') applyInput();
  };
  scheme?.addEventListener?.('change', onScheme);

  applyInput();

  return {
    update(partial: Partial<RendererInput>): void {
      let changed = false;
      if (partial.settings && partial.settings !== input.settings) {
        input = { ...input, settings: partial.settings };
        changed = true;
      }
      if ('frame' in partial && partial.frame !== input.frame) {
        input = { ...input, frame: partial.frame ?? null };
        changed = true;
      }
      if ('geometry' in partial && !geometryEqual(partial.geometry ?? null, input.geometry)) {
        input = { ...input, geometry: partial.geometry ?? null };
        changed = true;
      }
      if (changed) applyInput();
    },
    resize(width: number, height: number): void {
      env.W = Math.max(0, width);
      env.H = Math.max(0, height);
      setupCanvas();
      hud.resize(env.W, env.H);
      markDirty();
    },
    destroy(): void {
      destroyed = true;
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
      scheme?.removeEventListener?.('change', onScheme);
      window.clearTimeout(recheck);
      layoutCb = null;
      viewCb = null;
    },
    onView(cb): void {
      viewCb = cb;
    },
    hudBounds: () => hud.bounds(),
    onLayout(cb): void {
      layoutCb = cb;
      lastHud = null;
      markDirty();
    },
    hudHover(on: boolean): void {
      if (hud.setMode(on ? 'hover' : 'none')) markDirty();
    },
    hudDragStart(): void {
      hud.setMode('drag');
      hud.beginDrag();
      markDirty();
    },
    hudDragMove(x: number, y: number): void {
      hud.dragTo(x, y);
      markDirty();
    },
    hudDragEnd(): Corner {
      const c = hud.endDrag();
      hud.setMode('hover');
      markDirty();
      return c;
    },
  };
}

/** Contract entry point (see ../shared/renderer-api.ts). */
export const createRenderer: CreateRenderer = (canvas, initial) => createOverlayRenderer(canvas, initial);

