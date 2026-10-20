import type { Terminal } from "@xterm/xterm";
import { CommandTitle } from "./command-title.ts";
import { TabStream } from "./tab-stream.ts";
import type { Terminals } from "./terminals.ts";

export class Tab {
  readonly commandTitle: CommandTitle;
  dirty = false;
  private readonly element = document.createElement("div");
  private readonly terminals: Terminals;
  private readonly terminal: Terminal;
  private readonly stream: TabStream;
  private visible = false;

  constructor(
    tab: string,
    terminals: Terminals,
    container: HTMLElement,
    onChange: () => void,
  ) {
    this.terminals = terminals;
    this.element.className = "pane";
    container.append(this.element);
    this.terminal = terminals.create(this.element);
    this.element.hidden = true;
    this.commandTitle = new CommandTitle(this.terminal, onChange);
    this.stream = new TabStream(tab, this.terminal, () => {
      if (this.visible || this.dirty) return;
      this.dirty = true;
      onChange();
    });
    terminals.settled().then(() => this.stream.attach());
  }

  show() {
    this.visible = true;
    this.dirty = false;
    this.element.hidden = false;
    requestAnimationFrame(() => {
      this.terminal.refresh(0, this.terminal.rows - 1);
      this.terminal.focus();
    });
  }

  hide() {
    this.visible = false;
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
