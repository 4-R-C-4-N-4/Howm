import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  getLanStatus, lanScan, lanInvite,
  type LanPeer,
} from '../api/network';

function extractErrorMessage(err: unknown): string {
  if (err && typeof err === 'object' && 'response' in err) {
    const res = (err as { response?: { data?: { error?: string; message?: string } } }).response;
    if (res?.data?.error) return res.data.error;
    if (res?.data?.message) return res.data.message;
  }
  if (err instanceof Error) return err.message;
  return String(err);
}

export function LanDiscovery() {
  const qc = useQueryClient();
  const [scanResults, setScanResults] = useState<LanPeer[] | null>(null);
  const [connectingPeer, setConnectingPeer] = useState<string | null>(null);
  const [connectedPeers, setConnectedPeers] = useState<Set<string>>(new Set());

  // LAN status (is mDNS active, what's our LAN IP)
  const { data: lanStatus } = useQuery({
    queryKey: ['lan-status'],
    queryFn: getLanStatus,
    refetchInterval: 30000,
  });

  // Scan mutation
  const scanMutation = useMutation({
    mutationFn: lanScan,
    onSuccess: (data) => {
      setScanResults(data.peers);
    },
  });

  // Connect mutation (sends LAN invite to a discovered peer)
  const connectMutation = useMutation({
    mutationFn: ({ lan_ip, daemon_port }: { lan_ip: string; daemon_port: number }) =>
      lanInvite(lan_ip, daemon_port),
    onSuccess: (_data, variables) => {
      setConnectedPeers(prev => new Set(prev).add(variables.lan_ip));
      setConnectingPeer(null);
      qc.invalidateQueries({ queryKey: ['peers'] });
      // Re-scan after a moment to update already_peered flags
      setTimeout(() => scanMutation.mutate(), 2000);
    },
    onError: () => {
      setConnectingPeer(null);
    },
  });

  const handleConnect = (peer: LanPeer) => {
    setConnectingPeer(peer.lan_ip);
    connectMutation.mutate({ lan_ip: peer.lan_ip, daemon_port: peer.daemon_port });
  };

  const isDiscoverable = lanStatus?.lan_discoverable ?? false;
  const mdnsActive = lanStatus?.mdns_active ?? false;
  const lanIp = lanStatus?.lan_ip;

  const newPeers = scanResults?.filter(p => !p.already_peered) ?? [];
  const existingPeers = scanResults?.filter(p => p.already_peered) ?? [];

  return (
    <div className="bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-5">
      <div className="flex justify-between items-center mb-4">
        <h2 className="text-xl font-semibold m-0">Local Network</h2>
        {lanIp && (
          <span className="text-howm-text-muted text-xs font-mono">{lanIp}</span>
        )}
      </div>

      {/* Status indicator */}
      {!isDiscoverable ? (
        <div className="p-2 px-3 rounded mb-4 text-sm bg-[rgba(102,102,102,0.12)]">
          <span className="text-howm-text-muted font-semibold">
            ○ LAN discovery disabled
          </span>
          <p className="text-howm-text-muted text-xs mt-1 mb-0">
            Enable <code className="font-mono text-xs bg-white/[0.08] px-1 py-px rounded-sm">lan_discoverable</code> in config to let nearby howm nodes find you.
          </p>
        </div>
      ) : mdnsActive ? (
        <div className="p-2 px-3 rounded mb-4 text-sm bg-[rgba(34,197,94,0.12)]">
          <span className="text-howm-success font-semibold">
            ● Broadcasting on local network
          </span>
          <span className="text-howm-text-muted text-xs ml-2">
            Other howm nodes on this WiFi/LAN can discover you
          </span>
        </div>
      ) : (
        <div className="p-2 px-3 rounded mb-4 text-sm bg-[rgba(234,179,8,0.12)]">
          <span className="text-howm-warning font-semibold">
            ● mDNS not running
          </span>
        </div>
      )}

      {/* Scan button */}
      {isDiscoverable && (
        <div className="flex items-center gap-3 mb-4">
          <button
            onClick={() => scanMutation.mutate()}
            disabled={scanMutation.isPending}
            className="px-3.5 py-1.5 bg-howm-accent border-none rounded text-white cursor-pointer text-sm font-semibold"
          >
            {scanMutation.isPending ? (
              <span className="inline-flex items-center gap-1.5">
                <span className="inline-block w-3 h-3 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                Scanning…
              </span>
            ) : scanResults !== null ? (
              'Scan Again'
            ) : (
              '📡 Scan for Local Peers'
            )}
          </button>
          {scanMutation.isPending && (
            <span className="text-howm-text-muted text-xs">
              Searching local network (3 seconds)…
            </span>
          )}
        </div>
      )}

      {/* Scan error */}
      {scanMutation.isError && (
        <div className="bg-[rgba(239,68,68,0.1)] border border-howm-error rounded px-3 py-2 mb-3 text-sm text-howm-error">
          {extractErrorMessage(scanMutation.error)}
        </div>
      )}

      {/* Connect error */}
      {connectMutation.isError && (
        <div className="bg-[rgba(239,68,68,0.1)] border border-howm-error rounded px-3 py-2 mb-3 text-sm text-howm-error">
          Failed to connect: {extractErrorMessage(connectMutation.error)}
        </div>
      )}

      {/* Scan results */}
      {scanResults !== null && !scanMutation.isPending && (
        <div>
          {scanResults.length === 0 ? (
            <div className="text-center py-6">
              <p className="text-howm-text-muted text-sm mb-1">
                No howm nodes found on this network
              </p>
              <p className="text-howm-text-muted text-xs">
                Make sure other nodes have LAN discovery enabled and are on the same WiFi/LAN
              </p>
            </div>
          ) : (
            <>
              {/* New (connectable) peers */}
              {newPeers.length > 0 && (
                <div className="mb-3">
                  <h3 className="text-sm font-semibold text-howm-text-secondary mt-0 mb-2">
                    Discovered Peers ({newPeers.length})
                  </h3>
                  <ul className="list-none p-0 m-0">
                    {newPeers.map(peer => (
                      <LanPeerRow
                        key={peer.wg_pubkey}
                        peer={peer}
                        isConnecting={connectingPeer === peer.lan_ip}
                        justConnected={connectedPeers.has(peer.lan_ip)}
                        onConnect={() => handleConnect(peer)}
                      />
                    ))}
                  </ul>
                </div>
              )}

              {/* Already-peered nodes on LAN */}
              {existingPeers.length > 0 && (
                <div>
                  <h3 className="text-sm font-semibold text-howm-text-secondary mt-0 mb-2">
                    Already Connected ({existingPeers.length})
                  </h3>
                  <ul className="list-none p-0 m-0">
                    {existingPeers.map(peer => (
                      <LanPeerRow
                        key={peer.wg_pubkey}
                        peer={peer}
                        isConnecting={false}
                        justConnected={false}
                        onConnect={() => {}}
                      />
                    ))}
                  </ul>
                </div>
              )}
            </>
          )}
        </div>
      )}

      {/* Empty state when not scanned yet */}
      {scanResults === null && isDiscoverable && !scanMutation.isPending && (
        <p className="text-howm-text-muted text-sm m-0">
          Scan to find other howm nodes on your local network — no invite codes needed.
        </p>
      )}
    </div>
  );
}

