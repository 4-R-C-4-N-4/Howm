import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { createPost } from '../api/feed';

export function PostComposer() {
  const [content, setContent] = useState('');
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: createPost,
    onSuccess: () => {
      setContent('');
      queryClient.invalidateQueries({ queryKey: ['network-feed'] });
    },
  });

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (content.trim()) {
      mutation.mutate(content.trim());
    }
  };

  return (
    <form onSubmit={handleSubmit} style={{ marginBottom: '24px' }}>
      <textarea
        value={content}
        onChange={(e) => setContent(e.target.value)}
        placeholder="What's happening?"
        rows={3}
        style={{
          width: '100%',
          padding: '12px',
          borderRadius: '8px',
          border: '1px solid #ddd',
          fontSize: '1em',
          resize: 'vertical',
          boxSizing: 'border-box',
        }}
      />
      <button
        type="submit"
        disabled={!content.trim() || mutation.isPending}
        style={{
          marginTop: '8px',
          padding: '8px 20px',
          background: '#4f46e5',
          color: '#fff',
          border: 'none',
          borderRadius: '6px',
          cursor: 'pointer',
          fontSize: '0.95em',
        }}
      >
        {mutation.isPending ? 'Posting...' : 'Post'}
      </button>
      {mutation.isError && (
        <span style={{ color: 'red', marginLeft: '12px' }}>Failed to post</span>
      )}
    </form>
  );
}
