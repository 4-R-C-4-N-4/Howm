import { BUILT_IN_TIERS } from '../lib/access';

interface TierSelectorProps {
  currentTierGroupId: string | null;
  onSelect: (groupId: string) => void;
  disabled?: boolean;
}

export function TierSelector({ currentTierGroupId, onSelect, disabled }: TierSelectorProps) {
  return (
    <div className="flex">
      {BUILT_IN_TIERS.map(tier => {
        const isActive = currentTierGroupId === tier.id;
        return (
          <button
            key={tier.id}
            onClick={() => !isActive && onSelect(tier.id)}
            disabled={disabled || isActive}
            className="py-2 px-5 border border-howm-border text-sm transition-all"
            style={{
              background: isActive ? tier.bg : 'transparent',
              color: isActive ? tier.color : 'var(--howm-text-muted, #666666)',
              borderColor: isActive ? tier.color : 'var(--howm-border, #222222)',
              cursor: isActive || disabled ? 'default' : 'pointer',
              fontWeight: isActive ? 600 : 400,
            }}
          >
            {isActive ? `★ ${tier.label}` : tier.label}
          </button>
        );
      })}
    </div>
  );
}
