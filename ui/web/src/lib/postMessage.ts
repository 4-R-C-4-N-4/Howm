/**
 * postMessage contract between the Howm shell and capability iframes.
 *
 * Envelope: { type: string, payload?: any }
 *
 * Shell → Capability:
 *   howm:token:reply   { token: string }
 *   howm:theme:changed {}   (capability should reload /theme.css or re-apply)
 *
 * Capability → Shell:
 *   howm:ready         { name: string }          capability finished loading
 *   howm:token:request {}                         capability needs the API token
 *   howm:navigate      { path: string }           request shell-level navigation
 *   howm:notify        { level: NotifyLevel, message: string }
 */

export type NotifyLevel = 'info' | 'success' | 'warning' | 'error';

export interface HowmMessage {
  type: string;
  payload?: unknown;
}

// ── Shell → capability helpers ────────────────────────────────────────────────

export function sendTokenReply(iframe: HTMLIFrameElement, token: string) {
  iframe.contentWindow?.postMessage(
    { type: 'howm:token:reply', payload: { token } } satisfies HowmMessage,
    window.location.origin,
  );
}

export function sendThemeChanged(iframe: HTMLIFrameElement) {
  iframe.contentWindow?.postMessage(
    { type: 'howm:theme:changed' } satisfies HowmMessage,
    window.location.origin,
  );
}

// ── Capability → shell listener ───────────────────────────────────────────────

export interface ShellHandlers {
  onReady?: (capName: string) => void;
  onNavigate?: (path: string) => void;
  onNotify?: (level: NotifyLevel, message: string) => void;
  onTokenRequest?: () => void;
}

/**
 * Attach a window-level message listener that dispatches to ShellHandlers.
 * Returns an unsubscribe function.
 */
export function listenFromCapabilities(
  handlers: ShellHandlers,
  token: string | null,
): () => void {
  function handle(e: MessageEvent) {
    // Only accept same-origin messages (iframe is on same origin)
    if (e.origin !== window.location.origin) return;
    const msg = e.data as HowmMessage | null;
    if (!msg?.type) return;

    switch (msg.type) {
      case 'howm:ready': {
        const name = (msg.payload as { name?: string })?.name ?? '';
        handlers.onReady?.(name);
        break;
      }
      case 'howm:token:request': {
        handlers.onTokenRequest?.();
        // Reply directly to the source iframe if we have a token
        if (token && e.source) {
          (e.source as Window).postMessage(
            { type: 'howm:token:reply', payload: { token } } satisfies HowmMessage,
            e.origin,
          );
        }
        break;
      }
      case 'howm:navigate': {
        const path = (msg.payload as { path?: string })?.path ?? '/';
        handlers.onNavigate?.(path);
        break;
      }
      case 'howm:notify': {
        const p = msg.payload as { level?: NotifyLevel; message?: string };
        handlers.onNotify?.(p.level ?? 'info', p.message ?? '');
        break;
      }
    }
  }

  window.addEventListener('message', handle);
  return () => window.removeEventListener('message', handle);
}
