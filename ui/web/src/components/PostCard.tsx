import type { Post } from '../api/feed';

interface Props {
  post: Post;
}

export function PostCard({ post }: Props) {
  const date = new Date(post.timestamp * 1000).toLocaleString();
  return (
    <div style={{
      border: '1px solid #ddd',
      borderRadius: '8px',
      padding: '16px',
      marginBottom: '12px',
      background: '#fff',
    }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '8px' }}>
        <strong>{post.author_name}</strong>
        <span style={{ color: '#888', fontSize: '0.85em' }}>{date}</span>
      </div>
      <p style={{ margin: 0, whiteSpace: 'pre-wrap' }}>{post.content}</p>
    </div>
  );
}
