// Evaluation bar drawn beside the board. White is at the bottom of the bar when
// the board is not flipped, at the top when it is.
import type { AnalysisFrame, Settings } from '../shared/types';
import { Spring, clamp } from './anim';
import { formatEval } from './chess';
import { mixRgb, parseColor, rgba, onColor } from './color';
import { Env, font, roundRect, textWidth, snap } from './env';

export class EvalBar {
  private share = new Spring(0.5, 12, 1, 0.0004);
  private pawns = new Spring(0, 15, 1, 0.004);
  private vis = new Spring(0, 16, 1, 0.004);
  private mate: number | null = null;
  private hasFrame = false;
  private live = new Spring(0, 12, 1, 0.004);
  private ld = new Spring(0, 12, 1, 0.004);
  private prog = new Spring(0, 9, 1, 0.002);
  private motion = true;
  private busy = false;
  private numQ = NaN;
  private numStr = '0.0';
  private gradKey = '';
  private gradW: CanvasGradient | null = null;
  private gradB: CanvasGradient | null = null;
  private white = 0xf4f8fc;
  private black = 0x101a26;
  private opts: Settings['evalBar'] | null = null;

  update(frame: AnalysisFrame | null, opts: Settings['evalBar'], animate: boolean): void {
    this.opts = opts;
    this.white = parseColor(opts.whiteColor);
    this.black = parseColor(opts.blackColor);
    const anim = animate && opts.animate;
    this.hasFrame = frame !== null;
    this.live.set(frame ? 1 : 0, animate);
    this.mate = frame ? frame.mate : null;
    let share = 0.5;
    let pawns = 0;
    if (frame) {
      if (frame.mate !== null) share = frame.mate > 0 ? 1 : frame.mate < 0 ? 0 : 0.5;
      else share = clamp(frame.winPct / 100, 0.02, 0.98);
      pawns = frame.evalCp === null ? 0 : clamp(frame.evalCp / 100, -30, 30);
    }
    this.share.set(share, anim);
    this.pawns.set(pawns, anim);
    this.vis.set(opts.show ? 1 : 0, animate);
  }

  /** Analysing a new position: shimmer the bar and fill the slim depth track (frac = depth / target). */
  setLoading(on: boolean, frac: number, animate: boolean): void {
    this.busy = on;
    this.motion = animate;
    this.ld.set(on ? 1 : 0, animate);
    this.prog.set(on ? frac : 0, animate);
  }

  /** True while the shimmer must keep animating. */
  get loading(): boolean {
    return this.busy && this.vis.x >= 0.01;
  }

  /** True while the bar is (fading) on screen. */
  get visible(): boolean {
    return this.opts !== null && this.vis.x >= 0.01;
  }

  step(dt: number): boolean {
    const a = this.share.step(dt);
    const b = this.pawns.step(dt);
    const c = this.vis.step(dt);
    const d = this.live.step(dt);
    const e = this.ld.step(dt);
    const f = this.prog.step(dt);
    return a || b || c || d || e || f;
  }

  private label(): string {
    if (this.mate !== null) return formatEval(null, this.mate);
    const q = Math.round(this.pawns.x * 10);
    if (q !== this.numQ) {
      this.numQ = q;
      this.numStr = formatEval(q * 10, null, 1);
    }
    return this.numStr;
  }

  private ensureGradients(ctx: CanvasRenderingContext2D): void {
    const key = `${this.white}|${this.black}`;
    if (key === this.gradKey && this.gradW && this.gradB) return;
    this.gradKey = key;
    // Unit-space gradients: t = 0 at the white/black boundary, t = 1 at the bar end.
    const gw = ctx.createLinearGradient(0, 0, 0, 1);
    gw.addColorStop(0, `#${mixRgb(this.white, 0x8da2bd, 0.38).toString(16).padStart(6, '0')}`);
    gw.addColorStop(0.35, `#${mixRgb(this.white, 0x8da2bd, 0.08).toString(16).padStart(6, '0')}`);
    gw.addColorStop(1, `#${this.white.toString(16).padStart(6, '0')}`);
    const gb = ctx.createLinearGradient(0, 0, 0, 1);
    gb.addColorStop(0, `#${mixRgb(this.black, 0x6b84a6, 0.5).toString(16).padStart(6, '0')}`);
    gb.addColorStop(0.4, `#${mixRgb(this.black, 0x6b84a6, 0.12).toString(16).padStart(6, '0')}`);
    gb.addColorStop(1, `#${this.black.toString(16).padStart(6, '0')}`);
    this.gradW = gw;
    this.gradB = gb;
  }

  /** Bounds of the bar for a given board rectangle (CSS px). */
  rect(env: Env, gx: number, gy: number, gs: number): { x: number; y: number; w: number; h: number } {
    const o = this.opts;
    const w = clamp(o ? o.width : 14, 6, 44);
    const gap = 8;
    const x = !o || o.position === 'left' ? gx - gap - w : gx + gs + gap;
    return { x: clamp(x, 3, Math.max(3, env.W - w - 3)), y: gy, w, h: gs };
  }

