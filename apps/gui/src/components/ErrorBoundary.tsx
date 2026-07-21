import { Component, type ErrorInfo, type ReactNode } from "react";
import { reportFrontendError } from "../api";

interface Props {
  /** Shown in the message so the user knows what failed. */
  label: string;
  children: ReactNode;
}

interface State {
  error: Error | null;
  stack: string | null;
}

/**
 * Stops one broken page from blanking the whole window.
 *
 * In a Tauri app an uncaught render error produces a white void with no
 * devtools and no way back, so every page is wrapped individually: the rest of
 * the app keeps working and the failure is reported where it happened.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, stack: null };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(`[odysync] ${this.props.label} crashed:`, error, info);
    this.setState({ stack: info.componentStack ?? null });
    // Persist it: a release webview has no console the user can open.
    void reportFrontendError(
      this.props.label,
      error.stack ?? error.message,
      info.componentStack ?? undefined,
    );
  }

  private reset = () => this.setState({ error: null, stack: null });

  render() {
    const { error, stack } = this.state;
    if (!error) return this.props.children;

    return (
      <div className="max-w-2xl mx-auto space-y-4">
        <h1 className="text-2xl font-bold text-danger">{this.props.label} failed to render</h1>
        <p className="text-sm text-cyber-text-dim">
          This is a bug in Odysync. The rest of the app is still usable.
        </p>
        <pre className="text-xs whitespace-pre-wrap break-words font-mono bg-cyber-surface border border-danger/30 rounded-lg p-4 max-h-64 overflow-y-auto select-text text-danger">
          {error.message}
          {stack}
        </pre>
        <button
          type="button"
          onClick={this.reset}
          className="px-4 py-2 rounded-lg bg-accent/10 border border-accent/30 text-accent text-sm font-medium hover:bg-accent/20 transition-all"
        >
          Try again
        </button>
      </div>
    );
  }
}
