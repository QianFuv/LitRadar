'use client';

/**
 * Purpose-built desktop UI primitives for the rebuilt frontend.
 */

import { X } from 'lucide-react';
import {
  Children,
  isValidElement,
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type ChangeEvent,
  type InputHTMLAttributes,
  type OptionHTMLAttributes,
  type ReactNode,
  type KeyboardEvent as ReactKeyboardEvent,
  type SelectHTMLAttributes,
  type TextareaHTMLAttributes,
} from 'react';

type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger' | 'violet';
type BadgeTone = 'teal' | 'coral' | 'violet' | 'amber' | 'neutral';

/**
 * Join class names while skipping empty values.
 *
 * @param values - Candidate class names.
 * @returns Joined class name string.
 */
export function joinClassNames(...values: Array<string | false | null | undefined>): string {
  return values.filter(Boolean).join(' ');
}

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: 'small' | 'normal';
  wide?: boolean;
  icon?: ReactNode;
}

/**
 * Render a desktop command button.
 *
 * @param props - Button props.
 * @returns Button element.
 */
export function Button({
  children,
  className,
  icon,
  size = 'normal',
  variant = 'secondary',
  wide = false,
  ...props
}: ButtonProps) {
  return (
    <button
      className={joinClassNames(
        'btn',
        `btn--${variant}`,
        size === 'small' && 'btn--small',
        wide && 'btn--wide',
        className,
      )}
      {...props}
    >
      {icon}
      {children}
    </button>
  );
}

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  danger?: boolean;
}

/**
 * Render a square icon-only button.
 *
 * @param props - Icon button props.
 * @returns Icon button element.
 */
export function IconButton({ children, className, danger = false, ...props }: IconButtonProps) {
  return (
    <button
      className={joinClassNames('icon-btn', danger && 'icon-btn--danger', className)}
      {...props}
    >
      {children}
    </button>
  );
}

interface PanelProps {
  title?: ReactNode;
  meta?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  dense?: boolean;
  flush?: boolean;
}

/**
 * Render a framed desktop work panel.
 *
 * @param props - Panel props.
 * @returns Panel element.
 */
export function Panel({
  actions,
  children,
  className,
  dense = false,
  flush = false,
  meta,
  title,
}: PanelProps) {
  return (
    <section className={joinClassNames('panel', flush && 'panel--flush', className)}>
      {(title || actions || meta) && (
        <div className="panel__header">
          <div>
            {title && <h2 className="panel__title">{title}</h2>}
            {meta && <div className="panel__meta">{meta}</div>}
          </div>
          {actions && <div className="toolbar toolbar--wrap">{actions}</div>}
        </div>
      )}
      <div className={joinClassNames('panel__body', dense && 'panel__body--dense')}>{children}</div>
    </section>
  );
}

interface FieldProps {
  label: ReactNode;
  children: ReactNode;
  className?: string;
}

/**
 * Render a labeled form field.
 *
 * @param props - Field props.
 * @returns Field wrapper.
 */
export function Field({ children, className, label }: FieldProps) {
  return (
    <label className={joinClassNames('field', className)}>
      <span className="field__label">{label}</span>
      {children}
    </label>
  );
}

/**
 * Render a text input with desktop styling.
 *
 * @param props - Native input props.
 * @returns Input element.
 */
export function TextInput({ className, ...props }: InputHTMLAttributes<HTMLInputElement>) {
  return <input className={joinClassNames('field__control', className)} {...props} />;
}

/**
 * Render a textarea with desktop styling.
 *
 * @param props - Native textarea props.
 * @returns Textarea element.
 */
export function TextArea({ className, ...props }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea className={joinClassNames('field__control', className)} {...props} />;
}

interface SelectOptionItem {
  disabled: boolean;
  key: string;
  label: ReactNode;
  value: string;
}

/**
 * Convert option children into plain text for fallback option values.
 *
 * @param node - Option label node.
 * @returns Text represented by the node.
 */
function getOptionText(node: ReactNode): string {
  if (typeof node === 'string' || typeof node === 'number') {
    return String(node);
  }
  if (Array.isArray(node)) {
    return node.map(getOptionText).join('');
  }
  if (isValidElement<{ children?: ReactNode }>(node)) {
    return getOptionText(node.props.children);
  }
  return '';
}

/**
 * Normalize a select value into the string shape used by option values.
 *
 * @param value - Select value or default value.
 * @returns Normalized string value.
 */
function normalizeSelectValue(value: SelectHTMLAttributes<HTMLSelectElement>['value']) {
  if (value === undefined || value === null) {
    return undefined;
  }
  if (Array.isArray(value)) {
    return String(value[0] ?? '');
  }
  return String(value);
}

/**
 * Extract renderable option metadata from option children.
 *
 * @param children - Select option children.
 * @returns Option metadata.
 */
