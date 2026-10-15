import { events } from "./bindings.ts";
import type { TabManager } from "./tab-manager.ts";
import type { Tabs } from "./tabs.ts";

export function listenTabMenu(manager: TabManager, tabs: Tabs) {
  return events.tabMenuChosen.listen(({ payload: chosen }) => {
    switch (chosen.action) {
      case "new":
        manager.create();
        break;
      case "pin":
        manager.place(chosen.tab, chosen.pinned, chosen.pinned ? null : 0);
        break;
      case "rename":
        tabs.rename(chosen.tab);
        break;
      case "restart":
        manager.restart(chosen.tab);
        break;
      case "close":
        manager.close(chosen.tab);
        break;
      default:
        chosen satisfies never;
    }
  });
}
