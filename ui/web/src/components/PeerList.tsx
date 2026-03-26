import { useQuery } from '@tanstack/react-query';
import { useState, useEffect } from 'react';
import { Link } from 'react-router-dom';
import { getPeers } from '../api/nodes';
import { getPeerGroups } from '../api/access';
import type { AccessGroup } from '../api/access';
import {
  effectiveTier, peerIdToHex, formatLastSeen, isOnline,
} from '../lib/access';

export function PeerList() {
  const { data: peers = [], dataUpdatedAt } = useQuery({
    queryKey: ['peers'],
    queryFn: getPeers,
    refetchInterval: 30_000,
  });

  const [peerGroupsMap, setPeerGroupsMap] = useState<Record<string, AccessGroup[]>>({});

  const peerIds = peers.map(p => p.node_id).join(',');
  useEffect(() => {
    if (peers.length === 0) return;
    const fetchAll = async () => {
      const map: Record<string, AccessGroup[]> = {};
      await Promise.all(peers.map(async peer => {
        const hexId = peerIdToHex(peer.wg_pubkey);
        try { map[hexId] = await getPeerGroups(hexId); } catch { map[hexId] = []; }
      }));
      setPeerGroupsMap(map);
    };
    fetchAll();
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [peerIds]);

  const now = dataUpdatedAt;
  const onlineCount = peers.filter(p => isOnline(p.last_seen, now)).length;

  // Count by tier
  let friendsCount = 0, trustedCount = 0;
  for (const peer of peers) {
    const hexId = peerIdToHex(peer.wg_pubkey);
    const groups = peerGroupsMap[hexId] || [];
    const badge = effectiveTier(groups);
    if (badge.label === 'Friends') friendsCount++;
    if (badge.label === 'Trusted') trustedCount++;
  }

  // Top 3 most recently seen
  const recent = [...peers]
    .sort((a, b) => (b.last_seen || 0) - (a.last_seen || 0))
    .slice(0, 3);

  return (
    <div>
      <div className='flex justify-between items-center mb-3'>
        <h3 className='m-0'>Peers</h3>
      </div>

      <p className='m-0 text-sm text-howm-text-secondary'>
        {peers.length} peers  •  {onlineCount} online  •  {friendsCount} friends  •  {trustedCount} trusted
      </p>

      {recent.length > 0 && (
        <div className='mt-2'>
          <p className='text-howm-text-muted m-0 text-xs mb-1.5'>Recent:</p>
          {recent.map(peer => {
            const hexId = peerIdToHex(peer.wg_pubkey);
            const groups = peerGroupsMap[hexId] || [];
            const badge = effectiveTier(groups);
            const online = isOnline(peer.last_seen, now);
            return (
              <Link key={peer.node_id} to={`/peers/${hexId}`} className='no-underline text-inherit'>
                <div className='flex items-center py-1.5 px-2 rounded mb-0.5 cursor-pointer'>
                  <span style={{ color: online ? '#22c55e' : '#666666' }} className='mr-2'>
                    {online ? '●' : '○'}
                  </span>
                  <span className='font-medium mr-2'>{peer.name}</span>
                  <span style={{
                    fontSize: '0.75rem', padding: '1px 7px', borderRadius: '4px',
                    background: badge.bg, color: badge.color,
                  }}>
                    {badge.label}
                  </span>
                  <span className='ml-auto text-xs text-howm-text-muted'>
                    {formatLastSeen(peer.last_seen, now)}
                  </span>
                </div>
              </Link>
            );
          })}
        </div>
      )}

      <Link to="/peers" className='inline-block mt-3 text-howm-accent no-underline text-sm font-medium'>View All Peers →</Link>
    </div>
  );
}
