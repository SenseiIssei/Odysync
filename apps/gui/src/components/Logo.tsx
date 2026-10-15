/**
 * The Odysync mark: an orbit with a gap and a satellite — the journey, and
 * the sync — around a check mark: up to date, and safe.
 *
 * Kept in sync by hand with `public/favicon.svg` and
 * `src-tauri/icons/app-icon.svg`, which the platform icons are generated from.
 */
export function Logo({ className = "" }: { className?: string }) {
  return (
    <svg viewBox="0 0 64 64" className={className} role="img" aria-label="Odysync">
      <rect width="64" height="64" rx="16" fill="#5a6ad8" />
      <path
        d="M48.74 29.05 A17 17 0 1 1 42.93 18.98"
        fill="none"
        stroke="#fff"
        strokeWidth="4.5"
        strokeLinecap="round"
      />
      <circle cx="46.72" cy="23.5" r="3.2" fill="#fff" />
      <path
        d="M24 32.5 L29.5 38 L40 26.5"
        fill="none"
        stroke="#fff"
        strokeWidth="4.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
