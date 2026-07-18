import type { ProviderType } from "./types";
import { providerMeta } from "./format";

// Per-provider feature gating. Some things the dashboard can do aren't backed by every provider's
// API. Rather than hide them (which makes the UI feel different per provider), we keep the feature
// visible but **greyed out**, and clicking it pops the standard "{Provider} currently does not
// support this feature" message. This is the default treatment going forward: to toggle a feature
// off for a provider, add it to UNSUPPORTED — the UI stays the same shape, just disabled.

/** Features that may be unavailable on some providers. Extend as we gate more. */
export type ProviderFeature = "checks";

/** The features each provider's API can't back. Default is supported — only list exceptions.
 *  (Empty today — every provider reports checks; the mechanism stays for the next gated feature.) */
const UNSUPPORTED: Partial<Record<ProviderType, ProviderFeature[]>> = {};

export function providerSupports(provider: ProviderType, feature: ProviderFeature): boolean {
  return !(UNSUPPORTED[provider]?.includes(feature) ?? false);
}

/** The standard message shown when a provider doesn't support a gated feature. */
export function unsupportedMessage(provider: ProviderType): string {
  return `${providerMeta(provider).label} currently does not support this feature`;
}
