import { useParams, Link, useNavigate } from 'react-router-dom';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useState, useCallback, useRef, useEffect } from 'react';
import {
  getAccessGroup, updateAccessGroup, deleteAccessGroup,
  assignPeerToGroup, removePeerFromGroup, getGroupMembers,
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

  const { data: memberData } = useQuery({
    queryKey: ['group-members', groupId],
    queryFn: () => getGroupMembers(groupId!),
    enabled: !!groupId,
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
      queryClient.invalidateQueries({ queryKey: ['group-members', groupId] });
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
      queryClient.invalidateQueries({ queryKey: ['group-members', groupId] });
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
      <div className='max-w-[800px] mx-auto p-6'>
        <Link to="/access/groups" className='text-howm-text-muted no-underline text-sm'>← Back to Groups</Link>
        <p className='text-howm-text-muted text-sm m-0'>{isLoading ? 'Loading…' : 'Group not found'}</p>
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

  // Members — fetched from /access/groups/:id/members endpoint
  const memberPeerIds: string[] = memberData?.members ?? [];

  const memberPeers = peers.filter(p => memberPeerIds.includes(peerIdToHex(p.wg_pubkey)));

  const availablePeers = peers
    .filter(p => !memberPeerIds.includes(peerIdToHex(p.wg_pubkey)))
    .filter(p => !peerSearch || p.name.toLowerCase().includes(peerSearch.toLowerCase()));

  return (
    <div className='max-w-[800px] mx-auto p-6'>
      <Link to="/access/groups" className='text-howm-text-muted no-underline text-sm'>← Back to Groups</Link>

      <div className='flex justify-between items-start mt-3 mb-4'>
        <div className='flex-1'>
          {isBuiltIn ? (
            <h1 className='m-0 text-2xl'>{group.name}</h1>
          ) : (
            <input
              value={editName} onChange={e => setEditName(e.target.value)}
              onBlur={handleNameSave} onKeyDown={e => e.key === 'Enter' && handleNameSave()}
              className='bg-transparent border-none border-b border-b-transparent text-howm-text-primary outline-none w-full py-0.5 px-0 text-2xl font-semibold'
            />
          )}
          {isBuiltIn ? (
            <p className='text-howm-text-muted text-sm m-0 mt-1'>{group.description}</p>
          ) : (
            <input
              value={editDesc} onChange={e => setEditDesc(e.target.value)}
              onBlur={handleDescSave}
              placeholder="Add description..."
              className='bg-transparent border-none border-b border-b-transparent text-howm-text-muted outline-none w-full py-0.5 px-0 text-sm mt-1'
            />
          )}
        </div>
        {isBuiltIn && (
          <span className='text-xs py-1 px-2.5 rounded bg-gray-500/10 text-gray-400'>Built-in 🔒</span>
        )}
      </div>

      {/* Members */}
      <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-4'>
        <h3 className='text-base font-semibold m-0 mb-3 text-howm-text-secondary'>Members</h3>
        {memberPeers.length === 0 ? (
          <p className='text-howm-text-muted text-sm m-0'>No members in this group</p>
        ) : (
          <div>
            {memberPeers.map(p => {
              const hexId = peerIdToHex(p.wg_pubkey);
              return (
                <div key={p.node_id} className='flex items-center justify-between py-2 px-3 rounded bg-howm-bg-secondary mb-1'>
                  <Link to={`/peers/${hexId}`} className='no-underline text-inherit flex-1'>
                    <span className='font-medium'>{p.name}</span>
                  </Link>
                  <button
                    onClick={() => removePeerMutation.mutate(hexId)}
                    disabled={removePeerMutation.isPending}
                    className='bg-red-500/10 border border-red-500/30 rounded text-red-500 cursor-pointer text-xs py-0.5 px-2'
                  >
                    Remove from group
                  </button>
                </div>
              );
            })}
          </div>
        )}
        <div className='relative mt-2' ref={addRef}>
          <button onClick={() => setShowAddPeer(!showAddPeer)} className='bg-blue-500/10 border border-blue-500/25 rounded text-howm-accent cursor-pointer text-xs py-1.5 px-3'>
            + Add Peer
          </button>
          {showAddPeer && (
            <div className='absolute left-0 top-full mt-1 bg-howm-bg-surface border border-howm-border rounded-lg z-200 min-w-[200px] shadow-xl overflow-hidden'>
              <input
                autoFocus placeholder="Search peers..."
                value={peerSearch} onChange={e => setPeerSearch(e.target.value)}
                className='w-full py-2 px-3 border-none border-b border-b-howm-border bg-transparent text-howm-text-primary text-sm outline-none box-border'
              />
              {availablePeers.length === 0 ? (
                <div className='py-2 px-3 text-howm-text-muted text-xs'>No peers available</div>
              ) : availablePeers.map(p => (
                <button
                  key={p.node_id}
                  onClick={() => addPeerMutation.mutate(peerIdToHex(p.wg_pubkey))}
                  className='block w-full text-left bg-none border-none py-2 px-3 cursor-pointer text-sm text-howm-text-primary'
                >
                  {p.name}
                </button>
              ))}
            </div>
          )}
        </div>
      </section>

      {/* Capability Rules */}
      <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-4'>
        <h3 className='text-base font-semibold m-0 mb-3 text-howm-text-secondary'>Capability Rules</h3>
        <div className='flex flex-col gap-1'>
          {ALL_CAPABILITIES.map(cap => {
            const rule = capMap.get(cap);
            const allowed = rule?.allow ?? false;
            return (
              <div key={cap} className='flex items-center py-1'>
                {isBuiltIn ? (
                  <span style={{ color: allowed ? '#22c55e' : '#ef4444' }} className='mr-2 w-4'>
                    {allowed ? '✓' : '✕'}
                  </span>
                ) : (
                  <input
                    type="checkbox"
                    checked={allowed}
                    onChange={() => handleCapToggle(cap)}
                    className='mr-2 accent-howm-accent'
                  />
                )}
                <span className='text-sm font-mono'>
                  {cap}
                </span>
              </div>
            );
          })}
        </div>
      </section>

      {/* Delete (custom only) */}
      {!isBuiltIn && (
        <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-4' style={{ borderColor: 'rgba(239,68,68,0.3)' }}>
          <h3 className='text-base font-semibold m-0 mb-3 text-red-500'>Danger Zone</h3>
          <p className='text-howm-text-muted text-sm m-0 mb-3'>
            Delete this group? Members will be removed from this group but retain other group memberships.
          </p>
          {showDeleteConfirm ? (
            <div className='flex gap-2'>
              <button onClick={() => setShowDeleteConfirm(false)} className='py-1.5 px-3.5 bg-howm-bg-elevated border border-howm-border rounded text-howm-text-primary cursor-pointer text-sm'>Cancel</button>
              <button onClick={() => deleteMutation.mutate()} disabled={deleteMutation.isPending} className='py-1.5 px-3.5 bg-red-500/15 border border-red-500/40 rounded text-red-500 cursor-pointer text-sm font-semibold'>
                {deleteMutation.isPending ? 'Deleting…' : `Delete "${group.name}"`}
              </button>
            </div>
          ) : (
            <button onClick={() => setShowDeleteConfirm(true)} className='py-1.5 px-3.5 bg-red-500/15 border border-red-500/40 rounded text-red-500 cursor-pointer text-sm font-semibold'>
              🔴 Delete Group
            </button>
          )}
        </section>
      )}

      {toasts.length > 0 && (
        <div className='fixed bottom-6 left-1/2 -translate-x-1/2 flex flex-col gap-2 z-300'>
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
