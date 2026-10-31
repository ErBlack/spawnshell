import { events } from "./backend.ts";
import { errorMessage } from "./error-message.ts";
import { showError } from "./show-error.ts";
import type { TabManager } from "./tab-manager.ts";

export function listenTabs(manager: TabManager) {
  return Promise.all([
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
  ]);
}
