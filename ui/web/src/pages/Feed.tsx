import { useQuery } from '@tanstack/react-query';
import { getNetworkFeed } from '../api/feed';
import { PostCard } from '../components/PostCard';
import { PostComposer } from '../components/PostComposer';

export function Feed() {
  const { data, isLoading, isError, refetch } = useQuery({
    queryKey: ['network-feed'],
    queryFn: getNetworkFeed,
    refetchInterval: 30000,
  });

  return (
    <div style={{ maxWidth: '600px', margin: '0 auto', padding: '24px' }}>
      <h1 style={{ marginBottom: '24px' }}>Network Feed</h1>

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
