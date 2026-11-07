import help from "virtual:spawnshell-help";
import type { TabInfo } from "../../../../src/bindings.ts";
import { Shell } from "../shell/shell.ts";

const encoder = new TextEncoder();

function outputFrame(snapshot: boolean, text: string): ArrayBuffer {
  const bytes = encoder.encode(text);
  const message = new Uint8Array(bytes.length + 1);
  message[0] = snapshot ? 1 : 0;
  message.set(bytes, 1);
  return message.buffer;
}

export class DemoTab {
  readonly info: TabInfo;
  private readonly shell: Shell;
  private history = "";
  private sink?: (data: ArrayBuffer) => void;

  constructor(id: string, command: string) {
    this.info = {
      id,
      pinned: false,
      alive: true,
      active: false,
      title: null,
      process: "bash",
      cwd: "~",
      notified: false,
    };
    this.shell = new Shell(help, (text) => this.output(text));
    this.shell.start(command);
  }

  attach(sink: (data: ArrayBuffer) => void) {
    this.sink = sink;
    sink(outputFrame(true, this.history));
  }

  detach() {
    this.sink = undefined;
  }

  input(text: string) {
    this.shell.input(text);
  }

  private output(text: string) {
    this.history += text;
    this.sink?.(outputFrame(false, text));
  }
}
