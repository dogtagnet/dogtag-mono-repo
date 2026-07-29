# MAP_RESEARCH - location search and map providers (decision record)

**Status: DECLINED 2026-07-29. No in-app map, no location autocomplete, no paid third-party location provider.**

> **Decision.** dogtag ships no embedded map and no location autocomplete, and pays no location vendor.
> Nearby stays what it is today: a list, searched by provider name on-device, with distance computed on-device from a position the owner already holds.
> The research below is kept so that revisiting this costs a reading rather than a second round of vendor research.

**Audience:** anyone about to add a map view, a place-search field, an address geocoder, or a location vendor key to any dogtag surface.

**Why this document exists:** the survey was done, priced and read end to end, and then the feature was cancelled to avoid paying for anything for now.
Without the record, a later revisit re-runs the same work and re-learns the same two facts that decide it - the cost multiple, and Google's *No Use With Non-Google Maps* clause, both in §2.2.
The record also names the vendor a revisit should start from, so the revisit is one evaluation rather than three.

**Revisit trigger:** Nearby actually reaching roughly 10,000 searches/day.
Below that the money is small enough on every non-Google option that optimising it is not worth an afternoon, and the reason to decline is that nothing needs paying for at all.

Related: [`AGENTS.md`](../AGENTS.md) "Distance is computed on the device", "Provider directory reads are explicit", "Mobile Nearby is list-first" - the built feature this document declines to extend.

---

## 1. What is built today, and why it is not a gap

Read this section before concluding the file describes something missing.
Nearby is a shipped, complete feature.
What it does not have is an embedded map and a hosted place-search field, both deliberately.

| Capability | How it works today | Third party |
|---|---|---|
| Find a provider by name | On-device substring match over the already-fetched provider set (`apps/ios/DogTag/NearbyDecision.swift:344`, Kotlin mirror in `apps/android/.../nearby/NearbyDecision.kt`) | none |
| Filter to the right kind of provider | `vet` or `groomer`, and `active != false`, applied before proximity or contact search (`NearbyDecision.swift:369-370`) | none |
| Distance and ordering | `packages/ui/src/geo/` - haversine, bearing, display formatting, sorting, all pure arithmetic over a position the caller already holds | none |
| The owner's position | Coarse device fix after an explicit tap, or decimal coordinates typed by hand and parsed on-device | none |
| Getting there | The row hands the public destination to the platform's maps app after a deliberate tap; the origin is never included in the handoff | the OS's maps app, no key, no cost |

So there is already a map in the product: it is the operating system's, it is reached by an explicit tap, and it costs nothing.
What was declined is an *embedded* map inside dogtag and a *hosted* autocomplete field.

Three properties of the current shape are load-bearing and would be traded away by any hosted-autocomplete integration:

- `packages/ui/src/geo/` performs no I/O, and its header forbids turning a position into a query parameter, a request path, a network cache key, or a log line.
- `ProviderDirectory.read()` deliberately takes no query, so a position has nowhere to go even by accident.
- The manual-entry copy on both platforms promises exactly this, verbatim: *"They are parsed on this phone; DogTag does not geocode or send them anywhere."* (`apps/ios/DogTag/NearbyScreen.swift:361`, with the Kotlin equivalent at `apps/android/.../ui/screens/NearbyScreen.kt:460`.)

That last one is a claim, not a caption.
See §5.

---

## 2. What it would cost to change

Everything in this section is conditional on the revisit trigger above.
It is the pre-computed answer to "if we did this, what and how much", not a recommendation to do it now.

### 2.1 The recommendation, if revisited: one vendor, and it is not Google

**Stadia Maps**, which at Nearby's present volume would be **$20/month**, rising to about **$80/month** at a million searches a month.
Nothing is subscribed to today, and this figure is what a subscription would cost, not what is being spent.

One vendor covers all three capabilities (autocomplete, geocoding, vector basemap) on all three targets (web, iOS, Android), with first-party autocomplete SDKs published as live public repositories rather than marketing links: `maplibre-search-box` (TypeScript), `swiftui-autocomplete-search` (Swift), `jetpack-compose-autocomplete-search` (Kotlin), all retrieved 2026-07-29.
Its autocomplete is a geocoder, so a suggestion carries coordinates inline and no second call is needed to resolve the chosen place.
Its basemaps render through MapLibre, which has native iOS and Android SDKs.

One key, one account, three capabilities, three platforms.

