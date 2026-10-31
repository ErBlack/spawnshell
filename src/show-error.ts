import { commands, events, logError } from "./backend.ts";

export function showError(message: string) {
  logError(message);
  commands.reportError(message);
}

export function listenWarnings() {
  return events.warning.listen(({ payload }) => showError(payload));
}

export function showFatal(message: string) {
  logError(`failed to start: ${message}`);
  commands.reportFatal(message);
}

export async function showPendingWarning() {
  const warning = await commands.takeWarning();
  if (warning) showError(warning);
}
