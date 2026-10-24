import { Channel } from "@tauri-apps/api/core";
import { error as logError } from "@tauri-apps/plugin-log";
import { type Appearance, commands, events, type Font } from "./bindings.ts";
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

  const appearances = new Channel<Appearance>();
  let font: Font | undefined;
  const appearanceReady = new Promise<void>((resolve) => {
    appearances.onmessage = (appearance) => {
      tabs.setSide(appearance.tabBar);
      const next = appearance.font;
      if (font?.family !== next.family || font?.size !== next.size) {
        font = next;
        document.body.style.fontFamily = `${next.family}, monospace`;
        document.body.style.fontSize = `${next.size}px`;
        terminals.setFont(next);
      }
      resolve();
    };
  });
  await commands.watchAppearance(appearances);
  await appearanceReady;

  const unlisteners = await Promise.all([
    events.tabsChanged.listen(({ payload }) => manager.update(payload)),
    events.tabSelected.listen(({ payload }) => manager.select(payload)),
    events.tabTitleChanged.listen(({ payload }) =>
      manager.setTitle(payload.tab, payload.title),
    ),
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
