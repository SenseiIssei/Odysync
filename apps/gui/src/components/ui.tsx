/** Shared presentational building blocks used across every page. */

import { AlertTriangle, CheckCircle2, Info, Loader2, Search } from "lucide-react";
import {
  useEffect,
  useId,
  useRef,
  type ButtonHTMLAttributes,
  type KeyboardEvent,
  type ReactNode,
  type Ref,
} from "react";
import type { Resource } from "../store-context";

export function PageHeader({
  title,
  subtitle,
  actions,
}: {
  title: string;
  subtitle?: string;
  actions?: ReactNode;
}) {
  return (
    <header className="flex flex-wrap items-end justify-between gap-x-6 gap-y-3 pb-1">
      <div className="min-w-0">
        {/* Focusable so that navigation can move the focus here. */}
        <h1 tabIndex={-1} className="text-[26px] leading-tight font-bold text-fg">
          {title}
        </h1>
        {subtitle && <p className="text-sm text-muted mt-1 max-w-[70ch]">{subtitle}</p>}
      </div>
      {actions && <div className="flex items-center gap-2 flex-shrink-0">{actions}</div>}
    </header>
  );
}

type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";

const BUTTON_STYLES: Record<ButtonVariant, string> = {
  primary: "bg-gradient-accent text-accent-fg border border-transparent btn-glow",
  secondary:
    "bg-surface/70 text-fg border border-line hover:bg-raised hover:border-line-strong backdrop-blur",
  ghost: "text-muted hover:text-fg hover:bg-raised border border-transparent",
  danger:
    "bg-danger/10 text-danger border border-danger/25 hover:bg-danger/12 hover:border-danger/40",
};

export function Button({
  variant = "secondary",
  size = "md",
  icon,
  busy,
  children,
  className = "",
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  ref?: Ref<HTMLButtonElement>;
  variant?: ButtonVariant;
  size?: "sm" | "md" | "lg";
  icon?: ReactNode;
  busy?: boolean;
}) {
  const sizing =
    size === "sm"
      ? "h-7 px-2.5 text-xs gap-1.5 rounded-lg"
      : size === "lg"
        ? "h-12 px-6 text-[15px] gap-2.5 rounded-xl font-semibold"
        : "h-9 px-3.5 text-sm gap-2 rounded-lg";
  return (
    <button
      type="button"
      {...rest}
      disabled={rest.disabled || busy}
      className={`inline-flex items-center justify-center font-medium whitespace-nowrap transition-colors disabled:opacity-50 disabled:pointer-events-none ${sizing} ${BUTTON_STYLES[variant]} ${className}`}
    >
      {busy ? <Loader2 aria-hidden className="w-4 h-4 animate-spin" /> : icon}
      {children}
    </button>
  );
}

type Tone = "neutral" | "accent" | "success" | "warning" | "danger" | "info";

const TONE_BADGE: Record<Tone, string> = {
  neutral: "bg-raised text-muted border-line",
  accent: "bg-accent/10 text-accent border-accent/20",
  success: "bg-success/10 text-success border-success/20",
  warning: "bg-warning/10 text-warning border-warning/25",
  danger: "bg-danger/10 text-danger border-danger/25",
  info: "bg-info/10 text-info border-info/20",
};

export function Badge({ tone = "neutral", children }: { tone?: Tone; children: ReactNode }) {
  return (
    <span
      className={`inline-flex items-center gap-1 rounded-md border px-1.5 py-px text-[11px] font-medium leading-4 whitespace-nowrap ${TONE_BADGE[tone]}`}
    >
      {children}
    </span>
  );
}

function Notice({
  tone,
  icon,
  message,
  action,
}: {
  tone: Tone;
  icon: ReactNode;
  message: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div
      role={tone === "danger" ? "alert" : "status"}
      className={`flex items-start gap-2.5 text-sm rounded-lg px-3.5 py-2.5 border ${TONE_BADGE[tone]}`}
    >
      <span className="flex-shrink-0 mt-0.5">{icon}</span>
      <div className="min-w-0 flex-1 break-words text-fg">{message}</div>
      {action && <div className="flex-shrink-0">{action}</div>}
    </div>
  );
}

export function ErrorBar({ message, action }: { message: ReactNode; action?: ReactNode }) {
  return (
    <Notice
      tone="danger"
      icon={<AlertTriangle className="w-4 h-4" />}
      message={message}
      action={action}
    />
  );
}

export function SuccessBar({ message }: { message: string }) {
  return <Notice tone="success" icon={<CheckCircle2 className="w-4 h-4" />} message={message} />;
}

export function InfoBar({ message }: { message: ReactNode }) {
  return <Notice tone="info" icon={<Info className="w-4 h-4" />} message={message} />;
}

