import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useState, useEffect } from 'react';
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

  useEffect(() => {
    if (p2pcd) setP2pcdDraft(JSON.stringify(p2pcd, null, 2));
  }, [p2pcd]);

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
    <div style={pageStyle}>
      <h1 style={h1Style}>Settings</h1>

      {/* Node */}
      <section style={sectionStyle}>
        <h2 style={h2Style}>Node</h2>
        {node ? (
          <dl style={dlStyle}>
            <Row label="Node ID"    value={node.node_id} mono />
            <Row label="Name"       value={node.name} />
            <Row label="WG Address" value={node.wg_address} mono />
            <Row label="Listen Port" value={String(node.listen_port)} mono />
            <Row label="Data Dir"   value={node.data_dir} mono />
          </dl>
        ) : (
          <p style={mutedStyle}>Loading…</p>
        )}
      </section>

      {/* Identity */}
      <section style={sectionStyle}>
        <h2 style={h2Style}>Identity</h2>
        {identity ? (
          <dl style={dlStyle}>
            <Row label="Display Name" value={identity.display_name} />
            <Row label="Public Key"   value={identity.public_key} mono />
          </dl>
        ) : (
          <p style={mutedStyle}>Loading…</p>
        )}
      </section>

      {/* P2P-CD config */}
      <section style={sectionStyle}>
        <h2 style={h2Style}>P2P-CD</h2>
        <p style={{ ...mutedStyle, marginBottom: '12px' }}>
          Edit as JSON. Changes take effect after daemon restart.
        </p>
        <textarea
          value={p2pcdDraft}
          onChange={e => setP2pcdDraft(e.target.value)}
          rows={12}
          style={textareaStyle}
          spellCheck={false}
        />
        <div style={{ display: 'flex', alignItems: 'center', gap: '12px', marginTop: '8px' }}>
          <button onClick={handleSaveP2pcd} disabled={saveStatus === 'saving'} style={btnStyle}>
            {saveStatus === 'saving' ? 'Saving…' : 'Save'}
          </button>
          {saveStatus === 'ok'  && <span style={{ color: 'var(--howm-success, #4ade80)' }}>Saved</span>}
          {saveStatus === 'err' && <span style={{ color: 'var(--howm-error, #f87171)' }}>Failed — check JSON</span>}
        </div>
      </section>
    </div>
  );
}

function Row({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <>
      <dt style={dtStyle}>{label}</dt>
      <dd style={{ ...ddStyle, fontFamily: mono ? 'var(--howm-font-mono, monospace)' : 'inherit', wordBreak: 'break-all' }}>
        {value}
      </dd>
    </>
  );
}

const pageStyle: React.CSSProperties = { maxWidth: '720px', margin: '0 auto', padding: '24px' };
const h1Style: React.CSSProperties = { fontSize: 'var(--howm-font-size-2xl, 1.5rem)', marginBottom: '24px', fontWeight: 600 };
const h2Style: React.CSSProperties = { fontSize: 'var(--howm-font-size-xl, 1.25rem)', fontWeight: 600, marginTop: 0, marginBottom: '16px' };
const sectionStyle: React.CSSProperties = {
  background: 'var(--howm-bg-surface, #232733)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-lg, 12px)',
  padding: '20px',
  marginBottom: '20px',
};
const dlStyle: React.CSSProperties = { display: 'grid', gridTemplateColumns: 'auto 1fr', gap: '6px 16px', margin: 0 };
const dtStyle: React.CSSProperties = { fontWeight: 600, color: 'var(--howm-text-secondary, #8b91a0)', fontSize: '0.875rem', alignSelf: 'start', paddingTop: '1px' };
const ddStyle: React.CSSProperties = { margin: 0, fontSize: '0.9rem' };
const mutedStyle: React.CSSProperties = { color: 'var(--howm-text-muted, #5c6170)', margin: 0 };
const textareaStyle: React.CSSProperties = {
  width: '100%',
  background: 'var(--howm-bg-secondary, #1a1d27)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  color: 'var(--howm-text-primary, #e1e4eb)',
  fontFamily: 'var(--howm-font-mono, monospace)',
  fontSize: '0.875rem',
  padding: '10px 14px',
  resize: 'vertical',
  boxSizing: 'border-box',
};
const btnStyle: React.CSSProperties = {
  padding: '6px 18px',
  background: 'var(--howm-accent, #6c8cff)',
  color: '#fff',
  border: 'none',
  borderRadius: 'var(--howm-radius-sm, 4px)',
  cursor: 'pointer',
  fontSize: '0.875rem',
};
