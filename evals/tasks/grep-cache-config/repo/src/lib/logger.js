const lines = [];

export function log(level, event, fields = {}) {
  lines.push({ level, event, ...fields });
}

export function drainLogs() {
  return lines.splice(0, lines.length);
}
