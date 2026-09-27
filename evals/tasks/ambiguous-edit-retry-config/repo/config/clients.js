// HTTP client settings per environment. Every client retries idempotent
// requests with exponential backoff; see src/client.js.
export const CLIENTS = {
  staging: {
    auth: {
      baseUrl: "https://auth.staging.internal",
      timeoutMs: 2000,
      retry: { attempts: 3, backoffMs: 250 },
    },
    payments: {
      baseUrl: "https://payments.staging.internal",
      timeoutMs: 5000,
      retry: { attempts: 3, backoffMs: 250 },
    },
    paymentWebhooks: {
      baseUrl: "https://payment-webhooks.staging.internal",
      timeoutMs: 5000,
      retry: { attempts: 3, backoffMs: 250 },
    },
    search: {
      baseUrl: "https://search.staging.internal",
      timeoutMs: 1500,
      retry: { attempts: 3, backoffMs: 250 },
    },
    notifications: {
      baseUrl: "https://notifications.staging.internal",
      timeoutMs: 3000,
      retry: { attempts: 3, backoffMs: 250 },
    },
  },
  production: {
    auth: {
      baseUrl: "https://auth.internal",
      timeoutMs: 2000,
      retry: { attempts: 3, backoffMs: 250 },
    },
    payments: {
      baseUrl: "https://payments.internal",
      timeoutMs: 5000,
      retry: { attempts: 3, backoffMs: 250 },
    },
    paymentWebhooks: {
      baseUrl: "https://payment-webhooks.internal",
      timeoutMs: 5000,
      retry: { attempts: 3, backoffMs: 250 },
    },
    search: {
      baseUrl: "https://search.internal",
      timeoutMs: 1500,
      retry: { attempts: 3, backoffMs: 250 },
    },
    notifications: {
      baseUrl: "https://notifications.internal",
      timeoutMs: 3000,
      retry: { attempts: 3, backoffMs: 250 },
    },
  },
};
