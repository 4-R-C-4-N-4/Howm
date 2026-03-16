import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { getNetworkFeed } from '../api/feed';
import { PostCard } from '../components/PostCard';
import { PostComposer } from '../components/PostComposer';

export function Feed() {
  const [trustFilter, setTrustFilter] = useState<string | undefined>(undefined);

  const { data, isLoading, isError, refetch } = useQuery({
    queryKey: ['network-feed', trustFilter],
    queryFn: () => getNetworkFeed(trustFilter),
    refetchInterval: 30000,
  });

  return (
    <div style={{ maxWidth: '600px', margin: '0 auto', padding: '24px' }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '24px' }}>
        <h1 style={{ margin: 0 }}>Network Feed</h1>
        <div style={{ display: 'flex', gap: '4px', background: '#f3f4f6', borderRadius: '6px', padding: '2px' }}>
          <button
            onClick={() => setTrustFilter(undefined)}
            style={{
              ...toggleBtnStyle,
              ...(trustFilter === undefined ? activeToggleStyle : {}),
            }}
          >
            All
          </button>
          <button
            onClick={() => setTrustFilter('friend')}
            style={{
              ...toggleBtnStyle,
              ...(trustFilter === 'friend' ? activeToggleStyle : {}),
            }}
          >
            Friends Only
          </button>
        </div>
      </div>

      <PostComposer />

      {data?.errors && data.errors.length > 0 && (
        <div style={{
          background: '#fef3c7', border: '1px solid #f59e0b',
          borderRadius: '8px', padding: '10px 14px', marginBottom: '16px',
        }}>
          <strong>Partial results:</strong> Some peers were unreachable:
          <ul style={{ margin: '4px 0 0', paddingLeft: '20px' }}>
            {data.errors.map((e, i) => <li key={i}>{e}</li>)}
          </ul>
        </div>
      )}

      {isLoading && <p>Loading feed...</p>}
      {isError && (
        <div style={{ color: '#ef4444', marginBottom: '16px' }}>
          Failed to load feed.{' '}
          <button onClick={() => refetch()} style={{ color: '#4f46e5', background: 'none', border: 'none', cursor: 'pointer', textDecoration: 'underline' }}>
            Retry
          </button>
        </div>
      )}

      {data?.posts?.map(post => (
        <PostCard key={post.id} post={post} />
      ))}

      {!isLoading && !isError && data?.posts?.length === 0 && (
        <p style={{ color: '#888', textAlign: 'center' }}>No posts yet. Be the first to post!</p>
      )}
    </div>
  );
}

const toggleBtnStyle: React.CSSProperties = {
  padding: '4px 12px', border: 'none', borderRadius: '4px',
  cursor: 'pointer', fontSize: '0.85em', background: 'transparent', color: '#6b7280',
};
const activeToggleStyle: React.CSSProperties = {
  background: '#fff', color: '#111827', boxShadow: '0 1px 2px rgba(0,0,0,0.1)',
};
