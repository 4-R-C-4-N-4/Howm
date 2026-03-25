import { create } from 'zustand';

interface BadgeState {
  /** capability name → badge count */
  counts: Record<string, number>;
  setBadge: (capability: string, count: number) => void;
  clearBadge: (capability: string) => void;
}

export const useBadgeStore = create<BadgeState>((set) => ({
  counts: {},
  setBadge: (capability, count) =>
    set((state) => ({ counts: { ...state.counts, [capability]: count } })),
  clearBadge: (capability) =>
    set((state) => {
      const { [capability]: _, ...rest } = state.counts;
      void _;
      return { counts: rest };
    }),
}));
