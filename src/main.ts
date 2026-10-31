import { applyAppearance } from "./apply-appearance.ts";
import { errorMessage } from "./error-message.ts";
import { installKeybindings } from "./install-keybindings.ts";
import { listenTabMenu } from "./listen-tab-menu.ts";
import { listenTabs } from "./listen-tabs.ts";
import { setupWindow } from "./setup-window.ts";
import { listenWarnings, showFatal, showPendingWarning } from "./show-error.ts";
import { TabManager } from "./tab-manager.ts";
import { Tabs } from "./tabs.ts";
import { Terminals } from "./terminals.ts";
import "./styles.css";

function element(id: string): HTMLElement {
  const found = document.getElementById(id);
  if (!found) throw new Error(`#${id} not found`);
  return found;
}

async function main() {
  const panes = element("panes");

  const terminals = await Terminals.load();

  const tabs = new Tabs(
    element("tabbar"),
    element("pinned-tabs"),
    element("tabs"),
    {
      select(id) {
        if (manager.active === id && manager.restartable(id))
          manager.restart(id);
        else manager.select(id);
      },
      close(id) {
        manager.close(id);
      },
      menu(id) {
        manager.showMenu(id);
      },
      place(id, pinned, position) {
        manager.place(id, pinned, position);
      },
      rename(id, title) {
        manager.rename(id, title);
      },
      focus() {
        manager.focus();
      },
    },
  );
  const manager = new TabManager(terminals, panes, () =>
    tabs.render(manager.entries(), manager.active),
  );

  await applyAppearance(tabs, terminals);

  const unlisteners = await Promise.all([
    listenTabs(manager),
    listenWarnings(),
    listenTabMenu(manager, tabs),
  ]);
  import.meta.hot?.dispose(() => {
    for (const unlisten of unlisteners.flat()) unlisten();
  });

  await manager.refresh();
  installKeybindings(manager);

  await showPendingWarning();

  await setupWindow(panes, {
    fit: () => terminals.fit(),
    focus: () => manager.focus(),
    onShow: () => {
      manager.resume();
      if (manager.count === 0) manager.create();
    },
    onHide: () => manager.save(),
  });

  terminals.fit();
}

main().catch((error: unknown) => {
  showFatal(errorMessage(error));
});
