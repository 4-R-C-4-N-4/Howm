import { useEffect, useRef, useState } from "react";
import { useParams, useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { getCapabilities } from "../api/capabilities";
import { sendTokenReply, sendNavigateTo } from "../lib/postMessage";
import { getApiToken } from "../api/client";

/**
 * Full-height iframe wrapper for a capability UI.
 * Route: /app/:name   (SPA route — distinct from daemon's /cap/:name API proxy)
 *
 * The iframe src is built from the capability's ui.entry and routed through
 * the daemon proxy at /cap/<prefix>/ui/. The prefix is the last dot-segment
 * of the capability name (e.g. "social.feed" → prefix "feed").
 *
 * Token handshake: capability posts howm:token:request → shell replies via
 * postMessage with same-origin check. Tokens are NEVER placed in URLs
 * (they leak via Referer headers, browser history, and server logs).
 */
export function CapabilityPage() {
  const { name } = useParams<{ name: string }>();
  const [searchParams] = useSearchParams();
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const readySent = useRef(false);
  const [loadError, setLoadError] = useState(false);
  const [loading, setLoading] = useState(true);
  const loadTimeout = useRef<ReturnType<typeof setTimeout>>(undefined);

  const { data: capabilities } = useQuery({
    queryKey: ["capabilities"],
    queryFn: getCapabilities,
  });

  const cap = capabilities?.find((c) => c.name === name);
  const token = getApiToken();

  // Reset loading / error state during render when the token changes
  // (React-recommended "adjust state when a prop changes" pattern — no effect needed).
  // See: https://react.dev/learn/you-might-not-need-an-effect#adjusting-some-state-when-a-prop-changes
  const [prevToken, setPrevToken] = useState(token);
  if (prevToken !== token) {
    setPrevToken(token);
    setLoading(true);
    setLoadError(false);
  }

  // Reply to token requests and send deep-link params after howm:ready.
  // Also start a 10s timeout — if the capability never sends howm:ready,
  // assume it failed to load and show an error state.
  useEffect(() => {
    if (!token) return;
    readySent.current = false;

    // Start load timeout — capability should signal howm:ready within 10s
    loadTimeout.current = setTimeout(() => {
      if (!readySent.current) {
        setLoading(false);
        setLoadError(true);
      }
    }, 10_000);

    function handle(e: MessageEvent) {
      if (e.origin !== window.location.origin) return;
      if (e.data?.type === "howm:token:request" && iframeRef.current) {
        sendTokenReply(iframeRef.current, token!, name);
      }
      // After capability signals ready, send any URL search params as deep-link
      if (
        e.data?.type === "howm:ready" &&
        iframeRef.current &&
        !readySent.current
      ) {
        readySent.current = true;
        clearTimeout(loadTimeout.current);
        setLoading(false);
        setLoadError(false);
        const params: Record<string, string> = {};
        searchParams.forEach((v, k) => {
          params[k] = v;
        });
        if (Object.keys(params).length > 0) {
          sendNavigateTo(iframeRef.current, params);
        }
      }
    }
    window.addEventListener("message", handle);
    return () => {
      window.removeEventListener("message", handle);
      clearTimeout(loadTimeout.current);
    };
  }, [token, searchParams, name]);

  // Re-send deep-link params when searchParams change while iframe is open
  useEffect(() => {
    if (!iframeRef.current || !readySent.current) return;
    const params: Record<string, string> = {};
    searchParams.forEach((v, k) => {
      params[k] = v;
    });
    if (Object.keys(params).length > 0) {
      sendNavigateTo(iframeRef.current, params);
    }
  }, [searchParams]);

  if (!capabilities) {
    return (
      <div className="flex items-center justify-center h-[calc(100vh-48px)] text-howm-text-muted">
        Loading…
      </div>
    );
  }

  if (!cap?.ui) {
    return (
      <div className="flex items-center justify-center h-[calc(100vh-48px)] text-howm-text-muted">
        Capability <strong>{name}</strong> not found or has no UI.
      </div>
    );
  }

  // Build the iframe src — route through the daemon proxy at /cap/{prefix}/...
  // Use the authoritative route_name set at install time; fall back to last
  // segment of name only for capabilities installed before route_name existed.
  const proxyPrefix = cap.route_name ?? cap.name.split(".").pop()!;
  const src = cap.ui.entry.startsWith("/")
    ? `/cap/${proxyPrefix}${cap.ui.entry}`
    : `/cap/${proxyPrefix}/${cap.ui.entry}`;

  if (loadError) {
    return (
      <div
        className="flex flex-col items-center justify-center h-[calc(100vh-48px)] text-center gap-1"
        style={{ color: "var(--howm-text, #e0e0e0)" }}
      >
        <div className="text-3xl mb-2">⚠</div>
        <div>
          <strong>{cap.ui.label}</strong> failed to load.
        </div>
        <div className="text-howm-text-muted text-sm mt-1">
          The capability process may not be running.
        </div>
        <button
          onClick={() => {
            setLoadError(false);
            readySent.current = false;
          }}
          className="mt-4 py-2 px-6 rounded-md border border-howm-border bg-howm-bg-elevated cursor-pointer text-sm"
          style={{ color: "var(--howm-text, #e0e0e0)" }}
        >
          Retry
        </button>
      </div>
    );
  }

  return (
    <div className="relative w-full h-[calc(100vh-48px)]">
      {loading && (
        <div className="absolute inset-0 flex flex-col items-center justify-center bg-howm-bg-primary z-10">
          <div className="w-7 h-7 border-3 border-howm-border border-t-howm-accent rounded-full animate-spin" />
          <div className="mt-3 text-howm-text-muted text-sm">
            Loading {cap.ui.label}…
          </div>
        </div>
      )}
      <iframe
        ref={iframeRef}
        src={src}
        title={cap.ui.label}
        className="w-full h-full border-none block"
        // Restrict iframe capabilities; adjust as needed for specific caps
        sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox"
        onError={() => {
          setLoading(false);
          setLoadError(true);
        }}
      />
    </div>
  );
}
