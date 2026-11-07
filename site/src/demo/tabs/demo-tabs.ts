import type { TabInfo } from "../../../../src/bindings.ts";
import { events } from "../events.ts";
import { DemoTab } from "./demo-tab.ts";

const FIRST_COMMAND = "spawnshell --help";

export class DemoTabs {
  private readonly tabs: DemoTab[] = [];
  private created = 0;
  private readonly onEmpty: () => void;

  constructor(onEmpty: () => void) {
    this.onEmpty = onEmpty;
  }

  list(): TabInfo[] {
    return this.tabs.map((tab) => ({ ...tab.info }));
  }

  get(id: string): DemoTab | undefined {
    return this.tabs.find((tab) => tab.info.id === id);
  }

  open() {
    this.created += 1;
    const id = `demo-${this.created}`;
    this.tabs.push(new DemoTab(id, this.created === 1 ? FIRST_COMMAND : ""));
    this.activate(id);
    this.changed();
    events.tabSelected.emit(id);
  }

  activate(id: string) {
    for (const tab of this.tabs) tab.info.active = tab.info.id === id;
  }

  close(id: string) {
    const index = this.tabs.findIndex((tab) => tab.info.id === id);
    if (index === -1) return;
    this.tabs.splice(index, 1);
    this.changed();
    if (this.tabs.length === 0) this.onEmpty();
  }

  private changed() {
    events.tabsChanged.emit(this.list());
  }
}
