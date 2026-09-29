import type { CloudSttProvider } from "@/bindings";

/**
 * What a cloud speech-to-text provider still needs before it can run.
 *
 * Shared by Settings, the Models page summary, and the model picker so the
 * three cannot disagree about whether a provider is usable. Two rules, both of
 * which used to be one: "has an editable URL" was read as "needs no key",
 * which stopped being true with Azure, whose endpoint is the user's own
 * resource *and* needs a key.
 */

/** True when a request can go out without an API key (a local server). */
export const cloudSttKeyOptional = (
  provider: CloudSttProvider | null | undefined,
): boolean => !!provider?.key_optional;

/**
 * True when the provider ships no endpoint of its own and the user has not
 * entered one yet — Azure, until the resource URL is pasted in.
 */
export const cloudSttNeedsEndpoint = (
  provider: CloudSttProvider | null | undefined,
  endpointOverride: string | null | undefined,
): boolean =>
  !!provider?.allow_base_url_edit &&
  !provider.base_url?.trim() &&
  !endpointOverride?.trim();
