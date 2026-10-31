import { Channel } from "@tauri-apps/api/core";
import { error as logError } from "@tauri-apps/plugin-log";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  type Appearance,
  events,
  type Font,
  commands as generated,
  type TabBar,
  type TabInfo,
} from "./bindings.ts";

export type { Appearance, Font, TabBar, TabInfo };
export { events, logError, openUrl };

export const commands = {
  ...generated,
  tabAttach(tab: string, onData: (data: ArrayBuffer) => void) {
    const channel = new Channel<ArrayBuffer>();
    channel.onmessage = onData;
    return generated.tabAttach(tab, channel);
  },
  watchAppearance(onChange: (appearance: Appearance) => void) {
    const channel = new Channel<Appearance>();
    channel.onmessage = onChange;
    return generated.watchAppearance(channel);
  },
};
