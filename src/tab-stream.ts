import type { Terminal } from "@xterm/xterm";
import { commands } from "./backend.ts";
import { reportFailure } from "./report-failure.ts";

const HIGH_WATER = 128 * 1024;
const LOW_WATER = 16 * 1024;
const DISCARD_LIMIT = 5 * 1024 * 1024;

const encoder = new TextEncoder();

export class TabStream {
  private readonly tab: string;
  private readonly terminal: Terminal;
  private queued = 0;
  private paused = false;
  private discarded = 0;
  private replaying = 0;
  private closed = false;

  constructor(tab: string, terminal: Terminal) {
    this.tab = tab;
    this.terminal = terminal;
    terminal.onData((data) => this.write(encoder.encode(data)));
    terminal.onBinary((data) =>
      this.write(Uint8Array.from(data, (char) => char.charCodeAt(0))),
    );
  }

  attach() {
    if (this.closed) return;
    reportFailure(
      commands.tabAttach(this.tab, (data) =>
        this.receive(new Uint8Array(data)),
      ),
    );
  }

  resume() {
    this.setPaused(false);
  }

  close() {
    this.closed = true;
    reportFailure(commands.tabDetach(this.tab));
  }

  private write(bytes: Uint8Array) {
    if (this.replaying > 0) return;
    reportFailure(commands.tabWrite(this.tab, Array.from(bytes)));
  }

  private receive(message: Uint8Array) {
    if (this.closed) return;
    const snapshot = message[0] === 1;
    const chunk = message.subarray(1);
    if (this.queued > DISCARD_LIMIT) {
      this.discarded += chunk.length;
    } else {
      this.queued += chunk.length;
      if (snapshot) this.replaying++;
      this.terminal.write(chunk, () => {
        if (snapshot) this.replaying--;
        this.written(chunk.length);
      });
      if (this.queued > HIGH_WATER) this.setPaused(true);
    }
  }

  private written(length: number) {
    this.queued -= length;
    if (this.queued >= LOW_WATER) return;
    this.setPaused(false);
    if (this.discarded === 0) return;
    const lost = this.discarded;
    this.discarded = 0;
    this.terminal.write(`\r\n\x1b[33m[${lost} bytes discarded]\x1b[0m\r\n`);
  }

  private setPaused(paused: boolean) {
    if (this.paused === paused || this.closed) return;
    this.paused = paused;
    reportFailure(
      paused ? commands.tabPause(this.tab) : commands.tabResume(this.tab),
    );
  }
}