### 2.2 Why not Google, in the order the reasons should be weighed

**(a) Cost decides it on its own.**
At 10,000 searches/day Google is **$3,433 to $4,335/month** and Stadia is **$26/month**.
At a million searches/month Google is **$8,113 to $12,155** and Stadia is **$80**.
Dividing those figures gives a multiple of roughly **100x to 165x** for the same three capabilities, depending on scale and on which Google call pattern is used.

The Google figure is deliberately a range rather than a single number - see §3.1 for why, and do not quote a point estimate from it.

**(b) The obvious cheap hybrid is prohibited by Google's terms, before cost is even considered.**
Google Maps Platform Terms §3.2.3(e), verbatim as shipped:

> "**No Use With Non-Google Maps.** To avoid quality issues and/or brand confusion, Customer will not use the Google Maps Core Services **with or near a non-Google Map** in a Customer Application. For example, Customer will not (i) display or use Places content on a non-Google Map, (ii) display Street View imagery and non-Google Maps on the same screen, or (iii) link a Google Map to non-Google Maps Content or a non-Google Map."

"With or near" is broad on purpose.
So "Google's autocomplete because it is the best dropdown, MapLibre because the map is free" is not a bargain, it is a terms breach.
Taking Google's autocomplete is also choosing Google's map, and on web that map costs $7 per 1,000 loads.

There is one compliant shape worth knowing: Service Specific Terms §14.1 permits using Places content *"without a corresponding Google Map"* entirely.
A list-only Nearby with Google autocomplete and no map at all is allowed, with Google attribution.
It is only a *non-Google* map that is forbidden.

**(c) Billing must be enabled regardless of volume.**
Places API documentation, verbatim: *"To use the Places API, you must enable billing on each of your projects and include an API key or OAuth token with all API or SDK requests."*
There is no keyless or card-free tier.
The old $200/month credit was **withdrawn on 1 March 2025** and replaced by per-SKU free call caps (10,000/month for Essentials-tier SKUs, 5,000 for Pro, 1,000 for Enterprise).
Overrun does not degrade or hard-fail, it bills, unless a daily quota cap is set separately in the Cloud console.

**(d) A clause that may or may not point at directory products - a flag, not a reason.**
Google Maps Platform Terms §3.2.3(d), *No Re-Creating Google Products or Features*, verbatim:

> "Customer will not use the Services to create a product or service with features that are substantially similar to or that re-create the features of another Google product or service. **Customer's product or service must contain substantial, independent value and features beyond the Google products or services.** […] For example, Customer will not: […] **(iii) use the Google Maps Core Services in a listings or directory service** or to create or augment an advertising product"

Read narrowly, (iii) names the product category dogtag's provider directory is.
Read in light of its own chapeau, a credential-issuer directory with on-chain provenance plainly has substantial independent value beyond Google's products, and (iii) is aimed at directories that are *substitutes* for Google's own.
The second reading is probably the better one, and neither reading is confident.
Plenty of directory-shaped apps run on Google Maps.

**This clause is flagged, not relied on, and it must not be resolved from this document.**
It is the sentence to put in front of counsel or a Google representative *if* Google is chosen anyway despite (a) and (b).
Nothing in this record's recommendation rests on it.

### 2.3 The options that cost nothing, recorded rather than advocated

These exist and were priced.
They are here so a revisit knows what the $0 shapes are, not as a suggestion to build one now.

- **Apple `MKLocalSearchCompleter` on iOS.**
  No API key, no rate limit on the completer per Apple's documentation, and no cost beyond the Apple Developer Program membership dogtag already pays to ship.
  It has **no Android path at all**, so Apple can only ever be one half of a mixed set.
- **MapLibre with OpenFreeMap or Protomaps tiles.**
  BSD-licensed renderer, no key, no vendor.
  OpenFreeMap states *"no limits on the number of map views or requests. There's no registration, no user database, no API keys"*, and Protomaps is a single `.pmtiles` archive over HTTP range requests with no per-request billing at all.
  Both need OSM attribution, and neither does geocoding, so this is the map half only.
- **Geoapify free tier.**
  3,000 credits/day, commercial use explicitly allowed with a "Powered by Geoapify" attribution, no card.
  Their Terms and Conditions were read and are **silent on caching, on storing results, and on proxying** - a checked absence rather than an unchecked question, and the reason Geoapify sits behind Stadia despite being cheaper on day one.
  Those two questions would have to be asked before building on it.
