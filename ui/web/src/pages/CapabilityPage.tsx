import { useEffect, useRef } from 'react';
import { useParams } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { getCapabilities } from '../api/capabilities';
import { sendTokenReply } from '../lib/postMessage';
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
  const iframeRef = useRef<HTMLIFrameElement>(null);

  const { data: capabilities } = useQuery({
    queryKey: ['capabilities'],
    queryFn: getCapabilities,
  });

  const cap = capabilities?.find(c => c.name === name);
  const token = getApiToken();

  // Reply to token requests from the iframe
  useEffect(() => {
    if (!token) return;
    function handle(e: MessageEvent) {
      if (e.origin !== window.location.origin) return;
      if (e.data?.type === 'howm:token:request' && iframeRef.current) {
        sendTokenReply(iframeRef.current, token!);
      }
    }
    window.addEventListener('message', handle);
    return () => window.removeEventListener('message', handle);
  }, [token]);

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

  return (
    <iframe
      ref={iframeRef}
      src={src}
      title={cap.ui.label}
      style={iframeStyle}
      // Restrict iframe capabilities; adjust as needed for specific caps
      sandbox="allow-scripts allow-same-origin allow-forms"
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

const iframeStyle: React.CSSProperties = {
  width: '100%',
  height: 'calc(100vh - 48px)',
  border: 'none',
  display: 'block',
};
