import type {
  TabInfo,
  TabMenuChosen,
  TabTitleChanged,
} from "../../../src/bindings.ts";

type Handler<T> = (event: { payload: T }) => void;

function eventChannel<T>() {
  const handlers = new Set<Handler<T>>();
  return {
    listen(handler: Handler<T>) {
      handlers.add(handler);
      return Promise.resolve(() => {
        handlers.delete(handler);
      });
    },
    emit(payload: T) {
      for (const handler of handlers) handler({ payload });
    },
  };
}

export const events = {
  reconnected: eventChannel<null>(),
  tabMenuChosen: eventChannel<TabMenuChosen>(),
  tabSelected: eventChannel<string>(),
  tabTitleChanged: eventChannel<TabTitleChanged>(),
  tabsChanged: eventChannel<TabInfo[]>(),
  warning: eventChannel<string>(),
  windowHidden: eventChannel<null>(),
  windowShown: eventChannel<null>(),
};
