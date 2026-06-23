// Omnibox shape-routing (D2). One omnibox routes by input shape: URL-like input
// navigates the active tab; anything else becomes a chat turn in the rail.
// Acceptance #2: "example.com" navigates; "what is example.com" opens a rail turn.

export type OmniboxRoute =
  | { kind: "navigate"; url: string }
  | { kind: "chat"; text: string };

const SCHEME_RE = /^[a-z][a-z0-9+.-]*:\/\//i;
// A single token that looks like a host: label(.label)+ optional :port and path,
// with a known-ish TLD shape. Whitespace anywhere => treat as a chat turn.
const HOSTLIKE_RE =
  /^[a-z0-9-]+(\.[a-z0-9-]+)+(:\d+)?(\/\S*)?$/i;
const LOCALHOST_RE = /^localhost(:\d+)?(\/\S*)?$/i;

/** Normalize a host-like or scheme-prefixed string into a navigable URL. */
function toUrl(input: string): string {
  if (SCHEME_RE.test(input)) return input;
  return `https://${input}`;
}

/**
 * Decide whether omnibox input should navigate or open a chat turn.
 *
 * Rules, in order:
 * - Explicit scheme (http://, https://, file://, about:) => navigate.
 * - Contains internal whitespace => chat (e.g. "what is example.com").
 * - Single token shaped like host(.tld)(/path) or localhost => navigate.
 * - Otherwise => chat.
 */
export function routeOmnibox(raw: string): OmniboxRoute {
  const input = raw.trim();
  if (input.length === 0) return { kind: "chat", text: "" };

  if (SCHEME_RE.test(input) || /^about:/i.test(input)) {
    return { kind: "navigate", url: input };
  }

  if (/\s/.test(input)) {
    return { kind: "chat", text: input };
  }

  if (LOCALHOST_RE.test(input) || HOSTLIKE_RE.test(input)) {
    return { kind: "navigate", url: toUrl(input) };
  }

  return { kind: "chat", text: input };
}

/** Extract a registrable-ish domain from a URL for known-context recall keys. */
export function domainOf(url: string): string | null {
  try {
    const u = new URL(url);
    return u.hostname || null;
  } catch {
    return null;
  }
}