- **Self-hosted Photon.**
  The only configuration where no third party sees the partial search text at all.
  Apache-2.0 software over ODbL OpenStreetMap data, both free, so the cost is a server rather than a subscription (§3.4).

### 2.4 The privacy property, stated precisely

"No third party sees what the owner types" is a property of the **self-hosted** branch only.

It is **not** a property of the Stadia branch.
Stadia sees both the search text and the end user's IP, and unlike Google that IP cannot be hidden behind a dogtag backend, because Stadia's Terms of Service §8 forbids proxying (§3.2).
If the property matters, it is Photon or nothing.

| | Self-hosted Photon | Stadia Maps | Google |
|---|---|---|---|
| Sees partial keystrokes | nobody | Stadia | Google |
| Sees the end user's IP | nobody, it is our own server | Stadia, unavoidably | Google, or us if we relay, which is permitted |
| Money at 10,000 searches/day | one server | $26/month | $3,433 to $4,335/month |
| Place coverage | OSM only | OSM plus Stadia's POI layer | best available |
| Operations burden | ours | none | none |

---

## 3. Evidence

All figures below were retrieved **2026-07-29** from the vendors' own published pages.
Prices change.
A figure quoted from this file after that date is a historical figure and should be re-checked before money is committed against it.

Vendor pricing and terms pages were fetched directly and, where JS-rendered, read either by stripping the raw HTML or in an isolated browser session with the page URL asserted before every read.
Google's Terms of Service and Service Specific Terms were downloaded whole and searched, so the clauses quoted in §2.2 are verbatim from the shipped documents rather than from a search-result summary.
Cost arithmetic was computed from the published band tables by script.

### 3.0 Comparison table

The assumption that drives every figure: **a "search" is one user finding one place, costing 4 autocomplete requests** (roughly 12 typed characters, debounced to 4 network calls).
Scales are 100/day = 3,000/month, 10,000/day = 300,000/month, and 1,000,000/month.
Google is the only provider where sensitivity to that assumption changes the answer materially (§3.1).

| Provider | Key? | Card? | Billing unit | 3,000/mo | 300,000/mo | 1,000,000/mo | iOS | Android | Web |
|---|---|---|---|---|---|---|---|---|---|
| **Google Places** | yes | **required** | per request unless a session token is used *and* correctly terminated | $5.66 | **$3,432.70** | **$8,112.70** | yes | yes | yes |
| **Google map** | yes | required | mobile **$0 unlimited**; web $7/1k loads | $0 | $1,750 (web) | $4,970 (web) | free | free | paid |
| **Stadia Maps** | yes, or domain allowlist | yes (free tier forbids commercial use) | per request, 1 credit | **$20** | **$26** | **$80** | SwiftUI SDK | Compose SDK | MapLibre SDK |
| **Apple MapKit (native)** | **no key** | no (ADP $99/yr, already required) | free, no documented completer rate limit | **$0** | **$0** | **$0** | yes | **none** | no |
| **Apple MapKit JS** | yes (JWT via .p8) | no (ADP $99/yr) | 25,000 service calls/day + 250,000 map views/day free | $0 | **over cap** | **over cap** | - | no | yes |
| **Geoapify** | yes | **no card on free tier** | per request, 1 credit, **daily** cap | **$0** | $179 | $609 | yes | yes | yes |
| **MapTiler** | yes | yes (free forbids commercial use) | per session | $30 | $772.50 | $2,522.50 | yes | yes | yes |
| **Mapbox** | yes | yes | per session, every completed session bills including abandoned | $7.50 | ~$898.50 † | ~$2,998.50 † | yes | yes | yes |
| **LocationIQ** | yes | free tier ok with attribution, **2 req/s** | per request | $0 ‡ | $200 | $500 | yes | yes | yes |
| **Radar** | yes | **no published price at all** | unpublished | ? | ? | ? | yes | yes | yes |
| **Nominatim (public)** | no | no | free | **prohibited** | prohibited | prohibited | - | - | - |
| **Photon (self-host)** | no | no | our server | 1 server | 1 server | 1 server | yes | yes | yes |
| **MapLibre + OpenFreeMap / Protomaps** | **no key** | no | free, tiles only, no geocoding | $0 | $0 | $0 | yes | yes | yes |

