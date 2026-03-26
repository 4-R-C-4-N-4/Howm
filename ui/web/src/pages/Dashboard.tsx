import { useQuery, useQueryClient } from '@tanstack/react-query';
import { getNodeInfo } from '../api/nodes';
import { getApiToken, setApiToken } from '../api/client';
import { PeerList } from '../components/PeerList';
import { CapabilityList } from '../components/CapabilityList';
import { useState } from 'react';

export function Dashboard() {
  const queryClient = useQueryClient();
  const { data: nodeInfo, isLoading: nodeLoading } = useQuery({
    queryKey: ['node-info'],
    queryFn: getNodeInfo,
  });

  const [tokenInput, setTokenInput] = useState('');
  const hasToken = !!getApiToken();

  const handleSetToken = () => {
    if (tokenInput.trim()) {
      setApiToken(tokenInput.trim());
      setTokenInput('');
      queryClient.invalidateQueries();
    }
  };

  return (
    <div className='max-w-[800px] mx-auto p-6'>
      <h1 className='text-2xl mb-6 font-semibold'>Dashboard</h1>

      {/* Node Info */}
      <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-5'>
        <h2 className='text-xl font-semibold mt-0 mb-4'>Node Info</h2>
        {nodeLoading ? <p className='text-howm-text-muted text-sm'>Loading…</p> : nodeInfo && (
          <dl className='grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 m-0'>
            <Row label="Node ID" value={nodeInfo.node_id} mono />
            <Row label="Name"    value={nodeInfo.name} />
          </dl>
        )}
      </section>

      {/* API Token */}
      <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-5'>
      <h2 className='text-xl font-semibold mt-0 mb-4'>API Token</h2>
      {hasToken ? (
        <div className='flex items-center gap-3'>
        <span className='text-howm-success font-semibold'>● Connected</span>
        <span className='text-howm-text-muted text-sm'>Token is set</span>
        </div>
      ) : (
        <div>
        <p className='text-howm-warning mb-2 mt-0'>
        ⚠ No API token set — mutations (invites, peer removal, posting) will be rejected.
        </p>
        <p className='text-howm-text-muted text-sm mb-3 mt-0'>
        Copy the token printed by the daemon on first run (or from the howm.sh startup box).
        </p>
        <div className='flex gap-2'>
        <input
        type="password"
        placeholder="Paste API token..."
        value={tokenInput}
        onChange={e => setTokenInput(e.target.value)}
        onKeyDown={e => e.key === 'Enter' && handleSetToken()}
        className='flex-1 py-1.5 px-2.5 bg-howm-bg-secondary border border-howm-border rounded text-howm-text-primary text-sm font-mono'
      />
      <button onClick={handleSetToken} disabled={!tokenInput.trim()} className='py-1.5 px-3.5 bg-howm-accent border-none rounded text-white cursor-pointer text-sm'>
      Set Token
      </button>
      </div>
      </div>
      )}
      </section>

      {/* Peers */}
      <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-5'>
        <PeerList />
      </section>

      {/* Capabilities */}
      <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-5'>
        <CapabilityList />
      </section>
    </div>
  );
}

function Row({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <>
      <dt className='font-semibold text-howm-text-secondary text-sm self-start pt-px'>{label}</dt>
      <dd className={`m-0 text-sm break-all ${mono ? 'font-mono' : ''}`}>
        {value}
      </dd>
    </>
  );
}