function getSelectOptions(children: ReactNode): SelectOptionItem[] {
  const options: SelectOptionItem[] = [];
  Children.forEach(children, (child) => {
    if (
      !isValidElement<OptionHTMLAttributes<HTMLOptionElement>>(child) ||
      child.type !== 'option'
    ) {
      return;
    }
    const optionValue =
      child.props.value === undefined ? getOptionText(child.props.children) : child.props.value;
    const value = String(optionValue);
    options.push({
      disabled: Boolean(child.props.disabled),
      key: child.key?.toString() ?? value,
      label: child.props.children,
      value,
    });
  });
  return options;
}

/**
 * Create the minimal change event shape used by current select handlers.
 *
 * @param value - Next select value.
 * @param name - Select name.
 * @returns Change event compatible with select handlers.
 */
function createSelectChangeEvent(
  value: string,
  name: string | undefined,
): ChangeEvent<HTMLSelectElement> {
  const target = { name: name ?? '', value } as HTMLSelectElement;
  return { currentTarget: target, target } as ChangeEvent<HTMLSelectElement>;
}

/**
 * Render a custom select input with desktop styling.
 *
 * @param props - Select props.
 * @returns Select control.
 */
export function SelectInput({
  className,
  children,
  defaultValue,
  disabled = false,
  name,
  onChange,
  style,
  title,
  value,
  ...props
}: SelectHTMLAttributes<HTMLSelectElement>) {
  const listboxId = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const options = useMemo(() => getSelectOptions(children), [children]);
  const controlledValue = normalizeSelectValue(value);
  const defaultSelectedValue = normalizeSelectValue(defaultValue);
  const [open, setOpen] = useState(false);
  const [internalValue, setInternalValue] = useState(
    () => defaultSelectedValue ?? options[0]?.value ?? '',
  );
  const [menuPlacement, setMenuPlacement] = useState<'down' | 'up'>('down');
  const [menuMaxHeight, setMenuMaxHeight] = useState(240);
  const selectedValue = controlledValue ?? internalValue;
  const selectedOption = options.find((option) => option.value === selectedValue) ?? options[0];

  const updateMenuMetrics = useCallback(() => {
    const rootRect = rootRef.current?.getBoundingClientRect();
    if (!rootRect) {
      return;
    }
    const menuGap = 6;
    const estimatedMenuHeight = Math.min(240, options.length * 31 + 12);
    const spaceBelow = window.innerHeight - rootRect.bottom - menuGap;
    const spaceAbove = rootRect.top - menuGap;
    const nextPlacement =
      spaceBelow < estimatedMenuHeight && spaceAbove > spaceBelow ? 'up' : 'down';
    const availableSpace = nextPlacement === 'up' ? spaceAbove : spaceBelow;
    const nextMaxHeight = Math.max(72, Math.min(240, Math.floor(availableSpace)));
    setMenuPlacement((currentPlacement) =>
      currentPlacement === nextPlacement ? currentPlacement : nextPlacement,
    );
    setMenuMaxHeight((currentHeight) =>
      currentHeight === nextMaxHeight ? currentHeight : nextMaxHeight,
    );
  }, [options.length]);

  useEffect(() => {
    if (!open) {
      return;
    }
    const handlePointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    window.addEventListener('pointerdown', handlePointerDown);
    return () => window.removeEventListener('pointerdown', handlePointerDown);
  }, [open]);

  useEffect(() => {
    if (!open) {
      return;
    }
    window.addEventListener('resize', updateMenuMetrics);
    window.addEventListener('scroll', updateMenuMetrics, true);
    return () => {
      window.removeEventListener('resize', updateMenuMetrics);
      window.removeEventListener('scroll', updateMenuMetrics, true);
    };
  }, [open, updateMenuMetrics]);

  const selectValue = (nextValue: string) => {
    if (disabled) {
      return;
    }
    if (controlledValue === undefined) {
      setInternalValue(nextValue);
    }
    onChange?.(createSelectChangeEvent(nextValue, name));
    setOpen(false);
  };

  const openMenu = () => {
    if (disabled) {
      return;
    }
    updateMenuMetrics();
    setOpen(true);
  };

  const handleButtonKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (event.key === 'Escape') {
      setOpen(false);
      return;
    }
    if (event.key === 'Enter' || event.key === ' ' || event.key === 'ArrowDown') {
      event.preventDefault();
      openMenu();
    }
  };

  return (
    <div
      ref={rootRef}
      className={joinClassNames(
        'select-input',
        open && 'select-input--open',
        open && menuPlacement === 'up' && 'select-input--up',
        className,
      )}
      style={style}
    >
      <select
        aria-hidden="true"
        className="select-input__native"
        disabled={disabled}
        name={name}
        tabIndex={-1}
        value={selectedOption?.value ?? ''}
        onChange={onChange}
      >
        {children}
      </select>
      <button
        aria-controls={open ? listboxId : undefined}
        aria-expanded={open}
        aria-haspopup="listbox"
        aria-label={props['aria-label']}
        className="field__control select-input__button"
        disabled={disabled}
        title={title}
        type="button"
        onClick={() => {
          if (open) {
            setOpen(false);
            return;
          }
          openMenu();
        }}
        onKeyDown={handleButtonKeyDown}
      >
        <span className="select-input__value">{selectedOption?.label ?? selectedValue}</span>
      </button>
      {open ? (
        <div
          className="select-input__menu"
          id={listboxId}
          role="listbox"
          style={{ maxHeight: menuMaxHeight }}
        >
          {options.map((option) => (
            <button
              key={option.key}
              aria-selected={option.value === selectedValue}
              className={joinClassNames(
                'select-input__option',
                option.value === selectedValue && 'select-input__option--selected',
              )}
              disabled={option.disabled}
              role="option"
              type="button"
              onClick={() => selectValue(option.value)}
            >
              {option.label}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}

interface CheckboxRowProps extends InputHTMLAttributes<HTMLInputElement> {
  label: ReactNode;
  detail?: ReactNode;
}

/**
 * Render a checkbox row for filter and settings lists.
 *
 * @param props - Checkbox row props.
 * @returns Checkbox row.
 */
export function CheckboxRow({ detail, label, className, ...props }: CheckboxRowProps) {
  return (
    <label className={joinClassNames('check-row', className)}>
      <input type="checkbox" {...props} />
      <span>
        <span>{label}</span>
        {detail && <span className="panel__meta"> {detail}</span>}
      </span>
    </label>
  );
}

interface SwitchRowProps extends InputHTMLAttributes<HTMLInputElement> {
  label: ReactNode;
  detail?: ReactNode;
}

/**
 * Render a switch row.
 *
 * @param props - Switch row props.
 * @returns Switch row.
 */
export function SwitchRow({ detail, label, className, ...props }: SwitchRowProps) {
  return (
    <label className={joinClassNames('switch-row', className)}>
      <span>
        <span>{label}</span>
        {detail && <span className="panel__meta"> {detail}</span>}
      </span>
      <input className="switch" type="checkbox" {...props} />
    </label>
  );
}

interface BadgeProps {
  children: ReactNode;
  tone?: BadgeTone;
  className?: string;
}

/**
 * Render a compact status badge.
 *
 * @param props - Badge props.
 * @returns Badge element.
 */
export function Badge({ children, className, tone = 'neutral' }: BadgeProps) {
  return <span className={joinClassNames('badge', `badge--${tone}`, className)}>{children}</span>;
}

interface NoticeProps {
  children: ReactNode;
  tone?: 'default' | 'error';
  className?: string;
}

/**
 * Render a contextual notice.
 *
 * @param props - Notice props.
 * @returns Notice element.
 */
export function Notice({ children, className, tone = 'default' }: NoticeProps) {
  return (
    <div className={joinClassNames('notice', tone === 'error' && 'notice--error', className)}>
      {children}
    </div>
  );
}

/**
 * Render an empty state area.
 *
 * @param props - Empty state props.
 * @returns Empty state element.
 */
export function EmptyState({ children }: { children: ReactNode }) {
  return <div className="empty-state">{children}</div>;
}

/**
 * Render a loading placeholder block.
 *
 * @param props - Skeleton props.
 * @returns Skeleton element.
 */
export function Skeleton({ className }: { className?: string }) {
  return <div className={joinClassNames('skeleton', className)} />;
}

interface ModalProps {
  open: boolean;
  title: ReactNode;
  description?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  narrow?: boolean;
  onClose: () => void;
}

/**
 * Render an accessible modal dialog.
 *
 * @param props - Modal props.
 * @returns Modal dialog or null.
 */
export function Modal({
  children,
  description,
  footer,
  narrow = false,
  onClose,
  open,
  title,
}: ModalProps) {
  useEffect(() => {
    if (!open) {
      return;
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [onClose, open]);

  if (!open) {
    return null;
  }

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={onClose}>
      <section
        aria-modal="true"
        className={joinClassNames('modal', narrow && 'modal--narrow')}
        role="dialog"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header className="modal__header">
          <div>
            <h2 className="modal__title">{title}</h2>
            {description && <p className="modal__description">{description}</p>}
          </div>
          <IconButton aria-label="关闭" title="关闭" onClick={onClose}>
            <X size={16} />
          </IconButton>
        </header>
        <div className="modal__body">{children}</div>
        {footer && <footer className="modal__footer">{footer}</footer>}
      </section>
    </div>
  );
}
