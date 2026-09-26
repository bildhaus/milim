export function pad(text, width) {
  return String(text).padEnd(width);
}

export function shout(text) {
  return `${String(text).toUpperCase()}!`;
}

export function kebab(text) {
  return String(text).toLowerCase().replace(/\s+/g, "-");
}
