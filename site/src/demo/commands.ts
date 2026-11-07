import type { Appearance } from "../../../src/bindings.ts";
import { DemoTabs } from "./tabs/demo-tabs.ts";
import { DemoWindow } from "./window/demo-window.ts";

const HISTORY_LINES = 1000;
const APPEARANCE: Appearance = {
  font: { family: "Fira Code", size: 14 },
  tabBar: "bottom",
};

const demoWindow = new DemoWindow();
const tabs = new DemoTabs(() => demoWindow.hide());
const decoder = new TextDecoder();

export const commands = {
  async tabsList() {
    return tabs.list();
  },
  async tabNew() {
    tabs.open();
    return null;
  },
  async tabClose(tab: string) {
    tabs.close(tab);
    return null;
  },
  async tabActivate(tab: string) {
    tabs.activate(tab);
    return null;
  },
  async tabPlace(_tab: string, _pinned: boolean, _position: number | null) {
    return null;
  },
  async tabRestart(_tab: string) {
    return null;
  },
  async tabRename(_tab: string, _title: string) {
    return null;
  },
  async tabAttach(tab: string, onData: (data: ArrayBuffer) => void) {
    tabs.get(tab)?.attach(onData);
    return null;
  },
  async tabDetach(tab: string) {
    tabs.get(tab)?.detach();
  },
  async tabWrite(tab: string, data: number[]) {
    tabs.get(tab)?.input(decoder.decode(Uint8Array.from(data)));
  },
  async terminalResize(_rows: number, _cols: number) {},
  async tabPause(_tab: string) {},
  async tabResume(_tab: string) {},
  async sessionSave() {
    return null;
  },
  async takeWarning() {
    return null;
  },
  async historyLines() {
    return HISTORY_LINES;
  },
  async showTabMenu(_tab: string | null, _pinned: boolean) {
    return null;
  },
  async hideWindow() {
    demoWindow.hidden();
  },
  isOpen() {
    return demoWindow.isOpen();
  },
  async watchAppearance(onChange: (appearance: Appearance) => void) {
    const { font } = APPEARANCE;
    await document.fonts.load(`${font.size}px "${font.family}"`);
    onChange(APPEARANCE);
  },
  async reportFatal(_message: string) {},
  async reportError(_message: string) {},
};