Google Places figures are the **cheaper** of its two viable call patterns.
The alternative pattern costs $0 / $4,335 / $12,155 at the same three scales, cheaper at the smallest scale and dearer at the other two.
Both are derived in §3.1; neither changes the conclusion.

† Mapbox: the 501 to 100k band is $3.00 per 1,000 sessions; **the 100k+ and 500k+ band prices did not render and are not confirmed**, so the two larger figures are upper bounds and the real cost is lower.

‡ LocationIQ's free tier is 5,000 requests/day but rate-limited to **2 requests per second**, which real type-ahead will hit with a handful of concurrent users.
Treat that "$0" as development-only.

### 3.1 Google - the billing unit, which is the order-of-magnitude question

The SKU table (`developers.google.com/maps/billing-and-pricing/pricing`) gives, verbatim from the rendered table:

```
 Autocomplete Requests
 4EF4-B17C-B31A |   10,000 |  $2.83 | $2.27 | $1.70 | $0.85 | $0.21
 Autocomplete Session Usage
 EEA3-417B-DBA1 |  Unlimited |  - | - | - | - | -
 Geocoding
 BAC8-4E68-E261 |   10,000 |  $5.00 | $4.00 | $3.00 | $1.50 | $0.38
 Places API Place Details Essentials
 6E05-E1C3-8D85 |   10,000 |  $5.00 | $4.00 | $3.00 | $1.50 | $0.38
 Places API Place Details Essentials (IDs Only)
 5C36-E272-E88F |  Unlimited |  - | - | - | - | -
 Dynamic Maps
 FAF4-3B2D-51B2 |   10,000 |  $7.00 | $5.60 | $4.20 | $2.10 | $0.53
 Maps SDK
 6DE1-4D9C-5B67 |  Unlimited |  - | - | - | - | -
```

Bands are 0-100k / 100k-500k / 500k-1M / 1M-5M / 5M+ on cumulative monthly events.

**What triggers a charge.**
From the SKU details page, the `Autocomplete Requests` SKU's billable event is verbatim *"Request without a session token, or with an invalid or expired token"*, and it triggers when the request carries no session token, when it carries one but the session is abandoned, or when it carries one but the session is terminated under certain conditions.

The `Autocomplete Session Usage` SKU's billable event is *"Request with a session token"*, and the decisive rule is verbatim:

> "If the session ends with a **Place Details Essentials** request, the **first 12 Autocomplete requests** are billed at SKU: Autocomplete Requests. All subsequent Autocomplete requests are billed at SKU: Autocomplete Session Usage."

Session-pricing documentation adds that terminating with **Place Details Pro/Enterprise** makes *all* autocomplete requests free, that terminating with **IDs Only** reverts everything to per-request, and that an **abandoned** session reverts to per-request.

So there are three call patterns and the cheapest depends on volume:

- **Pattern A** - session token terminated by **Place Details Essentials**, the tier that contains `location`, which is the coordinate actually needed.
  At 4 requests per search all 4 fall inside the "first 12" and bill.
- **Pattern C** - session token terminated by **Place Details Pro** at $17 per 1,000, which makes autocomplete free.
- Pattern B, no token at all, is arithmetically identical to A below 12 requests per search.

Computed against the bands above at 4 autocomplete requests per search:

| | Pattern A (Essentials) | Pattern C (Pro) |
|---|---|---|
| 3,000 searches/month | 12,000 AC events, 2,000 billable = **$5.66**; details 3,000 under the 10k cap = $0 → **$5.66** | details 3,000 under the Pro 5k cap → **$0.00** |
| 300,000/month | AC $2,182.70 + details $1,250.00 → **$3,432.70** | **$4,335.00** |
| 1,000,000/month | AC $4,562.70 + details $3,550.00 → **$8,112.70** | **$12,155.00** |

**Break-even between the two patterns is at roughly 4.2 autocomplete requests per search**, which is uncomfortably close to the 4-request assumption.
This is why Google's cost is stated as a range - "somewhere between $3.4k and $4.3k per month at 10,000 searches/day" - and not as a single figure.
Either way it is three orders of magnitude above Stadia, so the imprecision does not touch the conclusion.

**The map is the one genuinely good piece of Google news.**
SKU `Maps SDK` (6DE1-4D9C-5B67) covers *Maps SDK for Android* and *Maps SDK for iOS*, billable event "Map load", free cap **Unlimited**, no price in any band.
So a Google map inside the native apps is free.
One caveat that is easy to trip: the trigger text is *"`GMSMapView` object **not loaded with a map ID**"*, so a cloud-styled Map ID bills to `Dynamic Maps` at $7 per 1,000 instead.
Web maps are always `Dynamic Maps`.

