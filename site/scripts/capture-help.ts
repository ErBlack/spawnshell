import { captureHelp } from "../build/capture-help.ts";

const binary = process.argv[2];
if (!binary) throw new Error("usage: node capture-help.ts <spawnshell binary>");
process.stdout.write(JSON.stringify(captureHelp(binary)));
