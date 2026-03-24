import { BUILT_IN_TIERS } from '../lib/access';

interface TierSelectorProps {
  currentTierGroupId: string | null;
  onSelect: (groupId: string) => void;
  disabled?: boolean;
}

export function TierSelector({ currentTierGroupId, onSelect, disabled }: TierSelectorProps) {
  return (
    <div style={containerStyle}>
      {BUILT_IN_TIERS.map(tier => {
        const isActive = currentTierGroupId === tier.id;
        return (
          <button
            key={tier.id}
            onClick={() => !isActive && onSelect(tier.id)}
            disabled={disabled || isActive}
            style={{
              ...btnStyle,
              background: isActive ? tier.bg : 'transparent',
              color: isActive ? tier.color : 'var(--howm-text-muted, #5c6170)',
              borderColor: isActive ? tier.color : 'var(--howm-border, #2e3341)',
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

const containerStyle: React.CSSProperties = {
  display: 'flex', gap: '0',
};

const btnStyle: React.CSSProperties = {
  padding: '8px 20px',
  border: '1px solid var(--howm-border, #2e3341)',
  fontSize: '0.875rem',
  transition: 'all 0.15s',
};
