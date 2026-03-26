import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { getNetworkStatus } from '../api/network';
import { NetworkStatus } from '../components/NetworkStatus';
import { ConnectionInfo } from '../components/ConnectionInfo';
import { InviteManager } from '../components/InviteManager';
import { RelayConfig } from '../components/RelayConfig';

export function Connection() {
  const [infoOpen, setInfoOpen] = useState(false);

  const { data: status, isLoading } = useQuery({
    queryKey: ['network-status'],
    queryFn: getNetworkStatus,
    refetchInterval: 15000,
  });

  if (isLoading || !status) {
    return (
      <div className='max-w-[800px] mx-auto p-6'>
        <h1 className='text-2xl mb-6 font-semibold'>Connection</h1>
        <p className='text-howm-text-muted m-0 text-sm'>Loading network status…</p>
      </div>
    );
  }

  return (
    <div className='max-w-[800px] mx-auto p-6'>
      {/* Header with info button */}
      <div className='flex justify-between items-center mb-6'>
        <h1 className='text-2xl font-semibold mb-0'>Connection</h1>
        <button onClick={() => setInfoOpen(true)} className='py-1.5 px-3.5 bg-howm-accent-dim border border-howm-accent rounded text-howm-accent cursor-pointer text-sm font-semibold' title="How does this work?">
          ⓘ Info
        </button>
      </div>

      {/* Network Status */}
      <NetworkStatus status={status} />

      {/* Invites */}
      <InviteManager reachability={status.reachability} />

      {/* Relay */}
      <RelayConfig relay={status.relay} />

      {/* Info drawer */}
      <ConnectionInfo open={infoOpen} onClose={() => setInfoOpen(false)} status={status} />
    </div>
  );
}
