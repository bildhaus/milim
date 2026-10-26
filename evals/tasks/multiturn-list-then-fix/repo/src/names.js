// Display names for customer records.

/** "  ada   LOVELACE " -> "Ada Lovelace". */
export function displayName(raw) {
  return String(raw)
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .map((part) => part[0].toUpperCase() + part.slice(1).toLowerCase())
    .join(" ");
}

/** Initials of a display name, at most three letters. */
export function initials(raw) {
  return displayName(raw)
    .split(" ")
    .slice(0, 3)
    .map((part) => part[0])
    .join("");
}
