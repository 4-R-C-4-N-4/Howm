import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import {
  getNodeSettings,
  getIdentity,
  getP2pcdConfig,
  updateP2pcdConfig,
  type P2pcdConfig,
} from '../api/settings';

export function Settings() {
  const qc = useQueryClient();
  const { data: node } = useQuery({ queryKey: ['settings-node'], queryFn: getNodeSettings });
  const { data: identity } = useQuery({ queryKey: ['settings-identity'], queryFn: getIdentity });
  const { data: p2pcd } = useQuery({ queryKey: ['settings-p2pcd'], queryFn: getP2pcdConfig });

  const [p2pcdDraft, setP2pcdDraft] = useState<string>('');
  const [saveStatus, setSaveStatus] = useState<'idle' | 'saving' | 'ok' | 'err'>('idle');

  // Sync draft when server data changes (adjust-state-during-render pattern)
  const [prevP2pcd, setPrevP2pcd] = useState(p2pcd);
  if (p2pcd && p2pcd !== prevP2pcd) {
    setPrevP2pcd(p2pcd);
    setP2pcdDraft(JSON.stringify(p2pcd, null, 2));
  }

  const mutation = useMutation({
    mutationFn: (patch: Partial<P2pcdConfig>) => updateP2pcdConfig(patch),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['settings-p2pcd'] });
      setSaveStatus('ok');
      setTimeout(() => setSaveStatus('idle'), 2500);
    },
    onError: () => {
      setSaveStatus('err');
      setTimeout(() => setSaveStatus('idle'), 3000);
    },
  });

  function handleSaveP2pcd() {
    try {
      const parsed = JSON.parse(p2pcdDraft) as Partial<P2pcdConfig>;
      setSaveStatus('saving');
      mutation.mutate(parsed);
    } catch {
      setSaveStatus('err');
      setTimeout(() => setSaveStatus('idle'), 3000);
    }
  }

  return (
    <div className='max-w-[720px] mx-auto p-6'>
      <h1 className='text-2xl mb-6 font-semibold'>Settings</h1>

      {/* Node */}
      <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-5'>
        <h2 className='text-xl font-semibold mt-0 mb-4'>Node</h2>
        {node ? (
          <dl className='grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 m-0'>
            <Row label="Node ID"    value={node.node_id} mono />
            <Row label="Name"       value={node.name} />
            <Row label="WG Address" value={node.wg_address} mono />
            <Row label="Listen Port" value={String(node.listen_port)} mono />
            <Row label="Data Dir"   value={node.data_dir} mono />
          </dl>
        ) : (
          <p className='text-howm-text-muted m-0'>Loading…</p>
        )}
      </section>

      {/* Identity */}
      <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-5'>
        <h2 className='text-xl font-semibold mt-0 mb-4'>Identity</h2>
        {identity ? (
          <dl className='grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 m-0'>
            <Row label="Display Name" value={identity.display_name} />
            <Row label="Public Key"   value={identity.public_key} mono />
          </dl>
        ) : (
          <p className='text-howm-text-muted m-0'>Loading…</p>
        )}
      </section>

      {/* P2P-CD config */}
      <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-5'>
        <h2 className='text-xl font-semibold mt-0 mb-4'>P2P-CD</h2>
        <p className='text-howm-text-muted m-0 mb-1'>
          Edit as JSON. Changes take effect after daemon restart.
        </p>
        {node?.data_dir && (
          <p className='text-howm-text-muted m-0 mb-3 text-xs font-mono'>
            {node.data_dir}/p2pcd-peer.toml
          </p>
        )}
        <textarea
          value={p2pcdDraft}
          onChange={e => setP2pcdDraft(e.target.value)}
          rows={12}
          className='w-full bg-howm-bg-secondary border border-howm-border rounded text-howm-text-primary font-mono text-sm py-2.5 px-3.5 resize-y box-border'
          spellCheck={false}
        />
        <div className='flex items-center gap-3 mt-2'>
          <button onClick={handleSaveP2pcd} disabled={saveStatus === 'saving'} className='py-1.5 px-4.5 bg-howm-accent text-white border-none rounded cursor-pointer text-sm'>
            {saveStatus === 'saving' ? 'Saving…' : 'Save'}
          </button>
          {saveStatus === 'ok'  && <span className='text-howm-success'>Saved</span>}
          {saveStatus === 'err' && <span className='text-howm-error'>Failed — check JSON</span>}
        </div>
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
