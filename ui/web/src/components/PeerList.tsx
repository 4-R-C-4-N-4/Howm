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
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
        <h3 style={{ margin: 0 }}>Peers</h3>
      </div>

      <p style={summaryStyle}>
        {peers.length} peers  •  {onlineCount} online  •  {friendsCount} friends  •  {trustedCount} trusted
      </p>

      {recent.length > 0 && (
        <div style={{ marginTop: '8px' }}>
          <p style={{ ...mutedStyle, fontSize: '0.8rem', marginBottom: '6px' }}>Recent:</p>
          {recent.map(peer => {
            const hexId = peerIdToHex(peer.wg_pubkey);
            const groups = peerGroupsMap[hexId] || [];
            const badge = effectiveTier(groups);
            const online = isOnline(peer.last_seen, now);
            return (
              <Link key={peer.node_id} to={`/peers/${hexId}`} style={{ textDecoration: 'none', color: 'inherit' }}>
                <div style={recentRowStyle}>
                  <span style={{ color: online ? '#4ade80' : '#5c6170', marginRight: '8px' }}>
                    {online ? '●' : '○'}
                  </span>
                  <span style={{ fontWeight: 500, marginRight: '8px' }}>{peer.name}</span>
                  <span style={{
                    fontSize: '0.75rem', padding: '1px 7px', borderRadius: '4px',
                    background: badge.bg, color: badge.color,
                  }}>
                    {badge.label}
                  </span>
                  <span style={{ marginLeft: 'auto', fontSize: '0.8rem', color: 'var(--howm-text-muted, #5c6170)' }}>
                    {formatLastSeen(peer.last_seen, now)}
                  </span>
                </div>
              </Link>
            );
          })}
        </div>
      )}

      <Link to="/peers" style={viewAllStyle}>View All Peers →</Link>
    </div>
  );
}

const summaryStyle: React.CSSProperties = {
  margin: 0, fontSize: '0.9rem', color: 'var(--howm-text-secondary, #8b91a0)',
};
const mutedStyle: React.CSSProperties = {
  color: 'var(--howm-text-muted, #5c6170)', margin: 0,
};
const recentRowStyle: React.CSSProperties = {
  display: 'flex', alignItems: 'center', padding: '6px 8px',
  borderRadius: '4px', marginBottom: '2px',
  cursor: 'pointer',
};
const viewAllStyle: React.CSSProperties = {
  display: 'inline-block', marginTop: '12px',
  color: 'var(--howm-accent, #6c8cff)', textDecoration: 'none',
  fontSize: '0.9rem', fontWeight: 500,
};
