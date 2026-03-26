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
    <div className='max-w-[800px] mx-auto p-6'>
      <div className='flex justify-between items-center mb-4'>
        <h1 className='text-2xl font-semibold m-0'>Peers ({peers.length})</h1>
        <Link to="/connection" className='py-2 px-4 bg-howm-accent border-none rounded-md text-white cursor-pointer text-sm font-semibold no-underline'>+ Invite</Link>
      </div>

      <div className='flex gap-2 mb-3'>
        <div className='relative flex-1'>
          <input
            placeholder="Search peers..."
            value={search}
            onChange={e => setSearch(e.target.value)}
            onKeyDown={e => e.key === 'Escape' && setSearch('')}
            className='w-full py-2 px-2.5 box-border bg-howm-bg-secondary border border-howm-border rounded text-howm-text-primary text-sm'
          />
        </div>
        <select
          value={filter}
          onChange={e => setFilter(e.target.value as FilterOption)}
          className='py-2 px-2.5 bg-howm-bg-secondary border border-howm-border rounded text-howm-text-primary text-sm cursor-pointer'
        >
          {(['All', 'Trusted', 'Friends', 'Default', 'Custom', 'Denied', 'Online'] as const).map(f => (
            <option key={f} value={f}>{f}</option>
          ))}
        </select>
      </div>

      {filtered.length === 0 ? (
        <p className='text-howm-text-muted text-sm'>
          {peers.length === 0
            ? <>No peers yet. Go to <Link to="/connection" className='text-howm-accent no-underline'>Connection</Link> to create or redeem an invite.</>
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
        <div className='fixed bottom-6 right-6 flex flex-col gap-2 z-300'>
          {newPeers.map(p => (
            <NewPeerToast key={p.node_id} peer={p} onDismiss={() => dismissNewPeer(p.node_id)} />
          ))}
        </div>
      )}

      {/* Inline toasts */}
      {toasts.length > 0 && (
        <div className='fixed bottom-6 left-1/2 -translate-x-1/2 flex flex-col gap-2 z-300'>
          {toasts.map(t => (
            <div key={t.id} className='py-2 px-4 rounded-lg text-sm shadow-[0_4px_12px_rgba(0,0,0,0.5)]' style={{
              background: t.level === 'success' ? '#14532d' : '#7f1d1d',
              color: t.level === 'success' ? '#86efac' : '#fca5a5',
              border: `1px solid ${t.level === 'success' ? '#16a34a' : '#dc2626'}`,
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
