import { useParams, Link, useNavigate } from 'react-router-dom';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useState, useCallback, useRef, useEffect } from 'react';
import {
  getAccessGroup, updateAccessGroup, deleteAccessGroup,
  assignPeerToGroup, removePeerFromGroup,
} from '../api/access';
import { getPeers } from '../api/nodes';
import type { CapabilityRule } from '../api/access';
import { peerIdToHex, ALL_CAPABILITIES } from '../lib/access';

export function GroupDetail() {
  const { groupId } = useParams<{ groupId: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const [showAddPeer, setShowAddPeer] = useState(false);
  const [peerSearch, setPeerSearch] = useState('');
  const addRef = useRef<HTMLDivElement>(null);

  const [toasts, setToasts] = useState<{ id: number; level: string; msg: string }[]>([]);
  const toastId = useRef(0);
  const showToast = useCallback((level: 'success' | 'error', msg: string) => {
    const id = ++toastId.current;
    setToasts(prev => [...prev, { id, level, msg }]);
    setTimeout(() => setToasts(prev => prev.filter(t => t.id !== id)), 4000);
  }, []);

  const { data: group, isLoading } = useQuery({
    queryKey: ['access-group', groupId],
    queryFn: () => getAccessGroup(groupId!),
    enabled: !!groupId,
  });

  const { data: peers = [] } = useQuery({
    queryKey: ['peers'],
    queryFn: getPeers,
  });

  // Editable name/description state for custom groups
  const [editName, setEditName] = useState('');
  const [editDesc, setEditDesc] = useState('');
  const [initGroupId, setInitGroupId] = useState<string | null>(null);
  if (group && group.group_id !== initGroupId) {
    setInitGroupId(group.group_id);
    setEditName(group.name);
    setEditDesc(group.description || '');
  }

  // Debounced save for capability toggles
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const updateMutation = useMutation({
    mutationFn: (updates: { name?: string; description?: string | null; capabilities?: CapabilityRule[] }) =>
      updateAccessGroup(groupId!, updates),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['access-group', groupId] });
      queryClient.invalidateQueries({ queryKey: ['access-groups'] });
    },
    onError: () => showToast('error', 'Failed to update group'),
  });

  const deleteMutation = useMutation({
    mutationFn: () => deleteAccessGroup(groupId!),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['access-groups'] });
      navigate('/access/groups');
    },
    onError: () => showToast('error', 'Failed to delete group'),
  });

  const addPeerMutation = useMutation({
    mutationFn: (peerId: string) => assignPeerToGroup(peerId, groupId!),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['access-group', groupId] });
      showToast('success', 'Peer added');
      setShowAddPeer(false);
      setPeerSearch('');
    },
    onError: () => showToast('error', 'Failed to add peer'),
  });

  const removePeerMutation = useMutation({
    mutationFn: (peerId: string) => removePeerFromGroup(peerId, groupId!),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['access-group', groupId] });
      showToast('success', 'Peer removed');
    },
    onError: () => showToast('error', 'Failed to remove peer'),
  });

  useEffect(() => {
    if (!showAddPeer) return;
    const handler = (e: MouseEvent) => {
      if (addRef.current && !addRef.current.contains(e.target as Node)) { setShowAddPeer(false); setPeerSearch(''); }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [showAddPeer]);

  if (isLoading || !group) {
    return (
      <div style={pageStyle}>
        <Link to="/access/groups" style={backStyle}>← Back to Groups</Link>
        <p style={mutedStyle}>{isLoading ? 'Loading…' : 'Group not found'}</p>
      </div>
    );
  }

  const isBuiltIn = group.built_in;
  const capMap = new Map((group.capabilities || []).map(c => [c.capability_name, c]));

  const handleCapToggle = (capName: string) => {
    if (isBuiltIn) return;
    const current = capMap.get(capName);
    const newCaps = ALL_CAPABILITIES.map(c => {
      const existing = capMap.get(c);
      if (c === capName) {
        return { capability_name: c, allow: !(current?.allow ?? false), rate_limit: null, ttl: null };
      }
      return existing || { capability_name: c, allow: false, rate_limit: null, ttl: null };
    });

    // Debounced save
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => {
      updateMutation.mutate({ capabilities: newCaps });
    }, 500);

    // Optimistic update of capMap
    capMap.set(capName, {
      capability_name: capName,
      allow: !(current?.allow ?? false),
      rate_limit: null,
      ttl: null,
    });
  };

  const handleNameSave = () => {
    if (!isBuiltIn && editName.trim() && editName.trim() !== group.name) {
      updateMutation.mutate({ name: editName.trim() });
    }
  };

  const handleDescSave = () => {
    if (!isBuiltIn && editDesc !== (group.description || '')) {
      updateMutation.mutate({ description: editDesc || null });
    }
  };

  // Members — group.capabilities contains member info via the detail endpoint
  // We get member peer IDs from the group endpoint and cross-ref with /node/peers
  // For now, list peers and show who's in this group
  const memberPeerIds: string[] = []; // Will be populated from group detail endpoint

  const availablePeers = peers
    .filter(p => !memberPeerIds.includes(peerIdToHex(p.wg_pubkey)))
    .filter(p => !peerSearch || p.name.toLowerCase().includes(peerSearch.toLowerCase()));

  return (
    <div style={pageStyle}>
      <Link to="/access/groups" style={backStyle}>← Back to Groups</Link>

      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginTop: '12px', marginBottom: '16px' }}>
        <div style={{ flex: 1 }}>
          {isBuiltIn ? (
            <h1 style={{ margin: 0, fontSize: '1.5rem' }}>{group.name}</h1>
          ) : (
            <input
              value={editName} onChange={e => setEditName(e.target.value)}
              onBlur={handleNameSave} onKeyDown={e => e.key === 'Enter' && handleNameSave()}
              style={{ ...editInputStyle, fontSize: '1.5rem', fontWeight: 600 }}
            />
          )}
          {isBuiltIn ? (
            <p style={{ ...mutedStyle, marginTop: '4px' }}>{group.description}</p>
          ) : (
            <input
              value={editDesc} onChange={e => setEditDesc(e.target.value)}
              onBlur={handleDescSave}
              placeholder="Add description..."
              style={{ ...editInputStyle, fontSize: '0.9rem', marginTop: '4px', color: 'var(--howm-text-muted, #5c6170)' }}
            />
          )}
        </div>
        {isBuiltIn && (
          <span style={lockedBadgeStyle}>Built-in 🔒</span>
        )}
      </div>

      {/* Members */}
      <section style={cardStyle}>
        <h3 style={h3Style}>Members</h3>
        {peers.length === 0 ? (
          <p style={mutedStyle}>No peers to show</p>
        ) : (
          <div>
            {peers.map(p => {
              const hexId = peerIdToHex(p.wg_pubkey);
              return (
                <div key={p.node_id} style={memberRowStyle}>
                  <Link to={`/peers/${hexId}`} style={{ textDecoration: 'none', color: 'inherit', flex: 1 }}>
                    <span style={{ fontWeight: 500 }}>{p.name}</span>
                  </Link>
                  <button
                    onClick={() => removePeerMutation.mutate(hexId)}
                    disabled={removePeerMutation.isPending}
                    style={removeBtnStyle}
                  >
                    Remove from group
                  </button>
                </div>
              );
            })}
          </div>
        )}
        <div style={{ position: 'relative', marginTop: '8px' }} ref={addRef}>
          <button onClick={() => setShowAddPeer(!showAddPeer)} style={addPeerBtnStyle}>
            + Add Peer
          </button>
          {showAddPeer && (
            <div style={dropdownStyle}>
              <input
                autoFocus placeholder="Search peers..."
                value={peerSearch} onChange={e => setPeerSearch(e.target.value)}
                style={searchInputStyle}
              />
              {availablePeers.length === 0 ? (
                <div style={{ padding: '8px 12px', color: 'var(--howm-text-muted, #5c6170)', fontSize: '0.8rem' }}>No peers available</div>
              ) : availablePeers.map(p => (
                <button
                  key={p.node_id}
                  onClick={() => addPeerMutation.mutate(peerIdToHex(p.wg_pubkey))}
                  style={dropItemStyle}
                >
                  {p.name}
                </button>
              ))}
            </div>
          )}
        </div>
      </section>

      {/* Capability Rules */}
      <section style={cardStyle}>
        <h3 style={h3Style}>Capability Rules</h3>
        <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
          {ALL_CAPABILITIES.map(cap => {
            const rule = capMap.get(cap);
            const allowed = rule?.allow ?? false;
            return (
              <div key={cap} style={{ display: 'flex', alignItems: 'center', padding: '4px 0' }}>
                {isBuiltIn ? (
                  <span style={{ color: allowed ? '#4ade80' : '#f87171', marginRight: '8px', width: '16px' }}>
                    {allowed ? '✓' : '✕'}
                  </span>
                ) : (
                  <input
                    type="checkbox"
                    checked={allowed}
                    onChange={() => handleCapToggle(cap)}
                    style={{ marginRight: '8px', accentColor: 'var(--howm-accent, #6c8cff)' }}
                  />
                )}
                <span style={{ fontSize: '0.875rem', fontFamily: 'var(--howm-font-mono, monospace)' }}>
                  {cap}
                </span>
              </div>
            );
          })}
        </div>
      </section>

      {/* Delete (custom only) */}
      {!isBuiltIn && (
        <section style={{ ...cardStyle, borderColor: 'rgba(248,113,113,0.3)' }}>
          <h3 style={{ ...h3Style, color: '#f87171' }}>Danger Zone</h3>
          <p style={{ ...mutedStyle, marginBottom: '12px' }}>
            Delete this group? Members will be removed from this group but retain other group memberships.
          </p>
          {showDeleteConfirm ? (
            <div style={{ display: 'flex', gap: '8px' }}>
              <button onClick={() => setShowDeleteConfirm(false)} style={cancelBtnStyle}>Cancel</button>
              <button onClick={() => deleteMutation.mutate()} disabled={deleteMutation.isPending} style={deleteBtnStyle}>
                {deleteMutation.isPending ? 'Deleting…' : `Delete "${group.name}"`}
              </button>
            </div>
          ) : (
            <button onClick={() => setShowDeleteConfirm(true)} style={deleteBtnStyle}>
              🔴 Delete Group
            </button>
          )}
        </section>
      )}

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

