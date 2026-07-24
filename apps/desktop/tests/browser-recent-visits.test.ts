class MemoryStorage implements Storage {
  private values = new Map<string, string>();

  get length(): number { return this.values.size; }
  clear(): void { this.values.clear(); }
  getItem(key: string): string | null { return this.values.get(key) ?? null; }
  key(index: number): string | null { return Array.from(this.values.keys())[index] ?? null; }
  removeItem(key: string): void { this.values.delete(key); }
  setItem(key: string, value: string): void { this.values.set(key, value); }
}

Object.defineProperty(globalThis, "localStorage", {
  value: new MemoryStorage(),
  configurable: true,
});

const { useBrowserRecentVisits } = await import("../src/browser/recentVisits.js");

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

useBrowserRecentVisits.getState().recordVisit("https://example.com/one", "First");
useBrowserRecentVisits.getState().recordVisit("https://example.com/two", "Second");
useBrowserRecentVisits.getState().recordVisit("https://example.com/one", "First again");
useBrowserRecentVisits.getState().recordVisit("javascript:alert(1)", "Blocked");

const visits = useBrowserRecentVisits.getState().visits;
assert(visits.length === 2, "recent visits should reject invalid URLs and deduplicate");
assert(visits[0]?.url === "https://example.com/one", "revisited URLs should move to the front");
assert(visits[0]?.title === "First again", "revisited URLs should refresh their title");
assert(localStorage.getItem("milim.browserRecentVisits")?.includes("https://example.com/one"), "recent visits should persist outside settings");

useBrowserRecentVisits.getState().clearVisits();
assert(useBrowserRecentVisits.getState().visits.length === 0, "recent visits should be clearable");

export {};
