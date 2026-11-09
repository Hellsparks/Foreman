/**
 * Address failover for one saved server (desktop app).
 *
 * - Checks the active address every `intervalMs`; after `failAfter` failed checks it tries
 *   the others in order. An address only counts if it answers with the same server id.
 * - While on a backup, keeps checking the primary and moves back after `backAfterMs` healthy.
 * - "lost" only when every address is down; then keeps trying all of them.
 */

export type AddrState = "ok" | "down" | "trying" | "standby" | "unknown";

export interface FailoverState {
  active: number;
  status: "connecting" | "ok" | "lost";
  perAddress: AddrState[];
}

export interface FailoverOptions {
  intervalMs?: number;
  failAfter?: number;
  backAfterMs?: number;
  now?: () => number;
}

export class Failover {
  private state: FailoverState;
  private fails = 0;
  private primaryOkSince: number | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private running = false;
  private busy = false;
  private listeners = new Set<(s: FailoverState) => void>();
  private readonly intervalMs: number;
  private readonly failAfter: number;
  private readonly backAfterMs: number;
  private readonly now: () => number;

  constructor(
    private readonly serverId: string,
    private readonly addresses: string[],
    private readonly check: (address: string) => Promise<string | null>,
    private readonly onSwitch: (address: string, index: number) => void,
    opts: FailoverOptions = {},
  ) {
    this.intervalMs = opts.intervalMs ?? 5_000;
    this.failAfter = opts.failAfter ?? 3;
    this.backAfterMs = opts.backAfterMs ?? 60_000;
    this.now = opts.now ?? Date.now;
    this.state = { active: -1, status: "connecting", perAddress: addresses.map(() => "unknown") };
  }

  get snapshot(): FailoverState {
    return this.state;
  }

  subscribe(cb: (s: FailoverState) => void): () => void {
    this.listeners.add(cb);
    return () => this.listeners.delete(cb);
  }

  private set(patch: Partial<FailoverState>) {
    this.state = { ...this.state, ...patch };
    this.listeners.forEach((l) => l(this.state));
  }

  private mark(i: number, s: AddrState) {
    const per = [...this.state.perAddress];
    per[i] = s;
    this.set({ perAddress: per });
  }

  private async healthy(i: number): Promise<boolean> {
    try {
      return (await this.check(this.addresses[i])) === this.serverId;
    } catch {
      return false;
    }
  }

  private switchTo(i: number) {
    this.fails = 0;
    this.primaryOkSince = null;
    const per = this.addresses.map((_, j): AddrState => (j === i ? "ok" : "standby"));
    this.set({ active: i, status: "ok", perAddress: per });
    this.onSwitch(this.addresses[i], i);
  }

  /** Try every address in order; switch to the first that answers. */
  private async findFirst(): Promise<boolean> {
    for (let i = 0; i < this.addresses.length; i++) {
      this.mark(i, "trying");
      if (await this.healthy(i)) {
        this.switchTo(i);
        return true;
      }
      this.mark(i, "down");
    }
    this.set({ status: "lost", active: -1 });
    return false;
  }

  /** One check cycle (exposed for tests). */
  async tick(): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    try {
      const { active, status } = this.state;
      if (status !== "ok" || active < 0) {
        await this.findFirst();
        return;
      }
      if (await this.healthy(active)) {
        this.fails = 0;
      } else {
        this.fails += 1;
        if (this.fails >= this.failAfter) {
          this.mark(active, "down");
          await this.findFirst();
          return;
        }
      }
      if (this.state.active > 0) {
        if (await this.healthy(0)) {
          this.primaryOkSince ??= this.now();
          if (this.now() - this.primaryOkSince >= this.backAfterMs) this.switchTo(0);
        } else {
          this.primaryOkSince = null;
        }
      }
    } finally {
      this.busy = false;
    }
  }

  /** Initial connect (first address that answers), then periodic checks. */
  async start(): Promise<void> {
    this.running = true;
    await this.tick();
    this.schedule();
  }

  private schedule() {
    if (!this.running) return;
    this.timer = setTimeout(async () => {
      await this.tick();
      this.schedule();
    }, this.intervalMs);
  }

  async retryNow(): Promise<void> {
    await this.tick();
  }

  stop() {
    this.running = false;
    if (this.timer !== null) clearTimeout(this.timer);
  }
}
