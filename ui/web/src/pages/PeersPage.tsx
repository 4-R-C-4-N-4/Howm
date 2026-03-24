import { useQuery } from '@tanstack/react-query';
import { useState, useRef, useEffect, useCallback } from 'react';
import { getPeers } from '../api/nodes';
import { getPeerGroups } from '../api/access';
import type { AccessGroup } from '../api/access';
import type { Peer } from '../api/nodes';
import { PeerRow } from '../components/PeerRow';
import { NewPeerToast } from '../components/NewPeerToast';
import {
  effectiveTier, peerIdToHex, isOnline,
} from '../lib/access';
import { Link } from 'react-router-dom';

type FilterOption = 'All' | 'Trusted' | 'Friends' | 'Default' | 'Custom' | 'Denied' | 'Online';

export function PeersPage() {
  const [search, setSearch] = useState('');
  const [filter, setFilter] = useState<FilterOption>('All');
  const [newPeers, setNewPeers] = useState<Peer[]>([]);
  const prevPeerIdsRef = useRef<Set<string> | null>(null);

  const { data: peers = [], dataUpdatedAt } = useQuery({
    queryKey: ['peers'],
    queryFn: getPeers,
    refetchInterval: 30_000,
  });

  // Per-peer group memberships
  const peerGroupsQueries = useQueries(peers);

  // Detect new peers
  useEffect(() => {
    const currentIds = new Set(peers.map(p => p.node_id));
    if (prevPeerIdsRef.current) {
      const arriving = peers.filter(p => !prevPeerIdsRef.current!.has(p.node_id));
      if (arriving.length > 0) setNewPeers(prev => [...prev, ...arriving]);
    }
    prevPeerIdsRef.current = currentIds;
  }, [peers]);

  const dismissNewPeer = useCallback((nodeId: string) => {
    setNewPeers(prev => prev.filter(p => p.node_id !== nodeId));
  }, []);

  // Auto-dismiss after 15s
  useEffect(() => {
    if (newPeers.length === 0) return;
    const timer = setTimeout(() => setNewPeers([]), 15000);
    return () => clearTimeout(timer);
  }, [newPeers]);

  const now = dataUpdatedAt;

  // Filter & search
  const filtered = peers.filter(peer => {
    if (search && !peer.name.toLowerCase().includes(search.toLowerCase())) return false;
    if (filter === 'All') return true;
    if (filter === 'Online') return isOnline(peer.last_seen, now);
    const hexId = peerIdToHex(peer.wg_pubkey);
    const pg = peerGroupsQueries[hexId] || [];
    const badge = effectiveTier(pg);
    return badge.label === filter;
  });

  const [toasts, setToasts] = useState<{ id: number; level: string; msg: string }[]>([]);
  const toastId = useRef(0);
  const showToast = useCallback((level: 'success' | 'error', msg: string) => {
    const id = ++toastId.current;
    setToasts(prev => [...prev, { id, level, msg }]);
    setTimeout(() => setToasts(prev => prev.filter(t => t.id !== id)), 4000);
  }, []);

  return (
    <div style={pageStyle}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '16px' }}>
        <h1 style={h1Style}>Peers ({peers.length})</h1>
        <Link to="/connection" style={inviteBtnStyle}>+ Invite</Link>
      </div>

      <div style={{ display: 'flex', gap: '8px', marginBottom: '12px' }}>
        <div style={{ position: 'relative', flex: 1 }}>
          <input
            placeholder="Search peers..."
            value={search}
            onChange={e => setSearch(e.target.value)}
            onKeyDown={e => e.key === 'Escape' && setSearch('')}
            style={searchStyle}
          />
        </div>
        <select
          value={filter}
          onChange={e => setFilter(e.target.value as FilterOption)}
          style={selectStyle}
        >
          {(['All', 'Trusted', 'Friends', 'Default', 'Custom', 'Denied', 'Online'] as const).map(f => (
            <option key={f} value={f}>{f}</option>
          ))}
        </select>
      </div>

      {filtered.length === 0 ? (
        <p style={mutedStyle}>
          {peers.length === 0
            ? <>No peers yet. Go to <Link to="/connection" style={linkStyle}>Connection</Link> to create or redeem an invite.</>
            : 'No peers match the current filter.'}
        </p>
      ) : (
        <div>
          {filtered.map(peer => {
            const hexId = peerIdToHex(peer.wg_pubkey);
            return (
              <PeerRow
                key={peer.node_id}
                peer={peer}
                groups={peerGroupsQueries[hexId] || []}
                now={now}
                onToast={showToast}
              />
            );
          })}
        </div>
      )}

      {/* New peer toasts */}
      {newPeers.length > 0 && (
        <div style={{ position: 'fixed', bottom: '24px', right: '24px', display: 'flex', flexDirection: 'column', gap: '8px', zIndex: 300 }}>
          {newPeers.map(p => (
            <NewPeerToast key={p.node_id} peer={p} onDismiss={() => dismissNewPeer(p.node_id)} />
          ))}
        </div>
      )}

      {/* Inline toasts */}
      {toasts.length > 0 && (
        <div style={{ position: 'fixed', bottom: '24px', left: '50%', transform: 'translateX(-50%)', display: 'flex', flexDirection: 'column', gap: '8px', zIndex: 300 }}>
          {toasts.map(t => (
            <div key={t.id} style={{
              padding: '8px 16px', borderRadius: '8px', fontSize: '0.85rem',
              background: t.level === 'success' ? '#14532d' : '#7f1d1d',
              color: t.level === 'success' ? '#86efac' : '#fca5a5',
              border: `1px solid ${t.level === 'success' ? '#16a34a' : '#dc2626'}`,
              boxShadow: '0 4px 12px rgba(0,0,0,0.5)',
            }}>
              {t.msg}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// Hook to fetch per-peer group memberships and return a map
function useQueries(peers: Peer[]): Record<string, AccessGroup[]> {
  const [peerGroupsMap, setPeerGroupsMap] = useState<Record<string, AccessGroup[]>>({});
  const peerIds = peers.map(p => p.node_id).join(',');

  useEffect(() => {
    if (peers.length === 0) return;

    const fetchAll = async () => {
      const map: Record<string, AccessGroup[]> = {};
      await Promise.all(peers.map(async peer => {
        const hexId = peerIdToHex(peer.wg_pubkey);
        try {
          const groups = await getPeerGroups(hexId);
          map[hexId] = groups;
        } catch {
          map[hexId] = [];
        }
      }));
      setPeerGroupsMap(map);
    };

    fetchAll();
    const interval = setInterval(fetchAll, 60_000);
    return () => clearInterval(interval);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [peerIds]);

  return peerGroupsMap;
}

const pageStyle: React.CSSProperties = { maxWidth: '800px', margin: '0 auto', padding: '24px' };
const h1Style: React.CSSProperties = { fontSize: '1.5rem', fontWeight: 600, margin: 0 };
const searchStyle: React.CSSProperties = {
  width: '100%', padding: '8px 10px', boxSizing: 'border-box',
  background: 'var(--howm-bg-secondary, #1a1d27)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: '4px', color: 'var(--howm-text-primary, #e1e4eb)',
  fontSize: '0.9rem',
};
const selectStyle: React.CSSProperties = {
  padding: '8px 10px',
  background: 'var(--howm-bg-secondary, #1a1d27)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: '4px', color: 'var(--howm-text-primary, #e1e4eb)',
  fontSize: '0.9rem', cursor: 'pointer',
};
const inviteBtnStyle: React.CSSProperties = {
  padding: '8px 16px', background: 'var(--howm-accent, #6c8cff)',
  border: 'none', borderRadius: '6px', color: '#fff',
  cursor: 'pointer', fontSize: '0.9rem', fontWeight: 600,
  textDecoration: 'none',
};
const mutedStyle: React.CSSProperties = { color: 'var(--howm-text-muted, #5c6170)', fontSize: '0.9rem' };
const linkStyle: React.CSSProperties = { color: 'var(--howm-accent, #6c8cff)', textDecoration: 'none' };