export function WarningBar({ message, action }: { message: ReactNode; action?: ReactNode }) {
  return (
    <Notice
      tone="warning"
      icon={<AlertTriangle className="w-4 h-4" />}
      message={message}
      action={action}
    />
  );
}

export function EmptyState({
  icon,
  title,
  hint,
  action,
}: {
  icon: ReactNode;
  title: string;
  hint?: string;
  action?: ReactNode;
}) {
  return (
    <div className="text-center py-12 px-6">
      <div className="mx-auto mb-3 w-12 h-12 rounded-2xl bg-accent/10 border border-accent/15 flex items-center justify-center text-accent [&>svg]:w-5 [&>svg]:h-5">
        {icon}
      </div>
      <p className="text-sm font-medium text-fg">{title}</p>
      {hint && <p className="text-sm text-muted mt-1 max-w-[52ch] mx-auto">{hint}</p>}
      {action && <div className="mt-4">{action}</div>}
    </div>
  );
}

function LoadingState({ label }: { label: string }) {
  return (
    <div className="flex items-center justify-center gap-2.5 py-12 text-sm text-muted">
      <Loader2 className="w-4 h-4 animate-spin text-accent" />
      {label}
    </div>
  );
}

/**
 * Renders the correct state for a background-loaded resource: the spinner only
 * while the first load is still running, the error once it has failed, and the
 * children as soon as there is data to show (even while it is refreshing).
 */
export function ResourceView<T>({
  resource,
  loadingLabel,
  children,
}: {
  resource: Resource<T>;
  loadingLabel: string;
  children: ReactNode;
}) {
  const neverLoaded = resource.loadedAt === null;

  if (neverLoaded && resource.loading) return <LoadingState label={loadingLabel} />;
  if (neverLoaded && resource.error) return <ErrorBar message={resource.error} />;

  return (
    <>
      {resource.error && <ErrorBar message={resource.error} />}
      {children}
    </>
  );
}

export function Toggle({
  label,
  description,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  description?: ReactNode;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <div className="flex items-center justify-between gap-6">
      <div className="min-w-0">
        <div className="text-sm font-medium text-fg">{label}</div>
        {description && <div className="text-sm text-muted mt-0.5">{description}</div>}
      </div>
      <Switch checked={checked} onChange={onChange} disabled={disabled} label={label} />
    </div>
  );
}

export function Switch({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`relative inline-flex h-[22px] w-[38px] flex-shrink-0 items-center rounded-full border transition-colors disabled:opacity-50 ${
        checked ? "bg-gradient-accent border-transparent" : "bg-raised border-control"
      }`}
    >
      <span
        className={`inline-block h-4 w-4 rounded-full shadow-sm transition-transform duration-150 ${
          checked ? "translate-x-[18px] bg-accent-fg" : "translate-x-[2px] bg-faint"
        }`}
      />
    </button>
  );
}

