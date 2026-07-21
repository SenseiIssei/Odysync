/** Shared presentational building blocks used across every page. */

import { motion } from "framer-motion";
import { AlertTriangle, CheckCircle, RefreshCw } from "lucide-react";
import type { ReactNode } from "react";
import type { Resource } from "../store";

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
    <div className="flex items-start justify-between gap-4">
      <div>
        <h1 className="text-2xl font-bold text-glow-cyan">{title}</h1>
        {subtitle && <p className="text-sm text-cyber-text-dim mt-1">{subtitle}</p>}
      </div>
      {actions && <div className="flex items-center gap-2 flex-shrink-0">{actions}</div>}
    </div>
  );
}

export function RefreshButton({
  onClick,
  spinning,
  label = "Refresh",
}: {
  onClick: () => void;
  spinning?: boolean;
  label?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={spinning}
      className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg border border-cyber-border text-xs text-cyber-text-dim hover:text-accent hover:border-accent/30 disabled:opacity-50 transition-all"
    >
      <RefreshCw className={`w-3.5 h-3.5 ${spinning ? "animate-spin" : ""}`} />
      {label}
    </button>
  );
}

export function ErrorBar({ message }: { message: string }) {
  return (
    <div className="flex items-start gap-2 text-xs text-danger bg-danger/5 rounded-lg px-4 py-3 border border-danger/30">
      <AlertTriangle className="w-4 h-4 flex-shrink-0 mt-px" />
      <span className="break-words">{message}</span>
    </div>
  );
}

export function SuccessBar({ message }: { message: string }) {
  return (
    <div className="flex items-start gap-2 text-xs text-success bg-success/5 rounded-lg px-4 py-3 border border-success/30">
      <CheckCircle className="w-4 h-4 flex-shrink-0 mt-px" />
      <span className="break-words">{message}</span>
    </div>
  );
}

export function EmptyState({
  icon,
  title,
  hint,
}: {
  icon: ReactNode;
  title: string;
  hint?: string;
}) {
  return (
    <div className="text-center py-12">
      <div className="mx-auto mb-3 w-12 h-12 flex items-center justify-center text-cyber-text-faint">
        {icon}
      </div>
      <p className="text-sm text-cyber-text-dim">{title}</p>
      {hint && <p className="text-xs text-cyber-text-faint mt-1">{hint}</p>}
    </div>
  );
}

export function LoadingState({ label }: { label: string }) {
  return (
    <div className="flex items-center justify-center gap-2 py-12 text-sm text-cyber-text-dim">
      <RefreshCw className="w-4 h-4 animate-spin text-accent" />
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
  description?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <div className="flex items-center justify-between gap-4">
      <div>
        <div className="text-sm font-medium">{label}</div>
        {description && <div className="text-xs text-cyber-text-dim">{description}</div>}
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        aria-label={label}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={`relative w-11 h-6 rounded-full transition-colors flex-shrink-0 disabled:opacity-50 ${
          checked ? "bg-accent/30" : "bg-cyber-bg border border-cyber-border"
        }`}
      >
        <motion.span
          animate={{ x: checked ? 22 : 2 }}
          transition={{ type: "spring", stiffness: 500, damping: 30 }}
          className="absolute top-0.5 left-0 w-5 h-5 rounded-full"
          style={{
            backgroundColor: checked ? "var(--accent-cyan)" : "var(--cyber-text-faint)",
          }}
        />
      </button>
    </div>
  );
}

export function Card({
  title,
  children,
  className = "",
}: {
  title?: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={`rounded-lg border border-cyber-border bg-cyber-surface p-4 ${className}`}
    >
      {title && <h3 className="font-bold text-sm text-accent mb-3">{title}</h3>}
      {children}
    </div>
  );
}

export function StatCard({
  icon,
  value,
  label,
  color = "text-accent",
  glow = "",
  delay = 0,
}: {
  icon: ReactNode;
  value: string;
  label: string;
  color?: string;
  glow?: string;
  delay?: number;
}) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ delay }}
      className={`relative p-4 rounded-xl bg-cyber-surface border border-cyber-border ${glow}`}
    >
      <div className={`${color} mb-2`}>{icon}</div>
      <div className="text-xl font-bold truncate" title={value}>
        {value}
      </div>
      <div className="text-xs text-cyber-text-dim mt-0.5">{label}</div>
    </motion.div>
  );
}

export function formatSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "-";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

/** Dates coming out of PowerShell/WMI are not always parseable — never throw. */
export function formatDate(value: string): string {
  if (!value) return "-";
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value;
  return d.toLocaleString();
}

export function errorText(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}
