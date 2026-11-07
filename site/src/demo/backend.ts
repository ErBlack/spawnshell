export type {
  Appearance,
  Font,
  TabBar,
  TabInfo,
} from "../../../src/bindings.ts";
export { commands } from "./commands.ts";
export { events } from "./events.ts";

export function logError(message: string) {
  console.error(message);
}

export function openUrl(url: string) {
  open(url, "_blank", "noopener");
}
