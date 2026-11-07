export function postMessage<T>(target: Window, message: T) {
  target.postMessage({ spawnshell: message }, location.origin);
}
