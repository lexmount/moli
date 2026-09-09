# Moli's web-audio-api 1.7.0 patch set

Source: the published `web-audio-api` 1.7.0 crate, upstream
<https://github.com/orottier/web-audio-api-rs>, commit
`2af21af6ff2bbee17201b2dd2a5ebc56f55c3511`.

Crate archive SHA-256:
`013903031fb735e7fd9aee7c159f3440a19c6969dc81365aec8c64caddcf39bc`.
The upstream MIT license is retained in `LICENSE`. Source, resources, README
and normalized/original manifests are otherwise copied from that archive.

Local changes:

1. `src/context/concrete_base.rs`: defer the HRTF database to the existing
   control-thread `PannerNode::set_panning_model(HRTF)` path. Context construction
   otherwise eagerly resamples the full database even with the `none` backend
   and no panner. This can exhaust a renderer script deadline in debug builds.
2. `src/param.rs`: apply automation events at the quantum boundary before
   publishing `AudioParam.value` and sampling k-rate. A k-rate boundary step
   updates the scalar buffer; a step later in the quantum remains deferred.
   The existing scalar fast path and k-rate computation are retained.
   A regression exercises a boundary step and another step within the quantum.

No host-audio backend is enabled by Moli. The native integration tests cover
PCM, FFT, automation, descriptors, JS reentrancy and document/context cleanup.
The upstream parameter unit tests should also be run when editing this patch.
