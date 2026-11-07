import { isShortcut } from "../bridge/is-shortcut.ts";
import { listenMessages } from "../bridge/listen-messages.ts";
import { postMessage } from "../bridge/post-message.ts";
import type { ToDemo } from "../demo/window/demo-window.ts";

export type FromDemo =
  | { type: "ready" }
  | { type: "toggle" }
  | { type: "hidden" }
  | { type: "key"; code: string; ctrl: boolean; down: boolean };

const CONTROL_CODES = new Set(["ControlLeft", "ControlRight"]);

function pressKey(
  keys: HTMLElement,
  code: string,
  ctrl: boolean,
  down: boolean,
) {
  if (!CONTROL_CODES.has(code))
    keys
      .querySelector(`[data-key="${code}"]`)
      ?.toggleAttribute("data-pressed", down);
  keys
    .querySelector('[data-key="Control"]')
    ?.toggleAttribute("data-pressed", ctrl);
}

export function mountDemoFrame(keys: HTMLButtonElement) {
  let frame: HTMLIFrameElement | undefined;
  let ready = false;
  let open = false;

  const send = (message: ToDemo) => {
    if (ready && frame?.contentWindow)
      postMessage<ToDemo>(frame.contentWindow, message);
  };

  const load = () => {
    if (!frame) {
      frame = document.createElement("iframe");
      frame.className = "demo";
      frame.src = "./demo.html";
      frame.title = "Spawnshell demo terminal";
      frame.id = "demo";
      document.body.append(frame);
    }
    return frame;
  };

  const setOpen = (value: boolean) => {
    open = value;
    keys.setAttribute("aria-expanded", String(value));
  };

  const reveal = () => {
    frame?.focus();
    send({ type: "show" });
  };

  const show = () => {
    if (open) return;
    setOpen(true);
    load().toggleAttribute("data-open", true);
    if (ready) reveal();
  };

  const hide = () => {
    if (!open) return;
    setOpen(false);
    send({ type: "hide" });
  };

  const toggle = () => (open ? hide() : show());

  listenMessages<FromDemo>(
    () => frame?.contentWindow,
    (message) => {
      switch (message.type) {
        case "ready":
          ready = true;
          if (open) reveal();
          else send({ type: "hide" });
          break;
        case "toggle":
          toggle();
          break;
        case "hidden":
          setOpen(false);
          frame?.removeAttribute("data-open");
          if (document.activeElement === frame) keys.focus();
          break;
        case "key":
          pressKey(keys, message.code, message.ctrl, message.down);
          break;
      }
    },
  );

  addEventListener(
    "keydown",
    (event) => {
      pressKey(keys, event.code, event.ctrlKey, true);
      if (event.ctrlKey) load();
      if (!isShortcut(event)) return;
      event.preventDefault();
      toggle();
    },
    { capture: true },
  );
  addEventListener("keyup", (event) =>
    pressKey(keys, event.code, event.ctrlKey, false),
  );
  addEventListener("blur", () => {
    for (const key of keys.querySelectorAll("[data-pressed]"))
      key.removeAttribute("data-pressed");
  });
  document.addEventListener("pointerdown", (event) => {
    if (!keys.contains(event.target as Node)) hide();
  });
  keys.addEventListener("pointerenter", load, { once: true });
  keys.addEventListener("focus", load, { once: true });
  keys.addEventListener("click", toggle);
}
