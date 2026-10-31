import { WebLinksAddon } from "@xterm/addon-web-links";
import { type ITerminalOptions, Terminal } from "@xterm/xterm";
import { commands, type Font, openUrl } from "./backend.ts";
import { reportFailure } from "./report-failure.ts";

interface Size {
  rows: number;
  cols: number;
}

const THEME = {
  background: "#00000000",
  foreground: "#d5dae1",
  cursor: "#6cb5de",
  cursorAccent: "#080b10",
  selectionBackground: "#6cb5de40",
  black: "#2b323d",
  red: "#df7676",
  green: "#72c99e",
  yellow: "#dcb66a",
  blue: "#6cb5de",
  magenta: "#b294db",
  cyan: "#5ec1c9",
  white: "#8b95a3",
  brightBlack: "#636d7a",
  brightRed: "#eba0a0",
  brightGreen: "#96dbb8",
  brightYellow: "#ead08f",
  brightBlue: "#9acfee",
  brightMagenta: "#cbb6ec",
  brightCyan: "#8ad6dc",
  brightWhite: "#f2f4f7",
  scrollbarSliderBackground: "#00000000",
  scrollbarSliderHoverBackground: "#00000000",
  scrollbarSliderActiveBackground: "#00000000",
};

for (const [name, color] of Object.entries(THEME)) {
  document.documentElement.style.setProperty(`--terminal-${name}`, color);
}

function openLink(event: MouseEvent, uri: string) {
  if (event.metaKey) openUrl(uri);
}

function measure(terminal: Terminal): Size | undefined {
  const pane = terminal.element?.parentElement;
  const screen = terminal.element?.querySelector<HTMLElement>(".xterm-screen");
  const { rows, cols } = terminal;
  if (!pane || !screen || rows < 1 || cols < 1) return undefined;
  const cellHeight = screen.clientHeight / rows;
  const cellWidth = screen.clientWidth / cols;
  if (cellHeight <= 0 || cellWidth <= 0) return undefined;
  const style = getComputedStyle(pane);
  const height =
    pane.clientHeight -
    parseFloat(style.paddingTop) -
    parseFloat(style.paddingBottom);
  const width =
    pane.clientWidth -
    parseFloat(style.paddingLeft) -
    parseFloat(style.paddingRight);
  return {
    rows: Math.max(1, Math.floor(height / cellHeight)),
    cols: Math.max(2, Math.floor(width / cellWidth)),
  };
}

export class Terminals {
  private readonly scrollback: number;
  private readonly open = new Set<Terminal>();
  private font: ITerminalOptions = {};
  private size: Size = { rows: 24, cols: 80 };
  private resized: Promise<void> = Promise.resolve();

  private constructor(scrollback: number) {
    this.scrollback = scrollback;
  }

  static async load(): Promise<Terminals> {
    return new Terminals(await commands.historyLines());
  }

  create(container: HTMLElement): Terminal {
    const terminal = new Terminal({
      scrollback: this.scrollback,
      linkHandler: { activate: openLink },
      ...this.font,
      cursorBlink: true,
      cursorStyle: "bar",
      cursorInactiveStyle: "bar",
      drawBoldTextInBrightColors: true,
      allowTransparency: true,
      theme: THEME,
    });
    terminal.loadAddon(new WebLinksAddon(openLink));
    terminal.open(container);
    terminal.resize(this.size.cols, this.size.rows);
    this.open.add(terminal);
    this.fit();
    return terminal;
  }

  dispose(terminal: Terminal) {
    this.open.delete(terminal);
    terminal.dispose();
  }

  settled(): Promise<void> {
    return this.resized;
  }

  setFont(font: Font) {
    this.font = {
      fontFamily: `${font.family}, monospace`,
      fontSize: font.size,
    };
    for (const terminal of this.open)
      Object.assign(terminal.options, this.font);
    requestAnimationFrame(() => this.fit());
  }

  fit() {
    for (const terminal of this.open) {
      const measured = measure(terminal);
      if (measured) return this.apply(measured);
    }
  }

  private apply(measured: Size) {
    if (measured.rows === this.size.rows && measured.cols === this.size.cols)
      return;
    this.size = measured;
    for (const terminal of this.open)
      terminal.resize(measured.cols, measured.rows);
    this.resized = reportFailure(
      commands.terminalResize(measured.rows, measured.cols),
    );
  }
}
