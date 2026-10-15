import { Channel } from "@tauri-apps/api/core";
import { error as logError } from "@tauri-apps/plugin-log";
import { commands, events, type Font } from "./bindings.ts";
import { errorMessage } from "./error-message.ts";
import { installKeybindings } from "./install-keybindings.ts";
import { listenTabMenu } from "./listen-tab-menu.ts";
import { reportFailure } from "./report-failure.ts";
import { setupWindow } from "./setup-window.ts";
import { showError } from "./show-error.ts";
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

  const terminals = new Terminals(await commands.historyLines());
  const fonts = new Channel<Font>();
  const fontReady = new Promise<void>((resolve) => {
    fonts.onmessage = (font) => {
      document.body.style.fontFamily = `${font.family}, monospace`;
      terminals.setFont(font);
      resolve();
    };
  });
  await commands.watchFont(fonts);
  await fontReady;

  const tabs = new Tabs(
    element("tabbar"),
    element("pinned-tabs"),
    element("tabs"),
    {
      select(id) {
        manager.select(id);
        if (manager.find(id)?.alive === false) manager.restart(id);
      },
      menu(id) {
        const pinned = Boolean(id && manager.find(id)?.pinned);
        reportFailure(commands.showTabMenu(id ?? null, pinned));
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

  const unlisteners = await Promise.all([
    events.tabsChanged.listen(({ payload }) => manager.update(payload)),
    events.tabSelected.listen(({ payload }) => manager.select(payload)),
    events.reconnected.listen(() => {
      manager.clear();
      manager
        .refresh()
        .catch((error: unknown) =>
          showError(`couldn't reload tabs: ${errorMessage(error)}`),
        );
    }),
    events.warning.listen(({ payload }) => showError(payload)),
    listenTabMenu(manager, tabs),
  ]);
  import.meta.hot?.dispose(() => {
    for (const unlisten of unlisteners) unlisten();
  });

  await manager.refresh();
  installKeybindings(manager);

  const warning = await commands.takeWarning();
  if (warning) showError(warning);

  await setupWindow(panes, {
    fit: () => terminals.fit(),
    focus: () => manager.focus(),
    onShow: () => {
      manager.resume();
      if (manager.count === 0) manager.create();
    },
    onHide: () => reportFailure(commands.sessionSave()),
  });

  terminals.fit();
}

main().catch((error: unknown) => {
  const message = errorMessage(error);
  logError(`failed to start: ${message}`);
  commands.reportFatal(message);
});
