'use client';

// A quantity a thumb can change: − and + as full-size buttons beside the number, which can still be
// typed. The number input's own steppers are half a control high, too small to hit on a phone.

import {Icon} from '@astryxdesign/core/Icon';
import {IconButton} from '@astryxdesign/core/IconButton';
import {HStack} from '@astryxdesign/core/Layout';
import {NumberInput} from '@astryxdesign/core/NumberInput';
import {MinusIcon, PlusIcon} from '@heroicons/react/24/outline';

export function QuantityInput({
  value,
  onChange,
  min = 1,
  label = 'Quantity',
  isLabelHidden = false,
  size = 'md',
  isDisabled = false,
}: {
  value: number;
  /** A whole number, `min` or more. */
  onChange: (quantity: number) => void;
  min?: number;
  label?: string;
  isLabelHidden?: boolean;
  size?: 'sm' | 'md' | 'lg';
  isDisabled?: boolean;
}) {
  const set = (n: number) => {
    const q = Math.max(min, Math.round(n || min));
    if (q !== value) onChange(q);
  };
  return (
    <HStack gap={1} vAlign="end">
      <IconButton label={`Fewer (${label.toLowerCase()} ${value})`} icon={<Icon icon={MinusIcon} />} variant="secondary" size={size} isDisabled={isDisabled || value <= min} onClick={() => set(value - 1)} />
      <NumberInput label={label} isLabelHidden={isLabelHidden} value={value} min={min} isIntegerOnly width={64} size={size} isDisabled={isDisabled} onChange={set} />
      <IconButton label={`More (${label.toLowerCase()} ${value})`} icon={<Icon icon={PlusIcon} />} variant="secondary" size={size} isDisabled={isDisabled} onClick={() => set(value + 1)} />
    </HStack>
  );
}
