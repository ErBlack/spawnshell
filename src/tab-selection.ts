import type { TabInfo } from "./backend.ts";

export class TabSelection {
  items: readonly TabInfo[] = [];
  active: string | undefined;
  private pending: string | undefined;

  find(id: string): TabInfo | undefined {
    return this.items.find((item) => item.id === id);
  }

  update(items: readonly TabInfo[]) {
    const previous = this.items.findIndex((item) => item.id === this.active);
    this.items = items;
    if (this.pending && this.find(this.pending)) {
      this.active = this.pending;
      this.pending = undefined;
    } else if (this.active === undefined) {
      this.active = (items.find((item) => item.active) ?? items[0])?.id;
    } else if (!this.find(this.active)) {
      this.active = items[Math.min(previous, items.length - 1)]?.id;
    }
  }

  clear() {
    this.items = [];
    this.active = undefined;
  }

  select(id: string) {
    if (this.find(id)) this.active = id;
    else this.pending = id;
  }

  at(index: number): string | undefined {
    return this.items[index]?.id;
  }

  step(offset: number): string | undefined {
    const index = this.items.findIndex((item) => item.id === this.active);
    if (index < 0) return undefined;
    const count = this.items.length;
    return this.items[(index + offset + count) % count]?.id;
  }
}
