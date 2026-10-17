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

/**
 * Amazon Transcribe, whose key, endpoint and streaming rows all differ: three
 * credential fields instead of one key, a region instead of a URL, and live
 * streaming that cannot be switched off.
 */
export const isAmazonTranscribe = (
  provider: CloudSttProvider | null | undefined,
): boolean => provider?.kind === "amazon_transcribe";

/**
 * True when live streaming is the provider's only way to transcribe
 * (`stt_cloud_stream::streams_only`), so "Transcribe as I speak" is always on.
 */
export const cloudSttStreamsOnly = (
  provider: CloudSttProvider | null | undefined,
): boolean => provider?.kind === "amazon_transcribe";

/**
 * A provider with one model and no listing has nothing to choose, so the
 * model picker is not shown for it.
 */
export const cloudSttFixedModel = (
  provider: CloudSttProvider | null | undefined,
): boolean =>
  !!provider && !provider.models_endpoint && provider.models?.length === 1;

/**
 * Regions where Amazon Transcribe streaming runs, with AWS's own names. Mirrors
 * `stt_aws::STREAMING_REGIONS`.
 */
export const AMAZON_TRANSCRIBE_REGIONS: ReadonlyArray<{
  code: string;
  name: string;
}> = [
  { code: "us-east-1", name: "US East (N. Virginia)" },
  { code: "us-east-2", name: "US East (Ohio)" },
  { code: "us-west-2", name: "US West (Oregon)" },
  { code: "ca-central-1", name: "Canada (Central)" },
  { code: "mx-central-1", name: "Mexico (Central)" },
  { code: "sa-east-1", name: "South America (São Paulo)" },
  { code: "eu-west-1", name: "Europe (Ireland)" },
  { code: "eu-west-2", name: "Europe (London)" },
  { code: "eu-central-1", name: "Europe (Frankfurt)" },
  { code: "eu-central-2", name: "Europe (Zurich)" },
  { code: "af-south-1", name: "Africa (Cape Town)" },
  { code: "ap-south-1", name: "Asia Pacific (Mumbai)" },
  { code: "ap-northeast-1", name: "Asia Pacific (Tokyo)" },
  { code: "ap-northeast-2", name: "Asia Pacific (Seoul)" },
  { code: "ap-southeast-1", name: "Asia Pacific (Singapore)" },
  { code: "ap-southeast-2", name: "Asia Pacific (Sydney)" },
  { code: "ap-southeast-5", name: "Asia Pacific (Malaysia)" },
  { code: "ap-southeast-7", name: "Asia Pacific (Thailand)" },
  { code: "us-gov-east-1", name: "AWS GovCloud (US-East)" },
  { code: "us-gov-west-1", name: "AWS GovCloud (US-West)" },
];

/**
 * Compose Amazon Transcribe's credentials into the one secret the backend
 * stores and parses (`stt_aws::AwsCredentials::parse`): `KEY_ID:SECRET`, or
 * `KEY_ID:SECRET:SESSION_TOKEN` for temporary credentials. A colon cannot
 * occur in any part of a real key, which is what makes it a safe separator.
 *
 * `null` when the parts cannot form a working key, so Save stays disabled
 * rather than storing something AWS would refuse on every dictation.
 */
export const composeAwsCredentials = (
  keyId: string,
  secret: string,
  sessionToken = "",
): string | null => {
  const id = keyId.trim();
  const key = secret.trim();
  const token = sessionToken.trim();
  if (!/^[A-Za-z0-9]{16,128}$/.test(id)) return null;
  if (!key || /[\s:]/.test(key)) return null;
  if (/[\s:]/.test(token)) return null;
  return token ? `${id}:${key}:${token}` : `${id}:${key}`;
};
