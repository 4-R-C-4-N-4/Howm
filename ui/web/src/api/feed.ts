import api from './client';

export interface Post {
  id: string;
  author_id: string;
  author_name: string;
  content: string;
  timestamp: number;
}

export const getNetworkFeed = () =>
  api.get<{ posts: Post[]; errors: string[] }>('/network/feed').then(r => r.data);
export const createPost = (content: string) =>
  api.post<{ post: Post }>('/cap/social/post', { content }).then(r => r.data.post);
