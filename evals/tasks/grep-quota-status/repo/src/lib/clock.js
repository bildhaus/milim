let offset = 0;

export function now() {
  return Date.now() + offset;
}

export function advance(ms) {
  offset += ms;
}
