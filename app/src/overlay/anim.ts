// Tiny animation primitives: spring-damper scalars and numeric helpers.
// Everything here is allocation-free after construction.

export const clamp = (v: number, lo: number, hi: number): number => (v < lo ? lo : v > hi ? hi : v);
export const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;

/**
 * A scalar spring. `w` is the natural angular frequency (rad/s): higher is snappier.
 * `z` is the damping ratio: 1 is critically damped (no overshoot), < 1 overshoots.
 * Integrated with fixed sub-steps (semi-implicit Euler), which is stable for w*h < 1.
 */
export class Spring {
  x: number;
  t: number;
  v = 0;
  constructor(value = 0, public w = 22, public z = 1, public eps = 1e-3) {
    this.x = value;
    this.t = value;
  }

  /** Retarget; when `animate` is false the value jumps there immediately. */
  set(target: number, animate = true): void {
    this.t = target;
    if (!animate) {
      this.x = target;
      this.v = 0;
    }
  }

  /** Jump to a value with no motion, optionally also moving the target. */
  snap(value: number): void {
    this.x = value;
    this.t = value;
    this.v = 0;
  }

  /** Advance by dt seconds. Returns true while the spring is still moving. */
  step(dt: number): boolean {
    if (this.x === this.t && this.v === 0) return false;
    const n = Math.min(24, Math.ceil(dt * 240));
    const h = dt / n;
    const k = this.w * this.w;
    const c = 2 * this.z * this.w;
    let x = this.x;
    let v = this.v;
    const t = this.t;
    for (let i = 0; i < n; i++) {
      v += (-k * (x - t) - c * v) * h;
      x += v * h;
    }
    if (Math.abs(x - t) < this.eps && Math.abs(v) < this.eps * 4) {
      this.x = t;
      this.v = 0;
      return false;
    }
    this.x = x;
    this.v = v;
    return true;
  }

  get moving(): boolean {
    return this.x !== this.t || this.v !== 0;
  }
}
