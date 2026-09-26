# Configuration

Configs use a simple `key = value` format:

```
host = 0.0.0.0
port = 9090
features = metrics, audit-log
```

Unknown keys are rejected. Missing keys fall back to `127.0.0.1`, `8080`,
and no features.
