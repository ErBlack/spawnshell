const PROMPT = "guest@spawnshell:~$ ";
const ESC = "\x1b";

export class Shell {
  private readonly help: Record<string, string>;
  private readonly print: (text: string) => void;
  private line = "";

  constructor(help: Record<string, string>, print: (text: string) => void) {
    this.help = help;
    this.print = (text) => print(text.replace(/\r?\n/g, "\r\n"));
  }

  start(line: string) {
    const now = new Date();
    const part = (options: Intl.DateTimeFormatOptions) =>
      now.toLocaleString("en-US", options);
    const day = String(now.getDate()).padStart(2, " ");
    const time = part({
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hourCycle: "h23",
    });
    this.line = line;
    this.print(
      `Last login: ${part({ weekday: "short" })} ${part({ month: "short" })} ${day} ${time} on ttys000\n${PROMPT}${line}`,
    );
  }

  input(data: string) {
    if (data.startsWith(ESC)) return;
    for (const char of data.replaceAll("\r\n", "\r")) this.key(char);
  }

  private key(key: string) {
    switch (key) {
      case "\r":
        this.print(`\n${this.run(this.line.trim())}${PROMPT}`);
        this.line = "";
        return;
      case "\x7f":
        if (!this.line) return;
        this.line = [...this.line].slice(0, -1).join("");
        this.print("\b \b");
        return;
      case "\x03":
        this.line = "";
        this.print(`^C\n${PROMPT}`);
        return;
    }
    if (key >= " ") {
      this.line += key;
      this.print(key);
    }
  }

  private run(command: string): string {
    if (!command) return "";
    const [name, ...args] = command.split(/\s+/);
    if (name !== "spawnshell") return `bash: ${name}: command not found\n`;
    return this.help[args.join(" ")] ?? this.help["--help"];
  }
}
