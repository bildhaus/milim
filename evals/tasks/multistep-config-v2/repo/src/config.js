const DEFAULTS = { host: "127.0.0.1", port: 8080, features: [] };

/**
 * Parse a v1 config: one `key = value` per line, `#` comments, and blank
 * lines. Known keys are `host`, `port`, and `features` (comma separated).
 */
export function loadConfig(text) {
  const config = { ...DEFAULTS, features: [...DEFAULTS.features] };
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (line === "" || line.startsWith("#")) continue;
    const [key, ...rest] = line.split("=");
    const value = rest.join("=").trim();
    switch (key.trim()) {
      case "host":
        config.host = value;
        break;
      case "port":
        config.port = Number(value);
        break;
      case "features":
        config.features = value.split(",").map((item) => item.trim()).filter(Boolean);
        break;
      default:
        throw new Error(`unknown config key: ${key.trim()}`);
    }
  }
  return config;
}
