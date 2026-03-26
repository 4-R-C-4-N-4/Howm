import { useState, useRef, useEffect } from 'react';
import { useQuery } from '@tanstack/react-query';
import { getCapabilities, type Capability } from '../api/capabilities';
import { getBadges } from '../api/notifications';
import { getApiToken } from '../api/client';
import { sendTokenReply } from '../lib/postMessage';
import { CapIcon } from './icons';

/**
 * Renders floating action buttons for capabilities with ui.style === "fab".
 * Each FAB opens an overlay panel containing the capability's iframe.
 */
export function FabLayer() {
  const { data: capabilities } = useQuery({
    queryKey: ['capabilities'],
    queryFn: getCapabilities,
    refetchInterval: 60000,
  });

  const { data: badgeData } = useQuery({
    queryKey: ['badges'],
    queryFn: getBadges,
    refetchInterval: 5_000,
  });
  const badges = badgeData?.badges ?? {};

  const fabCaps = capabilities?.filter(c => c.ui?.style === 'fab') ?? [];

  if (fabCaps.length === 0) return null;

  return (
    <div className="fixed bottom-6 right-6 flex flex-col-reverse gap-3 z-200">
      {fabCaps.map((cap, i) => (
        <FabButton key={cap.name} cap={cap} badgeCount={badges[cap.name] ?? 0} index={i} />
      ))}
    </div>
  );
}

function FabButton({ cap, badgeCount, index }: { cap: Capability; badgeCount: number; index: number }) {
  const [open, setOpen] = useState(false);
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const token = getApiToken();

  // Token handshake for iframe
  useEffect(() => {
    if (!open || !token) return;
    function handle(e: MessageEvent) {
      if (e.origin !== window.location.origin) return;
      if (e.data?.type === 'howm:token:request' && iframeRef.current) {
        sendTokenReply(iframeRef.current, token!, cap.name);
      }
    }
    window.addEventListener('message', handle);
    return () => window.removeEventListener('message', handle);
  }, [open, token, cap.name]);

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    function handleClick(e: MouseEvent) {
      if (panelRef.current && !panelRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    // Delay to avoid closing immediately from the FAB click
    const timer = setTimeout(() => document.addEventListener('mousedown', handleClick), 100);
    return () => { clearTimeout(timer); document.removeEventListener('mousedown', handleClick); };
  }, [open]);

  const routeName = cap.route_name ?? cap.name.split('.').pop() ?? cap.name;
  const iframeSrc = `/cap/${routeName}/ui/${cap.ui!.entry}`;

  return (
    <>
      {/* FAB button */}
      <button
        onClick={() => setOpen(!open)}
        className="relative w-14 h-14 rounded-full bg-howm-accent text-white shadow-[0_4px_16px_rgba(0,0,0,0.5)] cursor-pointer border-none flex items-center justify-center hover:bg-howm-accent-hover transition-colors"
        title={cap.ui!.label}
      >
        <CapIcon icon={cap.ui!.icon} />
        {badgeCount > 0 && (
          <span className="absolute -top-1 -right-1 bg-howm-error text-white text-[0.65rem] font-bold rounded-full min-w-5 h-5 flex items-center justify-center px-1">
            {badgeCount > 99 ? '99+' : badgeCount}
          </span>
        )}
      </button>

      {/* Panel overlay */}
      {open && (
        <div
          ref={panelRef}
          className="fixed bottom-24 right-6 w-[380px] h-[560px] max-w-[calc(100vw-48px)] max-h-[calc(100vh-120px)] bg-howm-bg-surface border border-howm-border rounded-xl shadow-[0_12px_40px_rgba(0,0,0,0.6)] flex flex-col overflow-hidden z-200 sm:max-w-[380px]"
          style={{ bottom: `${96 + index * 72}px` }}
        >
          {/* Panel header */}
          <div className="flex items-center justify-between px-4 py-2.5 border-b border-howm-border bg-howm-bg-elevated shrink-0">
            <span className="font-semibold text-sm text-howm-text-primary">{cap.ui!.label}</span>
            <button
              onClick={() => setOpen(false)}
              className="bg-transparent border-none text-howm-text-muted cursor-pointer text-base p-1 leading-none hover:text-howm-text-primary"
            >
              ✕
            </button>
          </div>

          {/* Iframe */}
          <iframe
            ref={iframeRef}
            src={iframeSrc}
            className="flex-1 border-none w-full"
            title={cap.ui!.label}
          />
        </div>
      )}
    </>
  );
}
