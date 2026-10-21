// Concurrency limits for the Guide: one question in flight per calling agent,
// and a few at once overall.

export class Semaphore {
  private waiting: Array<() => void> = [];
  private free: number;

  constructor(size: number) {
    this.free = size;
  }

  async acquire(): Promise<() => void> {
    if (this.free > 0) {
      this.free--;
    } else {
      await new Promise<void>((resolve) => this.waiting.push(resolve));
    }
    let released = false;
    return () => {
      if (released) return;
      released = true;
      const next = this.waiting.shift();
      if (next) next();
      else this.free++;
    };
  }
}

/** Runs tasks one at a time per key, in arrival order. */
export class KeyedLock {
  private tails = new Map<string, Promise<void>>();
  private counts = new Map<string, number>();

  /** Tasks for this key that are running or waiting. */
  pending(key: string): number {
    return this.counts.get(key) ?? 0;
  }

  /** All tasks running or waiting. */
  total(): number {
    let n = 0;
    for (const c of this.counts.values()) n += c;
    return n;
  }

  run<T>(key: string, task: () => Promise<T>): Promise<T> {
    this.counts.set(key, this.pending(key) + 1);
    const prev = this.tails.get(key) ?? Promise.resolve();
    const result = prev.then(task);
    const tail = result.then(
      () => undefined,
      () => undefined,
    );
    this.tails.set(key, tail);
    void tail.then(() => {
      const left = this.pending(key) - 1;
      if (left > 0) {
        this.counts.set(key, left);
      } else {
        this.counts.delete(key);
        if (this.tails.get(key) === tail) this.tails.delete(key);
      }
    });
    return result;
  }
}