**Caching, as a cost lever, is 30 days at best.**
Umbrella Terms §3.2.3(b): *"Customer will not cache Google Maps Content except as expressly permitted under the Maps Service Specific Terms."*
Service Specific Terms §14.3 (Places API): *"Customer may temporarily cache latitude and longitude values from the Places API for up to 30 consecutive calendar days, after which Customer must delete the cached latitude and longitude values."*
§3 permits caching `place_id` indefinitely, but a place ID alone does not save a call.
Note the asymmetry: the **Geocoding API** §6.3.2 additionally permits indefinite caching of lat/lng and formatted address *"solely to support the direct, End User facing functionality of the Customer Application that initiated the request"*, logically isolated per end user, and the **Places API has no equivalent clause**.

**Proxying.**
No clause prohibiting it was found, and Google publishes server-side REST endpoints plus IP-restricted server keys, so relaying through a dogtag backend to hide the end user's IP is architecturally supported.
State that as "not prohibited and structurally supported" rather than "expressly permitted" - no affirmative permission was found.

**What Google would learn.**
Every keystroke batch, the IP unless relayed, and - the part the billing mechanic makes structural - a **session token that exists specifically to bundle those keystrokes together**.
The token is what makes autocomplete cheap and it is also what makes the keystrokes explicitly linkable to each other and to the chosen place, by design rather than by inference.

### 3.2 Stadia Maps

- **Autocomplete Search v2 costs 1 credit per request** (down from 20 in v1), built on Pelias, available on all plans.
  Because it is a geocoder, the suggestion carries coordinates, so resolving the chosen place needs no second call.
- Pricing: **Free** 200,000 credits/month with *"Commercial use not allowed"* and no card; **Starter $20**/month for 1,000,000 credits with 3 cents per 1,000 overage; **Standard $80**/month for 7,500,000; **Professional $250**/month for 25,000,000.
  Basemap tiles are 1 credit per tile.
- Computed at 4 requests per search: 3,000 searches is 12,000 credits, so **Starter $20**; 300,000 is 1,200,000 credits, Starter plus 200,000 overage, so **$26**; 1,000,000 is 4,000,000 credits, so **Standard $80**, cheaper than Starter plus overage at $110.
- With a map view added, tiles dominate: 300,000 map views at roughly 15 tiles each is 4.5M credits, plus 1.2M for search, so 5.7M, still inside **Standard $80/month**.
  At 1M map views it is roughly 19M credits, so **Professional $250/month**, which is still a twentieth of Google's web-map line alone.
- Auth is an API key **or browser domain allowlisting** with no key in the client: *"Domain-based authentication is the easiest form of authentication for production web apps. No additional application code is required, and you don't need to worry about anyone scraping your API keys."*
- **Terms of Service §8 is the catch.**
  It prohibits *"proxying or caching access to our Services in any way, except for"* limited offline mobile caching (100 MB or less) and client-side caching, and states that *"server-side caching is prohibited"*.
  It also forbids *"permanently storing results for future use (e.g., as a database column) […] from the Stadia Maps Geocoding APIs without an active Standard, Professional, or Enterprise subscription"*.
  So the end user's IP reaches Stadia and cannot be hidden, and permanent storage needs the $80 tier.

### 3.3 Apple

- **Native iOS:** `MKLocalSearchCompleter` needs **no API key** and, per Apple's documentation, has no rate limit - *"you can update the queryFragment property as often as you want and there is no need to throttle the requests yourself"*, unlike `MKLocalSearch`, which does throttle and raises `MKError.loadingThrottled`.
  There is no cost beyond Apple Developer Program membership at $99/year, which dogtag already pays to ship on the App Store, making this free at the margin.
- **MapKit JS (web):** free daily limits are 250,000 map views and 25,000 service calls **per Apple Developer Program membership**, with the same 25,000 quota shared with Apple Maps Server APIs.
  Auth is a JWT signed with a `.p8` private key and a Maps identifier, refreshed every 30 minutes.
  Overage is *"contact us"*, and the free ceiling works out to roughly 6,250 searches/day at 4 requests each.
- **Android: nothing.** No SDK, no web-service path worth using.

### 3.4 Photon, the self-host branch

