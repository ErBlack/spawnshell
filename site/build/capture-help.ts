import { spawnSync } from "node:child_process";

function runHelp(binary: string, args: string[], color: boolean): string {
  const result = spawnSync(binary, args, {
    encoding: "utf8",
    env: {
      ...process.env,
      CLICOLOR_FORCE: color ? "1" : undefined,
      NO_COLOR: color ? undefined : "1",
    },
  });
  if (result.error) throw result.error;
  return result.stdout + result.stderr;
}

export function captureHelp(binary: string): Record<string, string> {
  const plain = runHelp(binary, ["--help"], false);
  const section = plain.split(/^Commands:$/m)[1]?.split(/^\S/m)[0] ?? "";

  const subcommands = [...section.matchAll(/^ {2}(\S+)/gm)]
    .map((match) => match[1])
    .filter((name) => name !== "help");

  const variants = [["--help"], ["-h"], ["help"]];

  for (const name of subcommands)
    variants.push([name, "--help"], [name, "-h"], ["help", name]);
  return Object.fromEntries(
    variants.map((args) => [args.join(" "), runHelp(binary, args, true)]),
  );
}
