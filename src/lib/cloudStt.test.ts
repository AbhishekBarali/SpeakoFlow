import { describe, expect, test } from "bun:test";
import type { CloudSttProvider } from "@/bindings";
import {
  AMAZON_TRANSCRIBE_REGIONS,
  cloudSttFixedModel,
  cloudSttStreamsOnly,
  composeAwsCredentials,
  isAmazonTranscribe,
} from "./cloudStt";

const provider = (overrides: Partial<CloudSttProvider>): CloudSttProvider => ({
  id: "x",
  label: "X",
  base_url: "",
  kind: "open_ai_compatible",
  default_model: "m",
  ...overrides,
});

describe("composeAwsCredentials", () => {
  test("joins a key ID and secret, and a session token when there is one", () => {
    expect(
      composeAwsCredentials(
        " AKIAIOSFODNN7EXAMPLE ",
        " wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY ",
      ),
    ).toBe("AKIAIOSFODNN7EXAMPLE:wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
    expect(
      composeAwsCredentials(
        "ASIAIOSFODNN7EXAMPLE",
        "secret+/=",
        "IQoJb3J+/tok==",
      ),
    ).toBe("ASIAIOSFODNN7EXAMPLE:secret+/=:IQoJb3J+/tok==");
  });

  test("refuses parts that cannot be a working key", () => {
    expect(composeAwsCredentials("", "secret")).toBeNull();
    expect(composeAwsCredentials("short", "secret")).toBeNull();
    expect(composeAwsCredentials("AKIA-IOSFODNN7EXAMPLE", "secret")).toBeNull();
    expect(composeAwsCredentials("AKIAIOSFODNN7EXAMPLE", "")).toBeNull();
    expect(composeAwsCredentials("AKIAIOSFODNN7EXAMPLE", "sec ret")).toBeNull();
    expect(composeAwsCredentials("AKIAIOSFODNN7EXAMPLE", "a:b")).toBeNull();
    expect(
      composeAwsCredentials("AKIAIOSFODNN7EXAMPLE", "secret", "to ken"),
    ).toBeNull();
  });
});

describe("provider traits", () => {
  const amazon = provider({
    id: "amazon_transcribe",
    kind: "amazon_transcribe",
    models: ["standard"],
    models_endpoint: null,
  });

  test("Amazon Transcribe streams only and has one fixed model", () => {
    expect(isAmazonTranscribe(amazon)).toBe(true);
    expect(cloudSttStreamsOnly(amazon)).toBe(true);
    expect(cloudSttFixedModel(amazon)).toBe(true);
  });

  test("providers with a choice of models keep the picker", () => {
    const elevenlabs = provider({
      kind: "eleven_labs",
      models: ["scribe_v2", "scribe_v2_realtime"],
    });
    expect(cloudSttFixedModel(elevenlabs)).toBe(false);
    expect(cloudSttStreamsOnly(elevenlabs)).toBe(false);
    // One suggestion, but a live listing: still a choice.
    expect(
      cloudSttFixedModel(
        provider({ models: ["one"], models_endpoint: "/models" }),
      ),
    ).toBe(false);
    expect(cloudSttFixedModel(null)).toBe(false);
  });

  test("the region list has no duplicates and starts at the shipped default", () => {
    const codes = AMAZON_TRANSCRIBE_REGIONS.map((region) => region.code);
    expect(new Set(codes).size).toBe(codes.length);
    expect(codes[0]).toBe("us-east-1");
  });
});
