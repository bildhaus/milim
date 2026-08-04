import { useCallback, useEffect, useRef, useState } from "react";
import {
  callMcpAppTool,
  readMcpAppResource,
  readMcpAppView,
  type McpAppDescriptor,
  type ToolApprovalMode,
} from "../api";
import { mcpAppFallbackText, parseMcpAppArguments } from "../lib/mcpApps";
import { useTheme } from "../theme/store";

type AppBridgeInstance = import("@modelcontextprotocol/ext-apps/app-bridge").AppBridge;
type AppToolHandler = NonNullable<AppBridgeInstance["oncalltool"]>;
type AppToolParams = Parameters<AppToolHandler>[0];
type AppToolResult = Awaited<ReturnType<AppToolHandler>>;
type AppReadHandler = NonNullable<AppBridgeInstance["onreadresource"]>;
type AppReadResult = Awaited<ReturnType<AppReadHandler>>;

type PendingApproval = {
  params: AppToolParams;
  resolve: (result: AppToolResult) => void;
  reject: (error: Error) => void;
};

type McpAppViewProps = {
  descriptor: McpAppDescriptor;
  argumentsText?: string;
  result?: unknown;
  status?: "running" | "done" | "error";
  approval: ToolApprovalMode;
};

const deniedResult: AppToolResult = {
  content: [{ type: "text", text: "The user denied this MCP App tool call." }],
  isError: true,
};