  draw(env: Env, gx: number, gy: number, gs: number, flipped: boolean, alpha: number): void {
    const o = this.opts;
    // Idle (no analysis yet): dimmed so a neutral 50/50 bar is not mistaken for a real 0.00 evaluation.
    const lift = Math.max(this.live.x, 0.75 * this.ld.x);
    const a = alpha * this.vis.x * (0.42 + 0.58 * lift);
    if (!o || a < 0.01 || gs < 8) return;
    const { ctx, dpr, pal } = env;
    const r = this.rect(env, gx, gy, gs);
    const bx = r.x;
    const by = r.y;
    const bh = r.h;
    const bw = r.w;
    const rad = Math.min(bw / 2, 7);
    const whiteBottom = !flipped;
    const hw = bh * this.share.x;
    const yb = whiteBottom ? by + bh - hw : by + hw;

    ctx.save();
    ctx.globalAlpha = a;

    // Soft drop shadow behind the bar.
    roundRect(ctx, bx, by, bw, bh, rad);
    ctx.shadowColor = 'rgba(0,0,0,0.38)';
    ctx.shadowBlur = 10 * dpr;
    ctx.shadowOffsetY = 2 * dpr;
    ctx.fillStyle = rgba(this.black, 1);
    ctx.fill();
    ctx.shadowColor = 'transparent';
    ctx.shadowBlur = 0;
    ctx.shadowOffsetY = 0;

    ctx.save();
    roundRect(ctx, bx, by, bw, bh, rad);
    ctx.clip();
    if (o.style === 'gradient') {
      this.ensureGradients(ctx);
      const blackLen = bh - hw;
      if (blackLen > 0.5) {
        ctx.save();
        ctx.translate(bx, yb);
        ctx.scale(bw, whiteBottom ? -blackLen : blackLen);
        ctx.fillStyle = this.gradB as CanvasGradient;
        ctx.fillRect(0, 0, 1, 1);
        ctx.restore();
      }
      if (hw > 0.5) {
        ctx.save();
        ctx.translate(bx, yb);
        ctx.scale(bw, whiteBottom ? hw : -hw);
        ctx.fillStyle = this.gradW as CanvasGradient;
        ctx.fillRect(0, 0, 1, 1);
        ctx.restore();
      }
    } else {
      ctx.fillStyle = rgba(this.black, 1);
      ctx.fillRect(bx, by, bw, bh);
      ctx.fillStyle = rgba(this.white, 1);
      if (whiteBottom) ctx.fillRect(bx, yb, bw, hw);
      else ctx.fillRect(bx, by, bw, hw);
    }
    // Equal-eval centre line, so the zero point stays readable while the bar moves.
    ctx.fillStyle = 'rgba(128,140,158,0.55)';
    ctx.fillRect(bx, snap(by + bh / 2, dpr) - 0.5, bw, 1);
    // Boundary highlight.
    if (hw > 1 && hw < bh - 1) {
      ctx.fillStyle = rgba(pal.accent, 0.95);
      ctx.fillRect(bx, snap(yb, dpr) - 1, bw, 2);
    }
    // Analysing: a soft light band sweeps along the bar.
    if (this.ld.x > 0.02 && this.motion) {
      const band = bh * 0.38;
      const c = by + bh - (((env.t * 0.75) % 1.5) - 0.25) * bh;
      const g = ctx.createLinearGradient(0, c - band, 0, c + band);
      const k = 0.42 * this.ld.x;
      g.addColorStop(0, 'rgba(255,255,255,0)');
      g.addColorStop(0.5, `rgba(255,255,255,${k.toFixed(3)})`);
      g.addColorStop(1, 'rgba(255,255,255,0)');
      ctx.fillStyle = g;
      ctx.fillRect(bx, by, bw, bh);
    }
    ctx.restore();

    // Hairline border for definition against any background.
    roundRect(ctx, bx + 0.5, by + 0.5, bw - 1, bh - 1, rad);
    ctx.lineWidth = 1;
    ctx.strokeStyle = 'rgba(255,255,255,0.22)';
    ctx.stroke();

    // Slim depth track on the outer edge of the bar: fills from depth to target depth.
    if (this.ld.x > 0.02) {
      const tw = 3;
      const outer = o.position === 'left' ? bx - tw - 3 : bx + bw + 3;
      const tx = outer >= 1 && outer + tw <= env.W - 1 ? outer : o.position === 'left' ? bx + bw - tw - 1 : bx + 1;
      ctx.save();
      ctx.globalAlpha = alpha * this.vis.x * this.ld.x * 0.95;
      roundRect(ctx, tx, by, tw, bh, tw / 2);
      ctx.fillStyle = 'rgba(128,140,158,0.35)';
      ctx.fill();
      const fh = Math.max(tw, bh * this.prog.x);
      roundRect(ctx, tx, by + bh - fh, tw, fh, tw / 2);
      ctx.fillStyle = rgba(pal.accent, 1);
      ctx.fill();
      ctx.restore();
    }

    if (o.showNumber && this.hasFrame && this.live.x > 0.5) {
      const whiteLeads = this.mate !== null ? this.mate > 0 : this.share.t >= 0.5;
      // The number sits at the end of the leading side's portion of the bar.
      const atBottom = whiteLeads === whiteBottom;
      const fpx = clamp(bw * 0.78, 9.5, 12);
      const f = font(700, fpx);
      const text = this.label();
      const tw = textWidth(ctx, f, text);
      const ph = fpx + 7;
      const pw = tw + 10;
      const px = bx + bw / 2 - pw / 2;
      const py = atBottom ? by + bh - ph - 5 : by + 5;
      const plateBg = whiteLeads ? 0x101722 : 0xf4f8fc;
      roundRect(ctx, px, py, pw, ph, ph / 2);
      ctx.fillStyle = rgba(plateBg, 0.9);
      ctx.fill();
      ctx.lineWidth = 1;
      ctx.strokeStyle = whiteLeads ? 'rgba(255,255,255,0.18)' : 'rgba(16,23,34,0.2)';
      ctx.stroke();
      ctx.font = f;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillStyle = rgba(onColor(plateBg), 1);
      ctx.fillText(text, snap(px + pw / 2, dpr), snap(py + ph / 2 + 0.5, dpr));
    }
    ctx.restore();
  }
}
