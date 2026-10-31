import type { Terminal } from "@xterm/xterm";
import { TabStream } from "./tab-stream.ts";
import type { Terminals } from "./terminals.ts";

export class Tab {
  private readonly element = document.createElement("div");
  private readonly terminals: Terminals;
  private readonly terminal: Terminal;
  private readonly stream: TabStream;
  private stopped = false;

  constructor(
    tab: string,
    terminals: Terminals,
    container: HTMLElement,
    restart: () => void,
  ) {
    this.terminals = terminals;
    this.element.className = "pane";
    container.append(this.element);
    this.terminal = terminals.create(this.element);
    this.element.hidden = true;
    this.stream = new TabStream(tab, this.terminal);
    terminals.settled().then(() => this.stream.attach());
    this.terminal.onData((data) => {
      if (this.stopped && data === "\r") restart();
    });
  }

  setStopped(stopped: boolean) {
    this.stopped = stopped;
    this.element.toggleAttribute("data-stopped", stopped);
  }

  show() {
    this.element.hidden = false;
    requestAnimationFrame(() => {
      this.terminal.refresh(0, this.terminal.rows - 1);
      this.terminal.focus();
    });
  }

  hide() {
    this.element.hidden = true;
  }

  focus() {
    this.terminal.focus();
  }

  resume() {
    this.stream.resume();
  }

  dispose() {
    this.stream.close();
    this.terminals.dispose(this.terminal);
    this.element.remove();
  }
}
