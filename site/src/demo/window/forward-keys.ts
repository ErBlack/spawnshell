import { isShortcut } from "../../bridge/is-shortcut.ts";
import { postMessage } from "../../bridge/post-message.ts";
import type { FromDemo } from "../../demo-frame/mount-demo-frame.ts";

export function forwardKeys() {
  const forward = (event: KeyboardEvent) =>
    postMessage<FromDemo>(parent, {
      type: "key",
      code: event.code,
      ctrl: event.ctrlKey,
      down: event.type === "keydown",
    });
  addEventListener(
    "keydown",
    (event) => {
      forward(event);
      if (!isShortcut(event)) return;
      event.preventDefault();
      event.stopPropagation();
      postMessage<FromDemo>(parent, { type: "toggle" });
    },
    { capture: true },
  );
  addEventListener("keyup", forward, { capture: true });
}
