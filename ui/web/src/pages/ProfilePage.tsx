import { useState, useRef } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { getProfile, updateProfile, uploadAvatar, setHomepage } from '../api/profile';

export function ProfilePage() {
  const qc = useQueryClient();
  const { data: profile, isLoading } = useQuery({
    queryKey: ['profile'],
    queryFn: getProfile,
  });

  const [name, setName] = useState('');
  const [bio, setBio] = useState('');
  const [homepagePath, setHomepagePath] = useState('');
  const [synced, setSynced] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  // Sync form state when profile loads
  const [prevProfile, setPrevProfile] = useState(profile);
  if (profile && profile !== prevProfile) {
    setPrevProfile(profile);
    if (!synced) {
      setName(profile.name);
      setBio(profile.bio);
      setHomepagePath(profile.homepage ?? '');
      setSynced(true);
    }
  }

  const updateMutation = useMutation({
    mutationFn: () => updateProfile({ name: name.trim(), bio }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['profile'] }),
  });

  const avatarMutation = useMutation({
    mutationFn: (file: File) => uploadAvatar(file),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['profile'] }),
  });

  const homepageMutation = useMutation({
    mutationFn: () => setHomepage(homepagePath.trim() || null),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['profile'] }),
  });

  const handleAvatarSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) avatarMutation.mutate(file);
  };

  if (isLoading || !profile) {
    return (
      <div className='max-w-[720px] mx-auto p-6'>
        <h1 className='text-2xl mb-6 font-semibold'>Profile</h1>
        <p className='text-howm-text-muted m-0 text-sm'>Loading…</p>
      </div>
    );
  }

  const avatarUrl = profile.has_avatar ? '/profile/avatar' : null;

  return (
    <div className='max-w-[720px] mx-auto p-6'>
      <h1 className='text-2xl mb-6 font-semibold'>Profile</h1>

      {/* Avatar */}
      <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-5'>
        <h2 className='text-xl font-semibold mt-0 mb-4'>Avatar</h2>
        <div className='flex items-center gap-5'>
          <div
            className='w-20 h-20 rounded-full bg-howm-bg-elevated border border-howm-border flex items-center justify-center overflow-hidden cursor-pointer'
            onClick={() => fileRef.current?.click()}
            title="Click to change avatar"
          >
            {avatarUrl ? (
              <img
                src={avatarUrl}
                alt="Avatar"
                className='w-full h-full object-cover'
              />
            ) : (
              <span className='text-3xl text-howm-text-muted'>👤</span>
            )}
          </div>
          <div>
            <button
              onClick={() => fileRef.current?.click()}
              disabled={avatarMutation.isPending}
              className='px-3.5 py-1.5 bg-howm-bg-elevated border border-howm-border rounded text-howm-text-primary cursor-pointer text-sm'
            >
              {avatarMutation.isPending ? 'Uploading…' : 'Change Avatar'}
            </button>
            <p className='text-howm-text-muted text-xs mt-1.5 mb-0'>
              PNG, JPG, or WebP · Max 1 MB
            </p>
            {avatarMutation.isError && (
              <p className='text-howm-error text-xs mt-1 mb-0'>Upload failed</p>
            )}
          </div>
          <input
            ref={fileRef}
            type="file"
            accept="image/png,image/jpeg,image/webp"
            onChange={handleAvatarSelect}
            className='hidden'
          />
        </div>
      </section>

      {/* Name & Bio */}
      <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-5'>
        <h2 className='text-xl font-semibold mt-0 mb-4'>Info</h2>

        <label className='block text-sm font-semibold text-howm-text-secondary mb-1'>Display Name</label>
        <input
          value={name}
          onChange={e => setName(e.target.value)}
          maxLength={64}
          className='w-full py-2 px-2.5 box-border bg-howm-bg-secondary border border-howm-border rounded text-howm-text-primary text-sm mb-3'
        />

        <label className='block text-sm font-semibold text-howm-text-secondary mb-1'>Bio</label>
        <textarea
          value={bio}
          onChange={e => setBio(e.target.value)}
          maxLength={280}
          rows={3}
          placeholder="Tell the mesh about yourself…"
          className='w-full py-2 px-2.5 box-border bg-howm-bg-secondary border border-howm-border rounded text-howm-text-primary text-sm resize-y mb-1'
        />
        <div className='flex justify-between items-center'>
          <span className='text-howm-text-muted text-xs'>{bio.length}/280</span>
          <div className='flex items-center gap-2'>
            {updateMutation.isSuccess && <span className='text-howm-success text-xs'>Saved ✓</span>}
            {updateMutation.isError && <span className='text-howm-error text-xs'>Failed</span>}
            <button
              onClick={() => updateMutation.mutate()}
              disabled={updateMutation.isPending || (!name.trim())}
              className='px-3.5 py-1.5 bg-howm-accent border-none rounded text-white cursor-pointer text-sm font-semibold'
            >
              {updateMutation.isPending ? 'Saving…' : 'Save'}
            </button>
          </div>
        </div>
      </section>

      {/* Homepage */}
      <section className='bg-howm-bg-surface border border-howm-border rounded-xl p-5 mb-5'>
        <h2 className='text-xl font-semibold mt-0 mb-4'>Homepage</h2>
        <p className='text-howm-text-muted text-sm mb-3'>
          Set a local HTML file as your personal page on the mesh. Peers can visit it live from your node.
          Place files in your profile directory, then enter the relative path below.
        </p>

        <label className='block text-sm font-semibold text-howm-text-secondary mb-1'>
          Homepage Path <span className='font-normal text-howm-text-muted'>(relative to profile dir)</span>
        </label>
        <div className='flex gap-2 items-center'>
          <input
            value={homepagePath}
            onChange={e => setHomepagePath(e.target.value)}
            placeholder="homepage/index.html"
            className='flex-1 py-2 px-2.5 box-border bg-howm-bg-secondary border border-howm-border rounded text-howm-text-primary text-sm font-mono'
          />
          <button
            onClick={() => homepageMutation.mutate()}
            disabled={homepageMutation.isPending}
            className='px-3.5 py-1.5 bg-howm-accent border-none rounded text-white cursor-pointer text-sm font-semibold'
          >
            {homepageMutation.isPending ? 'Setting…' : homepagePath.trim() ? 'Set' : 'Clear'}
          </button>
        </div>
        {homepageMutation.isSuccess && (
          <p className='text-howm-success text-xs mt-1 mb-0'>Homepage updated ✓</p>
        )}
        {homepageMutation.isError && (
          <p className='text-howm-error text-xs mt-1 mb-0'>Failed — check that the file exists in your profile directory</p>
        )}

        {/* Preview */}
        {profile.has_homepage && (
          <div className='mt-4'>
            <h3 className='text-sm font-semibold text-howm-text-secondary mb-2'>Preview</h3>
            <div className='border border-howm-border rounded overflow-hidden bg-white' style={{ height: 300 }}>
              <iframe
                src="/profile/home"
                title="Homepage preview"
                sandbox="allow-scripts allow-same-origin"
                className='w-full h-full border-none'
              />
            </div>
          </div>
        )}
      </section>
    </div>
  );
}