export function Checkbox({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <input
      type="checkbox"
      aria-label={label}
      checked={checked}
      disabled={disabled}
      onChange={(e) => onChange(e.target.checked)}
      className="h-4 w-4 rounded accent-[var(--accent)] cursor-pointer disabled:cursor-default"
    />
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: { value: T; label: string; icon?: ReactNode }[];
  onChange: (v: T) => void;
  label: string;
}) {
  // One tab stop for the group; the arrow keys move the choice within it, as
  // they do in a group of native radio buttons.
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step =
      e.key === "ArrowRight" || e.key === "ArrowDown"
        ? 1
        : e.key === "ArrowLeft" || e.key === "ArrowUp"
          ? -1
          : 0;
    if (step === 0 && e.key !== "Home" && e.key !== "End") return;
    e.preventDefault();
    const at = options.findIndex((o) => o.value === value);
    const next =
      e.key === "Home"
        ? 0
        : e.key === "End"
          ? options.length - 1
          : (Math.max(at, 0) + step + options.length) % options.length;
    onChange(options[next].value);
    e.currentTarget.querySelectorAll<HTMLElement>('[role="radio"]')[next]?.focus();
  };
  const chosen = options.some((o) => o.value === value);
  return (
    <div
      role="radiogroup"
      aria-label={label}
      onKeyDown={onKeyDown}
      className="inline-flex items-center gap-0.5 rounded-lg border border-line bg-canvas p-0.5"
    >
      {options.map((o, i) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          tabIndex={value === o.value || (!chosen && i === 0) ? 0 : -1}
          onClick={() => onChange(o.value)}
          className={`inline-flex items-center gap-1.5 h-7 px-2.5 rounded-md text-xs font-medium transition-colors ${
            value === o.value
              ? "bg-surface text-fg shadow-sm border border-line"
              : "text-muted hover:text-fg border border-transparent"
          }`}
        >
          {o.icon}
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Card({
  title,
  description,
  actions,
  children,
  className = "",
  padded = true,
}: {
  title?: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  padded?: boolean;
}) {
  return (
    <section
      className={`rounded-2xl border border-line glass ${padded ? "p-5" : ""} ${className}`}
    >
      {(title || actions) && (
        <div
          className={`flex flex-wrap items-start justify-between gap-x-6 gap-y-3 ${padded ? "mb-4" : "px-5 pt-4 pb-3"}`}
        >
          <div className="min-w-[240px] flex-1">
            {title && <h2 className="text-[16px] font-semibold text-fg">{title}</h2>}
            {description && <p className="text-sm text-muted mt-0.5">{description}</p>}
          </div>
          {actions && <div className="flex items-center gap-2 flex-shrink-0">{actions}</div>}
        </div>
      )}
      {children}
    </section>
  );
}

export function SearchInput({
  value,
  onChange,
  placeholder,
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder: string;
}) {
  return (
    <label className="flex items-center gap-2 h-9 px-3 rounded-lg border border-control bg-canvas/60 focus-within:border-accent min-w-[220px] flex-1">
      <Search aria-hidden className="w-4 h-4 text-faint flex-shrink-0" />
      <input
        type="search"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="flex-1 bg-transparent text-sm outline-none placeholder:text-faint"
        placeholder={placeholder}
        aria-label={placeholder}
      />
    </label>
  );
}

/** A two-column definition list for facts like versions and paths. */
export function Facts({ items }: { items: [string, ReactNode][] }) {
  return (
    <dl className="grid grid-cols-[minmax(120px,max-content)_1fr] gap-x-6 gap-y-2 text-sm">
      {items.map(([k, v]) => (
        <div key={k} className="contents">
          <dt className="text-muted">{k}</dt>
          <dd className="text-fg min-w-0 break-words selectable">{v}</dd>
        </div>
      ))}
    </dl>
  );
}

export function Meter({
  value,
  label,
  tone = "accent",
}: {
  /** 0 to 1. */
  value: number;
  /** What is being measured, for screen readers. */
  label: string;
  tone?: Tone;
}) {
  const pct = Math.max(0, Math.min(100, value * 100));
  const bar: Record<Tone, string> = {
    neutral: "bg-faint",
    accent: "bg-gradient-accent",
    success: "bg-success",
    warning: "bg-warning",
    danger: "bg-danger",
    info: "bg-info",
  };
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(pct)}
      className="h-1.5 w-full rounded-full bg-raised overflow-hidden"
    >
      <div className={`h-full rounded-full ${bar[tone]} transition-[width] duration-500 ease-out`} style={{ width: `${pct}%` }} />
    </div>
  );
}

/**
 * The second step of something that cannot be taken back.
 *
 * It is a bar of its own rather than a new label on the button that was
 * pressed, so a double click never confirms: the place that was clicked stays
 * what it was (disable it while this is open), and the keyboard lands on
 * Cancel. Show it below the thing it is about, and tie it to the selection it
 * was opened for, so that changing the selection closes it.
 */
export function ConfirmBar({
  message,
  confirmLabel,
  onConfirm,
  onCancel,
  cancelLabel = "Cancel",
  className = "",
}: {
  message: ReactNode;
  confirmLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
  cancelLabel?: string;
  className?: string;
}) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  const barRef = useRef<HTMLDivElement>(null);
  const messageId = useId();

  useEffect(() => {
    const before = document.activeElement;
    const bar = barRef.current;
    cancelRef.current?.focus();
    return () => {
      // Hand the focus back, unless it has already gone somewhere else.
      const now = document.activeElement;
      const lost = now === null || now === document.body || (bar?.contains(now) ?? false);
      if (lost && before instanceof HTMLElement) before.focus();
    };
  }, []);

  return (
    <div
      ref={barRef}
      role="alertdialog"
      aria-label="Please confirm"
      aria-describedby={messageId}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onCancel();
        }
      }}
      className={`rounded-lg border border-danger/25 bg-danger/10 px-3.5 py-3 text-sm animate-fade-in ${className}`}
    >
      <div id={messageId} className="flex items-start gap-2.5 break-words text-fg">
        <AlertTriangle aria-hidden className="w-4 h-4 mt-0.5 flex-shrink-0 text-danger" />
        <div className="min-w-0">{message}</div>
      </div>
      <div className="mt-3 flex flex-wrap items-center gap-2">
        <Button ref={cancelRef} size="sm" onClick={onCancel}>
          {cancelLabel}
        </Button>
        <Button size="sm" variant="danger" onClick={onConfirm}>
          {confirmLabel}
        </Button>
      </div>
    </div>
  );
}