function asAppToolResult(value: unknown): AppToolResult {
  return value as AppToolResult;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function maxAppHeight(): number {
  return Math.max(120, Math.min(window.innerHeight * 0.7, 720));
}

export function McpAppView({
  descriptor,
  argumentsText,
  result,
  status = "done",
  approval,
}: McpAppViewProps) {
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const bridgeRef = useRef<AppBridgeInstance | null>(null);
  const sentResultRef = useRef<unknown>(undefined);
  const activeRef = useRef(0);
  const approvalRef = useRef(approval);
  approvalRef.current = approval;
  const theme = useTheme((state) => state.theme);
  const [viewUrl, setViewUrl] = useState<string | null>(null);
  const [viewStatus, setViewStatus] = useState<
    "loading" | "connecting" | "ready" | "error"
  >("loading");
  const [height, setHeight] = useState(320);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const [pending, setPending] = useState<PendingApproval | null>(null);
  const pendingRef = useRef<PendingApproval | null>(null);
  pendingRef.current = pending;
  const fallbackText = mcpAppFallbackText(result);
  const tool = descriptor.tool ?? {};
  const toolTitle =
    typeof tool.title === "string" && tool.title.trim()
      ? tool.title
      : typeof tool.name === "string" && tool.name.trim()
        ? tool.name.replace(/[_-]+/g, " ")
        : "Interactive view";

  const callTool = useCallback(
    async (params: AppToolParams, approvalGranted = false) =>
      asAppToolResult(
        await callMcpAppTool(
          descriptor.server_id,
          params.name,
          params.arguments ?? {},
          approvalRef.current,
          approvalGranted,
        ),
      ),
    [descriptor.server_id],
  );

  useEffect(() => {
    let cancelled = false;
    setViewUrl(null);
    setViewStatus("loading");
    setError(null);
    void readMcpAppView(descriptor.server_id, descriptor.resource_uri)
      .then(({ viewUrl: url }) => {
        if (cancelled) return;
        setViewUrl(url);
        setViewStatus("connecting");
      })
      .catch((reason) => {
        if (cancelled) return;
        setError(errorMessage(reason));
        setViewStatus("error");
      });
    return () => {
      cancelled = true;
    };
  }, [descriptor.resource_uri, descriptor.server_id, retry]);

  const connect = useCallback(async () => {
    const iframe = iframeRef.current;
    const target = iframe?.contentWindow;
    if (!target || !viewUrl) return;
    const connection = ++activeRef.current;
    setViewStatus("connecting");
    setError(null);
    sentResultRef.current = undefined;
    const previous = bridgeRef.current;
    bridgeRef.current = null;
    previous?.close();
    try {
      const { AppBridge, PostMessageTransport } = await import(
        "@modelcontextprotocol/ext-apps/app-bridge"
      );
      if (connection !== activeRef.current) return;
      const bridge = new AppBridge(
        null,
        { name: "Milim", version: "0.1.29" },
        { serverTools: {}, serverResources: {} },
        {
          hostContext: {
            theme: theme.isDark ? "dark" : "light",
            displayMode: "inline",
            availableDisplayModes: ["inline"],
            containerDimensions: { maxHeight: maxAppHeight() },
            locale: navigator.language,
            timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
            platform: "desktop",
            toolInfo: {
              tool: tool as never,
            },
          },
        },
      );
      bridge.oncalltool = async (params) => {
        if (approvalRef.current !== "review") return callTool(params);
        return new Promise<AppToolResult>((resolve, reject) => {
          pendingRef.current?.reject(new Error("A newer MCP App tool call replaced this request"));
          setPending({ params, resolve, reject });
        });
      };
      bridge.onreadresource = async ({ uri }) =>
        (await readMcpAppResource(
          descriptor.server_id,
          uri,
        )) as AppReadResult;
      bridge.onsizechange = ({ height: requestedHeight }) => {
        if (typeof requestedHeight !== "number" || !Number.isFinite(requestedHeight)) return;
        setHeight(Math.max(120, Math.min(requestedHeight, maxAppHeight())));
      };
      bridge.onrequestdisplaymode = async () => ({ mode: "inline" });
      bridge.onrequestteardown = () => {
        void bridge.teardownResource({}).finally(() => {
          bridge.close();
          setError("MCP App disconnected");
          setViewStatus("error");
        });
      };
      bridge.oninitialized = () => {
        if (connection !== activeRef.current) return;
        setViewStatus("ready");
        void bridge.sendToolInput({
          arguments: parseMcpAppArguments(argumentsText),
        }).then(() => {
          if (status === "running" || result === undefined) return;
          sentResultRef.current = result;
          return bridge.sendToolResult(asAppToolResult(result));
        });
      };
      bridgeRef.current = bridge;
      await bridge.connect(new PostMessageTransport(target, target));
    } catch (reason) {
      if (connection !== activeRef.current) return;
      bridgeRef.current?.close();
      bridgeRef.current = null;
      setError(errorMessage(reason));
      setViewStatus("error");
    }
  }, [argumentsText, callTool, descriptor, result, status, theme.isDark, viewUrl]);

  useEffect(() => {
    if (!viewUrl) return;
    void connect().then(() => {
      const iframe = iframeRef.current;
      if (iframe && iframe.src !== viewUrl) iframe.src = viewUrl;
    });
  }, [viewUrl]);

  useEffect(() => {
    const bridge = bridgeRef.current;
    if (!bridge || viewStatus !== "ready" || status === "running" || result === undefined) return;
    if (sentResultRef.current === result) return;
    sentResultRef.current = result;
    void bridge.sendToolResult(asAppToolResult(result)).catch((reason) => {
      setError(errorMessage(reason));
      setViewStatus("error");
    });
  }, [result, status, viewStatus]);

  useEffect(() => {
    if (viewStatus !== "ready") return;
    void bridgeRef.current?.sendHostContextChange({
      theme: theme.isDark ? "dark" : "light",
    });
  }, [theme.isDark, viewStatus]);

  useEffect(
    () => () => {
      activeRef.current += 1;
      const bridge = bridgeRef.current;
      bridgeRef.current = null;
      if (!bridge) return;
      void Promise.race([
        bridge.teardownResource({}).catch(() => ({})),
        new Promise<Record<string, unknown>>((resolve) =>
          window.setTimeout(() => resolve({}), 250),
        ),
      ]).finally(() => bridge.close());
      pendingRef.current?.reject(new Error("MCP App closed"));
    },
    [],
  );

  const approve = () => {
    if (!pending) return;
    const request = pending;
    setPending(null);
    void callTool(request.params, true).then(request.resolve, request.reject);
  };

  const deny = () => {
    pending?.resolve(deniedResult);
    setPending(null);
  };

  return (
    <section className="mcp-app-card" data-testid="mcp-app-view" aria-label={`${toolTitle} MCP App`}>
      <div className="mcp-app-header">
        <div className="mcp-app-title" title={toolTitle}>
          <span>MCP App</span>
        </div>
        <span className="mcp-app-live" title={descriptor.resource_uri}>Live</span>
      </div>
      {viewStatus === "loading" || viewStatus === "connecting" ? (
        <div className="mcp-app-state" role="status">
          {viewStatus === "loading" ? "Loading app..." : "Connecting app..."}
        </div>
      ) : null}
      {error ? (
        <div className="mcp-app-state mcp-app-error" role="alert">
          <span>{error}</span>
          <button className="approval-btn" type="button" onClick={() => setRetry((value) => value + 1)}>
            Retry
          </button>
          {fallbackText ? <pre className="mcp-app-fallback">{fallbackText}</pre> : null}
        </div>
      ) : null}
      {pending ? (
        <div className="mcp-app-approval" role="alertdialog" aria-label="Approve MCP App tool call">
          <strong>Allow {pending.params.name}?</strong>
          <pre>{JSON.stringify(pending.params.arguments ?? {}, null, 2)}</pre>
          <div>
            <button className="approval-btn deny" type="button" onClick={deny}>Deny</button>
            <button className="approval-btn approve" type="button" onClick={approve}>Approve once</button>
          </div>
        </div>
      ) : null}
      {viewUrl ? (
        <iframe
          ref={iframeRef}
          title={`MCP App ${descriptor.resource_uri}`}
          className="mcp-app-frame"
          sandbox="allow-scripts allow-forms"
          style={{ height }}
        />
      ) : null}
    </section>
  );
}
