export function parseArgs(argv) {
  const [command = "report", ...rest] = argv;
  return { command, flags: parseFlags(rest) };
}

export function parseFlags(list) {
  return Object.fromEntries(list.map((flag) => flag.replace(/^--/, "").split("=")));
}

export function helpText() {
  return "usage: tool [report|sync] [--key=value]";
}
