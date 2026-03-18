import { useEffect, useRef } from 'react';
import { useParams } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { getCapabilities } from '../api/capabilities';
import { sendTokenReply } from '../lib/postMessage';
import { getApiToken } from '../api/client';

/**
 * Full-height iframe wrapper for a capability UI.
 * Route: /cap/:name
 *
 * The iframe src is the capability's ui.entry URL (routed through the daemon
 * at /cap/<name>/ui/ by default).
 *
 * Token handshake: capability posts howm:token:request → shell listens and
 * calls sendTokenReply on the iframe. We also pass it as a URL param as a
 * convenience for capabilities that prefer that.
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

  // Build the iframe src — append token as query param so the page can use it
  // immediately without waiting for the postMessage round-trip.
  const src = token
    ? `${cap.ui.entry}?token=${encodeURIComponent(token)}`
    : cap.ui.entry;

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
