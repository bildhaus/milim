export interface CommandPaletteItem {
  id: string;
  label: string;
  keywords?: string[];
  shortcut?: string;
  /** Secondary meta shown when the command has no shortcut, such as `/compact`. */
  detail?: string;
  /** Shown before the user types; everything else appears once a query matches. */
  featured?: boolean;
  available?: boolean;
}

function labelRank(item: CommandPaletteItem, query: string, terms: string[]): number {
  const label = item.label.toLowerCase();
  if (label.startsWith(query)) return 0;
  if (terms.every((term) => label.includes(term))) return 1;
  return 2;
}

export function filterCommandPaletteItems<T extends CommandPaletteItem>(
  items: readonly T[],
  query: string,
): T[] {
  const normalized = query.trim().toLowerCase();
  const terms = normalized.split(/\s+/).filter(Boolean);
  const matches = items.filter((item) => {
    if (item.available === false) return false;
    if (!terms.length) return true;
    const haystack = [item.label, item.detail ?? "", ...(item.keywords ?? [])].join(" ").toLowerCase();
    return terms.every((term) => haystack.includes(term));
  });
  if (!terms.length) return matches;
  // Stable: label matches first, then keyword-only matches, each in registry order.
  return matches
    .map((item, index) => ({ item, index, rank: labelRank(item, normalized, terms) }))
    .sort((a, b) => a.rank - b.rank || a.index - b.index)
    .map(({ item }) => item);
}

/** Featured commands for an empty query; the full registry once the user types. */
export function commandPaletteResults<T extends CommandPaletteItem>(
  items: readonly T[],
  query: string,
): T[] {
  if (!query.trim() && items.some((item) => item.featured)) {
    return filterCommandPaletteItems(items.filter((item) => item.featured), "");
  }
  return filterCommandPaletteItems(items, query);
}
