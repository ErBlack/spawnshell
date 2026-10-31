import { commands, type Font } from "./backend.ts";
import type { Tabs } from "./tabs.ts";
import type { Terminals } from "./terminals.ts";

export async function applyAppearance(
  tabs: Tabs,
  terminals: Terminals,
): Promise<void> {
  let font: Font | undefined;
  let applied = () => {};
  const ready = new Promise<void>((resolve) => {
    applied = resolve;
  });
  await commands.watchAppearance((appearance) => {
    tabs.setSide(appearance.tabBar);
    const next = appearance.font;
    if (font?.family !== next.family || font?.size !== next.size) {
      font = next;
      document.body.style.fontFamily = `${next.family}, monospace`;
      document.body.style.fontSize = `${next.size}px`;
      terminals.setFont(next);
    }
    applied();
  });
  await ready;
}
