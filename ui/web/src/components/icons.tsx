/**
 * Minimal SVG icon set for capability placement.
 * Each icon is a 24x24 SVG component with currentColor fill.
 */

const iconProps = { width: 24, height: 24, viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', strokeWidth: 2, strokeLinecap: 'round' as const, strokeLinejoin: 'round' as const };

export function ChatBubbleIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps} className={className}>
      <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
    </svg>
  );
}

export function FolderIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps} className={className}>
      <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
    </svg>
  );
}

export function FeedIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps} className={className}>
      <path d="M4 11a9 9 0 0 1 9 9" />
      <path d="M4 4a16 16 0 0 1 16 16" />
      <circle cx="5" cy="19" r="1" fill="currentColor" />
    </svg>
  );
}

export function GridIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps} className={className}>
      <rect x="3" y="3" width="7" height="7" />
      <rect x="14" y="3" width="7" height="7" />
      <rect x="3" y="14" width="7" height="7" />
      <rect x="14" y="14" width="7" height="7" />
    </svg>
  );
}

export function GlobeIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps} className={className}>
      <circle cx="12" cy="12" r="10" />
      <line x1="2" y1="12" x2="22" y2="12" />
      <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" />
    </svg>
  );
}

/** Resolve a capability's ui.icon string to a React component */
export function CapIcon({ icon, className }: { icon?: string; className?: string }) {
  switch (icon) {
    case 'chat-bubble': return <ChatBubbleIcon className={className} />;
    case 'folder':      return <FolderIcon className={className} />;
    case 'feed':        return <FeedIcon className={className} />;
    case 'grid':        return <GridIcon className={className} />;
    case 'globe':       return <GlobeIcon className={className} />;
    default:            return <GridIcon className={className} />;
  }
}
