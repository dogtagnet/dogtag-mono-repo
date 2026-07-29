/**
 * The one list of public contact channels a provider may publish, in listing order.
 *
 * This module imports nothing so every layer - the wire types, the directory seam, the demo presets
 * and both admin register forms - can derive from it without an import cycle. Each of those sites
 * previously restated the five keys, and a channel added to only some of them compiles clean while
 * silently never being sent or never being read. Adding a channel here must therefore fail to
 * compile everywhere that has not been updated, not merely widen a type.
 *
 * The native Kotlin and Swift mirrors cannot import this list; `apps/android/.../ProviderContact`
 * and `apps/ios/DogTag/NearbyDecision.swift` name it as the source they mirror by hand.
 */
export const PROVIDER_CONTACT_CHANNELS = [
  "phone",
  "whatsapp",
  "telegram",
  "email",
  "website",
] as const;

export type ProviderContactChannel = (typeof PROVIDER_CONTACT_CHANNELS)[number];

/** Every channel present, each carrying a published value or the explicit absence of one. */
export type ContactChannelRecord<V> = { [K in ProviderContactChannel]: V };
