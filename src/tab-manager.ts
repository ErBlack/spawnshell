import { commands, type TabInfo } from "./backend.ts";
import { errorMessage } from "./error-message.ts";
import { reportFailure } from "./report-failure.ts";
import { showError } from "./show-error.ts";
import { Tab } from "./tab.ts";
import { TabSelection } from "./tab-selection.ts";
import type { Terminals } from "./terminals.ts";

export interface TabEntry {
  info: TabInfo;
  title: string | null;
  label: string;
}

interface Renaming {
  title: string | null;
  settled: boolean;
}

interface Placing {
  pinned: boolean;
  position: number | null;
}

function placed(
  items: readonly TabInfo[],
  id: string,
  { pinned, position }: Placing,
): readonly TabInfo[] {
  const moved = items.find((item) => item.id === id);
  if (!moved) return items;
  const rest = items.filter((item) => item !== moved);
  const pinnedCount = rest.filter((item) => item.pinned).length;
  const start = pinned ? 0 : pinnedCount;
  const length = pinned ? pinnedCount : rest.length - pinnedCount;
  const at = start + Math.min(position ?? length, length);
  return [...rest.slice(0, at), { ...moved, pinned }, ...rest.slice(at)];
}

function sameOrder(a: readonly TabInfo[], b: readonly TabInfo[]): boolean {
  return (
    a.length === b.length &&
    a.every(
      (item, index) =>
        item.id === b[index]?.id && item.pinned === b[index]?.pinned,
    )
  );
}

export class TabManager {
  private readonly selection = new TabSelection();
  private readonly tabs = new Map<string, Tab>();
  private readonly renaming = new Map<string, Renaming>();
  private readonly placing = new Map<string, Placing>();
  private received: readonly TabInfo[] = [];
  private readonly titles = new Map<string, string>();
  private readonly restarting = new Set<string>();
  private readonly terminals: Terminals;
  private readonly container: HTMLElement;
  private readonly onChange: () => void;

  constructor(
    terminals: Terminals,
    container: HTMLElement,
    onChange: () => void,
  ) {
    this.terminals = terminals;
    this.container = container;
    this.onChange = onChange;
  }

  get active(): string | undefined {
    return this.selection.active;
  }

  get count(): number {
    return this.selection.items.length;
  }

  find(id: string): TabInfo | undefined {
    return this.selection.find(id);
  }

  restartable(id: string): boolean {
    const info = this.find(id);
    return info !== undefined && !info.alive && info.pinned;
  }

  entries(): TabEntry[] {
    return this.selection.items.map((info) => {
      const renaming = this.renaming.get(info.id);
      const title = renaming ? renaming.title : info.title;
      const label = title || this.titles.get(info.id) || info.process || "";
      return { info, title, label };
    });
  }

  async refresh() {
    this.update(await commands.tabsList());
  }

  update(received: readonly TabInfo[]) {
    this.received = received;
    for (const [id, placing] of this.placing)
      if (sameOrder(placed(received, id, placing), received))
        this.placing.delete(id);
    let items = received;
    for (const [id, placing] of this.placing)
      items = placed(items, id, placing);

    const before = this.selection.active;
    this.selection.update(items);
    for (const { id } of items) {
      let tab = this.tabs.get(id);
      if (!tab) {
        tab = new Tab(id, this.terminals, this.container, () =>
          this.restart(id),
        );
        this.tabs.set(id, tab);
      }
      tab.setStopped(this.restartable(id));
    }
    for (const [id, tab] of this.tabs) {
      if (this.selection.find(id)) continue;
      tab.dispose();
      this.tabs.delete(id);
      this.titles.delete(id);
    }
    for (const [id, renaming] of this.renaming) {
      const info = this.selection.find(id);
      if (!info || renaming.settled || info.title === renaming.title)
        this.renaming.delete(id);
    }
    this.switchFrom(before);
    this.onChange();
  }

  clear() {
    for (const tab of this.tabs.values()) tab.dispose();
    this.tabs.clear();
    this.renaming.clear();
    this.placing.clear();
    this.received = [];
    this.titles.clear();
    this.selection.clear();
    this.onChange();
  }

  setTitle(id: string, title: string | null) {
    if (title) this.titles.set(id, title);
    else this.titles.delete(id);
    this.onChange();
  }

  select(id: string | undefined) {
    if (id === undefined) return;
    const before = this.selection.active;
    this.selection.select(id);
    this.switchFrom(before);
    this.onChange();
  }

  selectIndex(index: number) {
    this.select(this.selection.at(index));
  }

  selectLast() {
    this.selectIndex(this.count - 1);
  }

  next() {
    this.select(this.selection.step(1));
  }

  previous() {
    this.select(this.selection.step(-1));
  }

  create() {
    reportFailure(commands.tabNew());
  }

  close(id: string) {
    reportFailure(commands.tabClose(id));
  }

  closeActive() {
    if (this.active) this.close(this.active);
  }

  showMenu(id: string | undefined) {
    const pinned = Boolean(id && this.find(id)?.pinned);
    reportFailure(commands.showTabMenu(id ?? null, pinned));
  }

  save() {
    reportFailure(commands.sessionSave());
  }

  restart(id: string) {
    if (this.restarting.has(id)) return;
    this.restarting.add(id);
    reportFailure(
      commands.tabRestart(id).finally(() => this.restarting.delete(id)),
    );
  }

  rename(id: string, title: string) {
    const renaming = { title: title || null, settled: false };
    this.renaming.set(id, renaming);
    this.onChange();
    commands.tabRename(id, title).then(
      () => {
        renaming.settled = true;
      },
      (error: unknown) => {
        if (this.renaming.get(id) === renaming) this.renaming.delete(id);
        this.onChange();
        showError(errorMessage(error));
      },
    );
  }

  place(id: string, pinned: boolean, position: number | null) {
    const placing = { pinned, position };
    this.placing.delete(id);
    this.placing.set(id, placing);
    this.update(this.received);
    commands.tabPlace(id, pinned, position).catch((error: unknown) => {
      if (this.placing.get(id) === placing) {
        this.placing.delete(id);
        this.update(this.received);
      }
      showError(errorMessage(error));
    });
  }

  focus() {
    if (this.active) this.tabs.get(this.active)?.focus();
  }

  resume() {
    for (const tab of this.tabs.values()) tab.resume();
  }

  private switchFrom(before: string | undefined) {
    const active = this.selection.active;
    if (active === before) return;
    if (before) this.tabs.get(before)?.hide();
    if (!active) return;
    this.tabs.get(active)?.show();
    reportFailure(commands.tabActivate(active));
  }
}