- Apache-2.0 software over ODbL OpenStreetMap data, purpose-built for type-ahead, which is exactly the gap Nominatim leaves (§3.5).
- Hardware, verbatim from the README: *"A planet-wide database requires about 95GB disk space (as of 2026, grows by about 10% a year). Using SSDs for storage is strongly recommended, NVME would even be better. At least 64GB RAM are recommended for smooth operations, more, if the server takes significant load."*
  Photon 1.0.0 roughly halved the database size versus 0.7.
- GraphHopper publishes weekly-updated dumps *"for the world-wide dataset and for selected country datasets"*, so a country-scoped deployment is materially smaller than the planet figures suggest.
- The public demo instance is **not** a production option: *"Extensive usage will be throttled or completely banned […] If you have a larger number of requests to make, please consider setting up your own private instance."*
- **What is lost:** OSM place coverage only, no vendor SLA, and the operations burden of a 64 GB box plus periodic reindexing.

**The licence, checked rather than waved through**, because "the cache is ours" is exactly the framing that invites a share-alike problem.
ODbL's share-alike attaches only to publicly using a **Derivative Database**.
§4.5(b) is explicit that *"Using this Database […] to create a Produced Work does not create a Derivative Database for purposes of Section 4.4"*, and a Produced Work needs **attribution only**.
Resolving typed text to a coordinate, showing it to the owner, and using it as the origin of an on-device radius filter is a Produced Work at every step; nothing is written back into a database dogtag publishes.
So the self-host branch is attribution-only, and so are the hosted OSM-derived vendors (Stadia, Geoapify, LocationIQ, MapTiler, OpenFreeMap, Protomaps), each of which states its own attribution string.

**The case that would be different, and the line not to cross casually:** if the provider directory ever backfilled provider coordinates *from OSM* into dogtag's own store, that is extracting a substantial part of the Contents into a new Database and the share-alike question becomes live.
Provider coordinates today are supplied by the providers themselves at registration (`packages/ui/src/directory/registration.ts`), so that case does not exist.

### 3.5 Nominatim - one line, because it is disqualified

The OSM Foundation's Nominatim Usage Policy is explicit, verbatim: *"**Auto-complete search** This is not yet supported by Nominatim and you must not implement such a service on the client side using the API."*
Plus an *"absolute maximum of 1 request per second"*.
The free public geocoder everyone reaches for first cannot legitimately serve autocomplete at all.
For one-shot geocoding of a submitted string it is usable within the rate limit, with attribution and mandatory client-side caching.
This is why the free branch runs through Photon rather than Nominatim.

### 3.6 The map, separately - it is nearly free from anyone but Google-on-web

- **MapLibre GL JS** is BSD-3-Clause, forked from Mapbox GL JS 1.13 before Mapbox's December 2020 licence change, with native iOS and Android SDKs, and needs no key and no vendor.
- **OpenFreeMap** serves tiles with, verbatim, *"no limits on the number of map views or requests. There's no registration, no user database, no API keys"*, MIT-licensed, commercial use allowed, attribution *"OpenFreeMap © OpenMapTiles Data from OpenStreetMap"*.
- **Protomaps** is a single `.pmtiles` archive served over HTTP range requests from object storage or static hosting, with no per-request vendor billing at all.
  The planet basemap is *"roughly 120 gigabytes"* at z0-15, and `pmtiles extract` cuts a region.
  BSD plus ODbL, OSM attribution required.

So a map *view* is a solved, near-zero-cost problem **unless** Google's autocomplete is taken, at which point §3.2.3(e) forces Google's map too and the web half starts costing $1,750/month at 300,000 loads.

### 3.7 The rest, briefly

- **Mapbox** - Search Box is per-session, and the session rule is the friendliest of the session-based vendors: a session ends on `/suggest` to `/retrieve`, after 180 seconds of inactivity, or after 50 `/suggest` calls, and *"Each completed session, regardless of how it ends, counts as one billable session"*, so an abandoned search costs one session rather than N requests, which is the opposite of Google's abandonment penalty.
  Free 500 sessions/month, $3.00 per 1,000 in the 501 to 100k band.
  Results *"can not be stored"* on the Temporary tier, and permanent storage needs the pricier Permanent Geocoding API.
- **MapTiler** - per session too, but the cheapest plan bundles only 3,000 search sessions and overage is $2.50 per 1,000, so it scales badly for a search-heavy feature at $772.50/month for 300,000.
  The free plan forbids commercial use.