// ── Peer Row ──────────────────────────────────────────────────────────────────

function LanPeerRow({
  peer,
  isConnecting,
  justConnected,
  onConnect,
}: {
  peer: LanPeer;
  isConnecting: boolean;
  justConnected: boolean;
  onConnect: () => void;
}) {
  return (
    <li className="flex justify-between items-center px-3 py-2.5 border border-howm-border rounded mb-1.5 bg-howm-bg-secondary">
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="font-semibold text-sm text-howm-text-primary truncate">
            {peer.name}
          </span>
          <span className="text-howm-text-muted text-xs font-mono shrink-0">
            {peer.fingerprint}
          </span>
        </div>
        <div className="text-howm-text-muted text-xs mt-0.5 font-mono">
          {peer.lan_ip}:{peer.wg_port}
        </div>
      </div>

      <div className="ml-3 shrink-0">
        {peer.already_peered ? (
          <span className="text-howm-success text-sm font-semibold">✓ Connected</span>
        ) : justConnected ? (
          <span className="text-howm-success text-sm font-semibold">✓ Joined!</span>
        ) : (
          <button
            onClick={onConnect}
            disabled={isConnecting}
            className="px-3 py-1 bg-howm-accent border-none rounded text-white cursor-pointer text-sm font-semibold"
          >
            {isConnecting ? (
              <span className="inline-flex items-center gap-1.5">
                <span className="inline-block w-3 h-3 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                Connecting…
              </span>
            ) : (
              'Connect'
            )}
          </button>
        )}
      </div>
    </li>
  );
}
