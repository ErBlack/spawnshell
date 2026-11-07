import { listenMessages } from "../../bridge/listen-messages.ts";
import { postMessage } from "../../bridge/post-message.ts";
import type { FromDemo } from "../../demo-frame/mount-demo-frame.ts";
import { events } from "../events.ts";

export type ToDemo = { type: "show" } | { type: "hide" };

export class DemoWindow {
  private open = false;
  private answer?: (open: boolean) => void;

  constructor() {
    listenMessages<ToDemo>(
      () => parent,
      (message) => this.receive(message),
    );
  }

  isOpen(): Promise<boolean> {
    if (parent === window) {
      this.open = true;
      return Promise.resolve(true);
    }
    return new Promise((resolve) => {
      this.answer = resolve;
      this.post({ type: "ready" });
    });
  }

  hide() {
    if (!this.open) return;
    this.open = false;
    events.windowHidden.emit(null);
  }

  hidden() {
    this.post({ type: "hidden" });
  }

  private post(message: FromDemo) {
    postMessage<FromDemo>(parent, message);
  }

  private receive(message: ToDemo) {
    if (this.answer) {
      this.open = message.type === "show";
      this.answer(this.open);
      this.answer = undefined;
    } else if (message.type === "show" && !this.open) {
      this.open = true;
      events.windowShown.emit(null);
    } else if (message.type === "hide") {
      this.hide();
    }
  }
}
