import { useEffect, useRef, useState } from 'react';
import { useParams, useSearchParams } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { getCapabilities } from '../api/capabilities';
import { sendTokenReply, sendNavigateTo } from '../lib/postMessage';
import { getApiToken } from '../api/client';

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
  const loadTimeout = useRef<ReturnType<typeof setTimeout>>(undefined);

  const { data: capabilities } = useQuery({
    queryKey: ['capabilities'],
    queryFn: getCapabilities,
  });

  const cap = capabilities?.find(c => c.name === name);
  const token = getApiToken();

  // Reply to token requests and send deep-link params after howm:ready.
  // Also start a 10s timeout — if the capability never sends howm:ready,
  // assume it failed to load and show an error state.
  // Note: setLoadError is only called from callbacks (timeout, message handler),
  // never synchronously in the effect body, to satisfy the react-hooks lint rule.
  useEffect(() => {
    if (!token) return;
    readySent.current = false;

    // Start load timeout — capability should signal howm:ready within 10s
    loadTimeout.current = setTimeout(() => {
      if (!readySent.current) {
        setLoadError(true);
      }
    }, 10_000);

    function handle(e: MessageEvent) {
      if (e.origin !== window.location.origin) return;
      if (e.data?.type === 'howm:token:request' && iframeRef.current) {
        sendTokenReply(iframeRef.current, token!);
      }
      // After capability signals ready, send any URL search params as deep-link
      if (e.data?.type === 'howm:ready' && iframeRef.current && !readySent.current) {
        readySent.current = true;
        clearTimeout(loadTimeout.current);
        setLoadError(false);
        const params: Record<string, string> = {};
        searchParams.forEach((v, k) => { params[k] = v; });
        if (Object.keys(params).length > 0) {
          sendNavigateTo(iframeRef.current, params);
        }
      }
    }
    window.addEventListener('message', handle);
    return () => {
      window.removeEventListener('message', handle);
      clearTimeout(loadTimeout.current);
    };
  }, [token, searchParams]);

  // Re-send deep-link params when searchParams change while iframe is open
  useEffect(() => {
    if (!iframeRef.current || !readySent.current) return;
    const params: Record<string, string> = {};
    searchParams.forEach((v, k) => { params[k] = v; });
    if (Object.keys(params).length > 0) {
      sendNavigateTo(iframeRef.current, params);
    }
  }, [searchParams]);

  if (!capabilities) {
    return <div style={loadingStyle}>Loading…</div>;
  }

  if (!cap?.ui) {
    return (
      <div style={loadingStyle}>
        Capability <strong>{name}</strong> not found or has no UI.
      </div>
    );
  }

  // Build the iframe src — route through the daemon proxy at /cap/{prefix}/...
  // Capability name "social.feed" → proxy prefix "feed" (last segment after '.')
  const segments = cap.name.split('.');
  const proxyPrefix = segments[segments.length - 1];
  const src = cap.ui.entry.startsWith('/')
    ? `/cap/${proxyPrefix}${cap.ui.entry}`
    : `/cap/${proxyPrefix}/${cap.ui.entry}`;

  if (loadError) {
    return (
      <div style={errorStyle}>
        <div style={{ fontSize: '2rem', marginBottom: '0.5rem' }}>⚠</div>
        <div><strong>{cap.ui.label}</strong> failed to load.</div>
        <div style={{ color: 'var(--howm-text-muted, #5c6170)', fontSize: '0.85rem', marginTop: '0.25rem' }}>
          The capability process may not be running.
        </div>
        <button
          onClick={() => { setLoadError(false); readySent.current = false; }}
          style={retryButtonStyle}
        >
          Retry
        </button>
      </div>
    );
  }

  return (
    <iframe
      ref={iframeRef}
      src={src}
      title={cap.ui.label}
      style={iframeStyle}
      // Restrict iframe capabilities; adjust as needed for specific caps
      sandbox="allow-scripts allow-same-origin allow-forms"
      onError={() => setLoadError(true)}
    />
  );
}

const loadingStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  height: 'calc(100vh - 48px)',
  color: 'var(--howm-text-muted, #5c6170)',
};

const errorStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  alignItems: 'center',
  justifyContent: 'center',
  height: 'calc(100vh - 48px)',
  color: 'var(--howm-text, #e0e0e0)',
  textAlign: 'center',
  gap: '0.25rem',
};

const retryButtonStyle: React.CSSProperties = {
  marginTop: '1rem',
  padding: '0.5rem 1.5rem',
  borderRadius: '6px',
  border: '1px solid var(--howm-border, #333)',
  background: 'var(--howm-surface, #1a1a2e)',
  color: 'var(--howm-text, #e0e0e0)',
  cursor: 'pointer',
  fontSize: '0.9rem',
};

const iframeStyle: React.CSSProperties = {
  width: '100%',
  height: 'calc(100vh - 48px)',
  border: 'none',
  display: 'block',
};