- **Geoapify** - the best free offer on this list: 3,000 credits/day at 1 credit per request, **commercial use explicitly allowed** with a "Powered by Geoapify" attribution, no card, and limits described as *"soft"* (they email rather than cutting off).
  Two problems, one in the model and one in the terms.
  The caps are **daily**, so a million a month needs the $609 plan sized for the daily peak rather than the monthly total.
  And the terms require attribution (*"When using the Services, you must always provide OpenStreetMap attribution"*, plus *"Geoapify attribution is mandatory when using Free subscription plan"*) but are **silent on caching, on storing results, and on proxying**, and silence is neither permission nor prohibition.
- **LocationIQ** - 5,000 requests/day free with attribution, but **2 requests per second**, which type-ahead will breach with a few concurrent users.
  Paid from $100/month.
- **Radar** - **their pricing page publishes no figures at all.**
  Retrieved in-browser 2026-07-29, the entire page is "Get a quote" and "Volume discounts available".
  Their own marketing blog claims 100,000 free requests/month and $0.50 per 1,000, but that is a blog post rather than a price list, and it could not be confirmed anywhere they would be bound by it.

---

## 4. Explicitly not confirmed

Listed rather than smoothed over, because money may eventually be spent on this.
Each of these is an open question a revisit inherits, not a settled figure.

1. **Server price for the self-host branch.**
   The spec is confirmed (64 GB DDR5 ECC, 2x1.92 TB NVMe, a commodity dedicated box), but the monthly figure does not render on the vendor's public matrix page and was not confirmed.
   Price it against that spec directly rather than carrying a number forward from here.
2. **Mapbox Search Box band prices above 100k sessions/month.**
   The 501 to 100k band is $3.00 per 1,000, so the larger figures in §3.0 are upper bounds.
3. **Apple MapKit JS overage pricing.** Genuinely unpublished - "contact us".
4. **Apple's terms** on caching MapKit results, and on displaying them beside a non-Apple map.
   Both would need verifying before shipping an Apple-on-iOS route.
5. **Stadia Maps' privacy policy.**
   Their `/privacy-policy/` path returned 404 and the documentation FAQ returned 403, so what they log about end users and for how long is unknown.
6. **Caching and proxy terms for MapTiler and LocationIQ.** Not checked, because neither is the recommendation.
7. **Geoapify's position on caching and proxying.**
   Their Terms and Conditions were read and are silent on both.
   That is a checked absence rather than an unchecked question, but it is still an unknown, and it is why Geoapify sits behind Stadia despite being cheaper on day one.
8. **Google §3.2.3(d)(iii) as applied to dogtag.** Genuinely ambiguous, see §2.2(d).
   It is flagged, not relied on, and it should not be resolved from this document.

---

## 5. If this is ever revisited: where the integration goes, and the claim that must change with it

Recorded because both constraints are easy to breach by accident and neither is obvious from the call site.

**Where it goes.**
Not in `packages/ui/src/geo/`, whose header forbids acquiring I/O and forbids turning a position into a query parameter, a request path, a network cache key, or a log line.
Not in `packages/ui/src/directory/` either, whose `read()` deliberately takes no query so there is nowhere for a position to go.
An autocomplete client is a **new sibling module** - `packages/ui/src/placesearch/`, say - that resolves typed text to a `LatLng` and hands it to `geo/` as an already-held position, with native mirrors alongside `NearbyDecision.swift` and `NearbyDecision.kt`.

It should carry the same resolve-do-not-throw and explicit-unavailable discipline `ProviderDirectory.read()` already uses (`found | empty | unavailable`), because a place search that fails silently and a place search that found nothing are different answers to the owner.

That placement keeps both existing boundaries intact and keeps the live-position guarantee untouched, since such a path only ever carries a place the owner typed and chose.

**The claim that must change in the same commit.**
`apps/ios/DogTag/NearbyScreen.swift:361` currently promises *"They are parsed on this phone; DogTag does not geocode or send them anywhere."*, and `apps/android/.../ui/screens/NearbyScreen.kt:460` promises *"They are parsed here and never geocoded or sent anywhere."*

The moment any hosted autocomplete ships, both sentences become false.
This codebase treats a claim made to the owner as load-bearing, so the copy changes in the same commit as the integration, not in a follow-up.
That is the same rule that governs verdict badges and pillar states elsewhere: a surface may not state something the code no longer does.