const pageStyle: React.CSSProperties = { maxWidth: '800px', margin: '0 auto', padding: '24px' };
const backStyle: React.CSSProperties = { color: 'var(--howm-text-muted, #5c6170)', textDecoration: 'none', fontSize: '0.9rem' };
const h3Style: React.CSSProperties = { fontSize: '1rem', fontWeight: 600, margin: '0 0 12px', color: 'var(--howm-text-secondary, #8b91a0)' };
const cardStyle: React.CSSProperties = {
  background: 'var(--howm-bg-surface, #232733)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: '12px', padding: '20px', marginBottom: '16px',
};
const mutedStyle: React.CSSProperties = { color: 'var(--howm-text-muted, #5c6170)', fontSize: '0.9rem', margin: 0 };
const lockedBadgeStyle: React.CSSProperties = {
  fontSize: '0.8rem', padding: '4px 10px', borderRadius: '4px',
  background: 'rgba(156,163,175,0.12)', color: '#9ca3af',
};
const memberRowStyle: React.CSSProperties = {
  display: 'flex', alignItems: 'center', justifyContent: 'space-between',
  padding: '8px 12px', borderRadius: '4px',
  background: 'var(--howm-bg-secondary, #1a1d27)',
  marginBottom: '4px',
};
const removeBtnStyle: React.CSSProperties = {
  background: 'rgba(248,113,113,0.12)', border: '1px solid rgba(248,113,113,0.3)',
  borderRadius: '4px', color: '#f87171', cursor: 'pointer',
  fontSize: '0.75rem', padding: '3px 8px',
};
const addPeerBtnStyle: React.CSSProperties = {
  background: 'rgba(108,140,255,0.12)', border: '1px solid rgba(108,140,255,0.25)',
  borderRadius: '4px', color: 'var(--howm-accent, #6c8cff)',
  cursor: 'pointer', fontSize: '0.8rem', padding: '6px 12px',
};
const editInputStyle: React.CSSProperties = {
  background: 'transparent', border: 'none', borderBottom: '1px solid transparent',
  color: 'var(--howm-text-primary, #e1e4eb)', outline: 'none', width: '100%',
  padding: '2px 0',
};
const dropdownStyle: React.CSSProperties = {
  position: 'absolute', left: 0, top: '100%', marginTop: '4px',
  background: 'var(--howm-bg-surface, #232733)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: '8px', zIndex: 200, minWidth: '200px',
  boxShadow: '0 8px 24px rgba(0,0,0,0.5)', overflow: 'hidden',
};
const searchInputStyle: React.CSSProperties = {
  width: '100%', padding: '8px 12px', border: 'none',
  borderBottom: '1px solid var(--howm-border, #2e3341)',
  background: 'transparent', color: 'var(--howm-text-primary, #e1e4eb)',
  fontSize: '0.85rem', outline: 'none', boxSizing: 'border-box',
};
const dropItemStyle: React.CSSProperties = {
  display: 'block', width: '100%', textAlign: 'left',
  background: 'none', border: 'none', padding: '8px 12px',
  cursor: 'pointer', fontSize: '0.85rem',
  color: 'var(--howm-text-primary, #e1e4eb)',
};
const cancelBtnStyle: React.CSSProperties = {
  padding: '6px 14px', background: 'var(--howm-bg-elevated, #2a2e3d)',
  border: '1px solid var(--howm-border, #2e3341)', borderRadius: '4px',
  color: 'var(--howm-text-primary, #e1e4eb)', cursor: 'pointer', fontSize: '0.85rem',
};
const deleteBtnStyle: React.CSSProperties = {
  padding: '6px 14px', background: 'rgba(248,113,113,0.15)',
  border: '1px solid rgba(248,113,113,0.4)', borderRadius: '4px',
  color: '#f87171', cursor: 'pointer', fontSize: '0.85rem', fontWeight: 600,
};
